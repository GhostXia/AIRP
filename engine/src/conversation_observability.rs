//! Stable, redacted discovery and observability contracts for Conversation.
//!
//! Observability is projected from the authoritative turn journal. It never
//! copies message content, provider responses, prompts, credentials, or model
//! configuration into a second telemetry store.

use crate::conversation::{
    ConversationTurnFailure, CONVERSATION_EVENT_SCHEMA_VERSION, CONVERSATION_SCHEMA_VERSION,
};
use crate::conversation_context::{
    CONVERSATION_CONTEXT_SCHEMA_VERSION, DEFAULT_CONVERSATION_CONTEXT_MESSAGE_BUDGET,
    DEFAULT_CONVERSATION_CONTEXT_TOKEN_BUDGET,
};
use crate::conversation_policy::{
    CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION, MAX_CONVERSATION_POLICY_CONFIG_BYTES,
    MAX_CONVERSATION_POLICY_PARALLELISM, MAX_CONVERSATION_POLICY_PLANNING_TIMEOUT_MS,
    MAX_CONVERSATION_SPEAKERS_PER_TURN,
};
use crate::conversation_turn::{
    ConversationTurnLifecycleState, ConversationTurnSnapshot, TURN_ACCEPTED, TURN_CANCELLED,
    TURN_COMPLETED, TURN_FAILED, TURN_UNKNOWN_COMMIT,
};
use chrono::DateTime;
use serde::Serialize;
use serde_json::Value;

pub const CONVERSATION_CAPABILITY_SCHEMA_VERSION: u32 = 1;
pub const CONVERSATION_OBSERVABILITY_SCHEMA_VERSION: u32 = 1;
pub const CONVERSATION_TURN_ERROR_SCHEMA_VERSION: u32 = 1;
pub const CONVERSATION_API_CONTRACT_VERSION: &str = "airp.conversation.v1";
pub const CONVERSATION_TURN_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationRecoveryAction {
    InspectAndContinue,
    ResubmitNewTurn,
    ManualRetryOrContinue,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationErrorDescriptor {
    pub code: String,
    pub schema_version: u32,
    pub recovery: ConversationRecoveryAction,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSchemaCapabilities {
    pub http_error: u32,
    pub manifest: u32,
    pub event: u32,
    pub context_projection: u32,
    pub policy_descriptor: u32,
    pub migration_report: u32,
    pub turn_error: u32,
    pub turn_observability: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationAdapterCapabilities {
    pub legacy_migration: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationExecutionLimits {
    pub max_speakers_per_turn: usize,
    pub max_parallelism: usize,
    pub max_policy_config_bytes: usize,
    pub max_policy_planning_ms: u64,
    pub turn_timeout_secs: u64,
    pub default_context_tokens: usize,
    pub default_context_messages: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationObservabilityCapabilities {
    pub source: String,
    pub trace_identity: String,
    pub metrics: Vec<String>,
    pub excluded_data: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationCapabilities {
    pub schema_version: u32,
    pub contract_version: String,
    pub schemas: ConversationSchemaCapabilities,
    pub adapter_versions: ConversationAdapterCapabilities,
    pub execution_limits: ConversationExecutionLimits,
    pub observability: ConversationObservabilityCapabilities,
    pub turn_errors: Vec<ConversationErrorDescriptor>,
}

pub fn conversation_capabilities() -> ConversationCapabilities {
    ConversationCapabilities {
        schema_version: CONVERSATION_CAPABILITY_SCHEMA_VERSION,
        contract_version: CONVERSATION_API_CONTRACT_VERSION.to_string(),
        schemas: ConversationSchemaCapabilities {
            http_error: crate::error::AIRP_ERROR_SCHEMA_VERSION,
            manifest: CONVERSATION_SCHEMA_VERSION,
            event: CONVERSATION_EVENT_SCHEMA_VERSION,
            context_projection: CONVERSATION_CONTEXT_SCHEMA_VERSION,
            policy_descriptor: CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION,
            migration_report: crate::conversation_compat::CONVERSATION_MIGRATION_SCHEMA_VERSION,
            turn_error: CONVERSATION_TURN_ERROR_SCHEMA_VERSION,
            turn_observability: CONVERSATION_OBSERVABILITY_SCHEMA_VERSION,
        },
        adapter_versions: ConversationAdapterCapabilities {
            legacy_migration: crate::conversation_compat::CONVERSATION_COMPAT_ADAPTER_VERSION
                .to_string(),
        },
        execution_limits: ConversationExecutionLimits {
            max_speakers_per_turn: MAX_CONVERSATION_SPEAKERS_PER_TURN,
            max_parallelism: MAX_CONVERSATION_POLICY_PARALLELISM,
            max_policy_config_bytes: MAX_CONVERSATION_POLICY_CONFIG_BYTES,
            max_policy_planning_ms: MAX_CONVERSATION_POLICY_PLANNING_TIMEOUT_MS,
            turn_timeout_secs: CONVERSATION_TURN_TIMEOUT_SECS,
            default_context_tokens: DEFAULT_CONVERSATION_CONTEXT_TOKEN_BUDGET,
            default_context_messages: DEFAULT_CONVERSATION_CONTEXT_MESSAGE_BUDGET,
        },
        observability: ConversationObservabilityCapabilities {
            source: "authoritative_conversation_journal".to_string(),
            trace_identity: "turn_id".to_string(),
            metrics: vec![
                "policy_planning_ms".to_string(),
                "turn_latency_ms".to_string(),
                "speaker_latency_ms".to_string(),
                "quota_reserved_requests".to_string(),
                "recorded_output_tokens".to_string(),
                "cancelled".to_string(),
                "partial_commit".to_string(),
            ],
            excluded_data: vec![
                "message_content".to_string(),
                "prompt".to_string(),
                "provider_response".to_string(),
                "provider_error_body".to_string(),
                "credentials".to_string(),
                "model_configuration".to_string(),
            ],
        },
        turn_errors: TURN_ERROR_CODES
            .iter()
            .map(|code| ConversationErrorDescriptor {
                code: (*code).to_string(),
                schema_version: CONVERSATION_TURN_ERROR_SCHEMA_VERSION,
                recovery: recovery_for_code(code),
            })
            .collect(),
    }
}

const TURN_ERROR_CODES: &[&str] = &[
    "context_budget_exceeded",
    "context_projection_failed",
    "empty_generation",
    "generation_failed",
    "generation_preparation_failed",
    "response_serialization_failed",
    "turn_cancelled",
    "turn_timeout",
    "unknown_commit",
];

pub fn recovery_for_code(code: &str) -> ConversationRecoveryAction {
    match code {
        "turn_cancelled" => ConversationRecoveryAction::ResubmitNewTurn,
        "unknown_commit" => ConversationRecoveryAction::ManualRetryOrContinue,
        "context_budget_exceeded"
        | "context_projection_failed"
        | "empty_generation"
        | "generation_failed"
        | "generation_preparation_failed"
        | "response_serialization_failed"
        | "turn_timeout" => ConversationRecoveryAction::InspectAndContinue,
        _ => ConversationRecoveryAction::InspectAndContinue,
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationSpeakerOutcome {
    Planned,
    Committed,
    GeneratedNotCommitted,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyObservation {
    pub policy_id: String,
    pub policy_version: String,
    pub execution_mode: String,
    pub planning_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationSpeakerObservation {
    pub speaker_index: usize,
    pub participant_id: String,
    pub outcome: ConversationSpeakerOutcome,
    pub latency_ms: Option<u64>,
    pub recorded_output_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConversationTurnObservability {
    pub schema_version: u32,
    pub trace_id: String,
    pub turn_id: String,
    pub lifecycle_state: ConversationTurnLifecycleState,
    pub policy: Option<ConversationPolicyObservation>,
    pub speakers: Vec<ConversationSpeakerObservation>,
    pub quota_reserved_requests: u32,
    pub recorded_output_tokens: u64,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub latency_ms: Option<u64>,
    pub commit_state: String,
    pub cancelled: bool,
    pub partial_commit: bool,
    pub failure: Option<ConversationTurnFailure>,
}

pub fn project_turn_observability(
    snapshot: &ConversationTurnSnapshot,
) -> ConversationTurnObservability {
    let mut policy = None;
    let mut speakers = Vec::new();
    let mut quota_reserved_requests = 0;
    let mut terminal_payload = None;

    for event in &snapshot.events {
        if event.kind == TURN_ACCEPTED {
            let policy_value = event.payload.get("policy").unwrap_or(&Value::Null);
            let observation = event.payload.get("observability").unwrap_or(&Value::Null);
            if policy_value.is_object() {
                policy = Some(ConversationPolicyObservation {
                    policy_id: string_field(policy_value, "policy_id"),
                    policy_version: string_field(policy_value, "policy_version"),
                    execution_mode: string_field(policy_value, "execution_mode"),
                    planning_ms: u64_field(observation, "planning_ms"),
                });
            }
            if let Some(planned) = policy_value.get("speakers").and_then(Value::as_array) {
                quota_reserved_requests = u32::try_from(planned.len()).unwrap_or(u32::MAX);
                for speaker in planned {
                    if let Some(participant_id) =
                        speaker.get("participant_id").and_then(Value::as_str)
                    {
                        speakers.push(ConversationSpeakerObservation {
                            speaker_index: speakers.len(),
                            participant_id: participant_id.to_string(),
                            outcome: ConversationSpeakerOutcome::Planned,
                            latency_ms: None,
                            recorded_output_tokens: None,
                        });
                    }
                }
            }
            if let Some(reserved) = u64_field(observation, "quota_reserved_requests") {
                quota_reserved_requests = u32::try_from(reserved).unwrap_or(u32::MAX);
            }
        } else if event.kind == "message.created"
            && event.payload.get("role").and_then(Value::as_str) == Some("assistant")
        {
            let Some(participant_id) = event.actor_id.as_deref() else {
                continue;
            };
            let index = next_speaker_index(&mut speakers, participant_id);
            let observation = event.payload.get("observability").unwrap_or(&Value::Null);
            speakers[index].outcome = ConversationSpeakerOutcome::Committed;
            speakers[index].latency_ms = u64_field(observation, "speaker_latency_ms");
            speakers[index].recorded_output_tokens =
                u64_field(observation, "recorded_output_tokens")
                    .and_then(|value| u32::try_from(value).ok());
        } else if matches!(
            event.kind.as_str(),
            TURN_COMPLETED | TURN_FAILED | TURN_CANCELLED | TURN_UNKNOWN_COMMIT
        ) {
            terminal_payload = Some(&event.payload);
        }
    }

    if let Some(payload) = terminal_payload {
        if let Some(participant_id) = payload.get("participant_id").and_then(Value::as_str) {
            let index = next_speaker_index(&mut speakers, participant_id);
            speakers[index].outcome =
                if snapshot.lifecycle_state == ConversationTurnLifecycleState::Cancelled {
                    ConversationSpeakerOutcome::Cancelled
                } else {
                    ConversationSpeakerOutcome::Failed
                };
            let observation = payload.get("observability").unwrap_or(&Value::Null);
            speakers[index].latency_ms = u64_field(observation, "speaker_latency_ms");
            speakers[index].recorded_output_tokens =
                u64_field(observation, "recorded_output_tokens")
                    .and_then(|value| u32::try_from(value).ok());
        }
        if let Some(uncommitted) = payload
            .get("observability")
            .and_then(|value| value.get("uncommitted_speakers"))
            .and_then(Value::as_array)
        {
            for speaker in uncommitted {
                let Some(participant_id) = speaker.get("participant_id").and_then(Value::as_str)
                else {
                    continue;
                };
                let index = next_speaker_index(&mut speakers, participant_id);
                speakers[index].outcome = match speaker.get("outcome").and_then(Value::as_str) {
                    Some("failed") => ConversationSpeakerOutcome::Failed,
                    _ => ConversationSpeakerOutcome::GeneratedNotCommitted,
                };
                speakers[index].latency_ms = u64_field(speaker, "speaker_latency_ms");
                speakers[index].recorded_output_tokens =
                    u64_field(speaker, "recorded_output_tokens")
                        .and_then(|value| u32::try_from(value).ok());
            }
        }
    }

    let started_at = snapshot
        .events
        .first()
        .map(|event| event.occurred_at.clone());
    let finished_at = snapshot
        .lifecycle_state
        .is_terminal()
        .then(|| {
            snapshot
                .events
                .last()
                .map(|event| event.occurred_at.clone())
        })
        .flatten();
    let latency_ms = started_at
        .as_deref()
        .zip(finished_at.as_deref())
        .and_then(|(start, finish)| elapsed_ms(start, finish));
    let recorded_output_tokens = speakers
        .iter()
        .filter_map(|speaker| speaker.recorded_output_tokens)
        .map(u64::from)
        .sum();
    let commit_state = terminal_payload
        .and_then(|payload| payload.get("commit_state"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            if snapshot.lifecycle_state == ConversationTurnLifecycleState::Completed {
                "completed".to_string()
            } else {
                "pending".to_string()
            }
        });
    let partial_commit = commit_state == "partially_committed";

    ConversationTurnObservability {
        schema_version: CONVERSATION_OBSERVABILITY_SCHEMA_VERSION,
        trace_id: snapshot.turn_id.clone(),
        turn_id: snapshot.turn_id.clone(),
        lifecycle_state: snapshot.lifecycle_state,
        policy,
        speakers,
        quota_reserved_requests,
        recorded_output_tokens,
        started_at,
        finished_at,
        latency_ms,
        commit_state,
        cancelled: snapshot.lifecycle_state == ConversationTurnLifecycleState::Cancelled,
        partial_commit,
        failure: snapshot.failure.clone(),
    }
}

fn next_speaker_index(
    speakers: &mut Vec<ConversationSpeakerObservation>,
    participant_id: &str,
) -> usize {
    if let Some(index) = speakers.iter().position(|speaker| {
        speaker.participant_id == participant_id
            && speaker.outcome == ConversationSpeakerOutcome::Planned
    }) {
        return index;
    }
    let index = speakers.len();
    speakers.push(ConversationSpeakerObservation {
        speaker_index: index,
        participant_id: participant_id.to_string(),
        outcome: ConversationSpeakerOutcome::Planned,
        latency_ms: None,
        recorded_output_tokens: None,
    });
    index
}

fn string_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn u64_field(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn elapsed_ms(start: &str, finish: &str) -> Option<u64> {
    let start = DateTime::parse_from_rfc3339(start).ok()?;
    let finish = DateTime::parse_from_rfc3339(finish).ok()?;
    u64::try_from((finish - start).num_milliseconds().max(0)).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::ConversationEvent;
    use crate::types::SessionId;
    use std::collections::BTreeMap;

    fn event(sequence: u64, kind: &str, payload: Value) -> ConversationEvent {
        ConversationEvent {
            schema_version: 1,
            event_id: crate::ulid::new_id(),
            conversation_id: SessionId::new(),
            sequence,
            kind: kind.to_string(),
            actor_id: None,
            causation_id: None,
            correlation_id: Some("turn-1".to_string()),
            payload,
            extensions: BTreeMap::new(),
            occurred_at: format!("2026-07-30T00:00:0{sequence}.000Z"),
        }
    }

    #[test]
    fn capability_contract_exposes_versions_limits_and_redaction() {
        let capabilities = conversation_capabilities();
        assert_eq!(capabilities.schema_version, 1);
        assert_eq!(
            capabilities.execution_limits.max_speakers_per_turn,
            MAX_CONVERSATION_SPEAKERS_PER_TURN
        );
        assert!(capabilities
            .observability
            .excluded_data
            .contains(&"provider_error_body".to_string()));
        assert!(capabilities
            .turn_errors
            .iter()
            .any(|error| error.code == "unknown_commit"
                && error.recovery == ConversationRecoveryAction::ManualRetryOrContinue));
    }

    #[test]
    fn observation_is_deterministic_and_does_not_copy_sensitive_payloads() {
        let mut accepted = event(
            0,
            TURN_ACCEPTED,
            serde_json::json!({
                "request_fingerprint": "not-secret",
                "policy": {
                    "policy_id": "airp.scene.round_robin.v1",
                    "policy_version": "1.0.0",
                    "execution_mode": "serial",
                    "speakers": [{"participant_id": "character:alice", "resource_id": "alice"}]
                },
                "observability": {
                    "planning_ms": 4,
                    "quota_reserved_requests": 1
                }
            }),
        );
        accepted.actor_id = Some("human:gm".to_string());
        let mut message = event(
            1,
            "message.created",
            serde_json::json!({
                "role": "assistant",
                "content": "TOP SECRET PROMPT",
                "provider_response": "PRIVATE UPSTREAM BODY",
                "observability": {
                    "speaker_latency_ms": 20,
                    "recorded_output_tokens": 7
                }
            }),
        );
        message.actor_id = Some("character:alice".to_string());
        let completed = event(
            2,
            TURN_COMPLETED,
            serde_json::json!({"commit_state": "completed"}),
        );
        let snapshot = ConversationTurnSnapshot {
            turn_id: "turn-1".to_string(),
            lifecycle_state: ConversationTurnLifecycleState::Completed,
            events: vec![accepted, message, completed],
            next_sequence: 3,
            failure: None,
        };

        let first = serde_json::to_string(&project_turn_observability(&snapshot)).unwrap();
        let second = serde_json::to_string(&project_turn_observability(&snapshot)).unwrap();
        assert_eq!(first, second);
        assert!(!first.contains("TOP SECRET PROMPT"));
        assert!(!first.contains("PRIVATE UPSTREAM BODY"));
        assert!(first.contains("\"recorded_output_tokens\":7"));
        assert!(first.contains("\"latency_ms\":2000"));
    }

    #[test]
    fn observation_accounts_for_billed_parallel_outputs_that_were_not_committed() {
        let accepted = event(
            0,
            TURN_ACCEPTED,
            serde_json::json!({
                "policy": {
                    "policy_id": "vendor.parallel.v1",
                    "policy_version": "1.0.0",
                    "execution_mode": "parallel",
                    "speakers": [
                        {"participant_id": "character:alice"},
                        {"participant_id": "character:bob"},
                        {"participant_id": "character:carol"},
                        {"participant_id": "character:dave"}
                    ]
                }
            }),
        );
        let mut committed = event(
            1,
            "message.created",
            serde_json::json!({
                "role": "assistant",
                "content": "redacted from observation",
                "observability": {
                    "speaker_latency_ms": 10,
                    "recorded_output_tokens": 3
                }
            }),
        );
        committed.actor_id = Some("character:alice".to_string());
        let failed = event(
            2,
            TURN_FAILED,
            serde_json::json!({
                "code": "generation_failed",
                "participant_id": "character:bob",
                "commit_state": "partially_committed",
                "observability": {
                    "speaker_latency_ms": 20,
                    "recorded_output_tokens": 5,
                    "uncommitted_speakers": [
                        {
                            "participant_id": "character:carol",
                            "outcome": "generated_not_committed",
                            "speaker_latency_ms": 30,
                            "recorded_output_tokens": 7
                        },
                        {
                            "participant_id": "character:dave",
                            "outcome": "failed",
                            "speaker_latency_ms": 40,
                            "recorded_output_tokens": 11
                        }
                    ]
                }
            }),
        );
        let snapshot = ConversationTurnSnapshot {
            turn_id: "turn-1".to_string(),
            lifecycle_state: ConversationTurnLifecycleState::Failed,
            events: vec![accepted, committed, failed],
            next_sequence: 3,
            failure: Some(ConversationTurnFailure {
                schema_version: CONVERSATION_TURN_ERROR_SCHEMA_VERSION,
                code: "generation_failed".to_string(),
                participant_id: Some("character:bob".to_string()),
                recovery: ConversationRecoveryAction::InspectAndContinue,
            }),
        };

        let observation = project_turn_observability(&snapshot);
        assert_eq!(observation.recorded_output_tokens, 26);
        assert_eq!(
            observation.speakers[0].outcome,
            ConversationSpeakerOutcome::Committed
        );
        assert_eq!(
            observation.speakers[1].outcome,
            ConversationSpeakerOutcome::Failed
        );
        assert_eq!(
            observation.speakers[2].outcome,
            ConversationSpeakerOutcome::GeneratedNotCommitted
        );
        assert_eq!(
            observation.speakers[3].outcome,
            ConversationSpeakerOutcome::Failed
        );
    }

    #[test]
    fn repeated_participant_calls_keep_distinct_plan_ordinals() {
        let accepted = event(
            0,
            TURN_ACCEPTED,
            serde_json::json!({
                "policy": {
                    "policy_id": "vendor.repeat.v1",
                    "policy_version": "1.0.0",
                    "execution_mode": "serial",
                    "speakers": [
                        {"participant_id": "character:alice"},
                        {"participant_id": "character:alice"}
                    ]
                }
            }),
        );
        let mut first = event(
            1,
            "message.created",
            serde_json::json!({
                "role": "assistant",
                "observability": {"recorded_output_tokens": 2}
            }),
        );
        first.actor_id = Some("character:alice".to_string());
        let mut second = event(
            2,
            "message.created",
            serde_json::json!({
                "role": "assistant",
                "observability": {"recorded_output_tokens": 3}
            }),
        );
        second.actor_id = Some("character:alice".to_string());
        let completed = event(3, TURN_COMPLETED, serde_json::json!({}));
        let snapshot = ConversationTurnSnapshot {
            turn_id: "turn-1".to_string(),
            lifecycle_state: ConversationTurnLifecycleState::Completed,
            events: vec![accepted, first, second, completed],
            next_sequence: 4,
            failure: None,
        };

        let observation = project_turn_observability(&snapshot);
        assert_eq!(observation.speakers.len(), 2);
        assert_eq!(observation.speakers[0].speaker_index, 0);
        assert_eq!(observation.speakers[1].speaker_index, 1);
        assert_eq!(observation.speakers[0].recorded_output_tokens, Some(2));
        assert_eq!(observation.speakers[1].recorded_output_tokens, Some(3));
        assert_eq!(observation.recorded_output_tokens, 5);
    }

    #[test]
    #[ignore = "long-journal observability benchmark; run with --release --ignored --nocapture"]
    fn long_journal_turn_observability_benchmark() {
        const UNRELATED_EVENTS: u64 = 50_000;
        const READS: u32 = 50;
        let mut events = (0..UNRELATED_EVENTS)
            .map(|sequence| {
                let mut event = event(
                    sequence,
                    "vendor.unrelated",
                    serde_json::json!({"opaque": true}),
                );
                event.correlation_id = Some("other-turn".to_string());
                event
            })
            .collect::<Vec<_>>();
        let mut accepted = event(
            UNRELATED_EVENTS,
            TURN_ACCEPTED,
            serde_json::json!({
                "policy": {
                    "policy_id": "airp.scene.round_robin.v1",
                    "policy_version": "1.0.0",
                    "execution_mode": "serial",
                    "speakers": []
                }
            }),
        );
        accepted.occurred_at = "2026-07-30T00:00:00.000Z".to_string();
        events.push(accepted);
        let mut started = event(
            UNRELATED_EVENTS + 1,
            crate::conversation_turn::TURN_STARTED,
            serde_json::json!({}),
        );
        started.occurred_at = "2026-07-30T00:00:00.001Z".to_string();
        events.push(started);
        let mut completed = event(
            UNRELATED_EVENTS + 2,
            TURN_COMPLETED,
            serde_json::json!({"message_count": 0}),
        );
        completed.occurred_at = "2026-07-30T00:00:00.002Z".to_string();
        events.push(completed);

        let started = std::time::Instant::now();
        for _ in 0..READS {
            let snapshot = crate::conversation_turn::project_turn(&events, "turn-1")
                .unwrap()
                .unwrap();
            let observation = project_turn_observability(&snapshot);
            assert_eq!(
                observation.lifecycle_state,
                ConversationTurnLifecycleState::Completed
            );
        }
        let elapsed = started.elapsed();
        eprintln!(
            "conversation observability benchmark: journal_events={} reads={READS} total_ms={} mean_ms={:.3}",
            events.len(),
            elapsed.as_millis(),
            elapsed.as_secs_f64() * 1000.0 / f64::from(READS)
        );
    }
}
