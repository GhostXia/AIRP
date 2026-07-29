//! Engine-owned orchestration policy registry for Conversation turns.
//!
//! Storage keeps policy identifiers open. Execution is fail-closed: only an
//! explicitly registered, active policy may turn a manifest into an executable
//! plan. External policies are trusted in-process Rust implementations injected
//! by the host; manifest configuration is data and is never loaded as code.

use crate::conversation::{ConversationManifest, ConversationPolicyRef};
use crate::error::AirpError;
use async_trait::async_trait;
use futures_util::FutureExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// Identifier of the built-in scene round-robin policy.
pub const SCENE_ROUND_ROBIN_V1: &str = "airp.scene.round_robin.v1";
/// Schema version used by policy discovery descriptors.
pub const CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION: u32 = 2;
/// Hard safety ceiling for provider calls planned in one turn.
pub const MAX_CONVERSATION_SPEAKERS_PER_TURN: usize = 16;
/// Hard ceiling for provider calls running concurrently in one turn.
pub const MAX_CONVERSATION_POLICY_PARALLELISM: usize = 4;
/// Hard ceiling for serialized policy configuration.
pub const MAX_CONVERSATION_POLICY_CONFIG_BYTES: usize = 16 * 1024;
/// Hard ceiling for policy planning.
pub const MAX_CONVERSATION_POLICY_PLANNING_TIMEOUT_MS: u64 = 2_000;

/// Where a registered policy implementation came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPolicySource {
    BuiltIn,
    External,
}

/// Whether a registered policy may currently plan new turns.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConversationPolicyLifecycleState {
    Active,
    Disabled,
}

/// Provider-call scheduling semantics supported by a policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ConversationExecutionMode {
    Serial,
    Parallel,
}

/// Discoverable policy provenance.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyProvenance {
    pub source: ConversationPolicySource,
    pub provider: String,
    pub implementation: String,
}

/// Discoverable bounded execution capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyCapabilities {
    pub execution_modes: Vec<ConversationExecutionMode>,
    pub supports_message_limit: bool,
}

/// Engine-enforced resource budget for one policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyResourceLimits {
    pub max_speakers_per_turn: usize,
    pub max_parallelism: usize,
    pub max_config_bytes: usize,
    pub planning_timeout_ms: u64,
}

/// Discoverable metadata and configuration contract for one policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConversationPolicyDescriptor {
    pub schema_version: u32,
    pub policy_id: String,
    pub policy_version: String,
    pub description: String,
    pub provenance: ConversationPolicyProvenance,
    pub capabilities: ConversationPolicyCapabilities,
    pub resource_limits: ConversationPolicyResourceLimits,
    pub lifecycle_state: ConversationPolicyLifecycleState,
    pub config_schema: Value,
}

/// One attributed provider speaker selected by a policy.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ConversationSpeaker {
    pub participant_id: String,
    pub resource_id: String,
}

/// Executable speaker plan proposed by a policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationTurnPlan {
    pub scene_id: String,
    pub speakers: Vec<ConversationSpeaker>,
    pub execution_mode: ConversationExecutionMode,
    /// Declarative stop condition. The registry truncates the plan to this
    /// bound before quota reservation or provider execution.
    pub stop_after_messages: Option<usize>,
}

/// Engine-validated plan plus immutable registration evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedConversationTurnPlan {
    pub policy: ConversationPolicyDescriptor,
    pub plan: ConversationTurnPlan,
}

/// Engine extension point that maps an open policy reference to a safe plan.
#[async_trait]
pub trait ConversationPolicy: Send + Sync {
    /// Return stable discovery metadata for this policy.
    fn descriptor(&self) -> ConversationPolicyDescriptor;

    /// Validate policy configuration as data before planning.
    fn validate_config(&self, config: &Value) -> Result<(), AirpError>;

    /// Plan one turn. The registry applies timeout, panic, capability, resource,
    /// participant, and resource gates to the returned plan.
    async fn plan_turn(
        &self,
        manifest: &ConversationManifest,
        policy: &ConversationPolicyRef,
        user_actor_id: &str,
    ) -> Result<ConversationTurnPlan, AirpError>;
}

/// Thread-safe registry of explicitly trusted Conversation policies.
#[derive(Default)]
pub struct ConversationPolicyRegistry {
    policies: RwLock<HashMap<String, RegisteredPolicy>>,
}

#[derive(Clone)]
struct RegisteredPolicy {
    descriptor: ConversationPolicyDescriptor,
    policy: Arc<dyn ConversationPolicy>,
}

impl ConversationPolicyRegistry {
    /// Create an empty policy registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create an isolated registry containing AIRP's built-in policies.
    pub fn with_builtins() -> Arc<Self> {
        let registry = Arc::new(Self::new());
        registry
            .register_builtin(Arc::new(SceneRoundRobinV1))
            .expect("built-in conversation policies must have valid unique descriptors");
        registry
    }

    /// Inject one explicitly trusted external policy at runtime.
    ///
    /// External policies cannot claim AIRP's reserved `airp.` namespace.
    pub fn register_external(&self, policy: Arc<dyn ConversationPolicy>) -> Result<(), AirpError> {
        self.register(policy, ConversationPolicySource::External)
    }

    /// Enable or disable an external policy for future turn planning.
    pub fn set_external_lifecycle(
        &self,
        policy_id: &str,
        lifecycle_state: ConversationPolicyLifecycleState,
    ) -> Result<(), AirpError> {
        let mut policies = self
            .policies
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let registered = policies.get_mut(policy_id).ok_or_else(|| {
            AirpError::Config(format!(
                "conversation policy is not registered: {policy_id}"
            ))
        })?;
        if registered.descriptor.provenance.source != ConversationPolicySource::External {
            return Err(AirpError::Config(
                "built-in conversation policy lifecycle is Engine-owned".to_string(),
            ));
        }
        registered.descriptor.lifecycle_state = lifecycle_state;
        Ok(())
    }

    /// Remove an external policy from discovery and future turn planning.
    pub fn unregister_external(&self, policy_id: &str) -> Result<(), AirpError> {
        let mut policies = self
            .policies
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let registered = policies.get(policy_id).ok_or_else(|| {
            AirpError::Config(format!(
                "conversation policy is not registered: {policy_id}"
            ))
        })?;
        if registered.descriptor.provenance.source != ConversationPolicySource::External {
            return Err(AirpError::Config(
                "built-in conversation policy registration is Engine-owned".to_string(),
            ));
        }
        policies.remove(policy_id);
        Ok(())
    }

    /// Return stable descriptors sorted by policy identity.
    pub fn list(&self) -> Vec<ConversationPolicyDescriptor> {
        let policies = self
            .policies
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut descriptors = policies
            .values()
            .map(|registered| registered.descriptor.clone())
            .collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.policy_id.cmp(&right.policy_id));
        descriptors
    }

    /// Resolve, execute, and validate the policy referenced by a manifest.
    pub async fn plan_turn(
        &self,
        manifest: &ConversationManifest,
        user_actor_id: &str,
    ) -> Result<ResolvedConversationTurnPlan, AirpError> {
        let policy_ref = manifest.orchestration.as_ref().ok_or_else(|| {
            AirpError::BadRequest("conversation has no orchestration policy".into())
        })?;
        let registered = {
            let policies = self
                .policies
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            policies
                .get(&policy_ref.policy_id)
                .cloned()
                .ok_or_else(|| {
                    AirpError::BadRequest(format!(
                        "conversation policy is not registered: {}",
                        policy_ref.policy_id
                    ))
                })?
        };
        if registered.descriptor.lifecycle_state != ConversationPolicyLifecycleState::Active {
            return Err(AirpError::BadRequest(format!(
                "conversation policy is disabled: {}",
                policy_ref.policy_id
            )));
        }
        let config_bytes = serde_json::to_vec(&policy_ref.config)?.len();
        if config_bytes > registered.descriptor.resource_limits.max_config_bytes {
            return Err(AirpError::BadRequest(format!(
                "conversation policy config is {config_bytes} bytes; maximum is {}",
                registered.descriptor.resource_limits.max_config_bytes
            )));
        }

        let policy = registered.policy.clone();
        let planning = AssertUnwindSafe(async {
            policy.validate_config(&policy_ref.config)?;
            policy.plan_turn(manifest, policy_ref, user_actor_id).await
        })
        .catch_unwind();
        let plan = match tokio::time::timeout(
            Duration::from_millis(registered.descriptor.resource_limits.planning_timeout_ms),
            planning,
        )
        .await
        {
            Ok(Ok(Ok(plan))) => plan,
            Ok(Ok(Err(error))) => {
                tracing::warn!(
                    policy_id = %registered.descriptor.policy_id,
                    %error,
                    "conversation policy rejected configuration or failed to plan"
                );
                return Err(AirpError::BadRequest(format!(
                    "conversation policy planning failed: {}",
                    registered.descriptor.policy_id
                )));
            }
            Ok(Err(_)) => {
                tracing::error!(
                    policy_id = %registered.descriptor.policy_id,
                    "conversation policy panicked while planning"
                );
                return Err(AirpError::BadRequest(format!(
                    "conversation policy planning failed: {}",
                    registered.descriptor.policy_id
                )));
            }
            Err(_) => {
                tracing::warn!(
                    policy_id = %registered.descriptor.policy_id,
                    timeout_ms = registered.descriptor.resource_limits.planning_timeout_ms,
                    "conversation policy planning timed out"
                );
                return Err(AirpError::BadRequest(format!(
                    "conversation policy planning timed out: {}",
                    registered.descriptor.policy_id
                )));
            }
        };
        let plan = validate_and_bound_plan(manifest, user_actor_id, &registered.descriptor, plan)?;
        Ok(ResolvedConversationTurnPlan {
            policy: registered.descriptor,
            plan,
        })
    }

    fn register_builtin(&self, policy: Arc<dyn ConversationPolicy>) -> Result<(), AirpError> {
        self.register(policy, ConversationPolicySource::BuiltIn)
    }

    fn register(
        &self,
        policy: Arc<dyn ConversationPolicy>,
        expected_source: ConversationPolicySource,
    ) -> Result<(), AirpError> {
        let mut descriptor = std::panic::catch_unwind(AssertUnwindSafe(|| policy.descriptor()))
            .map_err(|_| {
                AirpError::Config(
                    "conversation policy panicked while describing its registration".to_string(),
                )
            })?;
        validate_descriptor(&descriptor, expected_source)?;
        descriptor.lifecycle_state = ConversationPolicyLifecycleState::Active;
        let mut policies = self
            .policies
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if policies.contains_key(&descriptor.policy_id) {
            return Err(AirpError::Config(format!(
                "duplicate conversation policy registration: {}",
                descriptor.policy_id
            )));
        }
        policies.insert(
            descriptor.policy_id.clone(),
            RegisteredPolicy { descriptor, policy },
        );
        Ok(())
    }
}

fn validate_descriptor(
    descriptor: &ConversationPolicyDescriptor,
    expected_source: ConversationPolicySource,
) -> Result<(), AirpError> {
    let id = descriptor.policy_id.trim();
    if id.is_empty() || id.len() > 128 || id != descriptor.policy_id {
        return Err(AirpError::Config(
            "conversation policy id must be 1..=128 trimmed bytes".to_string(),
        ));
    }
    if expected_source == ConversationPolicySource::External && id.starts_with("airp.") {
        return Err(AirpError::Config(
            "external conversation policies cannot use the reserved airp. namespace".to_string(),
        ));
    }
    if descriptor.provenance.source != expected_source {
        return Err(AirpError::Config(
            "conversation policy provenance source does not match registration path".to_string(),
        ));
    }
    if descriptor.schema_version != CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION
        || descriptor.policy_version.trim().is_empty()
        || descriptor.policy_version.len() > 64
        || descriptor.description.trim().is_empty()
        || descriptor.provenance.provider.trim().is_empty()
        || descriptor.provenance.implementation.trim().is_empty()
        || !descriptor.config_schema.is_object()
    {
        return Err(AirpError::Config(
            "conversation policy descriptor metadata is invalid".to_string(),
        ));
    }
    let limits = &descriptor.resource_limits;
    if limits.max_speakers_per_turn == 0
        || limits.max_speakers_per_turn > MAX_CONVERSATION_SPEAKERS_PER_TURN
        || limits.max_parallelism == 0
        || limits.max_parallelism > MAX_CONVERSATION_POLICY_PARALLELISM
        || limits.max_config_bytes == 0
        || limits.max_config_bytes > MAX_CONVERSATION_POLICY_CONFIG_BYTES
        || limits.planning_timeout_ms == 0
        || limits.planning_timeout_ms > MAX_CONVERSATION_POLICY_PLANNING_TIMEOUT_MS
    {
        return Err(AirpError::Config(
            "conversation policy resource limits exceed Engine bounds".to_string(),
        ));
    }
    let modes = descriptor
        .capabilities
        .execution_modes
        .iter()
        .copied()
        .collect::<HashSet<_>>();
    if modes.is_empty() || modes.len() != descriptor.capabilities.execution_modes.len() {
        return Err(AirpError::Config(
            "conversation policy execution modes must be non-empty and unique".to_string(),
        ));
    }
    Ok(())
}

fn validate_and_bound_plan(
    manifest: &ConversationManifest,
    user_actor_id: &str,
    descriptor: &ConversationPolicyDescriptor,
    mut plan: ConversationTurnPlan,
) -> Result<ConversationTurnPlan, AirpError> {
    let user = manifest
        .participants
        .iter()
        .find(|participant| participant.participant_id == user_actor_id)
        .ok_or_else(|| {
            AirpError::BadRequest(format!(
                "user actor {user_actor_id} is not a conversation participant"
            ))
        })?;
    if user.kind == "character" {
        return Err(AirpError::BadRequest(
            "user_actor_id must not identify a character participant".to_string(),
        ));
    }
    let valid_scene = manifest
        .resources
        .iter()
        .any(|resource| resource.kind == "scene" && resource.id == plan.scene_id);
    if !valid_scene {
        return Err(AirpError::BadRequest(
            "conversation policy selected an unknown scene resource".to_string(),
        ));
    }
    if !descriptor
        .capabilities
        .execution_modes
        .contains(&plan.execution_mode)
    {
        return Err(AirpError::BadRequest(
            "conversation policy returned an undeclared execution mode".to_string(),
        ));
    }
    if let Some(limit) = plan.stop_after_messages {
        if !descriptor.capabilities.supports_message_limit || limit == 0 {
            return Err(AirpError::BadRequest(
                "conversation policy returned an invalid message stop condition".to_string(),
            ));
        }
        plan.speakers.truncate(limit);
    }
    if plan.speakers.is_empty() {
        return Err(AirpError::BadRequest(
            "conversation policy requires at least one character speaker".to_string(),
        ));
    }
    if plan.speakers.len() > descriptor.resource_limits.max_speakers_per_turn {
        return Err(AirpError::BadRequest(format!(
            "conversation turn planned {} speakers; policy maximum is {}",
            plan.speakers.len(),
            descriptor.resource_limits.max_speakers_per_turn
        )));
    }

    for speaker in &plan.speakers {
        let participant = manifest
            .participants
            .iter()
            .find(|participant| participant.participant_id == speaker.participant_id)
            .ok_or_else(|| {
                AirpError::BadRequest(
                    "conversation policy selected an unknown participant".to_string(),
                )
            })?;
        let resource_matches = participant.kind == "character"
            && participant.resource.as_ref().is_some_and(|resource| {
                resource.kind == "character" && resource.id == speaker.resource_id
            });
        if !resource_matches {
            return Err(AirpError::BadRequest(
                "conversation policy selected an invalid character resource".to_string(),
            ));
        }
    }
    Ok(plan)
}

struct SceneRoundRobinV1;

#[async_trait]
impl ConversationPolicy for SceneRoundRobinV1 {
    fn descriptor(&self) -> ConversationPolicyDescriptor {
        ConversationPolicyDescriptor {
            schema_version: CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION,
            policy_id: SCENE_ROUND_ROBIN_V1.to_string(),
            policy_version: "1.0.0".to_string(),
            description:
                "Execute each character participant once in manifest order for one scene turn."
                    .to_string(),
            provenance: ConversationPolicyProvenance {
                source: ConversationPolicySource::BuiltIn,
                provider: "AIRP Engine".to_string(),
                implementation: "airp_core::conversation_policy::SceneRoundRobinV1".to_string(),
            },
            capabilities: ConversationPolicyCapabilities {
                execution_modes: vec![ConversationExecutionMode::Serial],
                supports_message_limit: false,
            },
            resource_limits: ConversationPolicyResourceLimits {
                max_speakers_per_turn: MAX_CONVERSATION_SPEAKERS_PER_TURN,
                max_parallelism: 1,
                max_config_bytes: 4,
                planning_timeout_ms: 100,
            },
            lifecycle_state: ConversationPolicyLifecycleState::Active,
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

    fn validate_config(&self, config: &Value) -> Result<(), AirpError> {
        if !config.is_null() && !config.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err(AirpError::BadRequest(
                "scene round-robin does not accept policy config".to_string(),
            ));
        }
        Ok(())
    }

    async fn plan_turn(
        &self,
        manifest: &ConversationManifest,
        _policy: &ConversationPolicyRef,
        _user_actor_id: &str,
    ) -> Result<ConversationTurnPlan, AirpError> {
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

        Ok(ConversationTurnPlan {
            scene_id: scene_ids.into_iter().next().expect("length checked"),
            speakers,
            execution_mode: ConversationExecutionMode::Serial,
            stop_after_messages: None,
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

    struct TestPolicy {
        descriptor_calls: AtomicUsize,
        mode: ConversationExecutionMode,
        stop_after_messages: Option<usize>,
        delay: Option<Duration>,
        panic_on_plan: bool,
    }

    impl TestPolicy {
        fn external(mode: ConversationExecutionMode) -> Self {
            Self {
                descriptor_calls: AtomicUsize::new(0),
                mode,
                stop_after_messages: None,
                delay: None,
                panic_on_plan: false,
            }
        }
    }

    #[async_trait]
    impl ConversationPolicy for TestPolicy {
        fn descriptor(&self) -> ConversationPolicyDescriptor {
            let call = self.descriptor_calls.fetch_add(1, Ordering::Relaxed);
            external_descriptor(if call == 0 {
                "vendor.test.v1"
            } else {
                "vendor.changed.v1"
            })
        }

        fn validate_config(&self, config: &Value) -> Result<(), AirpError> {
            if config.get("code").is_some() {
                return Err(AirpError::BadRequest(
                    "configuration cannot contain executable code".to_string(),
                ));
            }
            Ok(())
        }

        async fn plan_turn(
            &self,
            _manifest: &ConversationManifest,
            _policy: &ConversationPolicyRef,
            _user_actor_id: &str,
        ) -> Result<ConversationTurnPlan, AirpError> {
            assert!(!self.panic_on_plan, "test policy planning panic");
            if let Some(delay) = self.delay {
                tokio::time::sleep(delay).await;
            }
            Ok(ConversationTurnPlan {
                scene_id: "tavern".to_string(),
                speakers: vec![
                    ConversationSpeaker {
                        participant_id: "character:alice".to_string(),
                        resource_id: "alice".to_string(),
                    },
                    ConversationSpeaker {
                        participant_id: "character:bob".to_string(),
                        resource_id: "bob".to_string(),
                    },
                ],
                execution_mode: self.mode,
                stop_after_messages: self.stop_after_messages,
            })
        }
    }

    fn external_descriptor(policy_id: &str) -> ConversationPolicyDescriptor {
        ConversationPolicyDescriptor {
            schema_version: CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION,
            policy_id: policy_id.to_string(),
            policy_version: "1.2.3".to_string(),
            description: "Test external policy".to_string(),
            provenance: ConversationPolicyProvenance {
                source: ConversationPolicySource::External,
                provider: "AIRP test host".to_string(),
                implementation: "conversation_policy::tests::TestPolicy".to_string(),
            },
            capabilities: ConversationPolicyCapabilities {
                execution_modes: vec![
                    ConversationExecutionMode::Serial,
                    ConversationExecutionMode::Parallel,
                ],
                supports_message_limit: true,
            },
            resource_limits: ConversationPolicyResourceLimits {
                max_speakers_per_turn: 4,
                max_parallelism: 2,
                max_config_bytes: 1024,
                planning_timeout_ms: 50,
            },
            lifecycle_state: ConversationPolicyLifecycleState::Active,
            config_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn manifest(policy_id: &str, config: Value) -> ConversationManifest {
        crate::conversation::ConversationService::new(tempdir().unwrap().path())
            .create_blocking(CreateConversationRequest {
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

    #[tokio::test]
    async fn builtins_are_versioned_and_preserve_manifest_order() {
        let registry = ConversationPolicyRegistry::with_builtins();
        let descriptors = registry.list();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].policy_id, SCENE_ROUND_ROBIN_V1);
        assert_eq!(descriptors[0].policy_version, "1.0.0");
        assert_eq!(
            descriptors[0].provenance.source,
            ConversationPolicySource::BuiltIn
        );
        let resolved = registry
            .plan_turn(
                &manifest(SCENE_ROUND_ROBIN_V1, serde_json::json!({})),
                "human:gm",
            )
            .await
            .unwrap();
        assert_eq!(
            resolved.plan.execution_mode,
            ConversationExecutionMode::Serial
        );
        assert_eq!(
            resolved
                .plan
                .speakers
                .iter()
                .map(|speaker| speaker.participant_id.as_str())
                .collect::<Vec<_>>(),
            ["character:alice", "character:bob"]
        );
        registry
            .plan_turn(&manifest(SCENE_ROUND_ROBIN_V1, Value::Null), "human:gm")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn runtime_registration_lifecycle_and_descriptor_snapshot_are_enforced() {
        let registry = ConversationPolicyRegistry::with_builtins();
        registry
            .register_external(Arc::new(TestPolicy::external(
                ConversationExecutionMode::Parallel,
            )))
            .unwrap();
        assert_eq!(registry.list()[1].policy_id, "vendor.test.v1");
        registry
            .set_external_lifecycle("vendor.test.v1", ConversationPolicyLifecycleState::Disabled)
            .unwrap();
        let error = registry
            .plan_turn(
                &manifest("vendor.test.v1", serde_json::json!({})),
                "human:gm",
            )
            .await
            .unwrap_err();
        assert!(matches!(error, AirpError::BadRequest(_)));
        registry.unregister_external("vendor.test.v1").unwrap();
        assert_eq!(registry.list().len(), 1);
    }

    #[tokio::test]
    async fn external_parallel_plan_and_message_stop_are_engine_bounded() {
        let registry = ConversationPolicyRegistry::new();
        let mut policy = TestPolicy::external(ConversationExecutionMode::Parallel);
        policy.stop_after_messages = Some(1);
        registry.register_external(Arc::new(policy)).unwrap();
        let resolved = registry
            .plan_turn(
                &manifest("vendor.test.v1", serde_json::json!({})),
                "human:gm",
            )
            .await
            .unwrap();
        assert_eq!(
            resolved.plan.execution_mode,
            ConversationExecutionMode::Parallel
        );
        assert_eq!(resolved.plan.speakers.len(), 1);
        assert_eq!(resolved.policy.resource_limits.max_parallelism, 2);
    }

    #[tokio::test]
    async fn unknown_config_and_planning_timeout_fail_closed() {
        let registry = ConversationPolicyRegistry::new();
        let mut policy = TestPolicy::external(ConversationExecutionMode::Serial);
        policy.delay = Some(Duration::from_millis(100));
        registry.register_external(Arc::new(policy)).unwrap();

        let config_error = registry
            .plan_turn(
                &manifest("vendor.test.v1", serde_json::json!({"code": "run()"})),
                "human:gm",
            )
            .await
            .unwrap_err();
        assert!(matches!(config_error, AirpError::BadRequest(_)));

        let timeout = registry
            .plan_turn(
                &manifest("vendor.test.v1", serde_json::json!({})),
                "human:gm",
            )
            .await
            .unwrap_err();
        assert!(
            timeout.to_string().contains("timed out"),
            "unexpected error: {timeout}"
        );

        let panic_registry = ConversationPolicyRegistry::new();
        let mut panicking = TestPolicy::external(ConversationExecutionMode::Serial);
        panicking.panic_on_plan = true;
        panic_registry
            .register_external(Arc::new(panicking))
            .unwrap();
        let panic = panic_registry
            .plan_turn(
                &manifest("vendor.test.v1", serde_json::json!({})),
                "human:gm",
            )
            .await
            .unwrap_err();
        assert!(
            panic.to_string().contains("planning failed"),
            "unexpected error: {panic}"
        );
    }

    #[test]
    fn duplicate_and_reserved_external_registration_are_rejected() {
        let registry = ConversationPolicyRegistry::new();
        registry
            .register_external(Arc::new(TestPolicy::external(
                ConversationExecutionMode::Serial,
            )))
            .unwrap();
        let duplicate = registry
            .register_external(Arc::new(TestPolicy::external(
                ConversationExecutionMode::Serial,
            )))
            .unwrap_err();
        assert!(matches!(duplicate, AirpError::Config(_)));

        let mut descriptor = external_descriptor("airp.impostor.v1");
        descriptor.provenance.source = ConversationPolicySource::External;
        struct Reserved(ConversationPolicyDescriptor);
        #[async_trait]
        impl ConversationPolicy for Reserved {
            fn descriptor(&self) -> ConversationPolicyDescriptor {
                self.0.clone()
            }
            fn validate_config(&self, _config: &Value) -> Result<(), AirpError> {
                Ok(())
            }
            async fn plan_turn(
                &self,
                _manifest: &ConversationManifest,
                _policy: &ConversationPolicyRef,
                _user_actor_id: &str,
            ) -> Result<ConversationTurnPlan, AirpError> {
                unreachable!()
            }
        }
        let reserved = ConversationPolicyRegistry::new()
            .register_external(Arc::new(Reserved(descriptor)))
            .unwrap_err();
        assert!(matches!(reserved, AirpError::Config(_)));
    }
}
