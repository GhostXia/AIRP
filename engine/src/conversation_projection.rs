//! Rebuildable views derived only from a Conversation manifest and journal.

use crate::adapter::{ChatMessage, MessageRole};
use crate::conversation::{ConversationEvent, ConversationManifest, ConversationService};
use crate::error::AirpError;
use crate::types::SessionId;
use serde::Serialize;
use std::collections::HashSet;

/// Message projection schema version emitted by this Engine.
pub const CONVERSATION_MESSAGE_PROJECTION_SCHEMA_VERSION: u32 = 1;

/// Stable role snapshot carried by a projected message.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationProjectedRole {
    User,
    Assistant,
}

/// One valid `message.created` event projected for history consumers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProjectedConversationMessage {
    pub event_id: String,
    pub sequence: u64,
    pub actor_id: String,
    pub role: ConversationProjectedRole,
    pub content: String,
}

/// Deterministic counters explaining how the source journal was projected.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversationMessageProjectionStats {
    pub source_event_count: usize,
    pub ignored_non_message_event_count: usize,
    pub ignored_invalid_message_count: usize,
    pub unresolved_actor_count: usize,
}

/// Versioned message view rebuilt from authoritative Conversation inputs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversationMessageProjection {
    pub schema_version: u32,
    pub conversation_id: SessionId,
    pub messages: Vec<ProjectedConversationMessage>,
    pub stats: ConversationMessageProjectionStats,
}

impl ConversationMessageProjection {
    /// Convert the stable projection into the chat pipeline's prompt shape.
    pub fn into_chat_messages(self) -> Vec<ChatMessage> {
        self.messages
            .into_iter()
            .map(|message| {
                let (role, content) = match message.role {
                    ConversationProjectedRole::User => (MessageRole::User, message.content),
                    ConversationProjectedRole::Assistant => (
                        MessageRole::Assistant,
                        format!("[{}] {}", message.actor_id, message.content),
                    ),
                };
                ChatMessage { role, content }
            })
            .collect()
    }
}

/// Project messages without consulting mutable character, scene, or UI state.
///
/// Unknown event kinds are intentionally ignored. A `message.created` event is
/// included only when it carries a non-empty actor, a string `content`, and an
/// explicit `user` or `assistant` role. Unknown actors remain attributable and
/// are counted instead of being guessed into a role.
pub fn project_conversation_messages(
    manifest: &ConversationManifest,
    events: &[ConversationEvent],
) -> ConversationMessageProjection {
    let mut messages = Vec::new();
    let participant_ids = manifest
        .participants
        .iter()
        .map(|participant| participant.participant_id.as_str())
        .collect::<HashSet<_>>();
    let mut ignored_non_message_event_count = 0usize;
    let mut ignored_invalid_message_count = 0usize;
    let mut unresolved_actor_count = 0usize;

    for event in events {
        if event.kind != "message.created" {
            ignored_non_message_event_count += 1;
            continue;
        }
        let Some(actor_id) = event
            .actor_id
            .as_deref()
            .filter(|actor_id| !actor_id.trim().is_empty())
        else {
            ignored_invalid_message_count += 1;
            continue;
        };
        let Some(content) = event
            .payload
            .get("content")
            .and_then(|value| value.as_str())
        else {
            ignored_invalid_message_count += 1;
            continue;
        };
        let role = match event.payload.get("role").and_then(|value| value.as_str()) {
            Some("user") => ConversationProjectedRole::User,
            Some("assistant") => ConversationProjectedRole::Assistant,
            _ => {
                ignored_invalid_message_count += 1;
                continue;
            }
        };
        if !participant_ids.contains(actor_id) {
            unresolved_actor_count += 1;
        }
        messages.push(ProjectedConversationMessage {
            event_id: event.event_id.clone(),
            sequence: event.sequence,
            actor_id: actor_id.to_string(),
            role,
            content: content.to_string(),
        });
    }

    ConversationMessageProjection {
        schema_version: CONVERSATION_MESSAGE_PROJECTION_SCHEMA_VERSION,
        conversation_id: manifest.conversation_id,
        messages,
        stats: ConversationMessageProjectionStats {
            source_event_count: events.len(),
            ignored_non_message_event_count,
            ignored_invalid_message_count,
            unresolved_actor_count,
        },
    }
}

impl ConversationService {
    /// Rebuild the full message projection from manifest and journal truth.
    pub async fn message_projection(
        &self,
        conversation_id: SessionId,
    ) -> Result<ConversationMessageProjection, AirpError> {
        let manifest = self.get(conversation_id).await?;
        let events = self.all_events(conversation_id).await?;
        Ok(project_conversation_messages(&manifest, &events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{
        AppendConversationEventRequest, ConversationParticipant, ConversationPolicyRef,
        ConversationResourceRef, CreateConversationRequest,
    };
    use serde_json::json;
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    fn manifest() -> ConversationManifest {
        let conversation_id = SessionId::new();
        ConversationManifest {
            schema_version: 1,
            conversation_id,
            title: None,
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
                    resource: Some(ConversationResourceRef {
                        kind: "character".to_string(),
                        id: "shared-card".to_string(),
                        revision: Some("1".to_string()),
                        extensions: BTreeMap::new(),
                    }),
                    extensions: BTreeMap::new(),
                },
                ConversationParticipant {
                    participant_id: "bob".to_string(),
                    kind: "character".to_string(),
                    display_name: None,
                    resource: Some(ConversationResourceRef {
                        kind: "character".to_string(),
                        id: "shared-card".to_string(),
                        revision: Some("1".to_string()),
                        extensions: BTreeMap::new(),
                    }),
                    extensions: BTreeMap::new(),
                },
            ],
            resources: Vec::new(),
            orchestration: None,
            extensions: BTreeMap::new(),
            created_at: "2026-07-29T00:00:00Z".to_string(),
        }
    }

    fn event(
        manifest: &ConversationManifest,
        sequence: u64,
        kind: &str,
        actor_id: Option<&str>,
        payload: serde_json::Value,
    ) -> ConversationEvent {
        ConversationEvent {
            schema_version: 1,
            event_id: format!("event-{sequence}"),
            conversation_id: manifest.conversation_id,
            sequence,
            kind: kind.to_string(),
            actor_id: actor_id.map(str::to_string),
            causation_id: None,
            correlation_id: None,
            payload,
            extensions: BTreeMap::new(),
            occurred_at: "2026-07-29T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn projection_preserves_per_event_role_and_multi_actor_attribution() {
        let manifest = manifest();
        let events = vec![
            event(
                &manifest,
                0,
                "message.created",
                Some("alice"),
                json!({"role": "assistant", "content": "first"}),
            ),
            event(
                &manifest,
                1,
                "message.created",
                Some("bob"),
                json!({"role": "assistant", "content": "second"}),
            ),
            event(
                &manifest,
                2,
                "message.created",
                Some("alice"),
                json!({"role": "user", "content": "human-controlled"}),
            ),
        ];

        let projection = project_conversation_messages(&manifest, &events);

        assert_eq!(projection.schema_version, 1);
        assert_eq!(
            projection
                .messages
                .iter()
                .map(|message| (
                    message.actor_id.as_str(),
                    message.role,
                    message.content.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("alice", ConversationProjectedRole::Assistant, "first"),
                ("bob", ConversationProjectedRole::Assistant, "second"),
                ("alice", ConversationProjectedRole::User, "human-controlled"),
            ]
        );
    }

    #[test]
    fn unknown_events_and_invalid_messages_have_deterministic_semantics() {
        let manifest = manifest();
        let events = vec![
            event(&manifest, 0, "vendor.audit.v1", None, json!({"x": 1})),
            event(
                &manifest,
                1,
                "message.created",
                Some("alice"),
                json!({"role": "assistant", "content": {"broken": true}}),
            ),
            event(
                &manifest,
                2,
                "message.created",
                Some("external-agent"),
                json!({"role": "assistant", "content": "kept"}),
            ),
            event(
                &manifest,
                3,
                "message.created",
                Some("external-agent"),
                json!({"content": "missing role"}),
            ),
        ];

        let first = project_conversation_messages(&manifest, &events);
        let second = project_conversation_messages(&manifest, &events);

        assert_eq!(first, second);
        assert_eq!(first.messages.len(), 1);
        assert_eq!(first.messages[0].actor_id, "external-agent");
        assert_eq!(
            first.stats,
            ConversationMessageProjectionStats {
                source_event_count: 4,
                ignored_non_message_event_count: 1,
                ignored_invalid_message_count: 2,
                unresolved_actor_count: 1,
            }
        );
    }

    #[test]
    fn long_projection_is_ordered_and_independent_of_live_resources() {
        let manifest = manifest();
        let events = (0..10_000)
            .map(|sequence| {
                event(
                    &manifest,
                    sequence,
                    "message.created",
                    Some(if sequence % 2 == 0 { "alice" } else { "bob" }),
                    json!({"role": "assistant", "content": format!("message-{sequence}")}),
                )
            })
            .collect::<Vec<_>>();

        let projection = project_conversation_messages(&manifest, &events);

        assert_eq!(projection.messages.len(), 10_000);
        assert_eq!(projection.messages[0].sequence, 0);
        assert_eq!(projection.messages[9_999].sequence, 9_999);
        assert_eq!(projection.stats.unresolved_actor_count, 0);
    }

    #[tokio::test]
    async fn service_rebuild_matches_projection_from_authoritative_inputs() {
        let tmp = tempdir().unwrap();
        let service = ConversationService::new(tmp.path());
        let source = manifest();
        let created = service
            .create(CreateConversationRequest {
                user_id: None,
                title: source.title,
                participants: source.participants,
                resources: source.resources,
                orchestration: Some(ConversationPolicyRef {
                    policy_id: "test".to_string(),
                    config: json!({}),
                    extensions: BTreeMap::new(),
                }),
                extensions: source.extensions,
            })
            .await
            .unwrap();
        for (sequence, actor_id, role, content) in [
            (0, "human:gm", "user", "hello"),
            (1, "alice", "assistant", "hi"),
        ] {
            service
                .append_event(
                    created.conversation_id,
                    AppendConversationEventRequest {
                        user_id: None,
                        kind: "message.created".to_string(),
                        actor_id: Some(actor_id.to_string()),
                        causation_id: None,
                        correlation_id: None,
                        payload: json!({"role": role, "content": content}),
                        extensions: BTreeMap::new(),
                        expected_next_sequence: Some(sequence),
                    },
                )
                .await
                .unwrap();
        }

        let rebuilt = service
            .message_projection(created.conversation_id)
            .await
            .unwrap();
        let direct = project_conversation_messages(
            &service.get(created.conversation_id).await.unwrap(),
            &service.all_events(created.conversation_id).await.unwrap(),
        );

        assert_eq!(rebuilt, direct);
        assert_eq!(
            rebuilt
                .into_chat_messages()
                .into_iter()
                .map(|message| message.content)
                .collect::<Vec<_>>(),
            vec!["hello", "[alice] hi"]
        );
    }
}
