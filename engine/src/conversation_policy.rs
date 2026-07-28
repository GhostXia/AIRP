//! Engine-owned orchestration policy registry for Conversation turns.
//!
//! Storage keeps policy identifiers open. Execution is fail-closed: only a
//! registered policy may turn a manifest into an executable plan.

use crate::conversation::{ConversationManifest, ConversationPolicyRef};
use crate::error::AirpError;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

pub const SCENE_ROUND_ROBIN_V1: &str = "airp.scene.round_robin.v1";
pub const CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyDescriptor {
    pub schema_version: u32,
    pub policy_id: String,
    pub description: String,
    pub config_schema: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationSpeaker {
    pub participant_id: String,
    pub resource_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurnPlan {
    pub scene_id: String,
    pub speakers: Vec<ConversationSpeaker>,
}

pub trait ConversationPolicy: Send + Sync {
    fn descriptor(&self) -> ConversationPolicyDescriptor;

    fn plan_turn(
        &self,
        manifest: &ConversationManifest,
        policy: &ConversationPolicyRef,
        user_actor_id: &str,
    ) -> Result<ConversationTurnPlan, AirpError>;
}

#[derive(Default)]
pub struct ConversationPolicyRegistry {
    policies: HashMap<String, RegisteredPolicy>,
}

struct RegisteredPolicy {
    descriptor: ConversationPolicyDescriptor,
    policy: Arc<dyn ConversationPolicy>,
}

impl ConversationPolicyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, policy: Arc<dyn ConversationPolicy>) -> Result<(), AirpError> {
        let descriptor = policy.descriptor();
        if descriptor.policy_id.trim().is_empty() {
            return Err(AirpError::Config(
                "conversation policy id must not be empty".to_string(),
            ));
        }
        if self.policies.contains_key(&descriptor.policy_id) {
            return Err(AirpError::Config(format!(
                "duplicate conversation policy registration: {}",
                descriptor.policy_id
            )));
        }
        self.policies.insert(
            descriptor.policy_id.clone(),
            RegisteredPolicy { descriptor, policy },
        );
        Ok(())
    }

    pub fn list(&self) -> Vec<ConversationPolicyDescriptor> {
        let mut policies = self
            .policies
            .values()
            .map(|registered| registered.descriptor.clone())
            .collect::<Vec<_>>();
        policies.sort_by(|left, right| left.policy_id.cmp(&right.policy_id));
        policies
    }

    pub fn plan_turn(
        &self,
        manifest: &ConversationManifest,
        user_actor_id: &str,
    ) -> Result<ConversationTurnPlan, AirpError> {
        let policy_ref = manifest.orchestration.as_ref().ok_or_else(|| {
            AirpError::BadRequest("conversation has no orchestration policy".into())
        })?;
        let registered = self.policies.get(&policy_ref.policy_id).ok_or_else(|| {
            AirpError::BadRequest(format!(
                "conversation policy is not registered: {}",
                policy_ref.policy_id
            ))
        })?;
        registered
            .policy
            .plan_turn(manifest, policy_ref, user_actor_id)
    }
}

pub fn builtin_conversation_policy_registry() -> &'static ConversationPolicyRegistry {
    static REGISTRY: OnceLock<ConversationPolicyRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let mut registry = ConversationPolicyRegistry::new();
        registry
            .register(Arc::new(SceneRoundRobinV1))
            .expect("built-in conversation policies must have unique valid ids");
        registry
    })
}

struct SceneRoundRobinV1;

impl ConversationPolicy for SceneRoundRobinV1 {
    fn descriptor(&self) -> ConversationPolicyDescriptor {
        ConversationPolicyDescriptor {
            schema_version: CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION,
            policy_id: SCENE_ROUND_ROBIN_V1.to_string(),
            description:
                "Execute each character participant once in manifest order for one scene turn."
                    .to_string(),
            config_schema: serde_json::json!({
                "oneOf": [
                    {
                        "type": "object",
                        "maxProperties": 0,
                        "additionalProperties": false
                    },
                    {"type": "null"}
                ],
                "default": {}
            }),
        }
    }

    fn plan_turn(
        &self,
        manifest: &ConversationManifest,
        policy: &ConversationPolicyRef,
        user_actor_id: &str,
    ) -> Result<ConversationTurnPlan, AirpError> {
        if !policy.config.is_null()
            && !policy
                .config
                .as_object()
                .is_some_and(serde_json::Map::is_empty)
        {
            return Err(AirpError::BadRequest(format!(
                "{} does not accept policy config",
                policy.policy_id
            )));
        }

        let user_participant = manifest
            .participants
            .iter()
            .find(|participant| participant.participant_id == user_actor_id)
            .ok_or_else(|| {
                AirpError::BadRequest(format!(
                    "user actor {user_actor_id} is not a conversation participant"
                ))
            })?;
        if user_participant.kind == "character" {
            return Err(AirpError::BadRequest(
                "user_actor_id must not identify a character participant".to_string(),
            ));
        }

        let scene_ids = manifest
            .resources
            .iter()
            .filter(|resource| resource.kind == "scene")
            .map(|resource| resource.id.clone())
            .collect::<Vec<_>>();
        if scene_ids.len() != 1 {
            return Err(AirpError::BadRequest(
                "scene round-robin requires exactly one scene resource".to_string(),
            ));
        }

        let speakers = manifest
            .participants
            .iter()
            .filter_map(|participant| {
                let resource = participant.resource.as_ref()?;
                (participant.kind == "character" && resource.kind == "character").then(|| {
                    ConversationSpeaker {
                        participant_id: participant.participant_id.clone(),
                        resource_id: resource.id.clone(),
                    }
                })
            })
            .collect::<Vec<_>>();
        if speakers.is_empty() {
            return Err(AirpError::BadRequest(
                "scene round-robin requires at least one character participant".to_string(),
            ));
        }

        Ok(ConversationTurnPlan {
            scene_id: scene_ids.into_iter().next().expect("length checked"),
            speakers,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversation::{
        ConversationParticipant, ConversationResourceRef, CreateConversationRequest,
    };
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::tempdir;

    struct ChangingDescriptor(AtomicUsize);

    impl ConversationPolicy for ChangingDescriptor {
        fn descriptor(&self) -> ConversationPolicyDescriptor {
            let call = self.0.fetch_add(1, Ordering::Relaxed);
            ConversationPolicyDescriptor {
                schema_version: CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION,
                policy_id: if call == 0 { "stable" } else { "changed" }.to_string(),
                description: String::new(),
                config_schema: Value::Null,
            }
        }

        fn plan_turn(
            &self,
            _manifest: &ConversationManifest,
            _policy: &ConversationPolicyRef,
            _user_actor_id: &str,
        ) -> Result<ConversationTurnPlan, AirpError> {
            unreachable!("descriptor snapshot test does not execute the policy")
        }
    }

    fn manifest(policy_id: &str, config: Value) -> ConversationManifest {
        crate::conversation::ConversationService::new(tempdir().unwrap().path())
            .create(CreateConversationRequest {
                user_id: None,
                title: None,
                participants: vec![
                    ConversationParticipant {
                        participant_id: "human:gm".to_string(),
                        kind: "human".to_string(),
                        display_name: None,
                        resource: None,
                        extensions: BTreeMap::new(),
                    },
                    character("alice"),
                    character("bob"),
                ],
                resources: vec![ConversationResourceRef {
                    kind: "scene".to_string(),
                    id: "tavern".to_string(),
                    revision: None,
                    extensions: BTreeMap::new(),
                }],
                orchestration: Some(ConversationPolicyRef {
                    policy_id: policy_id.to_string(),
                    config,
                    extensions: BTreeMap::new(),
                }),
                extensions: BTreeMap::new(),
            })
            .unwrap()
    }

    fn character(id: &str) -> ConversationParticipant {
        ConversationParticipant {
            participant_id: format!("character:{id}"),
            kind: "character".to_string(),
            display_name: None,
            resource: Some(ConversationResourceRef {
                kind: "character".to_string(),
                id: id.to_string(),
                revision: None,
                extensions: BTreeMap::new(),
            }),
            extensions: BTreeMap::new(),
        }
    }

    #[test]
    fn builtins_are_sorted_and_describe_fail_closed_config() {
        let descriptors = builtin_conversation_policy_registry().list();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(
            descriptors[0].schema_version,
            CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION
        );
        assert_eq!(descriptors[0].policy_id, SCENE_ROUND_ROBIN_V1);
        assert_eq!(
            descriptors[0].config_schema["oneOf"][0]["additionalProperties"],
            false
        );
    }

    #[test]
    fn round_robin_preserves_manifest_speaker_order() {
        let plan = builtin_conversation_policy_registry()
            .plan_turn(
                &manifest(SCENE_ROUND_ROBIN_V1, serde_json::json!({})),
                "human:gm",
            )
            .unwrap();
        assert_eq!(plan.scene_id, "tavern");
        assert_eq!(
            plan.speakers
                .iter()
                .map(|speaker| speaker.participant_id.as_str())
                .collect::<Vec<_>>(),
            ["character:alice", "character:bob"]
        );
    }

    #[test]
    fn unknown_policy_and_unknown_config_fail_closed() {
        let unknown = builtin_conversation_policy_registry()
            .plan_turn(&manifest("vendor.future.v1", Value::Null), "human:gm")
            .unwrap_err();
        assert!(matches!(unknown, AirpError::BadRequest(_)));

        let config = builtin_conversation_policy_registry()
            .plan_turn(
                &manifest(SCENE_ROUND_ROBIN_V1, serde_json::json!({"future": true})),
                "human:gm",
            )
            .unwrap_err();
        assert!(matches!(config, AirpError::BadRequest(_)));
    }

    #[test]
    fn duplicate_registration_is_rejected() {
        let mut registry = ConversationPolicyRegistry::new();
        registry.register(Arc::new(SceneRoundRobinV1)).unwrap();
        let error = registry.register(Arc::new(SceneRoundRobinV1)).unwrap_err();
        assert!(matches!(error, AirpError::Config(_)));
    }

    #[test]
    fn registration_snapshots_descriptor_identity() {
        let mut registry = ConversationPolicyRegistry::new();
        registry
            .register(Arc::new(ChangingDescriptor(AtomicUsize::new(0))))
            .unwrap();
        assert_eq!(registry.list()[0].policy_id, "stable");
    }
}
