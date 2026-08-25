use std::{
    collections::{HashMap, VecDeque},
    path::Path,
};

use airp_state_protocol::{
    validate_surface_patch_event, validate_surface_snapshot, BlueprintV2, LayoutNodeV2,
    SplitOrientation, SurfaceMessageKind, SurfacePatchEvent, SurfacePatchOp, SurfacePatchOperation,
    SurfaceProtocolVersion, SurfaceRevision, SurfaceSnapshot, SurfaceValidationError,
    WidgetInstanceV2, SURFACE_PROTOCOL_MAJOR,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const DEFAULT_SURFACE_RING_EVENTS: usize = 256;
pub const DEFAULT_SURFACE_RING_BYTES: usize = 1_048_576;
pub const DEFAULT_SURFACE_ENTRIES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SurfaceScope {
    data_root: String,
    character_id: String,
    session_id: String,
    user_id: Option<String>,
}

impl SurfaceScope {
    pub fn new(
        data_root: &Path,
        character_id: impl Into<String>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            data_root: data_root.to_string_lossy().into_owned(),
            character_id: character_id.into(),
            session_id: session_id.into(),
            user_id: None,
        }
    }

    pub fn for_user(
        data_root: &Path,
        character_id: impl Into<String>,
        session_id: impl Into<String>,
        user_id: Option<String>,
    ) -> Self {
        Self {
            data_root: data_root.to_string_lossy().into_owned(),
            character_id: character_id.into(),
            session_id: session_id.into(),
            user_id,
        }
    }

    pub fn character_id(&self) -> &str {
        &self.character_id
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    pub fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionSurfaceProps {
    pub chat: Value,
    pub memory: Value,
    pub character_state: Value,
    pub activity: Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SurfaceCursor(String);

impl SurfaceCursor {
    pub fn from_opaque(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SurfaceCursor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceMessage {
    Snapshot(SurfaceSnapshot),
    Patch(SurfacePatchEvent),
}

#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceEvent {
    pub cursor: SurfaceCursor,
    pub message: SurfaceMessage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SurfacePublish {
    Changed(SurfaceEvent),
    Unchanged {
        revision: SurfaceRevision,
        cursor: SurfaceCursor,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum SurfaceReplay {
    Snapshot(SurfaceEvent),
    Events(Vec<SurfaceEvent>),
}

#[derive(Debug)]
pub enum SurfaceRegistryError {
    InvalidScope(&'static str),
    UnknownScope,
    AmbiguousSurface,
    UnknownInstance,
    RevisionExhausted,
    Validation(SurfaceValidationError),
}

impl std::fmt::Display for SurfaceRegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidScope(field) => write!(formatter, "surface {field} must not be empty"),
            Self::UnknownScope => formatter.write_str("surface scope is not registered"),
            Self::AmbiguousSurface => formatter.write_str("surface id resolves to multiple scopes"),
            Self::UnknownInstance => {
                formatter.write_str("widget instance is not in the accepted surface")
            }
            Self::RevisionExhausted => formatter.write_str("surface revision is exhausted"),
            Self::Validation(error) => write!(formatter, "surface validation failed: {error}"),
        }
    }
}

impl std::error::Error for SurfaceRegistryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Validation(error) => Some(error),
            Self::InvalidScope(_)
            | Self::UnknownScope
            | Self::AmbiguousSurface
            | Self::UnknownInstance
            | Self::RevisionExhausted => None,
        }
    }
}

impl From<SurfaceValidationError> for SurfaceRegistryError {
    fn from(error: SurfaceValidationError) -> Self {
        Self::Validation(error)
    }
}

pub fn build_session_surface(
    scope: &SurfaceScope,
    revision: SurfaceRevision,
    props: SessionSurfaceProps,
) -> Result<SurfaceSnapshot, SurfaceRegistryError> {
    validate_scope(scope)?;
    let widgets = vec![
        widget("chat", "core.chat", props.chat),
        widget("memory", "core.memory", props.memory),
        widget(
            "character-state",
            "core.character-state",
            props.character_state,
        ),
        widget("activity", "core.activity", props.activity),
    ];
    let snapshot = SurfaceSnapshot {
        kind: SurfaceMessageKind::Snapshot,
        protocol: SurfaceProtocolVersion::default(),
        surface_id: format!("session:{}", scope.session_id),
        revision,
        blueprint: BlueprintV2 {
            version: SURFACE_PROTOCOL_MAJOR,
            root: LayoutNodeV2::Split {
                id: "session-root".into(),
                orientation: SplitOrientation::Horizontal,
                children: vec![
                    LayoutNodeV2::Tabs {
                        id: "session-tabs".into(),
                        active: "chat-node".into(),
                        children: vec![
                            widget_node("chat-node", "chat"),
                            widget_node("memory-node", "memory"),
                        ],
                    },
                    LayoutNodeV2::Stack {
                        id: "session-context".into(),
                        children: vec![
                            widget_node("character-state-node", "character-state"),
                            widget_node("activity-node", "activity"),
                        ],
                    },
                ],
            },
            widgets,
        },
    };
    validate_surface_snapshot(&snapshot)?;
    Ok(snapshot)
}

fn validate_scope(scope: &SurfaceScope) -> Result<(), SurfaceRegistryError> {
    if scope.data_root.is_empty() {
        return Err(SurfaceRegistryError::InvalidScope("data_root"));
    }
    if scope.character_id.is_empty() {
        return Err(SurfaceRegistryError::InvalidScope("character_id"));
    }
    if scope.session_id.is_empty() {
        return Err(SurfaceRegistryError::InvalidScope("session_id"));
    }
    if scope.user_id.as_deref().is_some_and(str::is_empty) {
        return Err(SurfaceRegistryError::InvalidScope("user_id"));
    }
    Ok(())
}

fn widget(id: &str, widget_type: &str, props: Value) -> WidgetInstanceV2 {
    WidgetInstanceV2 {
        id: id.into(),
        widget_type: widget_type.into(),
        props: Some(props),
    }
}

fn widget_node(id: &str, instance_id: &str) -> LayoutNodeV2 {
    LayoutNodeV2::Widget {
        id: id.into(),
        instance_id: instance_id.into(),
    }
}

#[derive(Clone)]
struct SurfaceEntry {
    snapshot: SurfaceSnapshot,
    cursor: SurfaceCursor,
    sequence: u64,
    last_touched: u64,
}

struct RingEvent {
    scope: SurfaceScope,
    sequence: u64,
    bytes: usize,
    event: SurfaceEvent,
}

pub struct SurfaceRegistry {
    boot_id: String,
    entries: HashMap<SurfaceScope, SurfaceEntry>,
    ring: VecDeque<RingEvent>,
    ring_bytes: usize,
    max_entries: usize,
    max_events: usize,
    max_bytes: usize,
    touch_clock: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceIntentTarget {
    pub scope: SurfaceScope,
    pub widget_type: String,
}

impl Default for SurfaceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceRegistry {
    pub fn new() -> Self {
        Self::with_all_limits(
            DEFAULT_SURFACE_ENTRIES,
            DEFAULT_SURFACE_RING_EVENTS,
            DEFAULT_SURFACE_RING_BYTES,
        )
    }

    pub fn with_limits(max_events: usize, max_bytes: usize) -> Self {
        Self::with_all_limits(DEFAULT_SURFACE_ENTRIES, max_events, max_bytes)
    }

    pub fn with_all_limits(max_entries: usize, max_events: usize, max_bytes: usize) -> Self {
        Self {
            boot_id: uuid::Uuid::new_v4().simple().to_string(),
            entries: HashMap::new(),
            ring: VecDeque::new(),
            ring_bytes: 0,
            max_entries: max_entries.max(1),
            max_events,
            max_bytes,
            touch_clock: 0,
        }
    }

    pub fn publish(
        &mut self,
        scope: SurfaceScope,
        props: SessionSurfaceProps,
    ) -> Result<SurfacePublish, SurfaceRegistryError> {
        validate_scope(&scope)?;
        let last_touched = self.next_touch()?;
        let Some(previous) = self.entries.get(&scope).cloned() else {
            self.evict_for_insert();
            let sequence = 1;
            let snapshot = build_session_surface(&scope, SurfaceRevision::new(1), props)?;
            let cursor = self.cursor(&scope, sequence);
            let event = SurfaceEvent {
                cursor: cursor.clone(),
                message: SurfaceMessage::Snapshot(snapshot.clone()),
            };
            validate_message(&event.message)?;
            self.entries.insert(
                scope.clone(),
                SurfaceEntry {
                    snapshot,
                    cursor,
                    sequence,
                    last_touched,
                },
            );
            self.push_ring(scope, sequence, event.clone());
            return Ok(SurfacePublish::Changed(event));
        };

        let mut next = build_session_surface(&scope, previous.snapshot.revision, props)?;
        if previous.snapshot.blueprint == next.blueprint {
            if let Some(entry) = self.entries.get_mut(&scope) {
                entry.last_touched = last_touched;
            }
            return Ok(SurfacePublish::Unchanged {
                revision: previous.snapshot.revision,
                cursor: previous.cursor,
            });
        }
        let revision = previous
            .snapshot
            .revision
            .value()
            .checked_add(1)
            .ok_or(SurfaceRegistryError::RevisionExhausted)?;
        next.revision = SurfaceRevision::new(revision);
        validate_surface_snapshot(&next)?;

        let sequence = previous
            .sequence
            .checked_add(1)
            .ok_or(SurfaceRegistryError::RevisionExhausted)?;
        let cursor = self.cursor(&scope, sequence);
        let message = match props_patch(&previous.snapshot, &next) {
            Some(patch) => match validate_surface_patch_event(&patch) {
                Ok(()) => SurfaceMessage::Patch(patch),
                Err(error)
                    if error.code == airp_state_protocol::SurfaceErrorCode::DocumentTooLarge =>
                {
                    SurfaceMessage::Snapshot(next.clone())
                }
                Err(error) => return Err(error.into()),
            },
            None => SurfaceMessage::Snapshot(next.clone()),
        };
        let event = SurfaceEvent {
            cursor: cursor.clone(),
            message,
        };
        validate_message(&event.message)?;
        self.entries.insert(
            scope.clone(),
            SurfaceEntry {
                snapshot: next,
                cursor,
                sequence,
                last_touched,
            },
        );
        self.push_ring(scope, sequence, event.clone());
        Ok(SurfacePublish::Changed(event))
    }

    pub fn replay(
        &self,
        scope: &SurfaceScope,
        after: Option<&SurfaceCursor>,
    ) -> Result<SurfaceReplay, SurfaceRegistryError> {
        let entry = self
            .entries
            .get(scope)
            .ok_or(SurfaceRegistryError::UnknownScope)?;
        let Some(after) = after else {
            return self.snapshot_replay(entry);
        };
        let Some(sequence) = self.cursor_sequence(scope, after) else {
            return self.snapshot_replay(entry);
        };
        if sequence > entry.sequence {
            return self.snapshot_replay(entry);
        }
        if sequence == entry.sequence {
            return Ok(SurfaceReplay::Events(Vec::new()));
        }

        let events = self
            .ring
            .iter()
            .filter(|item| item.scope == *scope && item.sequence > sequence)
            .map(|item| item.event.clone())
            .collect::<Vec<_>>();
        let complete = events
            .first()
            .and_then(|event| self.cursor_sequence(scope, &event.cursor))
            == sequence.checked_add(1)
            && events
                .last()
                .and_then(|event| self.cursor_sequence(scope, &event.cursor))
                == Some(entry.sequence);
        if !complete {
            return self.snapshot_replay(entry);
        }
        for event in &events {
            validate_message(&event.message)?;
        }
        Ok(SurfaceReplay::Events(events))
    }

    pub fn current(&self, scope: &SurfaceScope) -> Result<SurfaceEvent, SurfaceRegistryError> {
        let entry = self
            .entries
            .get(scope)
            .ok_or(SurfaceRegistryError::UnknownScope)?;
        let event = SurfaceEvent {
            cursor: entry.cursor.clone(),
            message: SurfaceMessage::Snapshot(entry.snapshot.clone()),
        };
        validate_message(&event.message)?;
        Ok(event)
    }

    /// Resolve an intent against the exact host-accepted Surface snapshot.
    /// Surface ids are presentation identifiers rather than tenant keys, so a
    /// duplicate id across registered scopes is rejected instead of guessed.
    pub fn resolve_intent_target(
        &self,
        surface_id: &str,
        instance_id: &str,
    ) -> Result<SurfaceIntentTarget, SurfaceRegistryError> {
        let mut matches = self.entries.iter().filter_map(|(scope, entry)| {
            (entry.snapshot.surface_id == surface_id).then_some((scope, entry))
        });
        let Some((scope, entry)) = matches.next() else {
            return Err(SurfaceRegistryError::UnknownScope);
        };
        if matches.next().is_some() {
            return Err(SurfaceRegistryError::AmbiguousSurface);
        }
        let widget = entry
            .snapshot
            .blueprint
            .widgets
            .iter()
            .find(|widget| widget.id == instance_id)
            .ok_or(SurfaceRegistryError::UnknownInstance)?;
        Ok(SurfaceIntentTarget {
            scope: scope.clone(),
            widget_type: widget.widget_type.clone(),
        })
    }

    pub fn ring_event_count(&self) -> usize {
        self.ring.len()
    }

    pub fn ring_total_bytes(&self) -> usize {
        self.ring_bytes
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    fn next_touch(&mut self) -> Result<u64, SurfaceRegistryError> {
        self.touch_clock = self
            .touch_clock
            .checked_add(1)
            .ok_or(SurfaceRegistryError::RevisionExhausted)?;
        Ok(self.touch_clock)
    }

    fn evict_for_insert(&mut self) {
        if self.entries.len() < self.max_entries {
            return;
        }
        let Some(evicted) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.last_touched)
            .map(|(scope, _)| scope.clone())
        else {
            return;
        };
        self.entries.remove(&evicted);
        self.ring.retain(|event| event.scope != evicted);
        self.ring_bytes = self.ring.iter().map(|event| event.bytes).sum();
    }

    fn snapshot_replay(&self, entry: &SurfaceEntry) -> Result<SurfaceReplay, SurfaceRegistryError> {
        let event = SurfaceEvent {
            cursor: entry.cursor.clone(),
            message: SurfaceMessage::Snapshot(entry.snapshot.clone()),
        };
        validate_message(&event.message)?;
        Ok(SurfaceReplay::Snapshot(event))
    }

    fn cursor(&self, scope: &SurfaceScope, sequence: u64) -> SurfaceCursor {
        SurfaceCursor(format!(
            "surface-v1.{}.{}.{sequence:016x}",
            self.boot_id,
            scope_fingerprint(scope)
        ))
    }

    fn cursor_sequence(&self, scope: &SurfaceScope, cursor: &SurfaceCursor) -> Option<u64> {
        let mut parts = cursor.0.split('.');
        if parts.next()? != "surface-v1"
            || parts.next()? != self.boot_id
            || parts.next()? != scope_fingerprint(scope)
        {
            return None;
        }
        let sequence = u64::from_str_radix(parts.next()?, 16).ok()?;
        parts.next().is_none().then_some(sequence)
    }

    fn push_ring(&mut self, scope: SurfaceScope, sequence: u64, event: SurfaceEvent) {
        let bytes = event_bytes(&event);
        self.ring_bytes = self.ring_bytes.saturating_add(bytes);
        self.ring.push_back(RingEvent {
            scope,
            sequence,
            bytes,
            event,
        });
        while self.ring.len() > self.max_events || self.ring_bytes > self.max_bytes {
            let Some(removed) = self.ring.pop_front() else {
                break;
            };
            self.ring_bytes = self.ring_bytes.saturating_sub(removed.bytes);
        }
    }
}

fn props_patch(previous: &SurfaceSnapshot, next: &SurfaceSnapshot) -> Option<SurfacePatchEvent> {
    let aligned = previous.blueprint.version == next.blueprint.version
        && previous.blueprint.root == next.blueprint.root
        && previous.blueprint.widgets.len() == next.blueprint.widgets.len()
        && previous
            .blueprint
            .widgets
            .iter()
            .zip(&next.blueprint.widgets)
            .all(|(before, after)| {
                before.id == after.id && before.widget_type == after.widget_type
            });
    if !aligned {
        return None;
    }
    let patch = previous
        .blueprint
        .widgets
        .iter()
        .zip(&next.blueprint.widgets)
        .enumerate()
        .filter(|(_, (before, after))| before.props != after.props)
        .map(|(index, (_, after))| SurfacePatchOp {
            op: SurfacePatchOperation::Replace,
            path: format!("/blueprint/widgets/{index}/props"),
            value: after.props.clone(),
            from: None,
        })
        .collect();
    Some(SurfacePatchEvent {
        kind: SurfaceMessageKind::Patch,
        protocol: SurfaceProtocolVersion::default(),
        surface_id: next.surface_id.clone(),
        base_revision: previous.revision,
        revision: next.revision,
        patch,
    })
}

fn validate_message(message: &SurfaceMessage) -> Result<(), SurfaceRegistryError> {
    match message {
        SurfaceMessage::Snapshot(snapshot) => validate_surface_snapshot(snapshot)?,
        SurfaceMessage::Patch(patch) => validate_surface_patch_event(patch)?,
    }
    Ok(())
}

fn event_bytes(event: &SurfaceEvent) -> usize {
    let message_bytes = match &event.message {
        SurfaceMessage::Snapshot(snapshot) => {
            serde_json::to_vec(snapshot).map_or(usize::MAX, |raw| raw.len())
        }
        SurfaceMessage::Patch(patch) => {
            serde_json::to_vec(patch).map_or(usize::MAX, |raw| raw.len())
        }
    };
    event.cursor.0.len().saturating_add(message_bytes)
}

fn scope_fingerprint(scope: &SurfaceScope) -> String {
    let mut digest = Sha256::new();
    digest.update(scope.data_root.len().to_be_bytes());
    digest.update(scope.data_root.as_bytes());
    digest.update(scope.character_id.len().to_be_bytes());
    digest.update(scope.character_id.as_bytes());
    digest.update(scope.session_id.len().to_be_bytes());
    digest.update(scope.session_id.as_bytes());
    if let Some(user_id) = &scope.user_id {
        digest.update(user_id.len().to_be_bytes());
        digest.update(user_id.as_bytes());
    }
    digest
        .finalize()
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use airp_state_protocol::{
        validate_surface_patch_event, validate_surface_snapshot, LayoutNodeV2, SurfaceMessageKind,
        SurfacePatchOperation,
    };
    use serde_json::{json, Value};

    use super::{
        build_session_surface, props_patch, SessionSurfaceProps, SurfaceMessage, SurfacePublish,
        SurfaceRegistry, SurfaceReplay, SurfaceScope,
    };

    fn scope(character_id: &str, session_id: &str) -> SurfaceScope {
        SurfaceScope::new(Path::new("data"), character_id, session_id)
    }

    fn props(marker: &str) -> SessionSurfaceProps {
        SessionSurfaceProps {
            chat: json!({"messages": [marker]}),
            memory: json!({"entries": [marker]}),
            character_state: json!({"mood": marker}),
            activity: json!({"items": [marker]}),
        }
    }

    fn changed(publish: SurfacePublish) -> super::SurfaceEvent {
        match publish {
            SurfacePublish::Changed(event) => event,
            SurfacePublish::Unchanged { .. } => panic!("expected a changed publication"),
        }
    }

    fn snapshot(event: &super::SurfaceEvent) -> &airp_state_protocol::SurfaceSnapshot {
        match &event.message {
            SurfaceMessage::Snapshot(snapshot) => snapshot,
            SurfaceMessage::Patch(_) => panic!("expected snapshot"),
        }
    }

    #[test]
    fn builder_is_deterministic_and_keeps_dynamic_data_in_widget_props() {
        let scope = scope("character-a", "session-a");
        let first = build_session_surface(&scope, 7.into(), props("first")).unwrap();
        let same = build_session_surface(&scope, 7.into(), props("first")).unwrap();
        let changed = build_session_surface(&scope, 7.into(), props("second")).unwrap();

        assert_eq!(first, same);
        assert_eq!(first.kind, SurfaceMessageKind::Snapshot);
        assert_eq!(first.surface_id, "session:session-a");
        assert_eq!(first.blueprint.root, changed.blueprint.root);
        assert_eq!(first.blueprint.widgets.len(), 4);
        assert_eq!(
            first
                .blueprint
                .widgets
                .iter()
                .map(|widget| (widget.id.as_str(), widget.widget_type.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("chat", "core.chat"),
                ("memory", "core.memory"),
                ("character-state", "core.character-state"),
                ("activity", "core.activity"),
            ]
        );
        assert!(first
            .blueprint
            .widgets
            .iter()
            .zip(&changed.blueprint.widgets)
            .all(|(before, after)| before.id == after.id
                && before.widget_type == after.widget_type
                && before.props != after.props));

        let LayoutNodeV2::Split { children, .. } = &first.blueprint.root else {
            panic!("root must be a split");
        };
        assert!(matches!(children[0], LayoutNodeV2::Tabs { .. }));
        assert!(matches!(children[1], LayoutNodeV2::Stack { .. }));
        validate_surface_snapshot(&first).unwrap();
    }

    #[test]
    fn unchanged_content_does_not_advance_revision_or_cursor() {
        let scope = scope("character-a", "session-a");
        let mut registry = SurfaceRegistry::new();
        let initial = changed(registry.publish(scope.clone(), props("same")).unwrap());

        let unchanged = registry.publish(scope, props("same")).unwrap();
        assert_eq!(
            unchanged,
            SurfacePublish::Unchanged {
                revision: 1.into(),
                cursor: initial.cursor,
            }
        );
    }

    #[test]
    fn changes_emit_adjacent_patches_that_only_replace_widget_props() {
        let scope = scope("character-a", "session-a");
        let mut registry = SurfaceRegistry::new();
        let initial = changed(registry.publish(scope.clone(), props("before")).unwrap());
        let update = changed(registry.publish(scope.clone(), props("after")).unwrap());

        let SurfaceMessage::Patch(patch) = &update.message else {
            panic!("small update must be a patch");
        };
        assert_eq!(patch.base_revision.value(), 1);
        assert_eq!(patch.revision.value(), 2);
        assert_eq!(patch.patch.len(), 4);
        assert!(patch.patch.iter().enumerate().all(|(index, op)| {
            op.op == SurfacePatchOperation::Replace
                && op.path == format!("/blueprint/widgets/{index}/props")
                && op.value.is_some()
                && op.from.is_none()
        }));
        validate_surface_patch_event(patch).unwrap();

        let SurfaceReplay::Events(events) = registry.replay(&scope, Some(&initial.cursor)).unwrap()
        else {
            panic!("a retained cursor must replay events");
        };
        assert_eq!(events, vec![update]);
    }

    #[test]
    fn patch_over_protocol_limit_falls_back_to_valid_snapshot() {
        let scope = scope("character-a", "session-a");
        let mut registry = SurfaceRegistry::new();
        changed(registry.publish(scope.clone(), props("small")).unwrap());

        let large = "x".repeat(70_000);
        let event = changed(
            registry
                .publish(
                    scope,
                    SessionSurfaceProps {
                        chat: json!({"text": large}),
                        memory: Value::Null,
                        character_state: Value::Null,
                        activity: Value::Null,
                    },
                )
                .unwrap(),
        );

        let snapshot = snapshot(&event);
        assert_eq!(snapshot.revision.value(), 2);
        validate_surface_snapshot(snapshot).unwrap();
    }

    #[test]
    fn foreign_expired_and_previous_boot_cursors_resync_with_snapshot() {
        let scope_a = scope("character-a", "session-a");
        let scope_b = scope("character-b", "session-b");
        let mut registry = SurfaceRegistry::with_limits(2, usize::MAX);
        let first_a = changed(registry.publish(scope_a.clone(), props("a1")).unwrap());
        changed(registry.publish(scope_b.clone(), props("b1")).unwrap());

        assert!(matches!(
            registry.replay(&scope_b, Some(&first_a.cursor)).unwrap(),
            SurfaceReplay::Snapshot(_)
        ));

        changed(registry.publish(scope_a.clone(), props("a2")).unwrap());
        changed(registry.publish(scope_a.clone(), props("a3")).unwrap());
        changed(registry.publish(scope_a.clone(), props("a4")).unwrap());
        assert!(registry.ring_event_count() <= 2);
        assert!(matches!(
            registry.replay(&scope_a, Some(&first_a.cursor)).unwrap(),
            SurfaceReplay::Snapshot(_)
        ));

        let mut restarted = SurfaceRegistry::new();
        restarted
            .publish(scope_a.clone(), props("restart"))
            .unwrap();
        assert!(matches!(
            restarted.replay(&scope_a, Some(&first_a.cursor)).unwrap(),
            SurfaceReplay::Snapshot(_)
        ));
    }

    #[test]
    fn ring_is_also_bounded_by_total_serialized_bytes() {
        let scope = scope("character-a", "session-a");
        let mut registry = SurfaceRegistry::with_limits(32, 1);
        let initial = changed(registry.publish(scope.clone(), props("first")).unwrap());
        changed(registry.publish(scope.clone(), props("second")).unwrap());

        assert_eq!(registry.ring_event_count(), 0);
        assert_eq!(registry.ring_total_bytes(), 0);
        assert!(matches!(
            registry.replay(&scope, Some(&initial.cursor)).unwrap(),
            SurfaceReplay::Snapshot(_)
        ));
    }

    #[test]
    fn complete_character_and_session_scope_prevents_aliasing() {
        let first = scope("character-a", "shared-session");
        let second = scope("character-b", "shared-session");
        let mut registry = SurfaceRegistry::new();
        registry.publish(first.clone(), props("first")).unwrap();
        registry.publish(second.clone(), props("second")).unwrap();

        let first_snapshot = registry.current(&first).unwrap();
        let second_snapshot = registry.current(&second).unwrap();
        assert_eq!(
            snapshot(&first_snapshot).blueprint.widgets[0].props,
            Some(json!({"messages": ["first"]}))
        );
        assert_eq!(
            snapshot(&second_snapshot).blueprint.widgets[0].props,
            Some(json!({"messages": ["second"]}))
        );
    }

    #[test]
    fn effective_data_root_is_part_of_the_internal_scope() {
        let first = SurfaceScope::new(Path::new("tenant-a"), "character", "session");
        let second = SurfaceScope::new(Path::new("tenant-b"), "character", "session");
        let mut registry = SurfaceRegistry::new();
        registry.publish(first.clone(), props("first")).unwrap();
        registry.publish(second.clone(), props("second")).unwrap();

        assert_eq!(
            snapshot(&registry.current(&first).unwrap())
                .blueprint
                .widgets[0]
                .props,
            Some(json!({"messages": ["first"]}))
        );
        assert_eq!(
            snapshot(&registry.current(&second).unwrap())
                .blueprint
                .widgets[0]
                .props,
            Some(json!({"messages": ["second"]}))
        );
    }

    #[test]
    fn intent_resolution_requires_one_exact_registered_surface_and_widget() {
        let mut registry = SurfaceRegistry::new();
        let first = SurfaceScope::for_user(
            Path::new("tenant-a"),
            "character",
            "session",
            Some("alice".into()),
        );
        registry.publish(first.clone(), props("first")).unwrap();
        let target = registry
            .resolve_intent_target("session:session", "chat")
            .unwrap();
        assert_eq!(target.scope, first);
        assert_eq!(target.widget_type, "core.chat");
        assert!(registry
            .resolve_intent_target("session:session", "missing")
            .is_err());

        let second = SurfaceScope::for_user(
            Path::new("tenant-b"),
            "character",
            "session",
            Some("bob".into()),
        );
        registry.publish(second, props("second")).unwrap();
        assert!(registry
            .resolve_intent_target("session:session", "chat")
            .is_err());
    }

    #[test]
    fn entry_cache_evicts_the_least_recently_published_scope() {
        let first = scope("character-a", "session-a");
        let second = scope("character-b", "session-b");
        let third = scope("character-c", "session-c");
        let mut registry = SurfaceRegistry::with_all_limits(2, 32, usize::MAX);
        registry.publish(first.clone(), props("first")).unwrap();
        registry.publish(second.clone(), props("second")).unwrap();
        registry.publish(first.clone(), props("first")).unwrap();
        registry.publish(third.clone(), props("third")).unwrap();

        assert_eq!(registry.entry_count(), 2);
        assert!(registry.current(&first).is_ok());
        assert!(registry.current(&third).is_ok());
        assert!(matches!(
            registry.current(&second),
            Err(super::SurfaceRegistryError::UnknownScope)
        ));
    }

    #[test]
    fn structural_blueprint_changes_require_a_snapshot() {
        let scope = scope("character-a", "session-a");
        let previous = build_session_surface(&scope, 1.into(), props("before")).unwrap();
        let mut next = build_session_surface(&scope, 2.into(), props("after")).unwrap();
        next.blueprint.widgets.swap(0, 1);

        assert!(props_patch(&previous, &next).is_none());
    }

    #[test]
    fn protocol_validator_rejection_does_not_replace_current_snapshot() {
        let scope = scope("character-a", "session-a");
        let mut registry = SurfaceRegistry::new();
        registry.publish(scope.clone(), props("valid")).unwrap();

        let error = registry
            .publish(
                scope.clone(),
                SessionSurfaceProps {
                    chat: json!({"script": "not allowed"}),
                    memory: Value::Null,
                    character_state: Value::Null,
                    activity: Value::Null,
                },
            )
            .unwrap_err();

        assert!(matches!(
            error,
            super::SurfaceRegistryError::Validation(airp_state_protocol::SurfaceValidationError {
                code: airp_state_protocol::SurfaceErrorCode::ForbiddenExecutableField,
                ..
            })
        ));
        assert_eq!(
            snapshot(&registry.current(&scope).unwrap())
                .revision
                .value(),
            1
        );
    }
}
