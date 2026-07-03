//! Tauri-side State Protocol bridge — Phase 0 live link to the AIRP engine.
//!
//! Wires the UI's upstream envelopes (received via the `airp_dispatch` command)
//! into HTTP calls against the AIRP engine (`engine/`, the `airp-core` daemon),
//! and emits downstream envelopes back to the webview on the `airp:envelope`
//! event.
//!
//! **Phase 0 scope**: `chat.send` intents are routed to the engine's
//! `POST /v1/chat/completions` SSE endpoint. The streaming response is consumed
//! here and re-emitted as downstream `state` patches so the UI's `w-chat` scope
//! accumulates the assistant reply token-by-token (performance contract §6:
//! streaming incremental append, no per-token full re-parse). Other intents
//! fall back to a minimal ack until later phases wire them.
//!
//! The engine URL defaults to `http://127.0.0.1:8000` and is overridable via
//! the `AIRP_ENGINE_URL` env var (the Tauri shell is a sidecar client of the
//! headless engine service — see DEV-GUIDE §3.3).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};
use airp_state_protocol::{Body, Envelope, PatchOp, PatchOpKind, SetOrPatch, PROTOCOL_VERSION};

/// Tauri event name carrying a downstream envelope to the webview.
pub const ENVELOPE_EVENT: &str = "airp:envelope";

/// Default engine HTTP endpoint (the `airp-core` daemon on localhost).
const DEFAULT_ENGINE_URL: &str = "http://127.0.0.1:8000";

/// In-process relay bridging the UI's State-Protocol envelopes to the AIRP
/// engine over HTTP/SSE.
///
/// Holds a single downstream subscriber (the webview's `TauriBus`) and a shared
/// HTTP client + engine URL. `subscribe_downstream` is called once from `setup`;
/// `dispatch` is called per `airp_dispatch` command. The subscriber slot is a
/// `OnceLock` (set once at startup, read on every dispatch) and the sequence
/// counter is an `AtomicU64` — both lock-free, since `dispatch` is the hot path
/// and never contends on the subscriber.
pub struct BusRelay {
    subscriber: OnceLock<AppHandle>,
    seq: AtomicU64,
    engine_url: String,
    http: reqwest::Client,
    // Task 1.2: chat_lock removed. Streaming patches now address a fixed
    // assistant row index (`/messages/{idx}/text`) reserved synchronously in
    // dispatch before spawning, so concurrent streams no longer race on
    // `/messages/-/text` (the `-` last-element target that the lock existed
    // to serialize). See DEV-GUIDE Task 1.2.
}

impl BusRelay {
    pub fn new() -> Self {
        let engine_url = std::env::var("AIRP_ENGINE_URL")
            .unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
        // Bounded HTTP client: connect + request timeouts prevent spawned
        // tasks hanging forever if the engine stalls (CodeRabbit finding).
        let http = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(5))
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            subscriber: OnceLock::new(),
            seq: AtomicU64::new(0),
            engine_url,
            http,
        }
    }

    /// Register the webview as the downstream sink. Called once from `setup`.
    /// Subsequent calls are ignored (the first registration wins), matching the
    /// "set once" semantics of `OnceLock`.
    pub fn subscribe_downstream(&self, app: AppHandle) {
        let _ = self.subscriber.set(app);
    }

    /// Receive an upstream envelope from the UI.
    ///
    /// For `chat.send` intents: spawn an async task that POSTs to the engine's
    /// `/v1/chat/completions`, consumes the SSE stream, and emits downstream
    /// `state` patches so the UI sees the assistant reply stream in. A short
    /// `ack` is emitted first so the UI can mark the user message as sent.
    ///
    /// For any other intent: emit a minimal ack (later phases wire list/load/etc).
    pub fn dispatch(&self, env: Envelope) {
        let n = self.seq.fetch_add(1, Ordering::Relaxed) + 1;

        // Always ack so the UI knows the envelope was received.
        self.emit(&Envelope::new(
            format!("ack-{n}"),
            now_ms(),
            "gateway",
            Body::Ack(airp_state_protocol::AckMsg { ref_: env.id.clone() }),
        ));

        if let Body::Intent(i) = &env.body {
            if i.name == "chat.send" {
                // params shape (Phase 0): { character_id: string, text: string }
                let params = i.params.clone().unwrap_or(serde_json::Value::Null);
                let character_id = params
                    .get("character_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = params
                    .get("text")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                // Mirror the user's message into the chat scope immediately so
                // the UI shows it before the engine responds (the engine also
                // persists it, but this avoids a round-trip for the user echo).
                //
                // Task 1.2: messages is an id-keyed map + a parallel `order`
                // array of ids. We add by id (`/messages/{id}`) and append the
                // id to `/order/-`. This lets concurrent streams each patch
                // their own row (`/messages/{assistant_id}/text`) without racing
                // on `/messages/-/text`'s "last element" resolution — so the
                // chat_lock is gone. See DEV-GUIDE Task 1.2.
                let user_msg_id = format!("u{n}");
                let assistant_id = format!("a{n}");
                let user_echo = Envelope::new(
                    format!("state-u{n}"),
                    now_ms(),
                    "ui",
                    Body::State(airp_state_protocol::StateMsg {
                        scope: "w-chat".into(),
                        op: SetOrPatch::Patch,
                        state: None,
                        patch: Some(vec![
                            PatchOp {
                                op: PatchOpKind::Add,
                                path: format!("/messages/{}", user_msg_id),
                                from: None,
                                value: Some(serde_json::json!({
                                    "id": user_msg_id,
                                    "role": "user",
                                    "text": text,
                                })),
                            },
                            PatchOp {
                                op: PatchOpKind::Add,
                                path: "/order/-".into(),
                                from: None,
                                value: Some(serde_json::Value::String(user_msg_id.clone())),
                            },
                        ]),
                    }),
                );
                self.emit(&user_echo);

                // Reserve the assistant row synchronously (empty text) so its
                // id exists in the map before the streamed chunks start
                // replacing its text. Done in dispatch (serial) so the add
                // ordering vs the user echo is deterministic; the stream task
                // only ever replaces `/messages/{assistant_id}/text`.
                let assistant_open = Envelope::new(
                    format!("state-a{n}-open"),
                    now_ms(),
                    "ui",
                    Body::State(airp_state_protocol::StateMsg {
                        scope: "w-chat".into(),
                        op: SetOrPatch::Patch,
                        state: None,
                        patch: Some(vec![
                            PatchOp {
                                op: PatchOpKind::Add,
                                path: format!("/messages/{}", assistant_id),
                                from: None,
                                value: Some(serde_json::json!({
                                    "id": assistant_id,
                                    "role": "assistant",
                                    "text": "",
                                })),
                            },
                            PatchOp {
                                op: PatchOpKind::Add,
                                path: "/order/-".into(),
                                from: None,
                                value: Some(serde_json::Value::String(assistant_id.clone())),
                            },
                        ]),
                    }),
                );
                self.emit(&assistant_open);

                // Spawn the engine call so dispatch returns immediately; the
                // streaming patches are emitted from the task as chunks arrive.
                // No lock: each stream patches its own `/messages/{id}/text`,
                // so concurrent streams don't interfere (Task 1.2).
                let app_opt = self.subscriber.get().cloned();
                let http = self.http.clone();
                let engine_url = self.engine_url.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = run_chat_stream(
                        app_opt.clone(), http, &engine_url, &character_id, &text, &assistant_id, n,
                    )
                    .await
                    {
                        tracing::error!(err = %e, "engine chat stream failed");
                        if let Some(app) = app_opt {
                            let err_env = Envelope::new(
                                format!("err-{n}"),
                                now_ms(),
                                "gateway",
                                Body::Error(airp_state_protocol::ErrorMsg {
                                    code: "engine_error".into(),
                                    message: e.to_string(),
                                    detail: None,
                                }),
                            );
                            let _ = app.emit(ENVELOPE_EVENT, &err_env);
                        }
                    }
                });
            } else if i.name == "characters.list" {
                // Phase 0: fetch the engine's character list and push it into
                // the `w-characters` scope as a set, so the UI can render a
                // picker. The engine returns `Vec<String>` of character ids.
                let app_opt = self.subscriber.get().cloned();
                let http = self.http.clone();
                let engine_url = self.engine_url.clone();
                tauri::async_runtime::spawn(async move {
                    match fetch_character_list(&http, &engine_url).await {
                        Ok(ids) => {
                            emit_state_set(
                                &app_opt,
                                format!("state-chars-{n}"),
                                "w-characters",
                                serde_json::json!({ "ids": ids, "loaded": true }),
                            );
                        }
                        Err(e) => {
                            tracing::warn!(err = %e, "characters.list failed");
                            emit_state_set(
                                &app_opt,
                                format!("state-chars-{n}"),
                                "w-characters",
                                serde_json::json!({ "ids": [], "loaded": true, "error": e.to_string() }),
                            );
                        }
                    }
                });
            } else if i.name == "characters.import" {
                // Task 1.1: import a character card, path-first (守不变式6).
                // params: { card_path: string (绝对路径), character_id?: string }
                // 只传路径这个几十字节小串——引擎读盘+解析；绝不把 base64 大 blob 塞进
                // intent/store。导入是转瞬即转发引擎的请求，不 setState 存 blob。
                let params = i.params.clone().unwrap_or(serde_json::Value::Null);
                let card_path = params
                    .get("card_path")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let character_id = params
                    .get("character_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if card_path.is_empty() {
                    tracing::warn!("characters.import missing card_path");
                    emit_state_set(
                        &self.subscriber.get().cloned(),
                        format!("state-chars-{n}"),
                        "w-characters",
                        serde_json::json!({ "loaded": true, "importing": false, "error": "缺少 card_path" }),
                    );
                    return;
                }
                let app_opt = self.subscriber.get().cloned();
                let http = self.http.clone();
                let engine_url = self.engine_url.clone();
                tauri::async_runtime::spawn(async move {
                    // 标记导入中，UI 可显示进度/禁用按钮。
                    emit_state_set(
                        &app_opt,
                        format!("state-chars-imp-{n}"),
                        "w-characters",
                        serde_json::json!({ "importing": true }),
                    );
                    match import_character_via_path(
                        &http, &engine_url, &card_path, character_id.as_deref(),
                    )
                    .await
                    {
                        Ok(imported_id) => {
                            tracing::info!(path = %card_path, id = %imported_id, "character imported");
                            // 刷新列表让新 id 出现。
                            match fetch_character_list(&http, &engine_url).await {
                                Ok(ids) => emit_state_set(
                                    &app_opt,
                                    format!("state-chars-{n}"),
                                    "w-characters",
                                    serde_json::json!({ "ids": ids, "loaded": true, "importing": false, "last_imported": imported_id }),
                                ),
                                Err(e) => {
                                    tracing::warn!(err = %e, "post-import list refresh failed");
                                    emit_state_set(
                                        &app_opt,
                                        format!("state-chars-{n}"),
                                        "w-characters",
                                        serde_json::json!({ "loaded": true, "importing": false, "error": e.to_string() }),
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!(err = %e, "characters.import failed");
                            emit_state_set(
                                &app_opt,
                                format!("state-chars-{n}"),
                                "w-characters",
                                serde_json::json!({ "loaded": true, "importing": false, "error": e.to_string() }),
                            );
                        }
                    }
                });
            }
        }
    }

    fn emit(&self, env: &Envelope) {
        if let Some(app) = self.subscriber.get() {
            // Best-effort: a closed webview surfaces on next dispatch, not here.
            let _ = app.emit(ENVELOPE_EVENT, env);
        }
    }
}

impl Default for BusRelay {
    fn default() -> Self {
        Self::new()
    }
}

/// Shape of a single SSE `message` event data emitted by the engine's
/// `/v1/chat/completions` (see `chat_pipeline::build_sse_stream` →
/// `UnpackedChunk`: `#[serde(tag="type", content="text")]` with per-variant
/// renames `body_chunk` / `think_chunk`).
#[derive(Debug, serde::Deserialize)]
#[serde(tag = "type", content = "text")]
enum EngineChunk {
    #[serde(rename = "body_chunk")]
    Body(String),
    // `think_chunk` and `action_options` are parsed so malformed frames don't
    // abort the stream, but their content is not rendered until Phase 1
    // (reasoning display + action buttons). Silence the dead-field warning.
    #[serde(rename = "think_chunk")]
    #[allow(dead_code)]
    Think(String),
    #[serde(rename = "action_options")]
    #[allow(dead_code)]
    ActionOptions { options: Vec<String> },
}

/// POST the user message to the engine and stream the assistant reply back as
/// downstream `state` patches on `w-chat`.
///
/// Streaming protocol (performance contract §6 — incremental append, Task 1.2):
/// 1. dispatch **reserves** the assistant row synchronously before spawning:
///    `add /messages/{assistant_id}` (empty text) + `add /order/-` (its id).
///    Because dispatch is serial, the add ordering vs the user echo is
///    deterministic; the spawned stream task only ever *replaces* text.
/// 2. For each `body_chunk`: emit `replace /messages/{assistant_id}/text` with
///    the **accumulated** assistant text. Each stream targets its own id, so
///    concurrent streams don't race — no chat_lock needed.
/// 3. `think` chunks are dropped in Phase 0 (reasoning display is Phase 1).
async fn run_chat_stream(
    app: Option<AppHandle>,
    http: reqwest::Client,
    engine_url: &str,
    character_id: &str,
    text: &str,
    assistant_id: &str,
    n: u64,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use serde_json::json;

    let body = json!({
        "character_id": character_id,
        "message": text,
        "user_profile": { "name": "User", "variables": {} },
    });

    let resp = http
        .post(format!("{}/v1/chat/completions", engine_url))
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("engine HTTP {status}: {text}").into());
    }

    // axum SSE: stream of `Result<Event, Infallible>`. Each `event: message`
    // carries a JSON `EngineChunk`; `event: error` carries an error JSON.
    //
    // Buffer raw **bytes**, not a String: a multi-byte UTF-8 char (CJK is 3
    // bytes — the common case for RP) can be split across two network chunks.
    // Decoding each chunk eagerly with `from_utf8` would fail on the split and
    // drop data (garbled/missing characters). We only decode at `\n\n` frame
    // boundaries, where the bytes form a complete (valid-UTF-8) SSE frame.
    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    let mut acc = String::new();
    // Per-chunk counter so every emitted envelope has a unique id (the protocol
    // requires unique message ids; reusing `state-a{n}-body` for every streamed
    // chunk violated that and would break any downstream ack/dedup).
    let mut chunk_seq: u64 = 0;
    // The assistant row was already reserved (empty) by dispatch before
    // spawning, so we only ever replace its text — no `assistant_started` /
    // `/messages/-` add here. Concurrent streams each target their own
    // `/messages/{assistant_id}/text` and don't interfere (Task 1.2).
    let assistant_text_path = format!("/messages/{}/text", assistant_id);

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        buf.extend_from_slice(&bytes);

        // SSE frames are separated by blank lines. Process all complete frames.
        while let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
            let frame_bytes: Vec<u8> = buf.drain(..pos + 2).collect();
            let frame = String::from_utf8_lossy(&frame_bytes);
            // Each frame is `event: <name>\ndata: <json>`. Parse lines.
            let mut event_name = "message".to_string();
            let mut data_line = String::new();
            for line in frame.lines() {
                if let Some(rest) = line.strip_prefix("event:") {
                    event_name = rest.trim().to_string();
                } else if let Some(rest) = line.strip_prefix("data:") {
                    data_line.push_str(rest.trim_start_matches(' '));
                }
            }
            if data_line.is_empty() {
                continue;
            }

            if event_name == "error" {
                return Err(format!("engine stream error: {data_line}").into());
            }
            if event_name != "message" {
                continue;
            }

            let chunk: EngineChunk = match serde_json::from_str(&data_line) {
                Ok(c) => c,
                // Skip malformed frames rather than aborting the whole stream.
                Err(_) => continue,
            };

            match chunk {
                EngineChunk::Think(_) => continue, // Phase 1: reasoning display.
                EngineChunk::Body(piece) => {
                    acc.push_str(&piece);
                    chunk_seq += 1;
                    emit_state_patch(
                        &app,
                        format!("state-a{n}-body-{chunk_seq}"),
                        "w-chat",
                        vec![PatchOp {
                            op: PatchOpKind::Replace,
                            path: assistant_text_path.clone(),
                            from: None,
                            value: Some(serde_json::Value::String(acc.clone())),
                        }],
                    );
                }
                EngineChunk::ActionOptions { options: _ } => continue, // Phase 1: action UI.
            }
        }
    }

    // If the engine produced no body chunks at all, the assistant row reserved
    // by dispatch already stands as a (degenerate) empty turn boundary — no
    // extra emit needed.

    Ok(())
}

fn emit_state_patch(app: &Option<AppHandle>, id: String, scope: &str, patch: Vec<PatchOp>) {
    let Some(app) = app.as_ref() else { return };
    let env = Envelope::new(
        id,
        now_ms(),
        "agent:narrator",
        Body::State(airp_state_protocol::StateMsg {
            scope: scope.into(),
            op: SetOrPatch::Patch,
            state: None,
            patch: Some(patch),
        }),
    );
    let _ = app.emit(ENVELOPE_EVENT, &env);
}

/// Emit a downstream `state set` envelope (full scope replacement).
fn emit_state_set(app: &Option<AppHandle>, id: String, scope: &str, state: serde_json::Value) {
    let Some(app) = app.as_ref() else { return };
    let env = Envelope::new(
        id,
        now_ms(),
        "gateway",
        Body::State(airp_state_protocol::StateMsg {
            scope: scope.into(),
            op: SetOrPatch::Set,
            state: Some(state),
            patch: None,
        }),
    );
    let _ = app.emit(ENVELOPE_EVENT, &env);
}

/// GET `/v1/characters` from the engine. Returns the list of character ids.
async fn fetch_character_list(
    http: &reqwest::Client,
    engine_url: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let resp = http
        .get(format!("{}/v1/characters", engine_url))
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("engine HTTP {status}: {text}").into());
    }
    let ids: Vec<String> = resp.json().await?;
    Ok(ids)
}

/// POST `/v1/characters/import` with a **path** (path-first, 守不变式6).
/// `character_id` 为 None 时引擎从卡内 name slugify 派生。返回引擎落盘的最终 id。
/// 不接收/不返回大 blob——只传路径字符串。
async fn import_character_via_path(
    http: &reqwest::Client,
    engine_url: &str,
    card_path: &str,
    character_id: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut body = serde_json::json!({ "card_path": card_path });
    if let Some(id) = character_id {
        body["character_id"] = serde_json::json!(id);
    }
    let resp = http
        .post(format!("{}/v1/characters/import", engine_url))
        .json(&body)
        .send()
        .await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("engine HTTP {status}: {text}").into());
    }
    // 引擎返回 { character_id, card_format }。
    let v: serde_json::Value = resp.json().await?;
    let id = v
        .get("character_id")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    Ok(id)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// `airp_dispatch` command — the UI calls this with an upstream envelope.
/// Wire shape mirrors `src/protocol/tauri-bus.ts`: `invoke("airp_dispatch", { env })`.
#[tauri::command]
pub fn airp_dispatch(relay: tauri::State<'_, BusRelay>, env: Envelope) -> Result<(), String> {
    // Validate the envelope version so a malformed/foreign payload is rejected
    // at the boundary rather than processed. The body shape is already
    // enforced by serde deserialization.
    if env.v != PROTOCOL_VERSION {
        return Err(format!("unsupported protocol version: {}", env.v));
    }
    relay.dispatch(env);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use airp_state_protocol::IntentMsg;

    fn intent(name: &str, params: serde_json::Value) -> Envelope {
        Envelope::new("u1", 0, "ui", Body::Intent(IntentMsg {
            name: name.into(),
            params: Some(params),
            source: None,
        }))
    }

    /// The relay must not panic without a subscriber (emit is a no-op) and must
    /// not block on the engine (the HTTP call is spawned).
    #[tokio::test]
    async fn dispatch_handles_intent_without_subscriber() {
        let relay = BusRelay::new();
        relay.dispatch(intent("chat.send", serde_json::json!({ "text": "hello", "character_id": "x" })));
        relay.dispatch(intent("status.toggle", serde_json::json!({})));
    }

    #[tokio::test]
    async fn relay_increments_seq_per_dispatch() {
        let relay = BusRelay::new();
        let start = relay.seq.load(Ordering::Relaxed);
        relay.dispatch(intent("status.toggle", serde_json::json!({})));
        relay.dispatch(intent("status.toggle", serde_json::json!({})));
        assert_eq!(relay.seq.load(Ordering::Relaxed), start + 2);
    }

    /// chat.send must read `text` from the object params (regression guard for
    /// the old bug where the whole params value was treated as a string).
    #[tokio::test]
    async fn chat_send_reads_text_field_from_object_params() {
        let relay = BusRelay::new();
        relay.dispatch(intent("chat.send", serde_json::json!({ "text": "hello world", "character_id": "c1" })));
        relay.dispatch(intent("chat.send", serde_json::json!({})));
        // Yield so spawned tasks (which hit no subscriber → no-op) settle.
        tokio::task::yield_now().await;
    }

    /// EngineChunk deserialization matches the engine's SSE wire shape.
    #[test]
    fn engine_chunk_body_deserializes() {
        let s = r#"{"type":"body_chunk","text":"hi"}"#;
        let c: EngineChunk = serde_json::from_str(s).unwrap();
        match c {
            EngineChunk::Body(t) => assert_eq!(t, "hi"),
            _ => panic!("expected Body"),
        }
    }

    #[test]
    fn engine_chunk_think_deserializes() {
        let s = r#"{"type":"think_chunk","text":"pondering"}"#;
        let c: EngineChunk = serde_json::from_str(s).unwrap();
        match c {
            EngineChunk::Think(t) => assert_eq!(t, "pondering"),
            _ => panic!("expected Think"),
        }
    }
}
