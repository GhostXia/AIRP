//! Durable, UI-independent lifecycle contract for Conversation turns.
//!
//! Lifecycle state is reconstructed exclusively from the append-only
//! Conversation journal. The in-memory cancellation registry is only a
//! cooperative signal for work running in this daemon process; it is never the
//! source of truth after restart.

use crate::conversation::{
    ConversationEvent, ConversationTurnFailure, ConversationTurnOutcome, ConversationTurnRequest,
    ConversationTurnStatus,
};
use crate::error::AirpError;
use crate::types::SessionId;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use tokio_util::sync::CancellationToken;

pub const TURN_ACCEPTED: &str = "turn.accepted";
pub const TURN_STARTED: &str = "turn.started";
pub const TURN_COMPLETED: &str = "turn.completed";
pub const TURN_FAILED: &str = "turn.failed";
pub const TURN_CANCELLED: &str = "turn.cancelled";
pub const TURN_UNKNOWN_COMMIT: &str = "turn.unknown_commit";

/// Durable lifecycle states and their legal forward transitions.
///
/// `unknown_commit` means the Engine found durable evidence that a turn began
/// but no terminal journal record. It deliberately does not guess whether an
/// upstream provider committed side effects before the process stopped.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationTurnLifecycleState {
    Accepted,
    Running,
    Completed,
    Failed,
    Cancelled,
    UnknownCommit,
}

impl ConversationTurnLifecycleState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::UnknownCommit
        )
    }
}

/// Journal-derived view returned by the turn status and cancel endpoints.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnSnapshot {
    pub turn_id: String,
    pub lifecycle_state: ConversationTurnLifecycleState,
    pub events: Vec<ConversationEvent>,
    pub next_sequence: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<ConversationTurnFailure>,
}

/// Acknowledgement for an explicit cooperative cancellation request.
#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnCancelResponse {
    pub turn_id: String,
    pub cancel_requested: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_state: Option<ConversationTurnLifecycleState>,
}

impl ConversationTurnSnapshot {
    pub fn into_outcome(self) -> ConversationTurnOutcome {
        let status = if self.lifecycle_state == ConversationTurnLifecycleState::Completed {
            ConversationTurnStatus::Completed
        } else {
            // Additive compatibility with the original turn response. Precise
            // state lives in lifecycle_state.
            ConversationTurnStatus::PartiallyCommitted
        };
        ConversationTurnOutcome {
            turn_id: self.turn_id,
            status,
            lifecycle_state: self.lifecycle_state,
            events: self.events,
            next_sequence: self.next_sequence,
            failure: self.failure,
        }
    }
}

/// Stable digest of the semantic request, excluding its retry identity.
pub fn request_fingerprint(request: &ConversationTurnRequest) -> Result<String, AirpError> {
    let mut value = serde_json::to_value(request)?;
    if let Value::Object(object) = &mut value {
        object.remove("turn_id");
    }
    canonicalize_json(&mut value);
    let bytes = serde_json::to_vec(&value)?;
    let digest = Sha256::digest(bytes);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn canonicalize_json(value: &mut Value) {
    match value {
        Value::Object(object) => {
            let mut entries: Vec<_> = std::mem::take(object).into_iter().collect();
            for (_, value) in &mut entries {
                canonicalize_json(value);
            }
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            object.extend(entries);
        }
        Value::Array(values) => {
            for value in values {
                canonicalize_json(value);
            }
        }
        _ => {}
    }
}

/// Reconstruct one turn from correlated journal events.
pub fn project_turn(
    events: &[ConversationEvent],
    turn_id: &str,
) -> Result<Option<ConversationTurnSnapshot>, AirpError> {
    let correlated: Vec<_> = events
        .iter()
        .filter(|event| event.correlation_id.as_deref() == Some(turn_id))
        .cloned()
        .collect();
    if correlated.is_empty() {
        return Ok(None);
    }

    let mut state = None;
    let mut failure = None;
    for event in &correlated {
        let next = match event.kind.as_str() {
            TURN_ACCEPTED => Some(ConversationTurnLifecycleState::Accepted),
            TURN_STARTED => Some(ConversationTurnLifecycleState::Running),
            TURN_COMPLETED => Some(ConversationTurnLifecycleState::Completed),
            TURN_FAILED => {
                failure = Some(failure_from_event(event, "turn_failed"));
                Some(ConversationTurnLifecycleState::Failed)
            }
            TURN_CANCELLED => {
                failure = Some(failure_from_event(event, "turn_cancelled"));
                Some(ConversationTurnLifecycleState::Cancelled)
            }
            TURN_UNKNOWN_COMMIT => {
                failure = Some(failure_from_event(event, "unknown_commit"));
                Some(ConversationTurnLifecycleState::UnknownCommit)
            }
            _ => None,
        };
        if let Some(next) = next {
            validate_transition(state, next, turn_id)?;
            state = Some(next);
        }
    }

    let Some(lifecycle_state) = state else {
        return Err(AirpError::Conflict(format!(
            "turn {turn_id} has correlated events but no lifecycle record"
        )));
    };
    let next_sequence = correlated
        .last()
        .map_or(0, |event| event.sequence.saturating_add(1));
    Ok(Some(ConversationTurnSnapshot {
        turn_id: turn_id.to_string(),
        lifecycle_state,
        events: correlated,
        next_sequence,
        failure,
    }))
}

fn validate_transition(
    current: Option<ConversationTurnLifecycleState>,
    next: ConversationTurnLifecycleState,
    turn_id: &str,
) -> Result<(), AirpError> {
    use ConversationTurnLifecycleState as State;
    let legal = matches!(
        (current, next),
        (None, State::Accepted)
            | (Some(State::Accepted), State::Running)
            | (Some(State::Accepted), State::Failed | State::Cancelled | State::UnknownCommit)
            | (
                Some(State::Running),
                State::Completed | State::Failed | State::Cancelled | State::UnknownCommit
            )
            // Journals written before lifecycle events were introduced.
            | (None, State::Completed | State::Failed)
    );
    if legal {
        Ok(())
    } else {
        Err(AirpError::Conflict(format!(
            "turn {turn_id} has illegal lifecycle transition {current:?} -> {next:?}"
        )))
    }
}

fn failure_from_event(event: &ConversationEvent, default_code: &str) -> ConversationTurnFailure {
    ConversationTurnFailure {
        code: event
            .payload
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or(default_code)
            .to_string(),
        participant_id: event
            .payload
            .get("participant_id")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

pub fn accepted_fingerprint(snapshot: &ConversationTurnSnapshot) -> Option<&str> {
    snapshot
        .events
        .iter()
        .find(|event| event.kind == TURN_ACCEPTED)
        .and_then(|event| event.payload.get("request_fingerprint"))
        .and_then(Value::as_str)
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ActiveTurnKey {
    data_root: PathBuf,
    conversation_id: SessionId,
    turn_id: String,
}

struct ActiveTurn {
    cancellation: CancellationToken,
    snapshot: Arc<Mutex<Option<ConversationTurnSnapshot>>>,
    registrations: usize,
}

static ACTIVE_TURNS: OnceLock<Mutex<HashMap<ActiveTurnKey, ActiveTurn>>> = OnceLock::new();

pub(crate) struct ActiveTurnRegistration {
    key: ActiveTurnKey,
    cancellation: CancellationToken,
    snapshot: Arc<Mutex<Option<ConversationTurnSnapshot>>>,
}

impl ActiveTurnRegistration {
    pub(crate) fn cancellation(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    pub(crate) fn publish(
        &self,
        lifecycle_state: ConversationTurnLifecycleState,
        events: &[ConversationEvent],
    ) {
        let next_sequence = events
            .last()
            .map_or(0, |event| event.sequence.saturating_add(1));
        *self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ConversationTurnSnapshot {
            turn_id: self.key.turn_id.clone(),
            lifecycle_state,
            events: events.to_vec(),
            next_sequence,
            failure: None,
        });
    }

    pub(crate) fn publish_outcome(&self, outcome: &ConversationTurnOutcome) {
        *self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ConversationTurnSnapshot {
            turn_id: outcome.turn_id.clone(),
            lifecycle_state: outcome.lifecycle_state,
            events: outcome.events.clone(),
            next_sequence: outcome.next_sequence,
            failure: outcome.failure.clone(),
        });
    }
}

impl Drop for ActiveTurnRegistration {
    fn drop(&mut self) {
        let mut active = ACTIVE_TURNS
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let remove = if let Some(entry) = active.get_mut(&self.key) {
            entry.registrations = entry.registrations.saturating_sub(1);
            entry.registrations == 0
        } else {
            false
        };
        if remove {
            active.remove(&self.key);
        }
    }
}

fn active_key(data_root: &Path, conversation_id: SessionId, turn_id: &str) -> ActiveTurnKey {
    ActiveTurnKey {
        data_root: data_root.to_path_buf(),
        conversation_id,
        turn_id: turn_id.to_string(),
    }
}

/// Register process-local work. A concurrent retry shares the same signal.
pub(crate) fn register_active_turn(
    data_root: &Path,
    conversation_id: SessionId,
    turn_id: &str,
) -> ActiveTurnRegistration {
    let key = active_key(data_root, conversation_id, turn_id);
    let mut active = ACTIVE_TURNS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let entry = active.entry(key.clone()).or_insert_with(|| ActiveTurn {
        cancellation: CancellationToken::new(),
        snapshot: Arc::new(Mutex::new(None)),
        registrations: 0,
    });
    entry.registrations = entry.registrations.saturating_add(1);
    ActiveTurnRegistration {
        key,
        cancellation: entry.cancellation.clone(),
        snapshot: Arc::clone(&entry.snapshot),
    }
}

pub(crate) fn cancel_active_turn(
    data_root: &Path,
    conversation_id: SessionId,
    turn_id: &str,
) -> bool {
    let active = ACTIVE_TURNS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(entry) = active.get(&active_key(data_root, conversation_id, turn_id)) {
        entry.cancellation.cancel();
        true
    } else {
        false
    }
}

pub(crate) fn active_turn_snapshot(
    data_root: &Path,
    conversation_id: SessionId,
    turn_id: &str,
) -> Option<ConversationTurnSnapshot> {
    let snapshot = ACTIVE_TURNS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&active_key(data_root, conversation_id, turn_id))
        .map(|entry| Arc::clone(&entry.snapshot))?;
    let value = snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::CONVERSATION_EVENT_SCHEMA_VERSION;
    use std::collections::BTreeMap;

    fn event(sequence: u64, kind: &str) -> ConversationEvent {
        ConversationEvent {
            schema_version: CONVERSATION_EVENT_SCHEMA_VERSION,
            event_id: crate::ulid::new_id(),
            conversation_id: SessionId::new(),
            sequence,
            kind: kind.to_string(),
            actor_id: None,
            causation_id: None,
            correlation_id: Some("turn".to_string()),
            payload: Value::Null,
            extensions: BTreeMap::new(),
            occurred_at: String::new(),
        }
    }

    #[test]
    fn legal_lifecycle_reconstructs_terminal_state() {
        let events = vec![
            event(0, TURN_ACCEPTED),
            event(1, TURN_STARTED),
            event(2, "message.created"),
            event(3, TURN_COMPLETED),
        ];
        let snapshot = project_turn(&events, "turn").unwrap().unwrap();
        assert_eq!(
            snapshot.lifecycle_state,
            ConversationTurnLifecycleState::Completed
        );
        assert_eq!(snapshot.next_sequence, 4);
    }

    #[test]
    fn terminal_state_cannot_transition_again() {
        let events = vec![
            event(0, TURN_ACCEPTED),
            event(1, TURN_STARTED),
            event(2, TURN_COMPLETED),
            event(3, TURN_FAILED),
        ];
        assert!(matches!(
            project_turn(&events, "turn"),
            Err(AirpError::Conflict(_))
        ));
    }
}
