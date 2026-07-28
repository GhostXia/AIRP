//! UI-independent conversation aggregate and append-only event store.
//!
//! A conversation is an Engine-owned resource. Character chat, scene/group
//! roleplay, councils, external agents, and future interaction modes can refer
//! to the same stable contract without making a browser or desktop client the
//! source of truth.

use crate::error::AirpError;
use crate::types::{SessionId, UserId};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

pub const CONVERSATION_SCHEMA_VERSION: u32 = 1;
pub const CONVERSATION_EVENT_SCHEMA_VERSION: u32 = 1;
const DEFAULT_WINDOW_LIMIT: usize = 50;
const MAX_WINDOW_LIMIT: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ConversationLockKey {
    data_root: PathBuf,
    conversation_id: SessionId,
}

static CONVERSATION_LOCKS: OnceLock<
    Mutex<HashMap<ConversationLockKey, Weak<tokio::sync::Mutex<()>>>>,
> = OnceLock::new();

fn conversation_lock(data_root: &Path, conversation_id: SessionId) -> Arc<tokio::sync::Mutex<()>> {
    let key = ConversationLockKey {
        data_root: data_root.to_path_buf(),
        conversation_id,
    };
    let mut locks = CONVERSATION_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    locks.retain(|_, lock| lock.strong_count() > 0);
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

/// A reference to another Engine-owned or external resource.
///
/// `kind` and `id` are opaque at this layer. Domain adapters validate the
/// resource kinds they understand. Unknown resource kinds remain serializable
/// and round-trip through `extensions`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationResourceRef {
    pub kind: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

/// A stable participant identity within one conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationParticipant {
    pub participant_id: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ConversationResourceRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

/// Engine-side orchestration policy reference.
///
/// The core stores the policy identity and configuration but does not assume a
/// fixed set of scheduling algorithms. Adapters resolve the policy they own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyRef {
    pub policy_id: String,
    #[serde(default)]
    pub config: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
}

/// Immutable identity and configuration for a conversation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationManifest {
    pub schema_version: u32,
    pub conversation_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub participants: Vec<ConversationParticipant>,
    #[serde(default)]
    pub resources: Vec<ConversationResourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestration: Option<ConversationPolicyRef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
    pub created_at: String,
}

/// Request used by the HTTP layer and other Engine adapters.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateConversationRequest {
    #[serde(default)]
    pub user_id: Option<UserId>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub participants: Vec<ConversationParticipant>,
    #[serde(default)]
    pub resources: Vec<ConversationResourceRef>,
    #[serde(default)]
    pub orchestration: Option<ConversationPolicyRef>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

/// Additive adapter request for creating a generic conversation from an AIRP
/// scene. Scene characters are snapshotted as participants; callers may add
/// other participant kinds without changing the core contract.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateSceneConversationRequest {
    #[serde(default)]
    pub user_id: Option<UserId>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub additional_participants: Vec<ConversationParticipant>,
    #[serde(default)]
    pub orchestration: Option<ConversationPolicyRef>,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

/// One immutable record in the conversation event journal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationEvent {
    pub schema_version: u32,
    pub event_id: String,
    pub conversation_id: SessionId,
    pub sequence: u64,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: BTreeMap<String, Value>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppendConversationEventRequest {
    #[serde(default)]
    pub user_id: Option<UserId>,
    pub kind: String,
    #[serde(default)]
    pub actor_id: Option<String>,
    #[serde(default)]
    pub causation_id: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
    /// Optional optimistic-concurrency guard. Zero means the journal must be
    /// empty; N means the next committed event must receive sequence N.
    #[serde(default)]
    pub expected_next_sequence: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConversationScopeQuery {
    #[serde(default)]
    pub user_id: Option<UserId>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConversationEventsQuery {
    #[serde(default)]
    pub user_id: Option<UserId>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub before: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConversationEventWindow {
    pub events: Vec<ConversationEvent>,
    pub has_more: bool,
    pub oldest_id: Option<String>,
    pub total: usize,
    pub next_sequence: u64,
}

/// Execute one Engine-owned conversation turn.
///
/// `base` carries provider, model, persona, preset, and generation controls
/// already supported by the chat pipeline. Conversation scope and history are
/// deliberately excluded from client control and are derived by the Engine.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnRequest {
    #[serde(default)]
    pub user_id: Option<UserId>,
    pub user_actor_id: String,
    pub expected_next_sequence: u64,
    pub base: crate::daemon::ChatCompletionRequest,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationTurnStatus {
    Completed,
    PartiallyCommitted,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnFailure {
    pub code: String,
    pub participant_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnOutcome {
    pub turn_id: String,
    pub status: ConversationTurnStatus,
    pub events: Vec<ConversationEvent>,
    pub next_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<ConversationTurnFailure>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationJournalState {
    schema_version: u32,
    next_sequence: u64,
    committed_bytes: u64,
    last_event_id: String,
}

#[derive(Debug, Clone)]
pub struct ConversationService {
    data_root: PathBuf,
}

impl ConversationService {
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    pub fn create(
        &self,
        request: CreateConversationRequest,
    ) -> Result<ConversationManifest, AirpError> {
        validate_create_request(&request)?;
        let conversation_id = SessionId::new();
        let manifest = ConversationManifest {
            schema_version: CONVERSATION_SCHEMA_VERSION,
            conversation_id,
            title: request.title,
            participants: request.participants,
            resources: request.resources,
            orchestration: request.orchestration,
            extensions: request.extensions,
            created_at: Utc::now().to_rfc3339(),
        };

        let directory = self.conversation_dir(conversation_id);
        fs::create_dir_all(&directory)?;
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        if let Err(error) =
            crate::data_dir::replace_file(&directory.join("manifest.json"), &manifest_bytes)
        {
            let _ = fs::remove_dir(&directory);
            return Err(error);
        }
        Ok(manifest)
    }

    pub fn get(&self, conversation_id: SessionId) -> Result<ConversationManifest, AirpError> {
        let path = self.conversation_dir(conversation_id).join("manifest.json");
        if !path.is_file() {
            return Err(AirpError::NotFound(format!(
                "conversation {conversation_id} not found"
            )));
        }
        let manifest: ConversationManifest = serde_json::from_slice(&fs::read(path)?)?;
        validate_manifest_identity(&manifest, conversation_id)?;
        Ok(manifest)
    }

    pub fn list(&self) -> Result<Vec<ConversationManifest>, AirpError> {
        let root = self.data_root.join("conversations");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut manifests = Vec::new();
        for entry in fs::read_dir(root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let Ok(conversation_id) = SessionId::parse(&name) else {
                continue;
            };
            manifests.push(self.get(conversation_id)?);
        }
        manifests.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        Ok(manifests)
    }

    pub async fn append_event(
        &self,
        conversation_id: SessionId,
        request: AppendConversationEventRequest,
    ) -> Result<ConversationEvent, AirpError> {
        validate_event_request(&request)?;
        let lock = conversation_lock(&self.data_root, conversation_id);
        let _guard = lock.lock().await;
        self.append_event_locked(conversation_id, request)
    }

    pub(crate) async fn acquire_write(
        &self,
        conversation_id: SessionId,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        conversation_lock(&self.data_root, conversation_id)
            .lock_owned()
            .await
    }

    pub(crate) fn append_event_locked(
        &self,
        conversation_id: SessionId,
        request: AppendConversationEventRequest,
    ) -> Result<ConversationEvent, AirpError> {
        validate_event_request(&request)?;
        let _ = self.get(conversation_id)?;
        if let Some(actor_id) = request.actor_id.as_deref() {
            validate_nonempty("event.actor_id", actor_id)?;
        }

        let path = self.events_path(conversation_id);
        let next_sequence = self.next_sequence(conversation_id, &path)?;
        if let Some(expected) = request.expected_next_sequence {
            if expected != next_sequence {
                return Err(AirpError::Conflict(format!(
                    "conversation {conversation_id} expected next sequence {expected}, actual {next_sequence}"
                )));
            }
        }

        let event = ConversationEvent {
            schema_version: CONVERSATION_EVENT_SCHEMA_VERSION,
            event_id: crate::ulid::new_id(),
            conversation_id,
            sequence: next_sequence,
            kind: request.kind,
            actor_id: request.actor_id,
            causation_id: request.causation_id,
            correlation_id: request.correlation_id,
            payload: request.payload,
            extensions: request.extensions,
            occurred_at: Utc::now().to_rfc3339(),
        };
        let mut encoded = serde_json::to_vec(&event)?;
        encoded.push(b'\n');
        let parent = path
            .parent()
            .ok_or_else(|| AirpError::Internal("event journal has no parent".to_string()))?;
        fs::create_dir_all(parent)?;
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        file.write_all(&encoded)?;
        file.sync_data()?;
        let journal_state = ConversationJournalState {
            schema_version: CONVERSATION_EVENT_SCHEMA_VERSION,
            next_sequence: next_sequence.checked_add(1).ok_or_else(|| {
                AirpError::Internal("conversation sequence exhausted".to_string())
            })?,
            committed_bytes: file.metadata()?.len(),
            last_event_id: event.event_id.clone(),
        };
        let state_bytes = serde_json::to_vec(&journal_state)?;
        if let Err(error) = crate::data_dir::replace_file(
            &self
                .conversation_dir(conversation_id)
                .join("journal_state.json"),
            &state_bytes,
        ) {
            // The journal append is already durable. This file is only a
            // verified acceleration cache; later writes can recover by scan.
            tracing::warn!(
                %conversation_id,
                %error,
                "conversation event committed but journal state cache update failed"
            );
        }
        Ok(event)
    }

    pub fn events(
        &self,
        conversation_id: SessionId,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<ConversationEventWindow, AirpError> {
        let _ = self.get(conversation_id)?;
        let limit = limit
            .unwrap_or(DEFAULT_WINDOW_LIMIT)
            .clamp(1, MAX_WINDOW_LIMIT);
        let path = self.events_path(conversation_id);
        if !path.is_file() {
            if before.is_some() {
                return Err(AirpError::BadRequest(
                    "cursor not in this conversation".to_string(),
                ));
            }
            return Ok(ConversationEventWindow {
                events: Vec::new(),
                has_more: false,
                oldest_id: None,
                total: 0,
                next_sequence: 0,
            });
        }

        if let Some(cursor) = before {
            if !crate::ulid::is_valid_id(cursor) {
                return Err(AirpError::BadRequest(format!(
                    "cursor is not a valid durable event id: {cursor}"
                )));
            }
        }

        let file = File::open(path)?;
        let mut preceding = VecDeque::with_capacity(limit);
        let mut count_before_cursor = 0usize;
        let mut total = 0usize;
        let mut cursor_found = before.is_none();
        let mut expected_sequence = 0u64;

        for line in BufReader::new(file).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: ConversationEvent = serde_json::from_str(&line)?;
            validate_event_identity(&event, conversation_id, expected_sequence)?;
            expected_sequence = expected_sequence.checked_add(1).ok_or_else(|| {
                AirpError::Internal("conversation sequence exhausted".to_string())
            })?;
            total += 1;

            if let Some(cursor) = before {
                if crate::ulid::matches(&event.event_id, cursor) {
                    cursor_found = true;
                    continue;
                }
                if cursor_found {
                    continue;
                }
            }

            count_before_cursor += 1;
            if preceding.len() == limit {
                preceding.pop_front();
            }
            preceding.push_back(event);
        }

        if !cursor_found {
            return Err(AirpError::BadRequest(format!(
                "cursor {} not in this conversation",
                before.unwrap_or_default()
            )));
        }
        let events: Vec<_> = preceding.into_iter().collect();
        Ok(ConversationEventWindow {
            oldest_id: events.first().map(|event| event.event_id.clone()),
            has_more: count_before_cursor > events.len(),
            events,
            total,
            next_sequence: expected_sequence,
        })
    }

    /// Read the authoritative journal for an Engine-side projection.
    ///
    /// Unlike the paginated HTTP view, this internal path has no UI-oriented
    /// page cap. Context compaction belongs to execution policy, not storage.
    pub(crate) fn all_events(
        &self,
        conversation_id: SessionId,
    ) -> Result<Vec<ConversationEvent>, AirpError> {
        let _ = self.get(conversation_id)?;
        let path = self.events_path(conversation_id);
        if !path.is_file() {
            return Ok(Vec::new());
        }
        let mut events = Vec::new();
        let mut expected_sequence = 0u64;
        for line in BufReader::new(File::open(path)?).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let event: ConversationEvent = serde_json::from_str(&line)?;
            validate_event_identity(&event, conversation_id, expected_sequence)?;
            expected_sequence = expected_sequence.checked_add(1).ok_or_else(|| {
                AirpError::Internal("conversation sequence exhausted".to_string())
            })?;
            events.push(event);
        }
        Ok(events)
    }

    fn conversation_dir(&self, conversation_id: SessionId) -> PathBuf {
        self.data_root
            .join("conversations")
            .join(conversation_id.to_string())
    }

    fn events_path(&self, conversation_id: SessionId) -> PathBuf {
        self.conversation_dir(conversation_id).join("events.jsonl")
    }

    fn next_sequence(
        &self,
        conversation_id: SessionId,
        events_path: &Path,
    ) -> Result<u64, AirpError> {
        let committed_bytes = match events_path.metadata() {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(error.into()),
        };
        let state_path = self
            .conversation_dir(conversation_id)
            .join("journal_state.json");
        if let Ok(bytes) = fs::read(state_path) {
            if let Ok(state) = serde_json::from_slice::<ConversationJournalState>(&bytes) {
                if state.schema_version == CONVERSATION_EVENT_SCHEMA_VERSION
                    && state.committed_bytes == committed_bytes
                    && state.next_sequence > 0
                {
                    if let Some(last_event) = read_last_event(events_path)? {
                        if last_event.conversation_id == conversation_id
                            && last_event.schema_version == CONVERSATION_EVENT_SCHEMA_VERSION
                            && last_event.event_id == state.last_event_id
                            && last_event
                                .sequence
                                .checked_add(1)
                                .is_some_and(|next| next == state.next_sequence)
                        {
                            return Ok(state.next_sequence);
                        }
                    }
                }
            }
        }
        scan_next_sequence(events_path, conversation_id)
    }
}

fn validate_nonempty(field: &str, value: &str) -> Result<(), AirpError> {
    if value.trim().is_empty() {
        return Err(AirpError::BadRequest(format!("{field} must not be empty")));
    }
    Ok(())
}

fn validate_resource(resource: &ConversationResourceRef) -> Result<(), AirpError> {
    validate_nonempty("resource.kind", &resource.kind)?;
    validate_nonempty("resource.id", &resource.id)
}

fn validate_create_request(request: &CreateConversationRequest) -> Result<(), AirpError> {
    let mut participant_ids = HashSet::new();
    for participant in &request.participants {
        validate_nonempty("participant_id", &participant.participant_id)?;
        validate_nonempty("participant.kind", &participant.kind)?;
        if !participant_ids.insert(participant.participant_id.as_str()) {
            return Err(AirpError::BadRequest(format!(
                "duplicate participant_id: {}",
                participant.participant_id
            )));
        }
        if let Some(resource) = participant.resource.as_ref() {
            validate_resource(resource)?;
        }
    }
    for resource in &request.resources {
        validate_resource(resource)?;
    }
    if let Some(policy) = request.orchestration.as_ref() {
        validate_nonempty("orchestration.policy_id", &policy.policy_id)?;
    }
    Ok(())
}

fn validate_event_request(request: &AppendConversationEventRequest) -> Result<(), AirpError> {
    validate_nonempty("event.kind", &request.kind)
}

fn validate_manifest_identity(
    manifest: &ConversationManifest,
    conversation_id: SessionId,
) -> Result<(), AirpError> {
    if manifest.schema_version != CONVERSATION_SCHEMA_VERSION {
        return Err(AirpError::Config(format!(
            "unsupported conversation schema version: {}",
            manifest.schema_version
        )));
    }
    if manifest.conversation_id != conversation_id {
        return Err(AirpError::Internal(format!(
            "conversation manifest identity mismatch: path {conversation_id}, body {}",
            manifest.conversation_id
        )));
    }
    Ok(())
}

fn validate_event_identity(
    event: &ConversationEvent,
    conversation_id: SessionId,
    expected_sequence: u64,
) -> Result<(), AirpError> {
    if event.schema_version != CONVERSATION_EVENT_SCHEMA_VERSION {
        return Err(AirpError::Config(format!(
            "unsupported conversation event schema version: {}",
            event.schema_version
        )));
    }
    if event.conversation_id != conversation_id || event.sequence != expected_sequence {
        return Err(AirpError::Internal(format!(
            "conversation event journal invariant failed at sequence {expected_sequence}"
        )));
    }
    if !crate::ulid::is_valid_id(&event.event_id) {
        return Err(AirpError::Internal(format!(
            "invalid durable event id at sequence {expected_sequence}"
        )));
    }
    Ok(())
}

fn scan_next_sequence(path: &Path, conversation_id: SessionId) -> Result<u64, AirpError> {
    if !path.is_file() {
        return Ok(0);
    }
    let mut expected = 0u64;
    for line in BufReader::new(File::open(path)?).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let event: ConversationEvent = serde_json::from_str(&line)?;
        validate_event_identity(&event, conversation_id, expected)?;
        expected = expected
            .checked_add(1)
            .ok_or_else(|| AirpError::Internal("conversation sequence exhausted".to_string()))?;
    }
    Ok(expected)
}

fn read_last_event(path: &Path) -> Result<Option<ConversationEvent>, AirpError> {
    let mut file = File::open(path)?;
    let mut position = file.metadata()?.len();
    if position == 0 {
        return Ok(None);
    }

    let mut reversed = Vec::new();
    let mut skipping_trailing_newlines = true;
    while position > 0 {
        let start = position.saturating_sub(8192);
        let length = usize::try_from(position - start)
            .map_err(|_| AirpError::Internal("event journal block is too large".to_string()))?;
        let mut block = vec![0u8; length];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut block)?;
        for byte in block.into_iter().rev() {
            if skipping_trailing_newlines && (byte == b'\n' || byte == b'\r') {
                continue;
            }
            skipping_trailing_newlines = false;
            if byte == b'\n' {
                reversed.reverse();
                return Ok(Some(serde_json::from_slice(&reversed)?));
            }
            reversed.push(byte);
        }
        position = start;
    }
    if reversed.is_empty() {
        return Ok(None);
    }
    reversed.reverse();
    Ok(Some(serde_json::from_slice(&reversed)?))
}

pub fn effective_conversation_root(
    data_root: &Path,
    user_id: Option<&UserId>,
) -> Result<PathBuf, AirpError> {
    crate::data_dir::resolve_effective_root(data_root, user_id.map(UserId::as_str))
}

pub fn request_from_scene(
    scene: &crate::scene::SceneConfig,
    request: CreateSceneConversationRequest,
) -> CreateConversationRequest {
    let mut participants: Vec<ConversationParticipant> = scene
        .characters
        .iter()
        .map(|entry| ConversationParticipant {
            participant_id: format!("character:{}", entry.character_id),
            kind: "character".to_string(),
            display_name: None,
            resource: Some(ConversationResourceRef {
                kind: "character".to_string(),
                id: entry.character_id.clone(),
                revision: None,
                extensions: BTreeMap::new(),
            }),
            extensions: BTreeMap::from([(
                "airp.scene.v1".to_string(),
                serde_json::json!({
                    "role": entry.role,
                    "intro": entry.intro,
                }),
            )]),
        })
        .collect();
    participants.extend(request.additional_participants);
    CreateConversationRequest {
        user_id: request.user_id,
        title: request
            .title
            .or_else(|| (!scene.description.trim().is_empty()).then(|| scene.description.clone())),
        participants,
        resources: vec![ConversationResourceRef {
            kind: "scene".to_string(),
            id: scene.scene_id.to_string(),
            revision: None,
            extensions: BTreeMap::new(),
        }],
        orchestration: request.orchestration.or_else(|| {
            Some(ConversationPolicyRef {
                policy_id: "airp.scene.round_robin.v1".to_string(),
                config: serde_json::json!({}),
                extensions: BTreeMap::new(),
            })
        }),
        extensions: request.extensions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn request() -> CreateConversationRequest {
        CreateConversationRequest {
            user_id: None,
            title: Some("Tavern".to_string()),
            participants: vec![
                ConversationParticipant {
                    participant_id: "user".to_string(),
                    kind: "user".to_string(),
                    display_name: Some("User".to_string()),
                    resource: None,
                    extensions: BTreeMap::new(),
                },
                ConversationParticipant {
                    participant_id: "alice".to_string(),
                    kind: "character".to_string(),
                    display_name: Some("Alice".to_string()),
                    resource: Some(ConversationResourceRef {
                        kind: "character".to_string(),
                        id: "alice".to_string(),
                        revision: Some("7".to_string()),
                        extensions: BTreeMap::new(),
                    }),
                    extensions: BTreeMap::new(),
                },
            ],
            resources: vec![ConversationResourceRef {
                kind: "scene".to_string(),
                id: "tavern".to_string(),
                revision: None,
                extensions: BTreeMap::new(),
            }],
            orchestration: Some(ConversationPolicyRef {
                policy_id: "airp.round_robin.v1".to_string(),
                config: serde_json::json!({"include_user": false}),
                extensions: BTreeMap::new(),
            }),
            extensions: BTreeMap::from([(
                "example.future".to_string(),
                serde_json::json!({"enabled": true}),
            )]),
        }
    }

    fn append(actor_id: Option<&str>, expected: Option<u64>) -> AppendConversationEventRequest {
        AppendConversationEventRequest {
            user_id: None,
            kind: "message.created".to_string(),
            actor_id: actor_id.map(str::to_string),
            causation_id: None,
            correlation_id: Some("turn-1".to_string()),
            payload: serde_json::json!({"content": "hello"}),
            extensions: BTreeMap::new(),
            expected_next_sequence: expected,
        }
    }

    #[test]
    fn manifest_and_namespaced_extensions_round_trip() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let created = service.create(request()).unwrap();
        let loaded = service.get(created.conversation_id).unwrap();
        assert_eq!(created, loaded);
        assert_eq!(
            loaded.extensions["example.future"]["enabled"],
            serde_json::json!(true)
        );
        assert_eq!(service.list().unwrap(), vec![created]);
    }

    #[test]
    fn duplicate_participants_are_rejected() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let mut request = request();
        request.participants.push(request.participants[0].clone());
        let error = service.create(request).unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn append_is_ordered_and_optimistic_conflicts_are_explicit() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).unwrap();
        let first = service
            .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
            .await
            .unwrap();
        let second = service
            .append_event(manifest.conversation_id, append(Some("alice"), Some(1)))
            .await
            .unwrap();
        assert_eq!((first.sequence, second.sequence), (0, 1));
        assert!(crate::ulid::is_valid_id(&first.event_id));

        let error = service
            .append_event(manifest.conversation_id, append(Some("user"), Some(1)))
            .await
            .unwrap_err();
        assert!(matches!(error, AirpError::Conflict(_)));
    }

    #[tokio::test]
    async fn unknown_actor_and_event_kind_round_trip_for_domain_adapters() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).unwrap();
        let mut request = append(Some("external-agent-42"), None);
        request.kind = "vendor.example/custom-event.v9".to_string();
        request.extensions.insert(
            "vendor.example".to_string(),
            serde_json::json!({"opaque": ["future", 9]}),
        );
        let event = service
            .append_event(manifest.conversation_id, request)
            .await
            .unwrap();
        assert_eq!(event.actor_id.as_deref(), Some("external-agent-42"));
        assert_eq!(event.kind, "vendor.example/custom-event.v9");
        assert_eq!(
            event.extensions["vendor.example"]["opaque"][1],
            serde_json::json!(9)
        );
    }

    #[tokio::test]
    async fn event_window_uses_bounded_tail_and_strict_before_cursor() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).unwrap();
        let mut ids = Vec::new();
        for sequence in 0..6 {
            let event = service
                .append_event(
                    manifest.conversation_id,
                    append(Some("user"), Some(sequence)),
                )
                .await
                .unwrap();
            ids.push(event.event_id);
        }

        let tail = service
            .events(manifest.conversation_id, Some(2), None)
            .unwrap();
        assert_eq!(
            tail.events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![4, 5]
        );
        assert!(tail.has_more);
        assert_eq!(tail.total, 6);
        assert_eq!(tail.next_sequence, 6);

        let earlier = service
            .events(manifest.conversation_id, Some(2), Some(&ids[4]))
            .unwrap();
        assert_eq!(
            earlier
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert!(earlier.has_more);
    }

    #[tokio::test]
    async fn cursor_cannot_cross_conversations() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let first = service.create(request()).unwrap();
        let second = service.create(request()).unwrap();
        let foreign = service
            .append_event(first.conversation_id, append(Some("user"), None))
            .await
            .unwrap();
        let error = service
            .events(second.conversation_id, Some(10), Some(&foreign.event_id))
            .unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn missing_journal_cache_recovers_from_event_truth() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).unwrap();
        service
            .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
            .await
            .unwrap();
        fs::remove_file(
            service
                .conversation_dir(manifest.conversation_id)
                .join("journal_state.json"),
        )
        .unwrap();
        let second = service
            .append_event(manifest.conversation_id, append(Some("alice"), Some(1)))
            .await
            .unwrap();
        assert_eq!(second.sequence, 1);
    }

    #[test]
    fn per_user_roots_are_isolated() {
        let tmp = tempdir().unwrap();
        let alice = UserId::new("alice").unwrap();
        let bob = UserId::new("bob").unwrap();
        let alice_root = effective_conversation_root(tmp.path(), Some(&alice)).unwrap();
        let bob_root = effective_conversation_root(tmp.path(), Some(&bob)).unwrap();
        let created = ConversationService::new(&alice_root)
            .create(request())
            .unwrap();
        assert!(ConversationService::new(&bob_root)
            .get(created.conversation_id)
            .is_err());
    }

    #[test]
    fn scene_adapter_snapshots_resources_without_narrowing_core_types() {
        let scene = crate::scene::SceneConfig {
            scene_id: crate::types::SceneId::new("tavern").unwrap(),
            description: "Night shift".to_string(),
            characters: vec![crate::scene::CharacterEntry {
                character_id: "alice".to_string(),
                role: crate::scene::CharacterRole::Primary,
                intro: "Innkeeper".to_string(),
            }],
            narrator_style: String::new(),
            lorebook_merge: Default::default(),
            format_hint: String::new(),
        };
        let request = request_from_scene(
            &scene,
            CreateSceneConversationRequest {
                user_id: None,
                title: None,
                additional_participants: vec![ConversationParticipant {
                    participant_id: "human:gm".to_string(),
                    kind: "human".to_string(),
                    display_name: Some("GM".to_string()),
                    resource: None,
                    extensions: BTreeMap::new(),
                }],
                orchestration: None,
                extensions: BTreeMap::new(),
            },
        );
        assert_eq!(request.title.as_deref(), Some("Night shift"));
        assert_eq!(request.participants.len(), 2);
        assert_eq!(request.participants[0].participant_id, "character:alice");
        assert_eq!(request.participants[1].kind, "human");
        assert_eq!(request.resources[0].kind, "scene");
        assert_eq!(
            request.orchestration.unwrap().policy_id,
            "airp.scene.round_robin.v1"
        );
    }
}
