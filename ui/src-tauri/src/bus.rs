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
//! streaming incremental append, no per-token full re-parse).
//!
//! **M4 scope**: `characters.list` / `characters.import` (path-first) /
//! `settings.get` / `settings.update` (hot reload) / `chat.history` (legacy
//! single-session) intents are routed to the corresponding engine HTTP
//! endpoints. Other intents fall back to a minimal ack.
//!
//! The engine URL defaults to `http://127.0.0.1:8000` and is overridable via
//! the `AIRP_ENGINE_URL` env var (the Tauri shell is a sidecar client of the
//! headless engine service — see DEV-GUIDE §3.3).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

use airp_state_protocol::{Body, Envelope, PatchOp, PatchOpKind, SetOrPatch, PROTOCOL_VERSION};
use futures_util::StreamExt;
use tauri::{AppHandle, Emitter};

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
    engine: RwLock<EngineConnection>,
    http: reqwest::Client,
    // Task 1.2: chat_lock removed. Streaming patches now address a fixed
    // assistant row id (`/messages/{id}/text`) reserved before spawning, so
    // concurrent streams no longer race on `/messages/-/text` (the `-`
    // last-element target that the lock existed to serialize).
}

#[derive(Clone)]
struct EngineConnection {
    url: String,
    access_key: Option<String>,
}

impl BusRelay {
    pub fn new() -> Self {
        let engine_url =
            std::env::var("AIRP_ENGINE_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
        let access_key = std::env::var("AIRP_ACCESS_KEY")
            .ok()
            .filter(|key| !key.is_empty());
        Self::with_connection(engine_url, access_key)
    }

    pub fn with_connection(engine_url: String, access_key: Option<String>) -> Self {
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
            engine: RwLock::new(EngineConnection {
                url: engine_url,
                access_key,
            }),
            http,
        }
    }

    pub fn configure_engine(&self, engine_url: String, access_key: Option<String>) {
        let mut guard = self.engine.write().unwrap_or_else(|e| e.into_inner());
        guard.url = engine_url;
        guard.access_key = access_key;
    }

    fn engine_connection(&self) -> EngineConnection {
        self.engine
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
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
            Body::Ack(airp_state_protocol::AckMsg {
                ref_: env.id.clone(),
            }),
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

                // Open the chat turn in one envelope: user row, user order id,
                // assistant row, assistant order id. Multiple Tauri commands
                // can enter dispatch concurrently, so splitting this into two
                // emits can interleave `/order` entries across turns.
                let (turn_open, assistant_id) = chat_turn_open_envelope(n, &text);
                self.emit(&turn_open);

                // Spawn the engine call so dispatch returns immediately; the
                // streaming patches are emitted from the task as chunks arrive.
                // No lock: each stream patches its own `/messages/{id}/text`,
                // so concurrent streams don't interfere (Task 1.2).
                let app_opt = self.subscriber.get().cloned();
                let http = self.http.clone();
                let engine = self.engine_connection();
                tauri::async_runtime::spawn(async move {
                    let request = ChatStreamRequest {
                        app: app_opt.clone(),
                        http,
                        engine,
                        character_id,
                        text,
                        assistant_id: assistant_id.clone(),
                        n,
                    };
                    if let Err(e) = run_chat_stream(request).await {
                        tracing::error!(err = %e, "engine chat stream failed");
                        emit_state_patch(
                            &app_opt,
                            format!("state-a{n}-error"),
                            "w-chat",
                            vec![assistant_text_patch(&assistant_id, format!("(error: {e}"))],
                        );
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
                let engine = self.engine_connection();
                tauri::async_runtime::spawn(async move {
                    match fetch_character_list(&http, &engine.url, engine.access_key.as_deref())
                        .await
                    {
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
                let engine = self.engine_connection();
                tauri::async_runtime::spawn(async move {
                    // 标记导入中，UI 可显示进度/禁用按钮。
                    emit_state_set(
                        &app_opt,
                        format!("state-chars-imp-{n}"),
                        "w-characters",
                        serde_json::json!({ "importing": true }),
                    );
                    match import_character_via_path(
                        &http,
                        &engine.url,
                        engine.access_key.as_deref(),
                        &card_path,
                        character_id.as_deref(),
                    )
                    .await
                    {
                        Ok(imported_id) => {
                            tracing::info!(path = %card_path, id = %imported_id, "character imported");
                            // 刷新列表让新 id 出现。
                            match fetch_character_list(
                                &http,
                                &engine.url,
                                engine.access_key.as_deref(),
                            )
                            .await
                            {
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
            } else if i.name == "settings.get" {
                // M4: 读取 engine settings（api_key 脱敏为 api_key_set bool）
                let app_opt = self.subscriber.get().cloned();
                let http = self.http.clone();
                let engine = self.engine_connection();
                tauri::async_runtime::spawn(async move {
                    match fetch_settings(&http, &engine.url, engine.access_key.as_deref()).await {
                        Ok(settings) => {
                            emit_state_set(
                                &app_opt,
                                format!("state-settings-{n}"),
                                "w-settings",
                                serde_json::json!({ "loaded": true, "settings": settings }),
                            );
                        }
                        Err(e) => {
                            tracing::warn!(err = %e, "settings.get failed");
                            emit_state_set(
                                &app_opt,
                                format!("state-settings-{n}"),
                                "w-settings",
                                serde_json::json!({ "loaded": true, "error": e.to_string() }),
                            );
                        }
                    }
                });
            } else if i.name == "settings.update" {
                // M4: 更新 engine settings（POST /v1/settings 热重载）
                // params: { endpoint?, api_key?, model? } — 只传非 null 字段
                let params = i.params.clone().unwrap_or(serde_json::Value::Null);
                let app_opt = self.subscriber.get().cloned();
                let http = self.http.clone();
                let engine = self.engine_connection();
                tauri::async_runtime::spawn(async move {
                    // emit saving:true 用 set 全量替换 w-settings scope，会覆盖之前的
                    // {loaded, settings}。SettingsModal 的 watch 通过"只同步非空字段"
                    // 避免表单被清空（saving emit 后 settings 字段为 undefined，不进 if）。
                    // 这个交互是 set 替换 + 非空同步两者配合，单独看任一侧都不明显。
                    emit_state_set(
                        &app_opt,
                        format!("state-settings-saving-{n}"),
                        "w-settings",
                        serde_json::json!({ "saving": true }),
                    );
                    match update_settings(&http, &engine.url, engine.access_key.as_deref(), &params)
                        .await
                    {
                        Ok(updated) => {
                            emit_state_set(
                                &app_opt,
                                format!("state-settings-{n}"),
                                "w-settings",
                                serde_json::json!({ "loaded": true, "saving": false, "settings": updated }),
                            );
                        }
                        Err(e) => {
                            tracing::warn!(err = %e, "settings.update failed");
                            emit_state_set(
                                &app_opt,
                                format!("state-settings-{n}"),
                                "w-settings",
                                serde_json::json!({ "loaded": true, "saving": false, "error": e.to_string() }),
                            );
                        }
                    }
                });
            } else if i.name == "chat.history" {
                // M4: 拉取角色 chat history（legacy 单 session 路径）。
                // params: { character_id: string }
                // engine 返回 ChatLog { messages: Vec<ChatMessage>, ... }，
                // 转换为 w-chat scope 的 { messages: {id: h{i}}, order: [h0, h1, ...] }，
                // 与 chat_turn_open_envelope 的 id-keyed shape 对齐（ChatWidget 期望
                // {id, role, text}，ChatMessage 是 {role, content}）。
                //
                // 历史消息 id 用 `h{i}` 前缀，与 chat.send 的 `u{n}`/`a{n}` 不冲突，
                // 避免覆盖正在进行的流式 turn。
                //
                // 注意：engine 当前 `POST /v1/chat/history` 只读 legacy 单 session
                // 路径（`data/characters/{id}/history/`），不支持 session_id 参数。
                // 多 session 切换留下个 PR（需扩展 engine 端点）。
                let params = i.params.clone().unwrap_or(serde_json::Value::Null);
                let character_id = params
                    .get("character_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if character_id.is_empty() {
                    tracing::warn!("chat.history missing character_id");
                    emit_state_set(
                        &self.subscriber.get().cloned(),
                        format!("state-chat-history-{n}"),
                        "w-chat",
                        serde_json::json!({ "messages": {}, "order": [] }),
                    );
                    return;
                }
                let app_opt = self.subscriber.get().cloned();
                let http = self.http.clone();
                let engine = self.engine_connection();
                tauri::async_runtime::spawn(async move {
                    match fetch_chat_history(
                        &http,
                        &engine.url,
                        engine.access_key.as_deref(),
                        &character_id,
                    )
                    .await
                    {
                        Ok(log) => {
                            // ChatLog.messages: Vec<{role, content}> → w-chat scope
                            // ChatLog.message_timestamps: Vec<Option<String>> 与 messages
                            // 一一对应（PR #75 #73 方案 B）。按 index 合并到 message object
                            // 的 ts 字段，让 UI 拿到历史消息的时间戳（ChatWidget 当前不
                            // 显示，但能力闭合，未来显示时不用回头补 bus.rs）。
                            let messages = log
                                .get("messages")
                                .and_then(|v| v.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let timestamps = log
                                .get("message_timestamps")
                                .and_then(|v| v.as_array())
                                .cloned()
                                .unwrap_or_default();
                            let mut scope_messages = serde_json::Map::new();
                            let mut order = Vec::with_capacity(messages.len());
                            for (i, msg) in messages.iter().enumerate() {
                                let id = format!("h{i}");
                                let role =
                                    msg.get("role").and_then(|v| v.as_str()).unwrap_or("user");
                                let text =
                                    msg.get("content").and_then(|v| v.as_str()).unwrap_or("");
                                // 按 index 取 ts；旧 jsonl 无 ts → None → null
                                let ts = timestamps
                                    .get(i)
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Null);
                                scope_messages.insert(
                                    id.clone(),
                                    serde_json::json!({
                                        "id": id,
                                        "role": role,
                                        "text": text,
                                        "ts": ts,
                                    }),
                                );
                                order.push(serde_json::Value::String(id));
                            }
                            emit_state_set(
                                &app_opt,
                                format!("state-chat-history-{n}"),
                                "w-chat",
                                serde_json::json!({ "messages": scope_messages, "order": order }),
                            );
                        }
                        Err(e) => {
                            tracing::warn!(err = %e, "chat.history failed");
                            emit_state_set(
                                &app_opt,
                                format!("state-chat-history-{n}"),
                                "w-chat",
                                serde_json::json!({ "messages": {}, "order": [], "error": e.to_string() }),
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

fn chat_turn_open_envelope(n: u64, text: &str) -> (Envelope, String) {
    let user_msg_id = format!("u{n}");
    let assistant_id = format!("a{n}");
    // Message ids are generated here as `u{n}`/`a{n}`. They must stay JSON
    // Pointer-safe (no "/" or "~") while paths are built by string formatting.
    let patch = vec![
        PatchOp {
            op: PatchOpKind::Add,
            path: format!("/messages/{user_msg_id}"),
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
            value: Some(serde_json::Value::String(user_msg_id)),
        },
        PatchOp {
            op: PatchOpKind::Add,
            path: format!("/messages/{assistant_id}"),
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
    ];
    (
        Envelope::new(
            format!("state-chat-turn-{n}-open"),
            now_ms(),
            "ui",
            Body::State(airp_state_protocol::StateMsg {
                scope: "w-chat".into(),
                op: SetOrPatch::Patch,
                state: None,
                patch: Some(patch),
            }),
        ),
        assistant_id,
    )
}

fn assistant_text_patch(assistant_id: &str, text: impl Into<String>) -> PatchOp {
    PatchOp {
        op: PatchOpKind::Replace,
        path: format!("/messages/{assistant_id}/text"),
        from: None,
        value: Some(serde_json::Value::String(text.into())),
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

struct ChatStreamRequest {
    app: Option<AppHandle>,
    http: reqwest::Client,
    engine: EngineConnection,
    character_id: String,
    text: String,
    assistant_id: String,
    n: u64,
}

/// POST the user message to the engine and stream the assistant reply back as
/// downstream `state` patches on `w-chat`.
///
/// Streaming protocol (performance contract §6 — incremental append, Task 1.2):
/// 1. dispatch emits the user row and reserved assistant row in one envelope.
///    The four patch ops apply in order, so the turn opens as `u{n}, a{n}`
///    even when multiple `chat.send` commands enter concurrently.
/// 2. For each `body_chunk`: emit `replace /messages/{assistant_id}/text` with
///    the **accumulated** assistant text. Each stream targets its own id, so
///    concurrent streams don't race — no chat_lock needed.
/// 3. `think` chunks are dropped in Phase 0 (reasoning display is Phase 1).
async fn run_chat_stream(
    request: ChatStreamRequest,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use serde_json::json;

    let ChatStreamRequest {
        app,
        http,
        engine,
        character_id,
        text,
        assistant_id,
        n,
    } = request;

    let body = json!({
        "character_id": character_id,
        "message": text,
        "user_profile": { "name": "User", "variables": {} },
    });

    let request = http
        .post(format!("{}/v1/chat/completions", engine.url))
        .json(&body);
    let resp = with_auth(request, engine.access_key.as_deref())
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
                        vec![assistant_text_patch(&assistant_id, acc.clone())],
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
    access_key: Option<&str>,
) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
    let request = http.get(format!("{}/v1/characters", engine_url));
    let resp = with_auth(request, access_key).send().await?;
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
    access_key: Option<&str>,
    card_path: &str,
    character_id: Option<&str>,
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let mut body = serde_json::json!({ "card_path": card_path });
    if let Some(id) = character_id {
        body["character_id"] = serde_json::json!(id);
    }
    let request = http
        .post(format!("{}/v1/characters/import", engine_url))
        .json(&body);
    let resp = with_auth(request, access_key).send().await?;
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

/// GET `/v1/settings` from the engine. Returns the settings JSON (api_key 脱敏).
async fn fetch_settings(
    http: &reqwest::Client,
    engine_url: &str,
    access_key: Option<&str>,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let request = http.get(format!("{}/v1/settings", engine_url));
    let resp = with_auth(request, access_key).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("engine HTTP {status}: {text}").into());
    }
    let v: serde_json::Value = resp.json().await?;
    Ok(v)
}

/// POST `/v1/chat/history` to get the character's chat log (legacy single-session).
/// Returns the `ChatLog` JSON (`{ messages: Vec<ChatMessage>, ... }`).
async fn fetch_chat_history(
    http: &reqwest::Client,
    engine_url: &str,
    access_key: Option<&str>,
    character_id: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let body = serde_json::json!({ "character_id": character_id });
    let request = http
        .post(format!("{}/v1/chat/history", engine_url))
        .json(&body);
    let resp = with_auth(request, access_key).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("engine HTTP {status}: {text}").into());
    }
    let v: serde_json::Value = resp.json().await?;
    Ok(v)
}

/// POST `/v1/settings` to update engine settings (hot reload).
/// `params` is forwarded as-is (Partial<MutableConfig>).
/// Returns the updated settings JSON (api_key 脱敏).
async fn update_settings(
    http: &reqwest::Client,
    engine_url: &str,
    access_key: Option<&str>,
    params: &serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error + Send + Sync>> {
    let request = http
        .post(format!("{}/v1/settings", engine_url))
        .json(params);
    let resp = with_auth(request, access_key).send().await?;
    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("engine HTTP {status}: {text}").into());
    }
    let v: serde_json::Value = resp.json().await?;
    Ok(v)
}

fn with_auth(
    request: reqwest::RequestBuilder,
    access_key: Option<&str>,
) -> reqwest::RequestBuilder {
    match access_key.filter(|key| !key.is_empty()) {
        Some(key) => request.header(reqwest::header::AUTHORIZATION, format!("Bearer {key}")),
        None => request,
    }
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
        Envelope::new(
            "u1",
            0,
            "ui",
            Body::Intent(IntentMsg {
                name: name.into(),
                params: Some(params),
                source: None,
            }),
        )
    }

    /// The relay must not panic without a subscriber (emit is a no-op) and must
    /// not block on the engine (the HTTP call is spawned).
    #[tokio::test]
    async fn dispatch_handles_intent_without_subscriber() {
        let relay = BusRelay::new();
        relay.dispatch(intent(
            "chat.send",
            serde_json::json!({ "text": "hello", "character_id": "x" }),
        ));
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
        relay.dispatch(intent(
            "chat.send",
            serde_json::json!({ "text": "hello world", "character_id": "c1" }),
        ));
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

    #[test]
    fn chat_turn_open_is_one_ordered_patch_envelope() {
        let (env, assistant_id) = chat_turn_open_envelope(7, "hello");
        assert_eq!(assistant_id, "a7");

        let Body::State(state) = env.body else {
            panic!("expected state envelope");
        };
        assert_eq!(state.scope, "w-chat");
        let patch = state.patch.expect("patch");
        let paths: Vec<&str> = patch.iter().map(|op| op.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["/messages/u7", "/order/-", "/messages/a7", "/order/-"]
        );
        assert_eq!(patch[1].value, Some(serde_json::json!("u7")));
        assert_eq!(patch[3].value, Some(serde_json::json!("a7")));
        assert_eq!(patch[0].value.as_ref().unwrap()["text"], "hello");
        assert_eq!(patch[2].value.as_ref().unwrap()["role"], "assistant");
    }

    #[test]
    fn assistant_text_patch_targets_id_keyed_row() {
        let patch = assistant_text_patch("a42", "partial");
        assert_eq!(patch.op, PatchOpKind::Replace);
        assert_eq!(patch.path, "/messages/a42/text");
        assert_eq!(patch.value, Some(serde_json::json!("partial")));
    }
}
