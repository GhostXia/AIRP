//! AIRP State Protocol — Rust binding.
//!
//! Wire types and the [`AgentBus`] trait. This crate is the in-process contract
//! between an AIRP UI host (e.g. the Tauri core) and an `AgentBus` implementation
//! (e.g. AIRP-Gateway). The on-the-wire contract is defined by
//! `schema/airp-state-protocol.schema.json`; these types mirror it 1:1.
//!
//! Independence comes from this contract, not from how it is wired: any crate
//! that implements [`AgentBus`] can replace the default Gateway.

use std::collections::BTreeMap;

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Current protocol version (the `v` field on every [`Envelope`]).
pub const PROTOCOL_VERSION: u32 = 1;

/// Every message on the wire is an `Envelope`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    /// Protocol version. Always [`PROTOCOL_VERSION`].
    pub v: u32,
    /// Unique message id (UUID recommended).
    pub id: String,
    /// Creation time, epoch milliseconds.
    pub ts: i64,
    /// Origin: `"ui"`, `"gateway"`, or `"agent:<name>"`.
    pub src: String,
    /// The tagged message body.
    pub body: Body,
}

impl Envelope {
    /// Build an envelope with the current protocol version.
    pub fn new(id: impl Into<String>, ts: i64, src: impl Into<String>, body: Body) -> Self {
        Self { v: PROTOCOL_VERSION, id: id.into(), ts, src: src.into(), body }
    }
}

/// Message body, a tagged union discriminated by `kind`.
///
/// Direction is documented per variant but not enforced by the type system, so
/// a single shared type can be used on both ends of the bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    // ---- downstream: gateway -> ui ----
    /// Set or patch the UI blueprint.
    Blueprint(BlueprintMsg),
    /// Set or patch a state scope.
    State(StateMsg),
    /// Deliver widget manifests so the UI can auto-register widgets it cannot
    /// yet render (the open extension contract, over the wire).
    Manifest(ManifestMsg),
    /// Fire-and-forget event (toast, sfx, navigate, ...).
    Event(EventMsg),
    /// Error report.
    Error(ErrorMsg),
    // ---- upstream: ui -> gateway ----
    /// A user action emitted by a widget.
    Intent(IntentMsg),
    /// Subscribe to state scopes.
    Subscribe(SubscribeMsg),
    /// Unsubscribe from state scopes.
    Unsubscribe(SubscribeMsg),
    /// Handshake.
    Hello(HelloMsg),
    /// Acknowledge an envelope by id.
    Ack(AckMsg),
}

/// Whether a `blueprint`/`state` message carries a full value or a patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetOrPatch {
    Set,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintMsg {
    pub op: SetOrPatch,
    /// Present when `op = set`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub blueprint: Option<Blueprint>,
    /// Present when `op = patch`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub patch: Option<JsonPatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateMsg {
    /// Widget instance id, `"session"`, or a dotted path.
    pub scope: String,
    pub op: SetOrPatch,
    /// Full state value. Present when `op = set`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub state: Option<Value>,
    /// Present when `op = patch`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub patch: Option<JsonPatch>,
}

/// DOWNSTREAM. Deliver widget manifests so the UI can auto-register widgets it
/// cannot yet render.
///
/// `op = Set` replaces the UI's full known-manifest set; `op = Patch` upserts
/// the given subset by `type` (the incremental form — an upsert of `manifests`,
/// not an RFC 6902 JSON Patch), letting the Gateway ship only diffs. The UI
/// should process a manifest BEFORE any blueprint that references its types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestMsg {
    pub op: SetOrPatch,
    /// Manifests to set or upsert (third-party under their own namespace, or
    /// `core.*` first-party).
    pub manifests: Vec<WidgetDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventMsg {
    pub topic: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorMsg {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub detail: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentMsg {
    /// Intent name, e.g. `"chat.send"`, `"emotion.set"`.
    pub name: String,
    /// Originating widget instance id.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubscribeMsg {
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloMsg {
    /// Client name, e.g. `"airp-ui"`.
    pub client: String,
    pub version: String,
    /// Widget types this client can render.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub accept: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AckMsg {
    /// The acknowledged envelope id.
    #[serde(rename = "ref")]
    pub ref_: String,
}

/// Declarative description of the whole UI — the stable, RP-derived asset.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blueprint {
    /// Identity of this blueprint (UUID or content hash).
    pub version: String,
    /// RP / UI profile id this blueprint belongs to.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub theme: Option<Theme>,
    pub layout: Layout,
    pub widgets: Vec<WidgetInstance>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    /// e.g. `"cyberpunk"`.
    pub name: String,
    /// Design tokens (color/spacing/...).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tokens: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(rename = "type")]
    pub kind: LayoutKind,
    pub areas: Vec<Area>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LayoutKind {
    Dock,
    Grid,
    Stack,
    Tabs,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Area {
    pub id: String,
    /// Widget instance ids placed in this area.
    pub widgets: Vec<String>,
    /// Area-specific layout props (size, dock side, ...).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub props: Option<Value>,
}

/// A widget placed in the blueprint.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetInstance {
    /// Stable instance id; used as state scope and render key.
    pub id: String,
    /// Registry key, e.g. `"chat"`, `"emotion"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Static props for this instance.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub props: Option<Value>,
    /// State scope this widget binds to (defaults to its id).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub state: Option<String>,
    /// Permissions this instance requests.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub capabilities: Option<Vec<Capability>>,
}

/// A widget manifest as published in the registry — the OPEN extension contract.
///
/// Carried to the UI by a [`ManifestMsg`]; any third party can ship a widget by
/// publishing a manifest under its own namespace (see the `type` field). Mirrors
/// `schema/widget-manifest.schema.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetDef {
    /// Namespaced widget id, e.g. `"core.chat"` or `"acme.relationship-graph"`.
    /// Must be `namespace.name`; the `core.*` namespace is reserved for first-party widgets.
    #[serde(rename = "type")]
    pub kind: String,
    /// Semantic version.
    pub version: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub description: Option<String>,
    /// JSON Schema for this widget's props.
    #[serde(rename = "propsSchema", skip_serializing_if = "Option::is_none", default)]
    pub props_schema: Option<Value>,
    /// JSON Schema for this widget's state slice.
    #[serde(rename = "stateSchema", skip_serializing_if = "Option::is_none", default)]
    pub state_schema: Option<Value>,
    /// Permissions this widget requests; enforced by the Gateway.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub capabilities: Option<Vec<Capability>>,
    /// Intent names this widget can emit.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub intents: Option<Vec<String>>,
    /// How the UI's Widget Registry loads this widget.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub entry: Option<WidgetEntry>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub license: Option<String>,
}

/// How the UI's Widget Registry loads a widget.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetEntry {
    pub kind: EntryKind,
    /// For `EntryKind::Esm`: the module specifier or URL the UI imports.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub source: Option<String>,
    /// For `EntryKind::Esm`: if true, the host loads this widget inside a
    /// sandboxed iframe (no `allow-same-origin`) and bridges the
    /// [`WidgetContext`](crate) over `postMessage`, so the widget cannot touch
    /// the host DOM/global/same-origin resources. Recommended for untrusted
    /// third-party widgets (SECURITY.md).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub sandbox: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    /// Bundled into the UI.
    Builtin,
    /// Loaded as an ES module from `source`.
    Esm,
}

/// A permission a widget/agent requests; enforced by the Gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Capability {
    #[serde(rename = "read:memory")]
    ReadMemory,
    #[serde(rename = "write:memory")]
    WriteMemory,
    #[serde(rename = "read:worldbook")]
    ReadWorldbook,
    #[serde(rename = "read:state")]
    ReadState,
    #[serde(rename = "write:state")]
    WriteState,
    #[serde(rename = "call:tool")]
    CallTool,
}

/// An RFC 6902 JSON Patch document.
pub type JsonPatch = Vec<PatchOp>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatchOp {
    pub op: PatchOpKind,
    /// JSON Pointer (RFC 6901).
    pub path: String,
    /// Operand for add/replace/test.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<Value>,
    /// Source pointer for move/copy.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchOpKind {
    Add,
    Remove,
    Replace,
    Move,
    Copy,
    Test,
}

/// Error returned by an [`AgentBus`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusError {
    /// The bus is not connected to an upstream.
    NotConnected,
    /// The upstream rejected the message (e.g. permission denied).
    Rejected(String),
    /// Transport-level failure.
    Transport(String),
}

impl std::fmt::Display for BusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BusError::NotConnected => write!(f, "bus not connected"),
            BusError::Rejected(why) => write!(f, "rejected: {why}"),
            BusError::Transport(why) => write!(f, "transport error: {why}"),
        }
    }
}

impl std::error::Error for BusError {}

/// The in-process contract between a UI host and an upstream runtime.
///
/// `AIRP-Gateway` is the default implementation; any type implementing this
/// trait can replace it. The UI host sends upstream envelopes via [`dispatch`]
/// and renders the downstream stream from [`subscribe`].
///
/// [`dispatch`]: AgentBus::dispatch
/// [`subscribe`]: AgentBus::subscribe
#[async_trait::async_trait]
pub trait AgentBus: Send + Sync {
    /// UI -> bus: deliver one upstream envelope (intent/subscribe/hello/ack).
    async fn dispatch(&self, env: Envelope) -> Result<(), BusError>;

    /// bus -> UI: stream of downstream envelopes (blueprint/state/event/error).
    fn subscribe(&self) -> BoxStream<'static, Envelope>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn state_patch_roundtrips() {
        let env = Envelope::new(
            "01HF...",
            1_718_200_000_000,
            "gateway",
            Body::State(StateMsg {
                scope: "w-emotion".into(),
                op: SetOrPatch::Patch,
                state: None,
                patch: Some(vec![PatchOp {
                    op: PatchOpKind::Replace,
                    path: "/emotion".into(),
                    value: Some(json!(80)),
                    from: None,
                }]),
            }),
        );

        let text = serde_json::to_string(&env).unwrap();
        // discriminator is flattened onto the body
        assert!(text.contains("\"kind\":\"state\""));
        // absent options are omitted
        assert!(!text.contains("\"blueprint\""));

        let back: Envelope = serde_json::from_str(&text).unwrap();
        assert_eq!(env, back);
    }

    #[test]
    fn blueprint_set_shape_matches_schema() {
        let raw = json!({
            "v": 1,
            "id": "m1",
            "ts": 1,
            "src": "gateway",
            "body": {
                "kind": "blueprint",
                "op": "set",
                "blueprint": {
                    "version": "bp-uuid",
                    "theme": { "name": "cyberpunk" },
                    "layout": {
                        "type": "dock",
                        "areas": [{ "id": "main", "widgets": ["w-chat"] }]
                    },
                    "widgets": [
                        { "id": "w-chat", "type": "chat", "state": "session.chat" }
                    ]
                }
            }
        });

        let env: Envelope = serde_json::from_value(raw).unwrap();
        match env.body {
            Body::Blueprint(BlueprintMsg { op: SetOrPatch::Set, blueprint: Some(bp), .. }) => {
                assert_eq!(bp.version, "bp-uuid");
                assert_eq!(bp.layout.kind, LayoutKind::Dock);
                assert_eq!(bp.widgets[0].kind, "chat");
            }
            other => panic!("unexpected body: {other:?}"),
        }
    }

    #[test]
    fn capability_renders_with_colon() {
        let c = serde_json::to_string(&Capability::ReadMemory).unwrap();
        assert_eq!(c, "\"read:memory\"");
    }

    #[test]
    fn ack_uses_ref_key() {
        let env = Envelope::new("a1", 2, "ui", Body::Ack(AckMsg { ref_: "m1".into() }));
        let text = serde_json::to_string(&env).unwrap();
        assert!(text.contains("\"ref\":\"m1\""));
        assert!(!text.contains("ref_"));
    }

    #[test]
    fn manifest_roundtrips() {
        let env = Envelope::new(
            "m1",
            1_718_200_000_000,
            "gateway",
            Body::Manifest(ManifestMsg {
                op: SetOrPatch::Patch,
                manifests: vec![WidgetDef {
                    kind: "acme.status-pill".into(),
                    version: "1.2.0".into(),
                    title: "状态胶囊".into(),
                    description: None,
                    props_schema: None,
                    state_schema: None,
                    capabilities: Some(vec![Capability::ReadState]),
                    intents: Some(vec!["status.toggle".into()]),
                    entry: Some(WidgetEntry {
                        kind: EntryKind::Esm,
                        source: Some("https://cdn.example.com/status-pill.mjs".into()),
                        sandbox: None,
                    }),
                    author: None,
                    homepage: None,
                    license: None,
                }],
            }),
        );

        let text = serde_json::to_string(&env).unwrap();
        // discriminator flattened onto the body
        assert!(text.contains("\"kind\":\"manifest\""));
        assert!(text.contains("\"acme.status-pill\""));
        // the esm source survives serialization (substring, not a quoted token —
        // it is part of the full "https://.../status-pill.mjs" URL)
        assert!(text.contains("status-pill.mjs"));

        let back: Envelope = serde_json::from_str(&text).unwrap();
        assert_eq!(env, back);
    }
}
