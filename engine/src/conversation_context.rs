//! Bounded prompt context derived from the authoritative Conversation journal.

use crate::adapter::{ChatMessage, MessageRole};
use crate::conversation::{
    conversation_io_lock, run_conversation_io, validate_event_identity, ConversationEvent,
    ConversationManifest, ConversationService,
};
use crate::error::AirpError;
use crate::types::SessionId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Seek, SeekFrom};
use std::path::Path;

/// Reserved journal event carrying a verified summary of the preceding prefix.
pub const CONVERSATION_CONTEXT_SUMMARY_EVENT: &str = "context.summary.created";
pub const CONVERSATION_CONTEXT_SCHEMA_VERSION: u32 = 1;
pub const DEFAULT_CONVERSATION_CONTEXT_TOKEN_BUDGET: usize = 12_000;
pub const DEFAULT_CONVERSATION_CONTEXT_MESSAGE_BUDGET: usize = 128;
pub const MAX_CONVERSATION_CONTEXT_MESSAGE_BUDGET: usize = 256;

const CONTEXT_CHECKPOINT_SCHEMA_VERSION: u32 = 2;
const RECENT_MESSAGE_INDEX_LIMIT: usize = MAX_CONVERSATION_CONTEXT_MESSAGE_BUDGET + 1;

/// Engine-owned limits for the derived prompt history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationContextBudget {
    pub max_estimated_tokens: usize,
    pub max_messages: usize,
}

impl Default for ConversationContextBudget {
    fn default() -> Self {
        Self {
            max_estimated_tokens: DEFAULT_CONVERSATION_CONTEXT_TOKEN_BUDGET,
            max_messages: DEFAULT_CONVERSATION_CONTEXT_MESSAGE_BUDGET,
        }
    }
}

/// Exact prefix boundary summarized by one immutable journal event.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummarySource {
    pub first_sequence: u64,
    pub last_sequence: u64,
    pub event_count: u64,
    pub first_event_id: String,
    pub last_event_id: String,
    pub journal_sha256: String,
}

/// Generator identity required for a summary to enter prompt context.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummaryProvenance {
    pub policy_id: String,
    pub policy_version: String,
    pub provider: String,
    pub model: String,
    pub operation_id: String,
}

/// Payload accepted for [`CONVERSATION_CONTEXT_SUMMARY_EVENT`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSummaryPayload {
    pub schema_version: u32,
    pub content: String,
    pub target_token_budget: usize,
    pub source: ConversationSummarySource,
    pub provenance: ConversationSummaryProvenance,
}

/// Source metadata for the summary selected by a bounded context projection.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversationContextSummaryRef {
    pub event_id: String,
    pub sequence: u64,
    pub source: ConversationSummarySource,
    pub provenance: ConversationSummaryProvenance,
}

/// Bounded prompt history and evidence explaining what it represents.
#[derive(Debug, Clone, Serialize)]
pub struct ConversationContextProjection {
    pub schema_version: u32,
    pub conversation_id: SessionId,
    pub messages: Vec<ChatMessage>,
    pub estimated_tokens: usize,
    pub omitted_message_count: u64,
    pub source_next_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<ConversationContextSummaryRef>,
}

/// Add a current-turn message while keeping the same Engine-owned context
/// budget. The newest message and any verified summary stay pinned; older
/// ordinary messages are evicted only from this derived prompt view.
pub(crate) fn push_bounded_context_message(
    messages: &mut Vec<ChatMessage>,
    message: ChatMessage,
    budget: ConversationContextBudget,
) -> Result<(), AirpError> {
    validate_budget(budget)?;
    let mut candidate = messages.clone();
    candidate.push(message);
    loop {
        let token_count = candidate.iter().map(message_tokens).sum::<usize>();
        let ordinary_count = candidate
            .iter()
            .filter(|message| message.role != MessageRole::System)
            .count();
        if token_count <= budget.max_estimated_tokens && ordinary_count <= budget.max_messages {
            *messages = candidate;
            return Ok(());
        }
        let Some(index) = candidate
            .iter()
            .position(|message| message.role != MessageRole::System)
        else {
            return Err(AirpError::BadRequest(
                "conversation summary exceeds the Engine context budget".to_string(),
            ));
        };
        if index == candidate.len() - 1 {
            return Err(AirpError::BadRequest(
                "latest conversation message exceeds the Engine context budget".to_string(),
            ));
        }
        candidate.remove(index);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextEventRef {
    offset: u64,
    sequence: u64,
    event_id: String,
    estimated_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationContextCheckpoint {
    schema_version: u32,
    journal_bytes: u64,
    source_next_sequence: u64,
    total_message_count: u64,
    tail_event: Option<ContextEventRef>,
    latest_summary: Option<ContextEventRef>,
    recent_messages: Vec<ContextEventRef>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationContextCheckpointEnvelope {
    checkpoint: ConversationContextCheckpoint,
    checkpoint_sha256: String,
}

struct LoadedCheckpoint {
    checkpoint: ConversationContextCheckpoint,
    changed: bool,
}

impl ConversationService {
    /// Build bounded model context without changing or deleting journal truth.
    pub async fn context_projection(
        &self,
        conversation_id: SessionId,
        budget: ConversationContextBudget,
    ) -> Result<ConversationContextProjection, AirpError> {
        validate_budget(budget)?;
        let io_lock = conversation_io_lock(self.data_root(), conversation_id);
        let _io_guard = io_lock.lock().await;
        let service = self.clone();
        run_conversation_io("project bounded context", move || {
            service.context_projection_blocking(conversation_id, budget)
        })
        .await
    }

    fn context_projection_blocking(
        &self,
        conversation_id: SessionId,
        budget: ConversationContextBudget,
    ) -> Result<ConversationContextProjection, AirpError> {
        let manifest = self.get_blocking(conversation_id)?;
        let journal_path = self.events_path(conversation_id);
        if !journal_path.is_file() {
            return Ok(ConversationContextProjection {
                schema_version: CONVERSATION_CONTEXT_SCHEMA_VERSION,
                conversation_id,
                messages: Vec::new(),
                estimated_tokens: 0,
                omitted_message_count: 0,
                source_next_sequence: 0,
                summary: None,
            });
        }

        let checkpoint_path = self
            .conversation_dir(conversation_id)
            .join("context_checkpoint.json");
        let loaded = match load_verified_checkpoint(&manifest, &checkpoint_path, &journal_path)? {
            Some(loaded) => loaded,
            None => {
                let checkpoint = rebuild_checkpoint(&manifest, &journal_path)?;
                LoadedCheckpoint {
                    checkpoint,
                    changed: true,
                }
            }
        };
        if loaded.changed {
            if let Err(error) = write_checkpoint(&checkpoint_path, &loaded.checkpoint) {
                tracing::warn!(
                    %conversation_id,
                    %error,
                    "conversation context checkpoint write failed; using journal-derived projection"
                );
            }
        }
        project_from_checkpoint(&manifest, &journal_path, loaded.checkpoint, budget)
    }
}

pub(crate) fn validate_summary_append(
    path: &Path,
    conversation_id: SessionId,
    next_sequence: u64,
    payload: &serde_json::Value,
) -> Result<(), AirpError> {
    if next_sequence == 0 {
        return Err(AirpError::BadRequest(
            "context summary requires a non-empty source journal".to_string(),
        ));
    }
    let payload: ConversationSummaryPayload =
        serde_json::from_value(payload.clone()).map_err(|error| {
            AirpError::BadRequest(format!("invalid context summary payload: {error}"))
        })?;
    let scan = scan_prefix(path, conversation_id)?;
    if scan.next_sequence != next_sequence {
        return Err(AirpError::Conflict(
            "context summary source is not the current journal prefix".to_string(),
        ));
    }
    validate_summary_payload(
        &payload,
        next_sequence,
        scan.first_event_id.as_deref(),
        scan.last_event_id.as_deref(),
        &scan.journal_sha256,
    )
}

fn validate_budget(budget: ConversationContextBudget) -> Result<(), AirpError> {
    if budget.max_estimated_tokens == 0 {
        return Err(AirpError::BadRequest(
            "conversation context token budget must be positive".to_string(),
        ));
    }
    if budget.max_messages == 0 || budget.max_messages > MAX_CONVERSATION_CONTEXT_MESSAGE_BUDGET {
        return Err(AirpError::BadRequest(format!(
            "conversation context message budget must be between 1 and {MAX_CONVERSATION_CONTEXT_MESSAGE_BUDGET}"
        )));
    }
    Ok(())
}

struct PrefixScan {
    next_sequence: u64,
    first_event_id: Option<String>,
    last_event_id: Option<String>,
    journal_sha256: String,
}

fn scan_prefix(path: &Path, conversation_id: SessionId) -> Result<PrefixScan, AirpError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = Vec::new();
    let mut hasher = Sha256::new();
    let mut expected_sequence = 0u64;
    let mut first_event_id = None;
    let mut last_event_id = None;
    loop {
        buffer.clear();
        if reader.read_until(b'\n', &mut buffer)? == 0 {
            break;
        }
        let event = parse_event_line(&buffer)?;
        validate_event_identity(&event, conversation_id, expected_sequence)?;
        first_event_id.get_or_insert_with(|| event.event_id.clone());
        last_event_id = Some(event.event_id);
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| AirpError::Internal("conversation sequence exhausted".to_string()))?;
        hasher.update(&buffer);
    }
    Ok(PrefixScan {
        next_sequence: expected_sequence,
        first_event_id,
        last_event_id,
        journal_sha256: hex_digest(hasher.finalize()),
    })
}

fn rebuild_checkpoint(
    manifest: &ConversationManifest,
    path: &Path,
) -> Result<ConversationContextCheckpoint, AirpError> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = Vec::new();
    let mut hasher = Sha256::new();
    let mut expected_sequence = 0u64;
    let mut offset = 0u64;
    let mut first_event_id = None;
    let mut last_event_id = None;
    let mut total_message_count = 0u64;
    let mut tail_event = None;
    let mut latest_summary = None;
    let mut recent_messages = VecDeque::with_capacity(RECENT_MESSAGE_INDEX_LIMIT);

    loop {
        buffer.clear();
        let bytes_read = reader.read_until(b'\n', &mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        let event = parse_event_line(&buffer)?;
        validate_event_identity(&event, manifest.conversation_id, expected_sequence)?;
        first_event_id.get_or_insert_with(|| event.event_id.clone());

        if event.kind == CONVERSATION_CONTEXT_SUMMARY_EVENT {
            let payload = summary_payload(&event)?;
            validate_summary_payload(
                &payload,
                event.sequence,
                first_event_id.as_deref(),
                last_event_id.as_deref(),
                &hex_digest(hasher.clone().finalize()),
            )
            .map_err(|error| {
                AirpError::Internal(format!(
                    "invalid committed context summary at sequence {}: {error}",
                    event.sequence
                ))
            })?;
            latest_summary = Some(ContextEventRef {
                offset,
                sequence: event.sequence,
                event_id: event.event_id.clone(),
                estimated_tokens: summary_message_tokens(&payload),
            });
            recent_messages.clear();
        } else if let Some(message) = project_message(&event) {
            total_message_count = total_message_count.saturating_add(1);
            if recent_messages.len() == RECENT_MESSAGE_INDEX_LIMIT {
                recent_messages.pop_front();
            }
            recent_messages.push_back(ContextEventRef {
                offset,
                sequence: event.sequence,
                event_id: event.event_id.clone(),
                estimated_tokens: message_tokens(&message),
            });
        }

        tail_event = Some(ContextEventRef {
            offset,
            sequence: event.sequence,
            event_id: event.event_id.clone(),
            estimated_tokens: 0,
        });
        hasher.update(&buffer);
        last_event_id = Some(event.event_id);
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| AirpError::Internal("conversation sequence exhausted".to_string()))?;
        offset = offset
            .checked_add(u64::try_from(bytes_read).map_err(|_| {
                AirpError::Internal("conversation journal offset overflow".to_string())
            })?)
            .ok_or_else(|| {
                AirpError::Internal("conversation journal offset overflow".to_string())
            })?;
    }

    Ok(ConversationContextCheckpoint {
        schema_version: CONTEXT_CHECKPOINT_SCHEMA_VERSION,
        journal_bytes: offset,
        source_next_sequence: expected_sequence,
        total_message_count,
        tail_event,
        latest_summary,
        recent_messages: recent_messages.into_iter().collect(),
    })
}

fn load_verified_checkpoint(
    manifest: &ConversationManifest,
    checkpoint_path: &Path,
    journal_path: &Path,
) -> Result<Option<LoadedCheckpoint>, AirpError> {
    let Ok(bytes) = std::fs::read(checkpoint_path) else {
        return Ok(None);
    };
    let Ok(envelope) = serde_json::from_slice::<ConversationContextCheckpointEnvelope>(&bytes)
    else {
        return Ok(None);
    };
    if envelope.checkpoint.schema_version != CONTEXT_CHECKPOINT_SCHEMA_VERSION {
        return Ok(None);
    }
    let checkpoint_bytes = serde_json::to_vec(&envelope.checkpoint)?;
    if sha256_bytes(&checkpoint_bytes) != envelope.checkpoint_sha256 {
        return Ok(None);
    }
    let journal_bytes = journal_path.metadata()?.len();
    if journal_bytes < envelope.checkpoint.journal_bytes {
        return Ok(None);
    }
    let mut reader = BufReader::new(File::open(journal_path)?);
    match envelope.checkpoint.tail_event.as_ref() {
        Some(tail_reference) => {
            let Ok((tail_event, tail_end)) = read_event_ref(&mut reader, tail_reference) else {
                return Ok(None);
            };
            if tail_end != envelope.checkpoint.journal_bytes
                || tail_event
                    .sequence
                    .checked_add(1)
                    .is_none_or(|next| next != envelope.checkpoint.source_next_sequence)
            {
                return Ok(None);
            }
        }
        None => {
            if envelope.checkpoint.journal_bytes != 0
                || envelope.checkpoint.source_next_sequence != 0
            {
                return Ok(None);
            }
        }
    }
    if let Some(reference) = envelope.checkpoint.latest_summary.as_ref() {
        let Ok((event, _)) = read_event_ref(&mut reader, reference) else {
            return Ok(None);
        };
        if event.kind != CONVERSATION_CONTEXT_SUMMARY_EVENT {
            return Ok(None);
        }
        let Ok(payload) = summary_payload(&event) else {
            return Ok(None);
        };
        if summary_message_tokens(&payload) != reference.estimated_tokens {
            return Ok(None);
        }
    }
    for reference in &envelope.checkpoint.recent_messages {
        let Ok((event, _)) = read_event_ref(&mut reader, reference) else {
            return Ok(None);
        };
        let Some(message) = project_message(&event) else {
            return Ok(None);
        };
        if message_tokens(&message) != reference.estimated_tokens {
            return Ok(None);
        }
    }
    if journal_bytes == envelope.checkpoint.journal_bytes {
        return Ok(Some(LoadedCheckpoint {
            checkpoint: envelope.checkpoint,
            changed: false,
        }));
    }
    let Some(checkpoint) =
        extend_checkpoint(manifest, journal_path, envelope.checkpoint, journal_bytes)?
    else {
        return Ok(None);
    };
    Ok(Some(LoadedCheckpoint {
        checkpoint,
        changed: true,
    }))
}

fn extend_checkpoint(
    manifest: &ConversationManifest,
    journal_path: &Path,
    mut checkpoint: ConversationContextCheckpoint,
    journal_bytes: u64,
) -> Result<Option<ConversationContextCheckpoint>, AirpError> {
    let mut reader = BufReader::new(File::open(journal_path)?);
    reader.seek(SeekFrom::Start(checkpoint.journal_bytes))?;
    let mut buffer = Vec::new();
    let mut offset = checkpoint.journal_bytes;
    let mut expected_sequence = checkpoint.source_next_sequence;

    loop {
        buffer.clear();
        let bytes_read = reader.read_until(b'\n', &mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        let event = parse_event_line(&buffer)?;
        validate_event_identity(&event, manifest.conversation_id, expected_sequence)?;
        if event.kind == CONVERSATION_CONTEXT_SUMMARY_EVENT {
            // A summary validates a digest and IDs for the complete preceding
            // prefix. Rebuild once at this uncommon boundary rather than
            // weakening that validation for the incremental hot path.
            return Ok(None);
        }
        if let Some(message) = project_message(&event) {
            checkpoint.total_message_count = checkpoint.total_message_count.saturating_add(1);
            if checkpoint.recent_messages.len() == RECENT_MESSAGE_INDEX_LIMIT {
                checkpoint.recent_messages.remove(0);
            }
            checkpoint.recent_messages.push(ContextEventRef {
                offset,
                sequence: event.sequence,
                event_id: event.event_id.clone(),
                estimated_tokens: message_tokens(&message),
            });
        }
        checkpoint.tail_event = Some(ContextEventRef {
            offset,
            sequence: event.sequence,
            event_id: event.event_id,
            estimated_tokens: 0,
        });
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| AirpError::Internal("conversation sequence exhausted".to_string()))?;
        offset = offset
            .checked_add(u64::try_from(bytes_read).map_err(|_| {
                AirpError::Internal("conversation journal offset overflow".to_string())
            })?)
            .ok_or_else(|| {
                AirpError::Internal("conversation journal offset overflow".to_string())
            })?;
    }
    if offset != journal_bytes {
        return Err(AirpError::Internal(
            "conversation journal changed while extending context checkpoint".to_string(),
        ));
    }
    checkpoint.journal_bytes = offset;
    checkpoint.source_next_sequence = expected_sequence;
    Ok(Some(checkpoint))
}

fn write_checkpoint(
    path: &Path,
    checkpoint: &ConversationContextCheckpoint,
) -> Result<(), AirpError> {
    let checkpoint_bytes = serde_json::to_vec(checkpoint)?;
    let envelope = ConversationContextCheckpointEnvelope {
        checkpoint: checkpoint.clone(),
        checkpoint_sha256: sha256_bytes(&checkpoint_bytes),
    };
    crate::data_dir::replace_file(path, &serde_json::to_vec(&envelope)?)
}

fn project_from_checkpoint(
    manifest: &ConversationManifest,
    path: &Path,
    checkpoint: ConversationContextCheckpoint,
    budget: ConversationContextBudget,
) -> Result<ConversationContextProjection, AirpError> {
    let mut remaining_tokens = budget.max_estimated_tokens;
    let mut summary = None;
    let mut messages = Vec::new();
    let mut reader = BufReader::new(File::open(path)?);

    if let Some(reference) = checkpoint.latest_summary.as_ref() {
        let (event, _) = read_event_ref(&mut reader, reference)?;
        if event.kind != CONVERSATION_CONTEXT_SUMMARY_EVENT {
            return Err(AirpError::Internal(
                "conversation context checkpoint summary mismatch".to_string(),
            ));
        }
        let payload = summary_payload(&event)?;
        let message = ChatMessage {
            role: MessageRole::System,
            content: summary_message_content(&payload),
        };
        let tokens = message_tokens(&message);
        if tokens > remaining_tokens {
            return Err(AirpError::BadRequest(format!(
                "conversation summary requires {tokens} estimated tokens, exceeding context budget {}",
                budget.max_estimated_tokens
            )));
        }
        remaining_tokens -= tokens;
        messages.push(message);
        summary = Some(ConversationContextSummaryRef {
            event_id: event.event_id,
            sequence: event.sequence,
            source: payload.source,
            provenance: payload.provenance,
        });
    }

    let mut selected = VecDeque::new();
    for reference in checkpoint.recent_messages.iter().rev() {
        if selected.len() == budget.max_messages {
            break;
        }
        if reference.estimated_tokens > remaining_tokens {
            if selected.is_empty() {
                return Err(AirpError::BadRequest(format!(
                    "latest conversation message requires {} estimated tokens, exceeding remaining context budget {remaining_tokens}",
                    reference.estimated_tokens
                )));
            }
            break;
        }
        remaining_tokens -= reference.estimated_tokens;
        selected.push_front(reference);
    }
    for reference in selected {
        let (event, _) = read_event_ref(&mut reader, reference)?;
        let message = project_message(&event).ok_or_else(|| {
            AirpError::Internal("conversation context checkpoint message mismatch".to_string())
        })?;
        messages.push(message);
    }

    let estimated_tokens = budget.max_estimated_tokens - remaining_tokens;
    let selected_message_count = messages
        .iter()
        .filter(|message| message.role != MessageRole::System)
        .count() as u64;
    Ok(ConversationContextProjection {
        schema_version: CONVERSATION_CONTEXT_SCHEMA_VERSION,
        conversation_id: manifest.conversation_id,
        messages,
        estimated_tokens,
        omitted_message_count: checkpoint
            .total_message_count
            .saturating_sub(selected_message_count),
        source_next_sequence: checkpoint.source_next_sequence,
        summary,
    })
}

fn read_event_ref(
    reader: &mut BufReader<File>,
    reference: &ContextEventRef,
) -> Result<(ConversationEvent, u64), AirpError> {
    reader.seek(SeekFrom::Start(reference.offset))?;
    let mut buffer = Vec::new();
    if reader.read_until(b'\n', &mut buffer)? == 0 {
        return Err(AirpError::Internal(
            "conversation context checkpoint offset is past journal end".to_string(),
        ));
    }
    let event = parse_event_line(&buffer)?;
    if event.sequence != reference.sequence || event.event_id != reference.event_id {
        return Err(AirpError::Internal(
            "conversation context checkpoint event mismatch".to_string(),
        ));
    }
    let end = reference
        .offset
        .checked_add(
            u64::try_from(buffer.len()).map_err(|_| {
                AirpError::Internal("conversation journal offset overflow".to_string())
            })?,
        )
        .ok_or_else(|| AirpError::Internal("conversation journal offset overflow".to_string()))?;
    Ok((event, end))
}

fn parse_event_line(bytes: &[u8]) -> Result<ConversationEvent, AirpError> {
    let line = std::str::from_utf8(bytes)
        .map_err(|error| {
            AirpError::Internal(format!("conversation journal is not UTF-8: {error}"))
        })?
        .trim();
    if line.is_empty() {
        return Err(AirpError::Internal(
            "conversation journal contains an empty record".to_string(),
        ));
    }
    Ok(serde_json::from_str(line)?)
}

fn project_message(event: &ConversationEvent) -> Option<ChatMessage> {
    if event.kind != "message.created" {
        return None;
    }
    let actor_id = event.actor_id.as_deref()?.trim();
    if actor_id.is_empty() {
        return None;
    }
    let content = event.payload.get("content")?.as_str()?;
    let role = match event.payload.get("role")?.as_str()? {
        "user" => MessageRole::User,
        "assistant" => MessageRole::Assistant,
        _ => return None,
    };
    let content = if role == MessageRole::Assistant {
        format!("[{actor_id}] {content}")
    } else {
        content.to_string()
    };
    Some(ChatMessage { role, content })
}

fn summary_payload(event: &ConversationEvent) -> Result<ConversationSummaryPayload, AirpError> {
    serde_json::from_value(event.payload.clone()).map_err(|error| {
        AirpError::Internal(format!(
            "invalid committed context summary at sequence {}: {error}",
            event.sequence
        ))
    })
}

fn validate_summary_payload(
    payload: &ConversationSummaryPayload,
    source_next_sequence: u64,
    first_event_id: Option<&str>,
    last_event_id: Option<&str>,
    journal_sha256: &str,
) -> Result<(), AirpError> {
    if payload.schema_version != CONVERSATION_CONTEXT_SCHEMA_VERSION {
        return Err(AirpError::BadRequest(format!(
            "unsupported conversation summary schema version: {}",
            payload.schema_version
        )));
    }
    for (field, value) in [
        ("summary.content", payload.content.as_str()),
        (
            "summary.provenance.policy_id",
            payload.provenance.policy_id.as_str(),
        ),
        (
            "summary.provenance.policy_version",
            payload.provenance.policy_version.as_str(),
        ),
        (
            "summary.provenance.provider",
            payload.provenance.provider.as_str(),
        ),
        (
            "summary.provenance.model",
            payload.provenance.model.as_str(),
        ),
        (
            "summary.provenance.operation_id",
            payload.provenance.operation_id.as_str(),
        ),
    ] {
        if value.trim().is_empty() {
            return Err(AirpError::BadRequest(format!("{field} must not be empty")));
        }
    }
    if !crate::ulid::is_valid_id(&payload.provenance.operation_id) {
        return Err(AirpError::BadRequest(
            "summary.provenance.operation_id must be a valid durable Engine ID".to_string(),
        ));
    }
    if payload.target_token_budget == 0
        || payload.target_token_budget > DEFAULT_CONVERSATION_CONTEXT_TOKEN_BUDGET
    {
        return Err(AirpError::BadRequest(format!(
            "summary.target_token_budget must be between 1 and {DEFAULT_CONVERSATION_CONTEXT_TOKEN_BUDGET}"
        )));
    }
    if summary_message_tokens(payload) > payload.target_token_budget {
        return Err(AirpError::BadRequest(
            "conversation summary content exceeds its target token budget".to_string(),
        ));
    }
    let Some(last_sequence) = source_next_sequence.checked_sub(1) else {
        return Err(AirpError::BadRequest(
            "context summary requires a non-empty source journal".to_string(),
        ));
    };
    if payload.source.first_sequence != 0
        || payload.source.last_sequence != last_sequence
        || payload.source.event_count != source_next_sequence
        || Some(payload.source.first_event_id.as_str()) != first_event_id
        || Some(payload.source.last_event_id.as_str()) != last_event_id
        || payload.source.journal_sha256 != journal_sha256
    {
        return Err(AirpError::BadRequest(
            "conversation summary source boundary does not match the authoritative journal prefix"
                .to_string(),
        ));
    }
    Ok(())
}

fn summary_message_content(payload: &ConversationSummaryPayload) -> String {
    format!(
        "[Conversation summary through event sequence {}]\n{}",
        payload.source.last_sequence, payload.content
    )
}

fn summary_message_tokens(payload: &ConversationSummaryPayload) -> usize {
    estimate_context_tokens(&summary_message_content(payload))
}

fn message_tokens(message: &ChatMessage) -> usize {
    estimate_context_tokens(&message.content)
}

fn estimate_context_tokens(content: &str) -> usize {
    // Provider tokenizers differ. UTF-8 bytes are a deterministic,
    // provider-neutral upper bound for byte-fallback tokenizers, so this
    // deliberately reserves more space than the volume-store heuristic.
    content.len()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize())
}

fn hex_digest(bytes: impl AsRef<[u8]>) -> String {
    bytes
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{
        AppendConversationEventRequest, ConversationParticipant, CreateConversationRequest,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::fs::{self, File};
    use std::io::{BufWriter, Write};
    use std::time::{Duration, Instant};
    use tempfile::tempdir;

    struct JournalFixture {
        first_event_id: String,
        last_event_id: String,
        event_count: u64,
        sha256: String,
    }

    fn create_request() -> CreateConversationRequest {
        CreateConversationRequest {
            user_id: None,
            title: Some("long history".to_string()),
            participants: vec![
                ConversationParticipant {
                    participant_id: "human:gm".to_string(),
                    kind: "human".to_string(),
                    display_name: None,
                    resource: None,
                    extensions: BTreeMap::new(),
                },
                ConversationParticipant {
                    participant_id: "alice".to_string(),
                    kind: "character".to_string(),
                    display_name: None,
                    resource: None,
                    extensions: BTreeMap::new(),
                },
            ],
            resources: Vec::new(),
            orchestration: None,
            extensions: BTreeMap::new(),
        }
    }

    fn write_message_journal(
        service: &ConversationService,
        conversation_id: SessionId,
        event_count: u64,
    ) -> JournalFixture {
        let path = service.events_path(conversation_id);
        let mut writer = BufWriter::new(File::create(&path).unwrap());
        let mut first_event_id = None;
        let mut last_event_id = None;
        for sequence in 0..event_count {
            let event_id = crate::ulid::new_id();
            first_event_id.get_or_insert_with(|| event_id.clone());
            last_event_id = Some(event_id.clone());
            let event = ConversationEvent {
                schema_version: crate::conversation::CONVERSATION_EVENT_SCHEMA_VERSION,
                event_id,
                conversation_id,
                sequence,
                kind: "message.created".to_string(),
                actor_id: Some(if sequence % 2 == 0 {
                    "human:gm".to_string()
                } else {
                    "alice".to_string()
                }),
                causation_id: None,
                correlation_id: None,
                payload: json!({
                    "role": if sequence % 2 == 0 { "user" } else { "assistant" },
                    "content": format!("message-{sequence}-{}", "x".repeat(32)),
                }),
                extensions: BTreeMap::new(),
                occurred_at: "2026-07-29T00:00:00Z".to_string(),
            };
            serde_json::to_writer(&mut writer, &event).unwrap();
            writer.write_all(b"\n").unwrap();
        }
        writer.flush().unwrap();
        drop(writer);
        let bytes = fs::read(path).unwrap();
        JournalFixture {
            first_event_id: first_event_id.unwrap(),
            last_event_id: last_event_id.unwrap(),
            event_count,
            sha256: sha256_bytes(&bytes),
        }
    }

    fn summary_payload(source: &JournalFixture) -> ConversationSummaryPayload {
        ConversationSummaryPayload {
            schema_version: CONVERSATION_CONTEXT_SCHEMA_VERSION,
            content: "The party reached the old observatory.".to_string(),
            target_token_budget: 128,
            source: ConversationSummarySource {
                first_sequence: 0,
                last_sequence: source.event_count - 1,
                event_count: source.event_count,
                first_event_id: source.first_event_id.clone(),
                last_event_id: source.last_event_id.clone(),
                journal_sha256: source.sha256.clone(),
            },
            provenance: ConversationSummaryProvenance {
                policy_id: "airp.context.prefix_summary".to_string(),
                policy_version: "1".to_string(),
                provider: "test-provider".to_string(),
                model: "test-model".to_string(),
                operation_id: crate::ulid::new_id(),
            },
        }
    }

    fn append_request(
        kind: &str,
        actor_id: Option<&str>,
        payload: serde_json::Value,
        expected_next_sequence: u64,
    ) -> AppendConversationEventRequest {
        AppendConversationEventRequest {
            user_id: None,
            kind: kind.to_string(),
            actor_id: actor_id.map(str::to_string),
            causation_id: None,
            correlation_id: None,
            payload,
            extensions: BTreeMap::new(),
            expected_next_sequence: Some(expected_next_sequence),
        }
    }

    #[test]
    fn current_turn_messages_share_the_engine_context_budget() {
        let budget = ConversationContextBudget {
            max_estimated_tokens: 32,
            max_messages: 2,
        };
        let mut messages = vec![ChatMessage {
            role: MessageRole::System,
            content: "summary".to_string(),
        }];
        push_bounded_context_message(
            &mut messages,
            ChatMessage {
                role: MessageRole::User,
                content: "old-message".to_string(),
            },
            budget,
        )
        .unwrap();
        push_bounded_context_message(
            &mut messages,
            ChatMessage {
                role: MessageRole::Assistant,
                content: "new-assistant-message".to_string(),
            },
            budget,
        )
        .unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, MessageRole::System);
        assert_eq!(messages[1].content, "new-assistant-message");
        assert!(messages.iter().map(message_tokens).sum::<usize>() <= 32);
    }

    #[test]
    fn oversized_latest_message_fails_without_mutating_bounded_context() {
        let mut messages = vec![
            ChatMessage {
                role: MessageRole::System,
                content: "summary".to_string(),
            },
            ChatMessage {
                role: MessageRole::User,
                content: "older".to_string(),
            },
        ];
        let before = messages
            .iter()
            .map(|message| (message.role, message.content.clone()))
            .collect::<Vec<_>>();
        let error = push_bounded_context_message(
            &mut messages,
            ChatMessage {
                role: MessageRole::User,
                content: "x".repeat(64),
            },
            ConversationContextBudget {
                max_estimated_tokens: 32,
                max_messages: 2,
            },
        )
        .unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
        assert_eq!(
            messages
                .iter()
                .map(|message| (message.role, message.content.clone()))
                .collect::<Vec<_>>(),
            before
        );
    }

    #[tokio::test]
    async fn bounded_context_keeps_authoritative_history_and_rebuilds_deleted_checkpoint() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(create_request()).await.unwrap();
        let fixture = write_message_journal(&service, manifest.conversation_id, 10_000);
        let budget = ConversationContextBudget {
            max_estimated_tokens: 256,
            max_messages: 12,
        };

        let first = service
            .context_projection(manifest.conversation_id, budget)
            .await
            .unwrap();
        assert!(first.messages.len() <= budget.max_messages);
        assert!(first.estimated_tokens <= budget.max_estimated_tokens);
        assert_eq!(
            first.messages.last().unwrap().content,
            format!("[alice] message-9999-{}", "x".repeat(32))
        );
        assert_eq!(
            first.omitted_message_count + first.messages.len() as u64,
            fixture.event_count
        );

        let checkpoint = service
            .conversation_dir(manifest.conversation_id)
            .join("context_checkpoint.json");
        assert!(checkpoint.is_file());
        fs::remove_file(&checkpoint).unwrap();
        let rebuilt = service
            .context_projection(manifest.conversation_id, budget)
            .await
            .unwrap();
        assert_eq!(rebuilt.source_next_sequence, fixture.event_count);
        assert_eq!(
            rebuilt
                .messages
                .iter()
                .map(|message| (&message.role, &message.content))
                .collect::<Vec<_>>(),
            first
                .messages
                .iter()
                .map(|message| (&message.role, &message.content))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            service
                .events(manifest.conversation_id, Some(7), None)
                .await
                .unwrap()
                .total as u64,
            fixture.event_count
        );
    }

    #[tokio::test]
    async fn summary_requires_verified_prefix_and_preserves_original_events() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(create_request()).await.unwrap();
        let fixture = write_message_journal(&service, manifest.conversation_id, 100);
        let payload = summary_payload(&fixture);
        let summary_event = service
            .append_event(
                manifest.conversation_id,
                append_request(
                    CONVERSATION_CONTEXT_SUMMARY_EVENT,
                    None,
                    serde_json::to_value(&payload).unwrap(),
                    fixture.event_count,
                ),
            )
            .await
            .unwrap();
        for sequence in 101..106 {
            service
                .append_event(
                    manifest.conversation_id,
                    append_request(
                        "message.created",
                        Some("alice"),
                        json!({"role": "assistant", "content": format!("tail-{sequence}")}),
                        sequence,
                    ),
                )
                .await
                .unwrap();
        }

        let projection = service
            .context_projection(
                manifest.conversation_id,
                ConversationContextBudget {
                    max_estimated_tokens: 512,
                    max_messages: 16,
                },
            )
            .await
            .unwrap();
        assert_eq!(projection.messages[0].role, MessageRole::System);
        assert!(projection.messages[0].content.contains(&payload.content));
        assert_eq!(projection.messages.len(), 6);
        assert_eq!(projection.omitted_message_count, 100);
        assert_eq!(
            projection.summary.as_ref().unwrap().event_id,
            summary_event.event_id
        );
        assert_eq!(
            service
                .events(manifest.conversation_id, Some(7), None)
                .await
                .unwrap()
                .total,
            106
        );
    }

    #[tokio::test]
    async fn invalid_summary_boundary_fails_before_append() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(create_request()).await.unwrap();
        let fixture = write_message_journal(&service, manifest.conversation_id, 3);
        let mut payload = summary_payload(&fixture);
        payload.source.journal_sha256 = "0".repeat(64);

        let error = service
            .append_event(
                manifest.conversation_id,
                append_request(
                    CONVERSATION_CONTEXT_SUMMARY_EVENT,
                    None,
                    serde_json::to_value(payload).unwrap(),
                    fixture.event_count,
                ),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
        assert_eq!(
            service
                .events(manifest.conversation_id, Some(10), None)
                .await
                .unwrap()
                .total,
            3
        );
    }

    #[tokio::test]
    async fn stale_checkpoint_extends_after_journal_append() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(create_request()).await.unwrap();
        let fixture = write_message_journal(&service, manifest.conversation_id, 20);
        service
            .context_projection(
                manifest.conversation_id,
                ConversationContextBudget::default(),
            )
            .await
            .unwrap();

        service
            .append_event(
                manifest.conversation_id,
                append_request(
                    "message.created",
                    Some("human:gm"),
                    json!({"role": "user", "content": "after-checkpoint"}),
                    fixture.event_count,
                ),
            )
            .await
            .unwrap();
        let checkpoint_path = service
            .conversation_dir(manifest.conversation_id)
            .join("context_checkpoint.json");
        let journal_path = service.events_path(manifest.conversation_id);
        let extended = load_verified_checkpoint(&manifest, &checkpoint_path, &journal_path)
            .unwrap()
            .unwrap();
        assert!(extended.changed);
        assert_eq!(
            extended.checkpoint.source_next_sequence,
            fixture.event_count + 1
        );
        assert_eq!(
            extended.checkpoint.recent_messages.last().unwrap().sequence,
            fixture.event_count
        );
        let projection = service
            .context_projection(
                manifest.conversation_id,
                ConversationContextBudget::default(),
            )
            .await
            .unwrap();
        assert_eq!(projection.source_next_sequence, fixture.event_count + 1);
        assert_eq!(
            projection.messages.last().unwrap().content,
            "after-checkpoint"
        );
    }

    #[tokio::test]
    async fn tampered_checkpoint_checksum_rebuilds_from_journal() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(create_request()).await.unwrap();
        write_message_journal(&service, manifest.conversation_id, 1_000);
        let budget = ConversationContextBudget::default();
        let original = service
            .context_projection(manifest.conversation_id, budget)
            .await
            .unwrap();
        let checkpoint_path = service
            .conversation_dir(manifest.conversation_id)
            .join("context_checkpoint.json");
        let mut envelope: ConversationContextCheckpointEnvelope =
            serde_json::from_slice(&fs::read(&checkpoint_path).unwrap()).unwrap();
        envelope.checkpoint_sha256 = "0".repeat(64);
        fs::write(&checkpoint_path, serde_json::to_vec(&envelope).unwrap()).unwrap();

        let rebuilt = service
            .context_projection(manifest.conversation_id, budget)
            .await
            .unwrap();
        assert_eq!(
            rebuilt
                .messages
                .iter()
                .map(|message| (message.role, message.content.as_str()))
                .collect::<Vec<_>>(),
            original
                .messages
                .iter()
                .map(|message| (message.role, message.content.as_str()))
                .collect::<Vec<_>>()
        );
        let repaired: ConversationContextCheckpointEnvelope =
            serde_json::from_slice(&fs::read(checkpoint_path).unwrap()).unwrap();
        assert_ne!(repaired.checkpoint_sha256, "0".repeat(64));
    }

    #[tokio::test]
    #[ignore = "long-history benchmark and soak; run explicitly with --release --ignored --nocapture"]
    async fn long_history_context_benchmark_and_soak() {
        const EVENTS: u64 = 50_000;
        const WARM_READS: usize = 50;
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let manifest = service.create(create_request()).await.unwrap();
        write_message_journal(&service, manifest.conversation_id, EVENTS);
        let budget = ConversationContextBudget::default();

        let cold_started = Instant::now();
        let cold = service
            .context_projection(manifest.conversation_id, budget)
            .await
            .unwrap();
        let cold_elapsed = cold_started.elapsed();
        let mut append_aware_elapsed = Duration::ZERO;
        for index in 0..WARM_READS {
            let expected_next_sequence = EVENTS + index as u64;
            service
                .append_event(
                    manifest.conversation_id,
                    append_request(
                        "message.created",
                        Some("human:gm"),
                        json!({
                            "role": "user",
                            "content": format!("append-aware-{index}")
                        }),
                        expected_next_sequence,
                    ),
                )
                .await
                .unwrap();
            let projection_started = Instant::now();
            let projection = service
                .context_projection(manifest.conversation_id, budget)
                .await
                .unwrap();
            append_aware_elapsed += projection_started.elapsed();
            assert!(projection.messages.len() <= budget.max_messages);
            assert!(projection.estimated_tokens <= budget.max_estimated_tokens);
            assert_eq!(projection.source_next_sequence, expected_next_sequence + 1);
        }
        eprintln!(
            "conversation context benchmark: events={EVENTS} cold_ms={} append_aware_reads={WARM_READS} append_aware_total_ms={} append_aware_mean_ms={:.3} retained_messages={}",
            cold_elapsed.as_millis(),
            append_aware_elapsed.as_millis(),
            append_aware_elapsed.as_secs_f64() * 1000.0 / WARM_READS as f64,
            cold.messages.len()
        );
    }
}
