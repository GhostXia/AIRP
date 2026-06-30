//! State-Protocol `Envelope` wire types.
//!
//! These mirror `AIRP-State-Protocol/schema/airp-state-protocol.schema.json`
//! (draft 2020-12). The adapter owns *only* the subset needed for the minimal
//! closed loop (`hello`, `intent`, `subscribe`, `ack` up; `blueprint`, `state`,
//! `manifest`, `error` down). Unknown variants deserialize to a catch-all so a
//! newer UI cannot crash the gateway — the envelope is logged and dropped.
//!
//! Design note: every struct is `#[serde(default)]`-tolerant where the schema
//! allows optional fields, and `Body` is `#[serde(tag = "kind", rename_all = "snake_case")]`
//! so the JSON `{ "kind": "intent", ... }` discriminates variants exactly as
//! the UI emits them.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol envelope version. The schema pins `v: 1`.
pub const ENVELOPE_VERSION: u64 = 1;

/// A State-Protocol envelope. The unit of traffic in both directions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u64,
    /// Opaque id, unique per source. Echoed in `ack`/`error` replies.
    pub id: String,
    /// Millisecond epoch timestamp. The gateway does not validate skew.
    pub ts: u64,
    /// Who sent this: `"ui"`, `"gateway"`, ...
    pub src: String,
    #[serde(default, rename = "ref")]
    /// Optional correlation id (e.g. the id of the intent an ack refers to).
    /// Renamed on the wire to `ref` (a Rust keyword) per the State-Protocol schema.
    pub ref_: Option<String>,
    pub body: Body,
}

/// Marker the gateway stamps onto every envelope it emits.
pub const SRC_GATEWAY: &str = "gateway";

/// The polymorphic payload. `tag = "kind"` matches the schema's discriminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    // --- Upstream (UI -> gateway) ---------------------------------------
    /// Handshake. Gateway replies with blueprint/manifest/initial state.
    Hello {
        client: String,
        version: String,
        #[serde(default)]
        accept: Vec<String>,
    },
    /// A request to do something. Maps onto a `RouteRule`.
    Intent {
        name: String,
        #[serde(default)]
        params: Value,
        /// Widget instance id that emitted this intent. Becomes the state scope.
        #[serde(default)]
        source: Option<String>,
    },
    /// Declare which scopes this connection wants patches for.
    Subscribe {
        #[serde(default)]
        scopes: Vec<String>,
    },
    /// Acknowledge receipt of a downstream envelope.
    Ack {
        #[serde(rename = "ref")]
        ref_: String,
        #[serde(default)]
        ok: bool,
    },

    // --- Downstream (gateway -> UI) -------------------------------------
    /// Replace the whole blueprint.
    Blueprint {
        op: BlueprintOp,
        blueprint: Blueprint,
    },
    /// Apply a state operation to a scope. The only kind the minimal closed
    /// loop strictly needs.
    State {
        scope: String,
        op: StateOp,
        /// For `op: "patch"`: a JSON Patch (RFC 6902) array.
        /// For `op: "set"`: a single-element patch replacing the scope root.
        #[serde(default)]
        patch: Vec<PatchOp>,
    },
    /// Register/replace widget manifests (third-party widget discovery).
    Manifest {
        op: ManifestOp,
        manifests: Vec<Manifest>,
    },
    /// Surface a gateway-side error tied to an upstream envelope id.
    Error {
        #[serde(default, rename = "ref")]
        ref_: Option<String>,
        code: String,
        message: String,
    },

    /// Forward-compat catch-all. Lets the gateway survive a newer protocol
    /// variant without panicking; the envelope is logged and dropped.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlueprintOp {
    Set,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateOp {
    Set,
    Patch,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestOp {
    Set,
}

/// A single JSON Patch operation (RFC 6902 subset). We only serialize, so the
/// fields are deliberately permissive `Value`s.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchOp {
    pub op: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

impl PatchOp {
    /// `add` at `path` with `value`. The common case for appending a message.
    pub fn add(path: impl Into<String>, value: Value) -> Self {
        Self {
            op: "add".into(),
            path: path.into(),
            value: Some(value),
            from: None,
        }
    }

    /// `replace` the whole scope root. Used for `state op: set`.
    pub fn replace_root(value: Value) -> Self {
        Self {
            op: "replace".into(),
            path: "".into(),
            value: Some(value),
            from: None,
        }
    }
}

/// The blueprint payload. Layout is left opaque (`Value`) — the adapter is not
/// the layout authority, it only forwards whatever the host wires in.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blueprint {
    pub version: String,
    #[serde(default)]
    pub layout: Value,
    #[serde(default)]
    pub widgets: Vec<Value>,
}

/// A third-party widget manifest entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(rename = "type")]
    pub ty: String,
    pub version: String,
    pub title: String,
    pub entry: Value,
}

impl Envelope {
    /// Construct a gateway-sourced envelope with a fresh id and current ts.
    pub fn gateway(body: Body) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            id: next_id(),
            ts: now_ms(),
            src: SRC_GATEWAY.to_string(),
            ref_: None,
            body,
        }
    }

    /// Same as [`gateway`] but with a correlation `ref` (e.g. the intent id).
    pub fn gateway_ref(ref_: impl Into<String>, body: Body) -> Self {
        Self {
            v: ENVELOPE_VERSION,
            id: next_id(),
            ts: now_ms(),
            src: SRC_GATEWAY.to_string(),
            ref_: Some(ref_.into()),
            body,
        }
    }

    /// Validate the envelope's static invariants. Returns the envelope on
    /// success so callers can chain. This is the *only* validation the adapter
    /// does — semantic validation lives in the handler.
    pub fn validate(self) -> Result<Self, EnvelopeError> {
        if self.v != ENVELOPE_VERSION {
            return Err(EnvelopeError::UnsupportedVersion(self.v));
        }
        Ok(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EnvelopeError {
    #[error("unsupported envelope version: {0} (expected 1)")]
    UnsupportedVersion(u64),
    #[error("invalid envelope: {0}")]
    Invalid(String),
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Monotonic-ish id: `gw-<counter>`. Good enough to be unique per process.
fn next_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(1);
    format!("gw-{}", N.fetch_add(1, Ordering::Relaxed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_intent_envelope() {
        let raw = serde_json::json!({
            "v": 1, "id": "ui-1", "ts": 1700000000000u64, "src": "ui",
            "body": { "kind": "intent", "name": "chat.send",
                      "params": {"text": "hello"}, "source": "w-chat" }
        });
        let env: Envelope = serde_json::from_value(raw).unwrap();
        match env.body {
            Body::Intent {
                name,
                params,
                source,
            } => {
                assert_eq!(name, "chat.send");
                assert_eq!(params["text"], "hello");
                assert_eq!(source.as_deref(), Some("w-chat"));
            }
            other => panic!("wrong body: {other:?}"),
        }
    }

    #[test]
    fn unknown_kind_does_not_panic() {
        let raw = serde_json::json!({
            "v": 1, "id": "ui-x", "ts": 0, "src": "ui",
            "body": { "kind": "future_thing", "payload": 42 }
        });
        let env: Envelope = serde_json::from_value(raw).unwrap();
        assert!(matches!(env.body, Body::Unknown));
    }

    #[test]
    fn roundtrips_state_set_envelope() {
        let env = Envelope::gateway_ref(
            "ui-1",
            Body::State {
                scope: "w-chat".into(),
                op: StateOp::Set,
                patch: vec![PatchOp::replace_root(serde_json::json!({"messages": []}))],
            },
        );
        let s = serde_json::to_string(&env).unwrap();
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert!(matches!(back.body, Body::State { .. }));
    }

    #[test]
    fn ref_field_serializes_as_ref_not_ref_() {
        // Wire contract: the correlation field is `ref` on the wire (a Rust
        // keyword), never `ref_`. Verified both ways.
        let env = Envelope::gateway_ref(
            "ui-9",
            Body::Ack {
                ref_: "ui-9".into(),
                ok: true,
            },
        );
        let s = serde_json::to_string(&env).unwrap();
        assert!(
            s.contains("\"ref\":\"ui-9\""),
            "envelope serialized ref wrong: {s}"
        );
        assert!(
            !s.contains("ref_"),
            "envelope leaked rust field name ref_: {s}"
        );

        // Deserialize from a wire `ref` (Ack body) too.
        let raw = serde_json::json!({
            "v": 1, "id": "ui-1", "ts": 0, "src": "ui", "ref": "ui-0",
            "body": { "kind": "ack", "ref": "ui-0", "ok": true }
        });
        let env: Envelope = serde_json::from_value(raw).unwrap();
        assert_eq!(env.ref_.as_deref(), Some("ui-0"));
        match env.body {
            Body::Ack { ref_, ok } => {
                assert_eq!(ref_, "ui-0");
                assert!(ok);
            }
            other => panic!("wrong body: {other:?}"),
        }
    }
}
