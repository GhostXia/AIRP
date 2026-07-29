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

/// Manifest schema version written by this Engine.
pub const CONVERSATION_SCHEMA_VERSION: u32 = 1;
/// Event journal schema version written by this Engine.
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

/// Request for appending one immutable event to a conversation journal.
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

/// Optional user scope for manifest reads and listings.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ConversationScopeQuery {
    #[serde(default)]
    pub user_id: Option<UserId>,
}

/// Cursor and user scope accepted by the event-window endpoint.
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

/// Bounded event-journal window returned to callers.
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
    /// Stable client-supplied identity for retries, status reads, and explicit
    /// cancellation. Omission preserves the legacy one-shot submission path.
    #[serde(default)]
    pub turn_id: Option<String>,
    pub user_actor_id: String,
    pub expected_next_sequence: u64,
    pub base: crate::daemon::ChatCompletionRequest,
    #[serde(default)]
    pub extensions: BTreeMap<String, Value>,
}

/// Durable completion state of a submitted turn.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationTurnStatus {
    Completed,
    PartiallyCommitted,
}

/// Stable failure details for a partially committed turn.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnFailure {
    pub code: String,
    pub participant_id: Option<String>,
}

/// Events and journal position produced by one Engine-owned turn.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnOutcome {
    pub turn_id: String,
    pub status: ConversationTurnStatus,
    pub lifecycle_state: crate::conversation_turn::ConversationTurnLifecycleState,
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

/// Persistent Conversation aggregate service rooted at one effective user root.
#[derive(Debug, Clone)]
pub struct ConversationService {
    data_root: PathBuf,
    #[cfg(test)]
    faults: Arc<Mutex<VecDeque<ConversationIoFault>>>,
}

impl ConversationService {
    /// Bind a service to an effective data root.
    pub fn new(data_root: impl AsRef<Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
            #[cfg(test)]
            faults: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// Validate and persist an immutable conversation manifest.
    pub async fn create(
        &self,
        request: CreateConversationRequest,
    ) -> Result<ConversationManifest, AirpError> {
        let service = self.clone();
        run_conversation_io("create", move || service.create_blocking(request)).await
    }

    pub(crate) fn create_blocking(
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

    /// Load and validate one conversation manifest.
    pub async fn get(&self, conversation_id: SessionId) -> Result<ConversationManifest, AirpError> {
        let service = self.clone();
        run_conversation_io("get", move || service.get_blocking(conversation_id)).await
    }

    pub(crate) fn get_blocking(
        &self,
        conversation_id: SessionId,
    ) -> Result<ConversationManifest, AirpError> {
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

    /// List valid manifests while isolating unreadable conversation entries.
    pub async fn list(&self) -> Result<Vec<ConversationManifest>, AirpError> {
        let service = self.clone();
        run_conversation_io("list", move || service.list_blocking()).await
    }

    pub(crate) fn list_blocking(&self) -> Result<Vec<ConversationManifest>, AirpError> {
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
            match self.get_blocking(conversation_id) {
                Ok(manifest) => manifests.push(manifest),
                Err(error) => tracing::warn!(
                    %conversation_id,
                    %error,
                    "skipping unreadable conversation manifest during list"
                ),
            }
        }
        manifests.sort_by(|left, right| left.created_at.cmp(&right.created_at));
        Ok(manifests)
    }

    /// Serialize and durably append one event under the conversation write lock.
    pub async fn append_event(
        &self,
        conversation_id: SessionId,
        request: AppendConversationEventRequest,
    ) -> Result<ConversationEvent, AirpError> {
        validate_event_request(&request)?;
        let lock = conversation_lock(&self.data_root, conversation_id);
        let _guard = lock.lock().await;
        self.append_event_locked_async(conversation_id, request)
            .await
    }

    pub(crate) async fn acquire_write(
        &self,
        conversation_id: SessionId,
    ) -> tokio::sync::OwnedMutexGuard<()> {
        conversation_lock(&self.data_root, conversation_id)
            .lock_owned()
            .await
    }

    pub(crate) async fn append_event_locked_async(
        &self,
        conversation_id: SessionId,
        request: AppendConversationEventRequest,
    ) -> Result<ConversationEvent, AirpError> {
        let service = self.clone();
        run_conversation_io("append", move || {
            service.append_event_locked_blocking(conversation_id, request)
        })
        .await
    }

    fn append_event_locked_blocking(
        &self,
        conversation_id: SessionId,
        request: AppendConversationEventRequest,
    ) -> Result<ConversationEvent, AirpError> {
        validate_event_request(&request)?;
        let _ = self.get_blocking(conversation_id)?;
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
        #[cfg(test)]
        let fault = self.take_fault();
        #[cfg(test)]
        if let Some(ConversationIoFault::Delay(duration)) = fault {
            std::thread::sleep(duration);
        }
        #[cfg(test)]
        if fault == Some(ConversationIoFault::ShortWrite) {
            file.write_all(&encoded[..encoded.len().max(2) / 2])?;
            file.sync_data()?;
            return Err(std::io::Error::other("injected conversation short write").into());
        }
        file.write_all(&encoded)?;
        #[cfg(test)]
        if fault == Some(ConversationIoFault::SyncData) {
            return Err(std::io::Error::other("injected conversation sync_data failure").into());
        }
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
        #[cfg(test)]
        let cache_result = if fault == Some(ConversationIoFault::CacheWrite) {
            Err(AirpError::Io(std::io::Error::other(
                "injected conversation cache write failure",
            )))
        } else {
            crate::data_dir::replace_file(
                &self
                    .conversation_dir(conversation_id)
                    .join("journal_state.json"),
                &state_bytes,
            )
        };
        #[cfg(not(test))]
        let cache_result = crate::data_dir::replace_file(
            &self
                .conversation_dir(conversation_id)
                .join("journal_state.json"),
            &state_bytes,
        );
        if let Err(error) = cache_result {
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

    /// Read a bounded reverse-cursor window from the authoritative journal.
    pub async fn events(
        &self,
        conversation_id: SessionId,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<ConversationEventWindow, AirpError> {
        let lock = conversation_lock(&self.data_root, conversation_id);
        let _guard = lock.lock().await;
        let service = self.clone();
        let before = before.map(str::to_owned);
        run_conversation_io("events", move || {
            service.events_blocking(conversation_id, limit, before.as_deref())
        })
        .await
    }

    pub(crate) fn events_blocking(
        &self,
        conversation_id: SessionId,
        limit: Option<usize>,
        before: Option<&str>,
    ) -> Result<ConversationEventWindow, AirpError> {
        let _ = self.get_blocking(conversation_id)?;
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
    pub(crate) async fn all_events(
        &self,
        conversation_id: SessionId,
    ) -> Result<Vec<ConversationEvent>, AirpError> {
        let lock = conversation_lock(&self.data_root, conversation_id);
        let _guard = lock.lock().await;
        self.all_events_locked_async(conversation_id).await
    }

    pub(crate) async fn all_events_locked_async(
        &self,
        conversation_id: SessionId,
    ) -> Result<Vec<ConversationEvent>, AirpError> {
        let service = self.clone();
        run_conversation_io("read journal", move || {
            service.all_events_blocking(conversation_id)
        })
        .await
    }

    pub(crate) fn all_events_blocking(
        &self,
        conversation_id: SessionId,
    ) -> Result<Vec<ConversationEvent>, AirpError> {
        let _ = self.get_blocking(conversation_id)?;
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
        let state = fs::read(state_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<ConversationJournalState>(&bytes).ok())
            .filter(|state| {
                state.schema_version == CONVERSATION_EVENT_SCHEMA_VERSION
                    && state.next_sequence > 0
                    && state.committed_bytes <= committed_bytes
            });
        if let Some(state) = state.as_ref() {
            if state.committed_bytes == committed_bytes {
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
        recover_next_sequence(events_path, conversation_id, state.as_ref())
    }

    #[cfg(test)]
    fn inject_fault(&self, fault: ConversationIoFault) {
        self.faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push_back(fault);
    }

    #[cfg(test)]
    fn take_fault(&self) -> Option<ConversationIoFault> {
        self.faults
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
    }
}

async fn run_conversation_io<T, F>(operation: &'static str, task: F) -> Result<T, AirpError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AirpError> + Send + 'static,
{
    tokio::task::spawn_blocking(task).await.map_err(|error| {
        AirpError::Internal(format!("conversation {operation} task failed: {error}"))
    })?
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConversationIoFault {
    ShortWrite,
    SyncData,
    CacheWrite,
    Delay(std::time::Duration),
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

fn recover_next_sequence(
    path: &Path,
    conversation_id: SessionId,
    cached_state: Option<&ConversationJournalState>,
) -> Result<u64, AirpError> {
    if !path.is_file() {
        return Ok(0);
    }
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    let mut expected = 0u64;
    let mut valid_bytes = 0u64;
    let mut cached_boundary_verified = false;
    let mut reader = BufReader::new(file.try_clone()?);
    loop {
        let mut encoded = Vec::new();
        let bytes_read = reader.read_until(b'\n', &mut encoded)?;
        if bytes_read == 0 {
            break;
        }
        let terminated = encoded.last() == Some(&b'\n');
        let mut line = encoded.as_slice();
        if line.ends_with(b"\n") {
            line = &line[..line.len() - 1];
        }
        if line.ends_with(b"\r") {
            line = &line[..line.len() - 1];
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            if !terminated {
                file.set_len(valid_bytes)?;
                file.sync_all()?;
                break;
            }
            valid_bytes = valid_bytes
                .checked_add(u64::try_from(bytes_read).map_err(|_| {
                    AirpError::Internal("conversation journal offset overflow".to_string())
                })?)
                .ok_or_else(|| {
                    AirpError::Internal("conversation journal offset overflow".to_string())
                })?;
            continue;
        }
        let event = match serde_json::from_slice::<ConversationEvent>(line) {
            Ok(event) if terminated => event,
            Ok(_) => {
                file.set_len(valid_bytes)?;
                file.sync_all()?;
                break;
            }
            Err(_) if !terminated && reader.fill_buf()?.is_empty() => {
                file.set_len(valid_bytes)?;
                file.sync_all()?;
                break;
            }
            Err(_) if cached_boundary_verified && reader.fill_buf()?.is_empty() => {
                file.set_len(valid_bytes)?;
                file.sync_all()?;
                break;
            }
            Err(error) => return Err(error.into()),
        };
        validate_event_identity(&event, conversation_id, expected)?;
        expected = expected
            .checked_add(1)
            .ok_or_else(|| AirpError::Internal("conversation sequence exhausted".to_string()))?;
        valid_bytes = valid_bytes
            .checked_add(u64::try_from(bytes_read).map_err(|_| {
                AirpError::Internal("conversation journal offset overflow".to_string())
            })?)
            .ok_or_else(|| {
                AirpError::Internal("conversation journal offset overflow".to_string())
            })?;
        if let Some(state) = cached_state {
            if valid_bytes == state.committed_bytes {
                if event.event_id == state.last_event_id && expected == state.next_sequence {
                    cached_boundary_verified = true;
                } else {
                    return Err(AirpError::Internal(
                        "conversation journal cache boundary mismatch".to_string(),
                    ));
                }
            } else if valid_bytes > state.committed_bytes && !cached_boundary_verified {
                return Err(AirpError::Internal(
                    "conversation journal cache boundary is not an event boundary".to_string(),
                ));
            }
        }
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

/// Resolve the per-user effective root used by Conversation storage.
pub fn effective_conversation_root(
    data_root: &Path,
    user_id: Option<&UserId>,
) -> Result<PathBuf, AirpError> {
    crate::data_dir::resolve_effective_root(data_root, user_id.map(UserId::as_str))
}

/// Snapshot an AIRP scene into a generic Conversation create request.
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
                policy_id: crate::conversation_policy::SCENE_ROUND_ROBIN_V1.to_string(),
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
        let created = service.create_blocking(request()).unwrap();
        let loaded = service.get_blocking(created.conversation_id).unwrap();
        assert_eq!(created, loaded);
        assert_eq!(
            loaded.extensions["example.future"]["enabled"],
            serde_json::json!(true)
        );
        assert_eq!(service.list_blocking().unwrap(), vec![created]);
    }

    #[test]
    fn list_skips_an_unreadable_manifest_without_hiding_valid_conversations() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let created = service.create_blocking(request()).unwrap();
        let corrupt_id = SessionId::new();
        let corrupt_dir = service.conversation_dir(corrupt_id);
        fs::create_dir_all(&corrupt_dir).unwrap();
        fs::write(corrupt_dir.join("manifest.json"), b"{not-json").unwrap();

        assert_eq!(service.list_blocking().unwrap(), vec![created]);
        assert!(matches!(
            service.get_blocking(corrupt_id),
            Err(AirpError::Json(_))
        ));
    }

    #[test]
    fn duplicate_participants_are_rejected() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let mut request = request();
        request.participants.push(request.participants[0].clone());
        let error = service.create_blocking(request).unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn append_is_ordered_and_optimistic_conflicts_are_explicit() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create_blocking(request()).unwrap();
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
    async fn concurrent_appends_are_serialized_per_conversation() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let service = service.clone();
            tasks.push(tokio::spawn(async move {
                service
                    .append_event(manifest.conversation_id, append(Some("user"), None))
                    .await
                    .unwrap()
            }));
        }
        let mut sequences = Vec::new();
        for task in tasks {
            sequences.push(task.await.unwrap().sequence);
        }
        sequences.sort_unstable();
        assert_eq!(sequences, (0..16).collect::<Vec<_>>());
    }

    #[tokio::test]
    async fn slow_append_does_not_take_a_cross_conversation_lock() {
        let tmp = tempdir().unwrap();
        let setup = ConversationService::new(tmp.path());
        let slow_manifest = setup.create(request()).await.unwrap();
        let fast_manifest = setup.create(request()).await.unwrap();
        let slow_service = ConversationService::new(tmp.path());
        slow_service.inject_fault(ConversationIoFault::Delay(
            std::time::Duration::from_millis(100),
        ));
        let slow = tokio::spawn(async move {
            slow_service
                .append_event(slow_manifest.conversation_id, append(Some("user"), Some(0)))
                .await
                .unwrap()
        });
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        let started = std::time::Instant::now();
        setup
            .append_event(fast_manifest.conversation_id, append(Some("user"), Some(0)))
            .await
            .unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(80),
            "a different conversation was blocked by unrelated file work"
        );
        slow.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_actor_and_event_kind_round_trip_for_domain_adapters() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create_blocking(request()).unwrap();
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
        let manifest = service.create_blocking(request()).unwrap();
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
            .events_blocking(manifest.conversation_id, Some(2), None)
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
            .events_blocking(manifest.conversation_id, Some(2), Some(&ids[4]))
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
        let first = service.create_blocking(request()).unwrap();
        let second = service.create_blocking(request()).unwrap();
        let foreign = service
            .append_event(first.conversation_id, append(Some("user"), None))
            .await
            .unwrap();
        let error = service
            .events_blocking(second.conversation_id, Some(10), Some(&foreign.event_id))
            .unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
    }

    #[tokio::test]
    async fn missing_journal_cache_recovers_from_event_truth() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create_blocking(request()).unwrap();
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

    #[tokio::test(flavor = "current_thread")]
    async fn conversation_file_io_does_not_block_the_async_runtime_worker() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        service.inject_fault(ConversationIoFault::Delay(
            std::time::Duration::from_millis(100),
        ));

        let append = service.append_event(manifest.conversation_id, append(Some("user"), Some(0)));
        tokio::pin!(append);
        let started = std::time::Instant::now();
        tokio::select! {
            result = &mut append => panic!("injected blocking append completed early: {result:?}"),
            _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
        }
        assert!(
            started.elapsed() < std::time::Duration::from_millis(80),
            "blocking file work stalled the current-thread runtime"
        );
        append.await.unwrap();
    }

    #[tokio::test]
    async fn short_write_tail_is_truncated_before_restart_append() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        service.inject_fault(ConversationIoFault::ShortWrite);
        assert!(matches!(
            service
                .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
                .await,
            Err(AirpError::Io(_))
        ));

        let restarted = ConversationService::new(tmp.path());
        let recovered = restarted
            .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
            .await
            .unwrap();
        assert_eq!(recovered.sequence, 0);
        assert_eq!(
            restarted
                .all_events(manifest.conversation_id)
                .await
                .unwrap(),
            vec![recovered]
        );
    }

    #[tokio::test]
    async fn sync_failure_preserves_a_complete_visible_event_for_restart_recovery() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        service.inject_fault(ConversationIoFault::SyncData);
        assert!(matches!(
            service
                .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
                .await,
            Err(AirpError::Io(_))
        ));

        let restarted = ConversationService::new(tmp.path());
        let second = restarted
            .append_event(manifest.conversation_id, append(Some("alice"), Some(1)))
            .await
            .unwrap();
        assert_eq!(second.sequence, 1);
        assert_eq!(
            restarted
                .all_events(manifest.conversation_id)
                .await
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn cache_write_failure_recovers_from_the_authoritative_journal_after_restart() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        service.inject_fault(ConversationIoFault::CacheWrite);
        let first = service
            .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
            .await
            .unwrap();
        assert_eq!(first.sequence, 0);

        let restarted = ConversationService::new(tmp.path());
        let second = restarted
            .append_event(manifest.conversation_id, append(Some("alice"), Some(1)))
            .await
            .unwrap();
        assert_eq!(second.sequence, 1);
    }

    #[tokio::test]
    async fn corrupt_final_line_is_removed_without_discarding_committed_events() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        let first = service
            .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
            .await
            .unwrap();
        let mut journal = OpenOptions::new()
            .append(true)
            .open(service.events_path(manifest.conversation_id))
            .unwrap();
        journal.write_all(b"{corrupt-tail}\n").unwrap();
        journal.sync_data().unwrap();
        drop(journal);

        let restarted = ConversationService::new(tmp.path());
        let second = restarted
            .append_event(manifest.conversation_id, append(Some("alice"), Some(1)))
            .await
            .unwrap();
        assert_eq!(
            restarted
                .all_events(manifest.conversation_id)
                .await
                .unwrap(),
            vec![first, second]
        );
    }

    #[tokio::test]
    async fn corruption_inside_the_committed_cache_boundary_fails_closed() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        service
            .append_event(manifest.conversation_id, append(Some("user"), Some(0)))
            .await
            .unwrap();
        let path = service.events_path(manifest.conversation_id);
        let mut bytes = fs::read(&path).unwrap();
        let payload_byte = bytes
            .iter_mut()
            .find(|byte| **byte == b'{')
            .expect("event JSON must contain an object");
        *payload_byte = b'[';
        fs::write(path, bytes).unwrap();

        let restarted = ConversationService::new(tmp.path());
        assert!(matches!(
            restarted
                .append_event(manifest.conversation_id, append(Some("alice"), Some(1)))
                .await,
            Err(AirpError::Json(_))
        ));
    }

    #[tokio::test]
    #[ignore = "durability benchmark; run explicitly with --release --ignored --nocapture"]
    async fn conversation_append_fsync_benchmark() {
        const APPENDS: u64 = 64;
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(request()).await.unwrap();
        let started = std::time::Instant::now();
        for sequence in 0..APPENDS {
            service
                .append_event(
                    manifest.conversation_id,
                    append(Some("user"), Some(sequence)),
                )
                .await
                .unwrap();
        }
        let elapsed = started.elapsed();
        eprintln!(
            "conversation append durability benchmark: appends={APPENDS} elapsed_ms={} mean_ms={:.3}",
            elapsed.as_millis(),
            elapsed.as_secs_f64() * 1000.0 / APPENDS as f64
        );
    }

    #[test]
    fn per_user_roots_are_isolated() {
        let tmp = tempdir().unwrap();
        let alice = UserId::new("alice").unwrap();
        let bob = UserId::new("bob").unwrap();
        let alice_root = effective_conversation_root(tmp.path(), Some(&alice)).unwrap();
        let bob_root = effective_conversation_root(tmp.path(), Some(&bob)).unwrap();
        let created = ConversationService::new(&alice_root)
            .create_blocking(request())
            .unwrap();
        assert!(ConversationService::new(&bob_root)
            .get_blocking(created.conversation_id)
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
