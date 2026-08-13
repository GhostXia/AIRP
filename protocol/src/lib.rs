//! AIRP State Protocol — Rust binding.
//!
//! Wire types and the [`AgentBus`] trait. This crate is the in-process contract
//! between an AIRP UI host (e.g. the Tauri core) and an `AgentBus` implementation
//! (normally the AIRP engine bridge). The on-the-wire contract is defined by
//! `schema/airp-state-protocol.schema.json`; these types mirror it 1:1.
//!
//! Independence comes from this contract, not from how it is wired: any crate
//! that implements [`AgentBus`] can replace the default engine bridge.

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
    /// Origin: `"ui"`, `"engine"`, or `"agent:<name>"`.
    pub src: String,
    /// The tagged message body.
    pub body: Body,
}

impl Envelope {
    /// Build an envelope with the current protocol version.
    pub fn new(id: impl Into<String>, ts: i64, src: impl Into<String>, body: Body) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            id: id.into(),
            ts,
            src: src.into(),
            body,
        }
    }
}

/// Message body, a tagged union discriminated by `kind`.
///
/// Direction is documented per variant but not enforced by the type system, so
/// a single shared type can be used on both ends of the bus.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Body {
    // ---- downstream: runtime -> ui ----
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
    // ---- upstream: ui -> runtime ----
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
/// not an RFC 6902 JSON Patch), letting the runtime ship only diffs. The UI
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
    #[serde(
        rename = "propsSchema",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub props_schema: Option<Value>,
    /// JSON Schema for this widget's state slice.
    #[serde(
        rename = "stateSchema",
        skip_serializing_if = "Option::is_none",
        default
    )]
    pub state_schema: Option<Value>,
    /// Permissions this widget requests; enforced by the engine/runtime.
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

/// A permission a widget/agent requests; enforced by the engine/runtime.
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

// ---------------------------------------------------------------------------
// Surface Protocol v2
// ---------------------------------------------------------------------------
//
// These types are deliberately separate from the v1 Envelope above.  The v1
// bus remains the compatibility input for the current demo while Surface v2
// establishes a versioned, reconstructable UI projection for later PRs.

pub const SURFACE_PROTOCOL_MAJOR: u16 = 2;
pub const SURFACE_PROTOCOL_MINOR: u16 = 0;
pub const SURFACE_PROTOCOL_COMPONENT_MAX: u64 = u16::MAX as u64;
pub const SURFACE_MAX_DOCUMENT_BYTES: usize = 1_048_576;
pub const SURFACE_MAX_PATCH_BYTES: usize = 65_536;
pub const SURFACE_MAX_PATCH_OPERATIONS: usize = 256;
pub const SURFACE_MAX_BLUEPRINT_DEPTH: usize = 16;
pub const SURFACE_MAX_BLUEPRINT_NODES: usize = 512;
pub const SURFACE_MAX_WIDGET_INSTANCES: usize = 128;
pub const SURFACE_MAX_CHILDREN: usize = 32;
pub const SURFACE_MAX_IDENTIFIER_LENGTH: usize = 128;

pub const SURFACE_FORBIDDEN_FIELDS: &[&str] = &[
    "html",
    "css",
    "styleSheet",
    "javascript",
    "js",
    "script",
    "vue",
    "template",
    "eval",
    "expression",
    "function",
    "renderFunction",
    "render_function",
    "componentSource",
    "component_source",
    "sourceCode",
    "source_code",
    "innerHTML",
    "outerHTML",
    "dangerouslySetInnerHTML",
];

pub const SURFACE_ERROR_CODES: &[&str] = &[
    "unsupported_major",
    "invalid_version",
    "invalid_revision",
    "revision_mismatch",
    "revision_gap",
    "invalid_blueprint",
    "duplicate_instance_id",
    "invalid_reference",
    "invalid_patch",
    "resource_limit",
    "document_too_large",
    "forbidden_executable_field",
    "resync_required",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SurfaceProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl Default for SurfaceProtocolVersion {
    fn default() -> Self {
        Self {
            major: SURFACE_PROTOCOL_MAJOR,
            minor: SURFACE_PROTOCOL_MINOR,
        }
    }
}

/// A revision is an unsigned 64-bit integer on the wire, encoded as a decimal
/// JSON string so JavaScript never rounds it through `number`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SurfaceRevision(pub u64);

impl SurfaceRevision {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let valid_digits = raw == "0"
            || (!raw.is_empty()
                && !raw.starts_with('0')
                && raw.bytes().all(|byte| byte.is_ascii_digit()));
        if !valid_digits {
            return Err(format!("revision must be a decimal u64 string: {raw:?}"));
        }
        raw.parse::<u64>()
            .map(Self)
            .map_err(|_| format!("revision is outside the u64 range: {raw:?}"))
    }

    pub const fn value(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for SurfaceRevision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<u64> for SurfaceRevision {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Serialize for SurfaceRevision {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for SurfaceRevision {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfaceMessageKind {
    Snapshot,
    Patch,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlueprintV2 {
    pub version: u16,
    pub root: LayoutNodeV2,
    pub widgets: Vec<WidgetInstanceV2>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LayoutNodeV2 {
    Split {
        id: String,
        orientation: SplitOrientation,
        children: Vec<LayoutNodeV2>,
    },
    Tabs {
        id: String,
        active: String,
        children: Vec<LayoutNodeV2>,
    },
    Stack {
        id: String,
        children: Vec<LayoutNodeV2>,
    },
    Widget {
        id: String,
        #[serde(rename = "instanceId")]
        instance_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SplitOrientation {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WidgetInstanceV2 {
    pub id: String,
    #[serde(rename = "type")]
    pub widget_type: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub props: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceSnapshot {
    pub kind: SurfaceMessageKind,
    pub protocol: SurfaceProtocolVersion,
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    pub revision: SurfaceRevision,
    pub blueprint: BlueprintV2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfacePatchEvent {
    pub kind: SurfaceMessageKind,
    pub protocol: SurfaceProtocolVersion,
    #[serde(rename = "surfaceId")]
    pub surface_id: String,
    #[serde(rename = "baseRevision")]
    pub base_revision: SurfaceRevision,
    pub revision: SurfaceRevision,
    pub patch: Vec<SurfacePatchOp>,
}

pub type SurfaceSnapshotV2 = SurfaceSnapshot;
pub type SurfacePatchEventV2 = SurfacePatchEvent;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfacePatchOp {
    pub op: SurfacePatchOperation,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub value: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SurfacePatchOperation {
    Add,
    Remove,
    Replace,
    Move,
    Copy,
    Test,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceErrorCode {
    UnsupportedMajor,
    InvalidVersion,
    InvalidRevision,
    RevisionMismatch,
    RevisionGap,
    InvalidBlueprint,
    DuplicateInstanceId,
    InvalidReference,
    InvalidPatch,
    ResourceLimit,
    DocumentTooLarge,
    ForbiddenExecutableField,
    ResyncRequired,
}

impl SurfaceErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedMajor => "unsupported_major",
            Self::InvalidVersion => "invalid_version",
            Self::InvalidRevision => "invalid_revision",
            Self::RevisionMismatch => "revision_mismatch",
            Self::RevisionGap => "revision_gap",
            Self::InvalidBlueprint => "invalid_blueprint",
            Self::DuplicateInstanceId => "duplicate_instance_id",
            Self::InvalidReference => "invalid_reference",
            Self::InvalidPatch => "invalid_patch",
            Self::ResourceLimit => "resource_limit",
            Self::DocumentTooLarge => "document_too_large",
            Self::ForbiddenExecutableField => "forbidden_executable_field",
            Self::ResyncRequired => "resync_required",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceValidationError {
    pub code: SurfaceErrorCode,
    pub message: String,
    pub path: Option<String>,
}

impl SurfaceValidationError {
    fn new(code: SurfaceErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
        }
    }

    fn at(code: SurfaceErrorCode, path: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: Some(path.into()),
        }
    }
}

impl std::fmt::Display for SurfaceValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for SurfaceValidationError {}

fn surface_identifier_is_valid(value: &str) -> bool {
    if value.is_empty() || value.len() > SURFACE_MAX_IDENTIFIER_LENGTH {
        return false;
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphanumeric()) {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | ':' | '-'))
}

fn surface_json_has_forbidden_field(value: &Value) -> Option<String> {
    match value {
        Value::Array(values) => values.iter().find_map(surface_json_has_forbidden_field),
        Value::Object(map) => {
            for (key, child) in map {
                if SURFACE_FORBIDDEN_FIELDS.contains(&key.as_str()) {
                    return Some(key.clone());
                }
                if let Some(found) = surface_json_has_forbidden_field(child) {
                    return Some(found);
                }
            }
            None
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn validate_surface_version(
    version: &SurfaceProtocolVersion,
) -> Result<(), SurfaceValidationError> {
    if version.major != SURFACE_PROTOCOL_MAJOR {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::UnsupportedMajor,
            format!("unsupported Surface protocol major {}", version.major),
        ));
    }
    Ok(())
}

fn validate_surface_version_json(raw: &Value) -> Result<(), SurfaceValidationError> {
    let Some(protocol) = raw.get("protocol").and_then(Value::as_object) else {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidVersion,
            "protocol",
            "protocol must be an object",
        ));
    };
    let Some(major) = protocol.get("major").and_then(Value::as_u64) else {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidVersion,
            "protocol.major",
            "protocol major must be an unsigned integer",
        ));
    };
    if major != u64::from(SURFACE_PROTOCOL_MAJOR) {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::UnsupportedMajor,
            "protocol.major",
            format!("unsupported Surface protocol major {major}"),
        ));
    }
    let Some(minor) = protocol.get("minor").and_then(Value::as_u64) else {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidVersion,
            "protocol.minor",
            "protocol minor must be an unsigned integer",
        ));
    };
    if minor > SURFACE_PROTOCOL_COMPONENT_MAX {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidVersion,
            "protocol.minor",
            "protocol minor exceeds the authority limit",
        ));
    }
    Ok(())
}

fn validate_surface_revision_json(raw: &Value, field: &str) -> Result<(), SurfaceValidationError> {
    let Some(revision) = raw.get(field).and_then(Value::as_str) else {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidRevision,
            field,
            "revision must be a decimal u64 string",
        ));
    };
    SurfaceRevision::parse(revision).map_err(|message| {
        SurfaceValidationError::at(SurfaceErrorCode::InvalidRevision, field, message)
    })?;
    Ok(())
}

fn validate_surface_identifier(value: &str, label: &str) -> Result<(), SurfaceValidationError> {
    if !surface_identifier_is_valid(value) {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::InvalidBlueprint,
            format!("{label} is not a valid bounded identifier"),
        ));
    }
    Ok(())
}

fn validate_surface_patch_pointer(
    pointer: &str,
    label: &str,
) -> Result<(), SurfaceValidationError> {
    if pointer.is_empty() {
        return Ok(());
    }
    if !pointer.starts_with('/') || pointer.contains('\0') {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidPatch,
            label,
            "patch path must be an RFC 6901 JSON Pointer",
        ));
    }
    let pointer_bytes = pointer.as_bytes();
    if pointer_bytes.iter().enumerate().any(|(index, byte)| {
        *byte == b'~'
            && (index + 1 >= pointer_bytes.len()
                || !matches!(pointer_bytes[index + 1], b'0' | b'1'))
    }) {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidPatch,
            label,
            "patch path contains an invalid RFC 6901 escape",
        ));
    }
    for segment in pointer.split('/').skip(1) {
        if matches!(segment, "__proto__" | "prototype" | "constructor") {
            return Err(SurfaceValidationError::at(
                SurfaceErrorCode::InvalidPatch,
                label,
                "prototype-pollution path segment is forbidden",
            ));
        }
    }
    Ok(())
}

fn validate_surface_patch_op(op: &SurfacePatchOp) -> Result<(), SurfaceValidationError> {
    validate_surface_patch_pointer(&op.path, "patch.path")?;
    if op.path.is_empty() && op.op != SurfacePatchOperation::Test {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidPatch,
            "patch.path",
            "patch cannot replace or remove the snapshot root",
        ));
    }
    let immutable = ["/kind", "/protocol", "/surfaceId", "/revision"];
    if immutable
        .iter()
        .any(|root| op.path == *root || op.path.starts_with(&format!("{root}/")))
    {
        return Err(SurfaceValidationError::at(
            SurfaceErrorCode::InvalidPatch,
            "patch.path",
            "patch cannot mutate immutable snapshot metadata",
        ));
    }
    match op.op {
        SurfacePatchOperation::Add
        | SurfacePatchOperation::Replace
        | SurfacePatchOperation::Test => {
            if op.value.is_none() {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::InvalidPatch,
                    "add, replace, and test require value",
                ));
            }
        }
        SurfacePatchOperation::Move | SurfacePatchOperation::Copy => {
            let Some(from) = op.from.as_deref() else {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::InvalidPatch,
                    "move and copy require from",
                ));
            };
            validate_surface_patch_pointer(from, "patch.from")?;
            if from.is_empty() {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::InvalidPatch,
                    "patch cannot read the snapshot root",
                ));
            }
            if immutable
                .iter()
                .any(|root| from == *root || from.starts_with(&format!("{root}/")))
            {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::InvalidPatch,
                    "patch cannot read immutable snapshot metadata",
                ));
            }
        }
        SurfacePatchOperation::Remove => {}
    }
    if let Some(value) = &op.value {
        if let Some(field) = surface_json_has_forbidden_field(value) {
            return Err(SurfaceValidationError::new(
                SurfaceErrorCode::ForbiddenExecutableField,
                format!("forbidden executable field {field:?}"),
            ));
        }
    }
    Ok(())
}

fn validate_surface_node(
    node: &LayoutNodeV2,
    depth: usize,
    nodes: &mut usize,
    node_ids: &mut BTreeMap<String, ()>,
    widget_refs: &mut BTreeMap<String, ()>,
    widget_ids: &BTreeMap<String, ()>,
) -> Result<(), SurfaceValidationError> {
    if depth > SURFACE_MAX_BLUEPRINT_DEPTH {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::ResourceLimit,
            "blueprint nesting depth exceeds the authority limit",
        ));
    }
    *nodes += 1;
    if *nodes > SURFACE_MAX_BLUEPRINT_NODES {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::ResourceLimit,
            "blueprint node count exceeds the authority limit",
        ));
    }

    let id = match node {
        LayoutNodeV2::Split { id, .. }
        | LayoutNodeV2::Tabs { id, .. }
        | LayoutNodeV2::Stack { id, .. }
        | LayoutNodeV2::Widget { id, .. } => id,
    };
    validate_surface_identifier(id, "layout node id")?;
    if node_ids.insert(id.clone(), ()).is_some() {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::DuplicateInstanceId,
            format!("duplicate layout node id {id:?}"),
        ));
    }

    match node {
        LayoutNodeV2::Split { children, .. } => {
            if children.len() != 2 {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::InvalidBlueprint,
                    "split must have exactly two children",
                ));
            }
            for child in children {
                validate_surface_node(child, depth + 1, nodes, node_ids, widget_refs, widget_ids)?;
            }
        }
        LayoutNodeV2::Tabs {
            active, children, ..
        } => {
            if children.is_empty() || children.len() > SURFACE_MAX_CHILDREN {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::ResourceLimit,
                    "tabs child count is outside the authority limit",
                ));
            }
            if !children
                .iter()
                .any(|child| surface_node_id(child) == active)
            {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::InvalidReference,
                    format!("tabs active reference {active:?} does not name a child"),
                ));
            }
            for child in children {
                validate_surface_node(child, depth + 1, nodes, node_ids, widget_refs, widget_ids)?;
            }
        }
        LayoutNodeV2::Stack { children, .. } => {
            if children.is_empty() || children.len() > SURFACE_MAX_CHILDREN {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::ResourceLimit,
                    "stack child count is outside the authority limit",
                ));
            }
            for child in children {
                validate_surface_node(child, depth + 1, nodes, node_ids, widget_refs, widget_ids)?;
            }
        }
        LayoutNodeV2::Widget { instance_id, .. } => {
            validate_surface_identifier(instance_id, "widget instance id")?;
            if !widget_ids.contains_key(instance_id) {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::InvalidReference,
                    format!("widget node references missing instance {instance_id:?}"),
                ));
            }
            if widget_refs.insert(instance_id.clone(), ()).is_some() {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::DuplicateInstanceId,
                    format!("widget instance {instance_id:?} is placed more than once"),
                ));
            }
        }
    }
    Ok(())
}

fn surface_node_id(node: &LayoutNodeV2) -> &str {
    match node {
        LayoutNodeV2::Split { id, .. }
        | LayoutNodeV2::Tabs { id, .. }
        | LayoutNodeV2::Stack { id, .. }
        | LayoutNodeV2::Widget { id, .. } => id,
    }
}

fn validate_surface_blueprint(blueprint: &BlueprintV2) -> Result<(), SurfaceValidationError> {
    if blueprint.version != SURFACE_PROTOCOL_MAJOR {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::InvalidVersion,
            format!("blueprint version must be {}", SURFACE_PROTOCOL_MAJOR),
        ));
    }
    if blueprint.widgets.len() > SURFACE_MAX_WIDGET_INSTANCES {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::ResourceLimit,
            "widget instance count exceeds the authority limit",
        ));
    }
    let mut widget_ids = BTreeMap::new();
    for widget in &blueprint.widgets {
        validate_surface_identifier(&widget.id, "widget instance id")?;
        if widget.widget_type.is_empty() || widget.widget_type.len() > SURFACE_MAX_IDENTIFIER_LENGTH
        {
            return Err(SurfaceValidationError::new(
                SurfaceErrorCode::InvalidBlueprint,
                "widget type is not a bounded non-empty string",
            ));
        }
        if widget_ids.insert(widget.id.clone(), ()).is_some() {
            return Err(SurfaceValidationError::new(
                SurfaceErrorCode::DuplicateInstanceId,
                format!("duplicate widget instance id {:?}", widget.id),
            ));
        }
        if let Some(props) = &widget.props {
            if let Some(field) = surface_json_has_forbidden_field(props) {
                return Err(SurfaceValidationError::new(
                    SurfaceErrorCode::ForbiddenExecutableField,
                    format!("forbidden executable field {field:?}"),
                ));
            }
        }
    }

    let mut nodes = 0;
    let mut node_ids = BTreeMap::new();
    let mut widget_refs = BTreeMap::new();
    validate_surface_node(
        &blueprint.root,
        1,
        &mut nodes,
        &mut node_ids,
        &mut widget_refs,
        &widget_ids,
    )?;
    if widget_refs.len() != widget_ids.len() {
        if let Some(orphan) = widget_ids.keys().find(|id| !widget_refs.contains_key(*id)) {
            return Err(SurfaceValidationError::new(
                SurfaceErrorCode::InvalidReference,
                format!("widget instance {orphan:?} is not placed in the layout"),
            ));
        }
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::InvalidReference,
            "layout widget references do not match declared instances",
        ));
    }
    Ok(())
}

/// Validate a typed v2 snapshot against the authority and its resource limits.
pub fn validate_surface_snapshot(snapshot: &SurfaceSnapshot) -> Result<(), SurfaceValidationError> {
    let bytes = serde_json::to_vec(snapshot).map_err(|error| {
        SurfaceValidationError::new(
            SurfaceErrorCode::InvalidBlueprint,
            format!("snapshot is not serializable JSON: {error}"),
        )
    })?;
    if bytes.len() > SURFACE_MAX_DOCUMENT_BYTES {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::DocumentTooLarge,
            "snapshot exceeds the document byte limit",
        ));
    }
    if snapshot.kind != SurfaceMessageKind::Snapshot {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::InvalidVersion,
            "snapshot kind must be snapshot",
        ));
    }
    validate_surface_version(&snapshot.protocol)?;
    validate_surface_identifier(&snapshot.surface_id, "surface id")?;
    validate_surface_blueprint(&snapshot.blueprint)
}

/// Parse and validate an untrusted JSON snapshot, including unknown-field
/// security checks before serde drops additive fields.
pub fn validate_surface_snapshot_json(
    raw: &Value,
) -> Result<SurfaceSnapshot, SurfaceValidationError> {
    let bytes = serde_json::to_vec(raw).map_err(|error| {
        SurfaceValidationError::new(
            SurfaceErrorCode::InvalidBlueprint,
            format!("snapshot is not serializable JSON: {error}"),
        )
    })?;
    if bytes.len() > SURFACE_MAX_DOCUMENT_BYTES {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::DocumentTooLarge,
            "snapshot exceeds the document byte limit",
        ));
    }
    if let Some(field) = surface_json_has_forbidden_field(raw) {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::ForbiddenExecutableField,
            format!("forbidden executable field {field:?}"),
        ));
    }
    validate_surface_version_json(raw)?;
    validate_surface_revision_json(raw, "revision")?;
    let snapshot: SurfaceSnapshot = serde_json::from_value(raw.clone()).map_err(|error| {
        SurfaceValidationError::new(
            SurfaceErrorCode::InvalidBlueprint,
            format!("snapshot shape is invalid: {error}"),
        )
    })?;
    validate_surface_snapshot(&snapshot)?;
    Ok(snapshot)
}

/// Validate a typed v2 patch event. The base/new revisions must be adjacent;
/// callers still compare `base_revision` with their local revision before
/// attempting to apply it.
pub fn validate_surface_patch_event(
    event: &SurfacePatchEvent,
) -> Result<(), SurfaceValidationError> {
    if event.kind != SurfaceMessageKind::Patch {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::InvalidVersion,
            "patch event kind must be patch",
        ));
    }
    validate_surface_version(&event.protocol)?;
    validate_surface_identifier(&event.surface_id, "surface id")?;
    if event.patch.len() > SURFACE_MAX_PATCH_OPERATIONS {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::ResourceLimit,
            "patch operation count exceeds the authority limit",
        ));
    }
    if event.base_revision.value().checked_add(1) != Some(event.revision.value()) {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::RevisionGap,
            "patch revision must be exactly base revision plus one",
        ));
    }
    for op in &event.patch {
        validate_surface_patch_op(op)?;
    }
    let bytes = serde_json::to_vec(event).map_err(|error| {
        SurfaceValidationError::new(
            SurfaceErrorCode::InvalidPatch,
            format!("patch is not serializable JSON: {error}"),
        )
    })?;
    if bytes.len() > SURFACE_MAX_PATCH_BYTES {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::DocumentTooLarge,
            "patch exceeds the patch byte limit",
        ));
    }
    Ok(())
}

/// Parse and validate an untrusted JSON patch event.
pub fn validate_surface_patch_event_json(
    raw: &Value,
) -> Result<SurfacePatchEvent, SurfaceValidationError> {
    let bytes = serde_json::to_vec(raw).map_err(|error| {
        SurfaceValidationError::new(
            SurfaceErrorCode::InvalidPatch,
            format!("patch event is not serializable JSON: {error}"),
        )
    })?;
    if bytes.len() > SURFACE_MAX_PATCH_BYTES {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::DocumentTooLarge,
            "patch exceeds the patch byte limit",
        ));
    }
    if let Some(field) = surface_json_has_forbidden_field(raw) {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::ForbiddenExecutableField,
            format!("forbidden executable field {field:?}"),
        ));
    }
    validate_surface_version_json(raw)?;
    validate_surface_revision_json(raw, "baseRevision")?;
    validate_surface_revision_json(raw, "revision")?;
    let event: SurfacePatchEvent = serde_json::from_value(raw.clone()).map_err(|error| {
        SurfaceValidationError::new(
            SurfaceErrorCode::InvalidPatch,
            format!("patch event shape is invalid: {error}"),
        )
    })?;
    validate_surface_patch_event(&event)?;
    Ok(event)
}

/// Convert the existing v1 Blueprint into a deterministic v2 default layout.
/// This is an explicit migration helper for the dedicated migration boundary;
/// v2 validators intentionally do not accept v1 objects themselves.
pub fn migrate_v1_blueprint(blueprint: &Blueprint) -> Result<BlueprintV2, SurfaceValidationError> {
    let widgets = blueprint
        .widgets
        .iter()
        .map(|widget| WidgetInstanceV2 {
            id: widget.id.clone(),
            widget_type: widget.kind.clone(),
            props: widget.props.clone(),
        })
        .collect::<Vec<_>>();

    let mut area_nodes = Vec::new();
    for (area_index, area) in blueprint.layout.areas.iter().enumerate() {
        let children = area
            .widgets
            .iter()
            .enumerate()
            .map(|(widget_index, instance_id)| LayoutNodeV2::Widget {
                id: format!("legacy-area-{area_index}-widget-{widget_index}"),
                instance_id: instance_id.clone(),
            })
            .collect::<Vec<_>>();
        if children.is_empty() {
            continue;
        }
        let node = if children.len() == 1 {
            children.into_iter().next().expect("one child")
        } else {
            LayoutNodeV2::Stack {
                id: format!("legacy-area-{area_index}"),
                children,
            }
        };
        area_nodes.push(node);
    }

    let Some(root) = area_nodes.into_iter().reduce(|left, right| match left {
        LayoutNodeV2::Stack { id, mut children } if id == "legacy-root" => {
            children.push(right);
            LayoutNodeV2::Stack { id, children }
        }
        left => LayoutNodeV2::Stack {
            id: "legacy-root".into(),
            children: vec![left, right],
        },
    }) else {
        return Err(SurfaceValidationError::new(
            SurfaceErrorCode::InvalidBlueprint,
            "v1 blueprint has no non-empty area to migrate",
        ));
    };

    let migrated = BlueprintV2 {
        version: SURFACE_PROTOCOL_MAJOR,
        root,
        widgets,
    };
    validate_surface_blueprint(&migrated)?;
    Ok(migrated)
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
/// The AIRP engine bridge is the default implementation; any type implementing
/// this trait can replace it. The UI host sends upstream envelopes via
/// [`dispatch`] and renders the downstream stream from [`subscribe`].
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
            "engine",
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
            "src": "engine",
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
            Body::Blueprint(BlueprintMsg {
                op: SetOrPatch::Set,
                blueprint: Some(bp),
                ..
            }) => {
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
            "engine",
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

    #[test]
    fn rust_wire_discriminants_match_shared_contract() {
        let contract: Value =
            serde_json::from_str(include_str!("../wire-discriminants.json")).unwrap();

        let bodies = vec![
            Body::Blueprint(BlueprintMsg {
                op: SetOrPatch::Set,
                blueprint: None,
                patch: None,
            }),
            Body::State(StateMsg {
                scope: "session".into(),
                op: SetOrPatch::Set,
                state: Some(Value::Null),
                patch: None,
            }),
            Body::Manifest(ManifestMsg {
                op: SetOrPatch::Set,
                manifests: vec![],
            }),
            Body::Event(EventMsg {
                topic: "test".into(),
                data: None,
            }),
            Body::Error(ErrorMsg {
                code: "test".into(),
                message: "test".into(),
                detail: None,
            }),
            Body::Intent(IntentMsg {
                name: "test".into(),
                source: None,
                params: None,
            }),
            Body::Subscribe(SubscribeMsg { scopes: vec![] }),
            Body::Unsubscribe(SubscribeMsg { scopes: vec![] }),
            Body::Hello(HelloMsg {
                client: "test".into(),
                version: "1".into(),
                accept: None,
            }),
            Body::Ack(AckMsg {
                ref_: "test".into(),
            }),
        ];
        let body_kinds: Vec<Value> = bodies
            .into_iter()
            .map(|body| serde_json::to_value(body).unwrap()["kind"].clone())
            .collect();
        assert_eq!(json!(body_kinds), contract["bodyKinds"]);

        assert_eq!(
            json!([SetOrPatch::Set, SetOrPatch::Patch,]),
            contract["setOrPatch"]
        );
        assert_eq!(
            json!([
                PatchOpKind::Add,
                PatchOpKind::Remove,
                PatchOpKind::Replace,
                PatchOpKind::Move,
                PatchOpKind::Copy,
                PatchOpKind::Test,
            ]),
            contract["patchOps"]
        );
        assert_eq!(
            json!([
                Capability::ReadMemory,
                Capability::WriteMemory,
                Capability::ReadWorldbook,
                Capability::ReadState,
                Capability::WriteState,
                Capability::CallTool,
            ]),
            contract["capabilities"]
        );
        assert_eq!(
            json!([EntryKind::Builtin, EntryKind::Esm]),
            contract["entryKinds"]
        );
        assert_eq!(
            json!([
                LayoutKind::Dock,
                LayoutKind::Grid,
                LayoutKind::Stack,
                LayoutKind::Tabs,
            ]),
            contract["layoutKinds"]
        );
    }

    #[test]
    fn surface_authority_matches_binding_constants() {
        let authority: Value =
            serde_json::from_str(include_str!("../surface-protocol-v2.json")).unwrap();
        assert_eq!(
            authority["protocol"]["major"],
            json!(SURFACE_PROTOCOL_MAJOR)
        );
        assert_eq!(
            authority["protocol"]["minor"],
            json!(SURFACE_PROTOCOL_MINOR)
        );
        assert_eq!(
            authority["$defs"]["protocolVersion"]["properties"]["minor"]["maximum"],
            json!(SURFACE_PROTOCOL_COMPONENT_MAX)
        );
        assert_eq!(
            authority["resourceLimits"]["maxDocumentBytes"],
            json!(SURFACE_MAX_DOCUMENT_BYTES)
        );
        assert_eq!(
            authority["resourceLimits"]["maxPatchBytes"],
            json!(SURFACE_MAX_PATCH_BYTES)
        );
        assert_eq!(
            authority["resourceLimits"]["maxPatchOperations"],
            json!(SURFACE_MAX_PATCH_OPERATIONS)
        );
        assert_eq!(
            authority["resourceLimits"]["maxBlueprintDepth"],
            json!(SURFACE_MAX_BLUEPRINT_DEPTH)
        );
        assert_eq!(
            authority["resourceLimits"]["maxBlueprintNodes"],
            json!(SURFACE_MAX_BLUEPRINT_NODES)
        );
        let codes = authority["errors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["code"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(codes, SURFACE_ERROR_CODES);
        assert_eq!(
            authority["unknownFields"]["forbiddenFields"],
            json!(SURFACE_FORBIDDEN_FIELDS)
        );
    }

    #[test]
    fn rust_snapshot_serializes_exactly_to_rust_to_ts_fixture() {
        let expected: Value =
            serde_json::from_str(include_str!("../fixtures/surface-v2/rust-to-ts.json")).unwrap();
        let snapshot = SurfaceSnapshot {
            kind: SurfaceMessageKind::Snapshot,
            protocol: SurfaceProtocolVersion::default(),
            surface_id: "story".into(),
            revision: SurfaceRevision::new(42),
            blueprint: BlueprintV2 {
                version: 2,
                root: LayoutNodeV2::Split {
                    id: "root".into(),
                    orientation: SplitOrientation::Horizontal,
                    children: vec![
                        LayoutNodeV2::Tabs {
                            id: "tabs".into(),
                            active: "chat-node".into(),
                            children: vec![
                                LayoutNodeV2::Widget {
                                    id: "chat-node".into(),
                                    instance_id: "chat-1".into(),
                                },
                                LayoutNodeV2::Widget {
                                    id: "tools-node".into(),
                                    instance_id: "tools-1".into(),
                                },
                            ],
                        },
                        LayoutNodeV2::Stack {
                            id: "stack".into(),
                            children: vec![LayoutNodeV2::Widget {
                                id: "notes-node".into(),
                                instance_id: "notes-1".into(),
                            }],
                        },
                    ],
                },
                widgets: vec![
                    WidgetInstanceV2 {
                        id: "chat-1".into(),
                        widget_type: "core.chat".into(),
                        props: Some(json!({ "mode": "story" })),
                    },
                    WidgetInstanceV2 {
                        id: "tools-1".into(),
                        widget_type: "core.tools".into(),
                        props: None,
                    },
                    WidgetInstanceV2 {
                        id: "notes-1".into(),
                        widget_type: "core.notes".into(),
                        props: Some(json!({ "pinned": true })),
                    },
                ],
            },
        };
        validate_surface_snapshot(&snapshot).unwrap();
        assert_eq!(serde_json::to_value(&snapshot).unwrap(), expected);
        let parsed = validate_surface_snapshot_json(&expected).unwrap();
        assert_eq!(parsed, snapshot);
    }

    #[test]
    fn rust_accepts_ts_to_rust_patch_fixture_and_preserves_decimal_revision() {
        let expected: Value =
            serde_json::from_str(include_str!("../fixtures/surface-v2/ts-to-rust.json")).unwrap();
        let event: SurfacePatchEvent = serde_json::from_value(expected.clone()).unwrap();
        validate_surface_patch_event(&event).unwrap();
        assert_eq!(serde_json::to_value(&event).unwrap(), expected);
        assert_eq!(
            serde_json::to_value(SurfaceRevision::new(u64::MAX)).unwrap(),
            json!(u64::MAX.to_string())
        );
        assert!(SurfaceRevision::parse("01").is_err());
        assert!(SurfaceRevision::parse("18446744073709551616").is_err());
    }

    #[test]
    fn surface_negative_fixtures_fail_closed() {
        let negative: Value =
            serde_json::from_str(include_str!("../fixtures/surface-v2/negative.json")).unwrap();
        assert_eq!(
            validate_surface_snapshot_json(&negative["unknownMajor"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::UnsupportedMajor
        );
        assert_eq!(
            validate_surface_snapshot_json(&negative["minorOverflow"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidVersion
        );
        assert_eq!(
            validate_surface_snapshot_json(&negative["invalidRevision"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidRevision
        );
        assert_eq!(
            validate_surface_snapshot_json(&negative["duplicateInstance"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::DuplicateInstanceId
        );
        assert_eq!(
            validate_surface_snapshot_json(&negative["invalidReference"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidReference
        );
        assert_eq!(
            validate_surface_snapshot_json(&negative["orphanInstance"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidReference
        );
        assert_eq!(
            validate_surface_snapshot_json(&negative["forbiddenExecutableField"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::ForbiddenExecutableField
        );
        assert_eq!(
            validate_surface_patch_event_json(&negative["revisionGap"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::RevisionGap
        );
        assert_eq!(
            validate_surface_patch_event_json(&negative["revisionOverflow"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::RevisionGap
        );
        assert_eq!(
            validate_surface_patch_event_json(&negative["invalidPatchRevision"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidRevision
        );
        assert_eq!(
            validate_surface_patch_event_json(&negative["rootReplacement"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidPatch
        );
        assert_eq!(
            validate_surface_patch_event_json(&negative["badPatch"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidPatch
        );
        assert_eq!(
            validate_surface_patch_event_json(&negative["invalidPointerEscape"])
                .unwrap_err()
                .code,
            SurfaceErrorCode::InvalidPatch
        );
    }

    #[test]
    fn surface_depth_and_document_limits_are_enforced() {
        let mut node = LayoutNodeV2::Widget {
            id: "leaf".into(),
            instance_id: "w-1".into(),
        };
        for depth in 0..SURFACE_MAX_BLUEPRINT_DEPTH {
            node = LayoutNodeV2::Stack {
                id: format!("deep-{depth}"),
                children: vec![node],
            };
        }
        let snapshot = SurfaceSnapshot {
            kind: SurfaceMessageKind::Snapshot,
            protocol: SurfaceProtocolVersion::default(),
            surface_id: "story".into(),
            revision: SurfaceRevision::new(42),
            blueprint: BlueprintV2 {
                version: 2,
                root: node,
                widgets: vec![WidgetInstanceV2 {
                    id: "w-1".into(),
                    widget_type: "core.chat".into(),
                    props: None,
                }],
            },
        };
        assert_eq!(
            validate_surface_snapshot(&snapshot).unwrap_err().code,
            SurfaceErrorCode::ResourceLimit
        );

        let mut raw = serde_json::to_value(&SurfaceSnapshot {
            kind: SurfaceMessageKind::Snapshot,
            protocol: SurfaceProtocolVersion::default(),
            surface_id: "story".into(),
            revision: SurfaceRevision::new(42),
            blueprint: BlueprintV2 {
                version: 2,
                root: LayoutNodeV2::Widget {
                    id: "w-node".into(),
                    instance_id: "w-1".into(),
                },
                widgets: vec![WidgetInstanceV2 {
                    id: "w-1".into(),
                    widget_type: "core.chat".into(),
                    props: None,
                }],
            },
        })
        .unwrap();
        raw["opaque"] = Value::String("x".repeat(SURFACE_MAX_DOCUMENT_BYTES));
        assert_eq!(
            validate_surface_snapshot_json(&raw).unwrap_err().code,
            SurfaceErrorCode::DocumentTooLarge
        );

        let typed_oversized_snapshot = SurfaceSnapshot {
            kind: SurfaceMessageKind::Snapshot,
            protocol: SurfaceProtocolVersion::default(),
            surface_id: "story".into(),
            revision: SurfaceRevision::new(42),
            blueprint: BlueprintV2 {
                version: 2,
                root: LayoutNodeV2::Widget {
                    id: "w-node".into(),
                    instance_id: "w-1".into(),
                },
                widgets: vec![WidgetInstanceV2 {
                    id: "w-1".into(),
                    widget_type: "core.chat".into(),
                    props: Some(json!({ "opaque": "x".repeat(SURFACE_MAX_DOCUMENT_BYTES) })),
                }],
            },
        };
        assert_eq!(
            validate_surface_snapshot(&typed_oversized_snapshot)
                .unwrap_err()
                .code,
            SurfaceErrorCode::DocumentTooLarge
        );

        let widgets = (0..128)
            .map(|index| WidgetInstanceV2 {
                id: format!("w-{index}"),
                widget_type: "core.chat".into(),
                props: None,
            })
            .collect::<Vec<_>>();
        let mut groups = Vec::new();
        for group_index in 0..4 {
            let children = (0..32)
                .map(|chain_index| {
                    let index = group_index * 32 + chain_index;
                    let mut chain = LayoutNodeV2::Widget {
                        id: format!("node-{index}"),
                        instance_id: format!("w-{index}"),
                    };
                    for level in 0..8 {
                        chain = LayoutNodeV2::Stack {
                            id: format!("chain-{index}-{level}"),
                            children: vec![chain],
                        };
                    }
                    chain
                })
                .collect::<Vec<_>>();
            groups.push(LayoutNodeV2::Stack {
                id: format!("group-{group_index}"),
                children,
            });
        }
        let node_limit_snapshot = SurfaceSnapshot {
            kind: SurfaceMessageKind::Snapshot,
            protocol: SurfaceProtocolVersion::default(),
            surface_id: "story".into(),
            revision: SurfaceRevision::new(42),
            blueprint: BlueprintV2 {
                version: 2,
                root: LayoutNodeV2::Stack {
                    id: "node-limit-root".into(),
                    children: groups,
                },
                widgets,
            },
        };
        assert_eq!(
            validate_surface_snapshot(&node_limit_snapshot)
                .unwrap_err()
                .code,
            SurfaceErrorCode::ResourceLimit
        );

        let patch_limit_event = SurfacePatchEvent {
            kind: SurfaceMessageKind::Patch,
            protocol: SurfaceProtocolVersion::default(),
            surface_id: "story".into(),
            base_revision: SurfaceRevision::new(42),
            revision: SurfaceRevision::new(43),
            patch: (0..=SURFACE_MAX_PATCH_OPERATIONS)
                .map(|_| SurfacePatchOp {
                    op: SurfacePatchOperation::Test,
                    path: "/blueprint/root".into(),
                    value: Some(json!(null)),
                    from: None,
                })
                .collect(),
        };
        assert_eq!(
            validate_surface_patch_event(&patch_limit_event)
                .unwrap_err()
                .code,
            SurfaceErrorCode::ResourceLimit
        );
    }

    #[test]
    fn v1_migration_is_explicit_and_v2_validator_rejects_v1() {
        let fixture: Value =
            serde_json::from_str(include_str!("../fixtures/surface-v2/v1-migration.json")).unwrap();
        let v1: Blueprint = serde_json::from_value(fixture["v1"].clone()).unwrap();
        let migrated = migrate_v1_blueprint(&v1).unwrap();
        assert_eq!(
            serde_json::to_value(&migrated).unwrap(),
            fixture["expectedV2"]
        );
        assert!(serde_json::from_value::<BlueprintV2>(fixture["v1"].clone()).is_err());
    }
}
