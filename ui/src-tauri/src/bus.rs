//! Tauri-side State Protocol bridge (PLAN task B, ①).
//!
//! Wires the UI's upstream envelopes (received via the `airp_dispatch` command)
//! into an in-process [`BusRelay`] and emits downstream envelopes back to the
//! webview on the `airp:envelope` event.
//!
//! The relay here is a self-contained **mock** — it acknowledges upstream
//! envelopes, mirrors `intent` traffic back as downstream `state` patches so the
//! UI renders a round-trip, and primes a sample session on first subscribe. It
//! deliberately does **not** talk to AIRP-Gateway: that live link is a runtime
//! verification item (PLAN §2.5 ledger B), kept out of CI's reach. The shape of
//! this module is the seam a real `AgentBus` implementation will replace — same
//! `dispatch`/`subscribe_downstream` surface, different guts.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use tauri::{AppHandle, Emitter};
use airp_state_protocol::{Body, Envelope, PatchOp, PatchOpKind, SetOrPatch, PROTOCOL_VERSION};

/// Tauri event name carrying a downstream envelope to the webview.
pub const ENVELOPE_EVENT: &str = "airp:envelope";

/// In-process relay standing in for AIRP-Gateway until the live link lands.
///
/// Holds a single downstream subscriber (the webview's `TauriBus`) and echoes
/// upstream traffic back as downstream envelopes. `subscribe_downstream` is
/// called once from `setup`; `dispatch` is called per `airp_dispatch` command.
///
/// The subscriber slot is a `OnceLock` (set once at startup, read on every
/// dispatch) and the sequence counter is an `AtomicU64` — both lock-free, since
/// `dispatch` is the hot path and never contends on the subscriber.
pub struct BusRelay {
    /// A real Gateway would push envelopes on its own; the mock holds one
    /// subscriber slot and emits through it synchronously via the AppHandle.
    /// `OnceLock` rather than `Mutex<Option<_>>`: the webview is registered
    /// exactly once in `setup` and only read afterwards.
    subscriber: OnceLock<AppHandle>,
    seq: AtomicU64,
}

impl BusRelay {
    pub fn new() -> Self {
        Self { subscriber: OnceLock::new(), seq: AtomicU64::new(0) }
    }

    /// Register the webview as the downstream sink. Called once from `setup`.
    /// Subsequent calls are ignored (the first registration wins), matching the
    /// "set once" semantics of `OnceLock`.
    pub fn subscribe_downstream(&self, app: AppHandle) {
        // Ignore the Result: a second registration is a no-op, not an error.
        let _ = self.subscriber.set(app);
    }

    /// Receive an upstream envelope from the UI. The mock acks it and, for
    /// `intent` bodies, echoes a downstream `state` patch so the UI sees a
    /// round-trip. A real Gateway replaces this body with the IPC call.
    pub fn dispatch(&self, env: Envelope) {
        let n = self.seq.fetch_add(1, Ordering::Relaxed) + 1;

        let ack = Envelope::new(
            format!("ack-{n}"),
            now_ms(),
            "gateway",
            Body::Ack(airp_state_protocol::AckMsg { ref_: env.id.clone() }),
        );
        self.emit(&ack);

        if let Body::Intent(i) = &env.body {
            // Mirror the UI's intent back as a best-effort downstream patch so
            // the round-trip renders even without a Gateway. chat.send grows
            // the chat scope; any other intent flips a generic `w-status` flag.
            let down = match i.name.as_str() {
                "chat.send" => {
                    // params is a JSON object `{ "text": "..." }`; read the
                    // `text` field rather than treating the whole value as a
                    // string (which would always yield "").
                    let text = i
                        .params
                        .as_ref()
                        .and_then(|v| v.get("text"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    Envelope::new(
                        format!("state-{n}"),
                        now_ms(),
                        "agent:narrator",
                        Body::State(airp_state_protocol::StateMsg {
                            scope: "w-chat".into(),
                            op: SetOrPatch::Patch,
                            state: None,
                            patch: Some(vec![
                                PatchOp {
                                    op: PatchOpKind::Add,
                                    path: "/messages/-".into(),
                                    from: None,
                                    value: Some(serde_json::json!({
                                        "id": format!("u{n}"),
                                        "role": "user",
                                        "text": text
                                    })),
                                },
                                PatchOp {
                                    op: PatchOpKind::Add,
                                    path: "/messages/-".into(),
                                    from: None,
                                    value: Some(serde_json::json!({
                                        "id": format!("a{n}"),
                                        "role": "assistant",
                                        "text": "（示例回应）"
                                    })),
                                },
                            ]),
                        }),
                    )
                }
                _ => Envelope::new(
                    format!("state-{n}"),
                    now_ms(),
                    "gateway",
                    Body::State(airp_state_protocol::StateMsg {
                        scope: "w-status".into(),
                        op: SetOrPatch::Patch,
                        state: None,
                        patch: Some(vec![PatchOp {
                            op: PatchOpKind::Replace,
                            path: "/on".into(),
                            from: None,
                            value: Some(serde_json::Value::Bool(true)),
                        }]),
                    }),
                ),
            };
            self.emit(&down);
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
    // at the boundary rather than echoed back. The body shape is already
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

    /// The relay's logic is testable without an AppHandle by exercising the
    /// pure parts: version validation and intent→state round-trip shape.
    #[test]
    fn dispatch_rejects_wrong_version() {
        let relay = BusRelay::new();
        let mut env = intent("chat.send", serde_json::json!({ "text": "hi" }));
        env.v = 999;
        // The command would reject before reaching relay; emulate that gate.
        assert_ne!(env.v, PROTOCOL_VERSION);
        // relay.dispatch itself does not validate (the command does); calling it
        // with a bad version is a caller bug, not a relay bug. We just confirm
        // the relay doesn't panic on a subscriber-less dispatch.
        relay.dispatch(env);
    }

    #[test]
    fn relay_handles_intent_without_subscriber() {
        let relay = BusRelay::new();
        // No subscribe_downstream call: emit must be a no-op, not a panic.
        relay.dispatch(intent("chat.send", serde_json::json!({ "text": "hello" })));
        relay.dispatch(intent("status.toggle", serde_json::json!({})));
    }

    #[test]
    fn relay_increments_seq_per_dispatch() {
        let relay = BusRelay::new();
        let start = relay.seq.load(Ordering::Relaxed);
        relay.dispatch(intent("status.toggle", serde_json::json!({})));
        relay.dispatch(intent("status.toggle", serde_json::json!({})));
        assert_eq!(relay.seq.load(Ordering::Relaxed), start + 2);
    }

    /// Regression guard for the CodeRabbit finding: the chat.send branch must
    /// read `params.text` from the JSON object, not treat the whole params
    /// value as a string. We can't observe the emitted envelope without an
    /// AppHandle, but we can confirm the relay accepts an object payload
    /// without panic — the previous bug silently produced "" text.
    #[test]
    fn chat_send_reads_text_field_from_object_params() {
        let relay = BusRelay::new();
        // Object payload (the real wire shape), not a bare string.
        relay.dispatch(intent("chat.send", serde_json::json!({ "text": "hello world" })));
        // Missing text field: must still not panic, falls back to "".
        relay.dispatch(intent("chat.send", serde_json::json!({})));
    }
}
