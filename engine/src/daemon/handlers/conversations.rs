//! UI-independent Conversation HTTP API.

use crate::conversation::{
    effective_conversation_root, AppendConversationEventRequest, ConversationEventsQuery,
    ConversationScopeQuery, ConversationService, ConversationTurnFailure, ConversationTurnOutcome,
    ConversationTurnRequest, ConversationTurnStatus, CreateConversationRequest,
    CreateSceneConversationRequest,
};
use crate::conversation_compat::{
    ConversationMigrationScope, ExecuteConversationMigrationRequest,
    PlanConversationMigrationRequest,
};
use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::types::SessionId;
use axum::{extract::Query, Json};
use futures_util::{stream, StreamExt};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const CONVERSATION_TURN_TIMEOUT: Duration =
    Duration::from_secs(crate::conversation_observability::CONVERSATION_TURN_TIMEOUT_SECS);

struct TurnExecution<'a> {
    root: &'a std::path::Path,
    conversation_id: SessionId,
    turn_id: String,
    fingerprint: String,
    planning_ms: u64,
    registration: &'a crate::conversation_turn::ActiveTurnRegistration,
}

/// Read-only compatibility analysis. This endpoint never creates or repairs
/// legacy files and returns `needs_review` when attribution is not provable.
pub(in crate::daemon) async fn plan_conversation_migration_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(request): Json<PlanConversationMigrationRequest>,
) -> Result<Json<crate::conversation_compat::ConversationMigrationReport>, AirpError> {
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    let report = tokio::task::spawn_blocking(move || {
        crate::conversation_compat::plan_conversation_migration(&root, request)
    })
    .await
    .map_err(|error| AirpError::Internal(format!("migration planning task failed: {error}")))??;
    Ok(Json(report))
}

/// Execute an explicitly confirmed copy migration after verifying the source
/// digest returned by the planning endpoint.
pub(in crate::daemon) async fn execute_conversation_migration_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(request): Json<ExecuteConversationMigrationRequest>,
) -> Result<Json<crate::conversation_compat::ConversationMigrationReport>, AirpError> {
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    Ok(Json(
        crate::conversation_compat::execute_conversation_migration(root, request).await?,
    ))
}

/// Remove the unmodified Conversation produced by one migration while keeping
/// its verified readable backup and the untouched legacy source.
pub(in crate::daemon) async fn rollback_conversation_migration_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(migration_id): axum::extract::Path<String>,
    Query(query): Query<ConversationMigrationScope>,
) -> Result<Json<crate::conversation_compat::ConversationMigrationReport>, AirpError> {
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    Ok(Json(
        crate::conversation_compat::rollback_conversation_migration(root, &migration_id).await?,
    ))
}

/// Return the integrity-checked, human-readable source export retained for a
/// completed or rolled-back migration.
pub(in crate::daemon) async fn get_conversation_migration_export_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(migration_id): axum::extract::Path<String>,
    Query(query): Query<ConversationMigrationScope>,
) -> Result<Json<crate::conversation_compat::LegacyConversationExport>, AirpError> {
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    let export = tokio::task::spawn_blocking(move || {
        crate::conversation_compat::load_migration_export(&root, &migration_id)
    })
    .await
    .map_err(|error| AirpError::Internal(format!("migration export task failed: {error}")))??;
    Ok(Json(export))
}

/// Create a generic Engine-owned Conversation manifest.
pub(in crate::daemon) async fn create_conversation_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<crate::conversation::ConversationManifest>, AirpError> {
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    Ok(Json(ConversationService::new(root).create(request).await?))
}

/// Snapshot a scene into a generic Conversation manifest.
pub(in crate::daemon) async fn create_scene_conversation_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(scene_id): axum::extract::Path<String>,
    Json(request): Json<CreateSceneConversationRequest>,
) -> Result<Json<crate::conversation::ConversationManifest>, AirpError> {
    let scene_id = crate::types::SceneId::new(scene_id)?;
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    let scene = crate::scene::SceneConfig::load(&root, &scene_id)?;
    let create = crate::conversation::request_from_scene(&scene, request);
    Ok(Json(ConversationService::new(root).create(create).await?))
}

/// List readable Conversation manifests within the selected user scope.
pub(in crate::daemon) async fn list_conversations_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Query(query): Query<ConversationScopeQuery>,
) -> Result<Json<Vec<crate::conversation::ConversationManifest>>, AirpError> {
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    Ok(Json(ConversationService::new(root).list().await?))
}

/// Load one Conversation manifest from the selected user scope.
pub(in crate::daemon) async fn get_conversation_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
    Query(query): Query<ConversationScopeQuery>,
) -> Result<Json<crate::conversation::ConversationManifest>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    Ok(Json(
        ConversationService::new(root).get(conversation_id).await?,
    ))
}

/// Append one caller-supplied event through the durable journal service.
pub(in crate::daemon) async fn append_conversation_event_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
    Json(request): Json<AppendConversationEventRequest>,
) -> Result<Json<crate::conversation::ConversationEvent>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    Ok(Json(
        ConversationService::new(root)
            .append_event(conversation_id, request)
            .await?,
    ))
}

/// Read a bounded cursor window from a Conversation journal.
pub(in crate::daemon) async fn get_conversation_events_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
    Query(query): Query<ConversationEventsQuery>,
) -> Result<Json<crate::conversation::ConversationEventWindow>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    Ok(Json(
        ConversationService::new(root)
            .events(conversation_id, query.limit, query.before.as_deref())
            .await?,
    ))
}

/// Discover registered Engine policy descriptors.
pub(in crate::daemon) async fn list_conversation_policies_endpoint(
    axum::Extension(registry): axum::Extension<
        Arc<crate::conversation_policy::ConversationPolicyRegistry>,
    >,
) -> Json<Vec<crate::conversation_policy::ConversationPolicyDescriptor>> {
    Json(registry.list())
}

/// Discover the versioned Conversation schemas, execution limits, redacted
/// observability fields, and stable turn recovery codes.
pub(in crate::daemon) async fn get_conversation_capabilities_endpoint(
) -> Json<crate::conversation_observability::ConversationCapabilities> {
    Json(crate::conversation_observability::conversation_capabilities())
}

/// Project a redacted turn trace from authoritative journal evidence.
pub(in crate::daemon) async fn get_conversation_turn_observability_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((conversation_id, turn_id)): axum::extract::Path<(String, String)>,
    Query(query): Query<ConversationScopeQuery>,
) -> Result<Json<crate::conversation_observability::ConversationTurnObservability>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    validate_turn_id(&turn_id)?;
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    let service = ConversationService::new(&root);
    service.get(conversation_id).await?;
    let snapshot = if let Some(active) =
        crate::conversation_turn::active_turn_snapshot(&root, conversation_id, &turn_id)
    {
        active
    } else {
        let events = service.all_events(conversation_id).await?;
        crate::conversation_turn::project_turn(&events, &turn_id)?
            .ok_or_else(|| AirpError::NotFound(format!("turn {turn_id} not found")))?
    };
    Ok(Json(
        crate::conversation_observability::project_turn_observability(&snapshot),
    ))
}

/// Read durable turn state. A non-terminal turn with no active executor is
/// closed as `unknown_commit` while holding the Conversation write lock.
pub(in crate::daemon) async fn get_conversation_turn_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((conversation_id, turn_id)): axum::extract::Path<(String, String)>,
    Query(query): Query<ConversationScopeQuery>,
) -> Result<Json<crate::conversation_turn::ConversationTurnSnapshot>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    validate_turn_id(&turn_id)?;
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    let service = ConversationService::new(&root);
    service.get(conversation_id).await?;
    if let Some(snapshot) =
        crate::conversation_turn::active_turn_snapshot(&root, conversation_id, &turn_id)
    {
        return Ok(Json(snapshot));
    }
    let events = service.all_events(conversation_id).await?;
    let snapshot = crate::conversation_turn::project_turn(&events, &turn_id)?
        .ok_or_else(|| AirpError::NotFound(format!("turn {turn_id} not found")))?;
    if snapshot.lifecycle_state.is_terminal()
        || crate::conversation_turn::has_active_turn(&root, conversation_id, &turn_id)
    {
        return Ok(Json(snapshot));
    }
    let _write_guard = service.acquire_write(conversation_id).await;
    let events = service.all_events_locked_async(conversation_id).await?;
    let snapshot = crate::conversation_turn::project_turn(&events, &turn_id)?
        .ok_or_else(|| AirpError::NotFound(format!("turn {turn_id} not found")))?;
    if snapshot.lifecycle_state.is_terminal() {
        return Ok(Json(snapshot));
    }
    let expected_next_sequence = events
        .last()
        .map_or(0, |event| event.sequence.saturating_add(1));
    Ok(Json(
        reconcile_unknown_commit(service, conversation_id, snapshot, expected_next_sequence)
            .await?,
    ))
}

/// Explicitly request cooperative cancellation of process-local turn work.
///
/// The executor records the terminal `turn.cancelled` event. If no executor
/// survives (for example after restart), this endpoint reconciles a dangling
/// journal lifecycle to `unknown_commit` instead of pretending rollback.
pub(in crate::daemon) async fn cancel_conversation_turn_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((conversation_id, turn_id)): axum::extract::Path<(String, String)>,
    Query(query): Query<ConversationScopeQuery>,
) -> Result<Json<crate::conversation_turn::ConversationTurnCancelResponse>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    validate_turn_id(&turn_id)?;
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    let service = ConversationService::new(&root);
    service.get(conversation_id).await?;
    if crate::conversation_turn::cancel_active_turn(&root, conversation_id, &turn_id) {
        return Ok(Json(
            crate::conversation_turn::ConversationTurnCancelResponse {
                turn_id,
                cancel_requested: true,
                lifecycle_state: None,
            },
        ));
    }

    let _write_guard = service.acquire_write(conversation_id).await;
    let events = service.all_events_locked_async(conversation_id).await?;
    let snapshot = crate::conversation_turn::project_turn(&events, &turn_id)?
        .ok_or_else(|| AirpError::NotFound(format!("turn {turn_id} not found")))?;
    let snapshot = if snapshot.lifecycle_state.is_terminal() {
        snapshot
    } else {
        let expected_next_sequence = events
            .last()
            .map_or(0, |event| event.sequence.saturating_add(1));
        reconcile_unknown_commit(service, conversation_id, snapshot, expected_next_sequence).await?
    };
    Ok(Json(
        crate::conversation_turn::ConversationTurnCancelResponse {
            turn_id,
            cancel_requested: false,
            lifecycle_state: Some(snapshot.lifecycle_state),
        },
    ))
}

/// Execute and durably record one bounded Engine-owned Conversation turn.
pub(in crate::daemon) async fn execute_conversation_turn_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::Extension(conversation_policies): axum::Extension<
        Arc<crate::conversation_policy::ConversationPolicyRegistry>,
    >,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
    Json(request): Json<ConversationTurnRequest>,
) -> Result<Json<ConversationTurnOutcome>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    validate_turn_base(&request)?;
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    let turn_id = request.turn_id.clone().unwrap_or_else(crate::ulid::new_id);
    validate_turn_id(&turn_id)?;
    let fingerprint = crate::conversation_turn::request_fingerprint(&request)?;
    let registration = crate::conversation_turn::register_active_turn(
        &root,
        conversation_id,
        &turn_id,
        request.expected_next_sequence,
    );
    let service = ConversationService::new(&root);
    let _write_guard = service.acquire_write(conversation_id).await;
    let manifest = service.get(conversation_id).await?;
    let existing_events = service.all_events_locked_async(conversation_id).await?;
    if let Some(snapshot) = crate::conversation_turn::project_turn(&existing_events, &turn_id)? {
        let Some(stored_fingerprint) = crate::conversation_turn::accepted_fingerprint(&snapshot)
        else {
            return Err(AirpError::Conflict(format!(
                "turn_id {turn_id} is already used by a legacy turn without an idempotency record"
            )));
        };
        if stored_fingerprint != fingerprint {
            return Err(AirpError::Conflict(format!(
                "turn_id {turn_id} was already submitted with a different request"
            )));
        }
        if snapshot.lifecycle_state.is_terminal() {
            return Ok(Json(snapshot.into_outcome()));
        }
        return Ok(Json(
            reconcile_unknown_commit(
                service,
                conversation_id,
                snapshot,
                existing_events
                    .last()
                    .map_or(0, |event| event.sequence.saturating_add(1)),
            )
            .await?
            .into_outcome(),
        ));
    }

    let planning_started = tokio::time::Instant::now();
    let resolved_plan = conversation_policies
        .plan_turn(&manifest, &request.user_actor_id)
        .await?;
    let planning_ms = u64::try_from(planning_started.elapsed().as_millis()).unwrap_or(u64::MAX);
    if resolved_plan.plan.speakers.len()
        > crate::conversation_policy::MAX_CONVERSATION_SPEAKERS_PER_TURN
    {
        return Err(AirpError::BadRequest(format!(
            "conversation turn planned {} speakers; maximum is {}",
            resolved_plan.plan.speakers.len(),
            crate::conversation_policy::MAX_CONVERSATION_SPEAKERS_PER_TURN
        )));
    }
    let provider_call_count = u32::try_from(resolved_plan.plan.speakers.len()).map_err(|_| {
        AirpError::BadRequest("conversation speaker count is too large".to_string())
    })?;
    let quota = state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .quota
        .clone();
    crate::quota::check_and_increment_by(&root, &quota, provider_call_count)?;
    let result = execute_new_turn(
        &state,
        service,
        request,
        resolved_plan,
        TurnExecution {
            root: &root,
            conversation_id,
            turn_id,
            fingerprint,
            planning_ms,
            registration: &registration,
        },
    )
    .await;
    if let Ok(outcome) = &result {
        registration.publish_outcome(outcome);
    }
    result.map(Json)
}

async fn execute_new_turn(
    state: &Arc<DaemonState>,
    service: ConversationService,
    request: ConversationTurnRequest,
    resolved_plan: crate::conversation_policy::ResolvedConversationTurnPlan,
    execution: TurnExecution<'_>,
) -> Result<ConversationTurnOutcome, AirpError> {
    let crate::conversation_policy::ResolvedConversationTurnPlan { policy, plan } = resolved_plan;
    let TurnExecution {
        root,
        conversation_id,
        turn_id,
        fingerprint,
        planning_ms,
        registration,
    } = execution;
    let cancellation = registration.cancellation();
    let mut committed = Vec::new();
    let accepted = service
        .append_event_locked_async(
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: crate::conversation_turn::TURN_ACCEPTED.to_string(),
                actor_id: Some(request.user_actor_id.clone()),
                causation_id: None,
                correlation_id: Some(turn_id.clone()),
                payload: serde_json::json!({
                    "request_fingerprint": fingerprint,
                    "expected_next_sequence": request.expected_next_sequence,
                    "policy": {
                        "policy_id": &policy.policy_id,
                        "policy_version": &policy.policy_version,
                        "provenance": &policy.provenance,
                        "execution_mode": plan.execution_mode,
                        "stop_after_speakers": plan.stop_after_speakers,
                        "max_parallelism": policy.resource_limits.max_parallelism,
                        "scene_id": &plan.scene_id,
                        "speakers": &plan.speakers,
                    },
                    "observability": {
                        "schema_version": crate::conversation_observability::CONVERSATION_OBSERVABILITY_SCHEMA_VERSION,
                        "planning_ms": planning_ms,
                        "quota_reserved_requests": plan.speakers.len(),
                    },
                }),
                extensions: request.extensions.clone(),
                expected_next_sequence: Some(request.expected_next_sequence),
            },
        )
        .await?;
    let accepted_id = accepted.event_id.clone();
    let mut next_sequence = accepted.sequence + 1;
    committed.push(accepted);
    registration.publish(
        crate::conversation_turn::ConversationTurnLifecycleState::Accepted,
        &committed,
    );
    if cancellation.is_cancelled() {
        return commit_turn_terminal(
            service,
            conversation_id,
            turn_id,
            crate::conversation_turn::TURN_CANCELLED,
            None,
            "turn_cancelled",
            TurnTerminalObservability::default(),
            accepted_id,
            next_sequence,
            committed,
        )
        .await;
    }
    let started = service
        .append_event_locked_async(
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: crate::conversation_turn::TURN_STARTED.to_string(),
                actor_id: None,
                causation_id: Some(accepted_id),
                correlation_id: Some(turn_id.clone()),
                payload: serde_json::json!({}),
                extensions: BTreeMap::new(),
                expected_next_sequence: Some(next_sequence),
            },
        )
        .await?;
    next_sequence += 1;
    let started_id = started.event_id.clone();
    committed.push(started);
    registration.publish(
        crate::conversation_turn::ConversationTurnLifecycleState::Running,
        &committed,
    );
    if cancellation.is_cancelled() {
        return commit_turn_terminal(
            service,
            conversation_id,
            turn_id,
            crate::conversation_turn::TURN_CANCELLED,
            None,
            "turn_cancelled",
            TurnTerminalObservability::default(),
            started_id,
            next_sequence,
            committed,
        )
        .await;
    }

    let context_budget = crate::conversation_context::ConversationContextBudget::default();
    let mut history = match service
        .context_projection(conversation_id, context_budget)
        .await
    {
        Ok(projection) => projection.messages,
        Err(error) => {
            tracing::error!(
                %conversation_id,
                %turn_id,
                %error,
                "conversation context projection failed before user event commit"
            );
            return commit_turn_terminal(
                service,
                conversation_id,
                turn_id,
                crate::conversation_turn::TURN_FAILED,
                None,
                "context_projection_failed",
                TurnTerminalObservability::default(),
                started_id,
                next_sequence,
                committed,
            )
            .await;
        }
    };
    if let Err(error) = crate::conversation_context::push_bounded_context_message(
        &mut history,
        crate::adapter::ChatMessage {
            role: crate::adapter::MessageRole::User,
            content: request.base.message.clone(),
        },
        context_budget,
    ) {
        tracing::error!(
            %conversation_id,
            %turn_id,
            %error,
            "conversation user message exceeds context budget"
        );
        return commit_turn_terminal(
            service,
            conversation_id,
            turn_id,
            crate::conversation_turn::TURN_FAILED,
            None,
            "context_budget_exceeded",
            TurnTerminalObservability::default(),
            started_id,
            next_sequence,
            committed,
        )
        .await;
    }
    let user_event = service
        .append_event_locked_async(
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: "message.created".to_string(),
                actor_id: Some(request.user_actor_id.clone()),
                causation_id: Some(started_id),
                correlation_id: Some(turn_id.clone()),
                payload: serde_json::json!({
                    "role": "user",
                    "content": request.base.message,
                }),
                extensions: request.extensions.clone(),
                expected_next_sequence: Some(next_sequence),
            },
        )
        .await?;
    next_sequence = user_event.sequence + 1;
    let user_event_id = user_event.event_id.clone();
    committed.push(user_event);
    registration.publish(
        crate::conversation_turn::ConversationTurnLifecycleState::Running,
        &committed,
    );
    let turn_deadline = tokio::time::Instant::now() + CONVERSATION_TURN_TIMEOUT;

    match plan.execution_mode {
        crate::conversation_policy::ConversationExecutionMode::Serial => {
            for speaker in plan.speakers {
                let output = match generate_conversation_speaker(
                    state,
                    &request,
                    &plan.scene_id,
                    history.clone(),
                    speaker,
                    &cancellation,
                    turn_deadline,
                    root,
                    conversation_id,
                    &turn_id,
                )
                .await
                {
                    Ok(output) => output,
                    Err(failure) => {
                        return commit_turn_terminal(
                            service,
                            conversation_id,
                            turn_id,
                            failure.event_kind,
                            Some(failure.participant_id.clone()),
                            failure.code,
                            TurnTerminalObservability::from_failure(&failure),
                            user_event_id,
                            next_sequence,
                            committed,
                        )
                        .await;
                    }
                };
                if let Some(participant_id) = commit_speaker_output(
                    &service,
                    conversation_id,
                    &turn_id,
                    &user_event_id,
                    output,
                    &mut next_sequence,
                    &mut committed,
                    &mut history,
                    context_budget,
                    registration,
                )
                .await?
                {
                    return commit_turn_terminal(
                        service,
                        conversation_id,
                        turn_id,
                        crate::conversation_turn::TURN_FAILED,
                        Some(participant_id),
                        "context_budget_exceeded",
                        TurnTerminalObservability::default(),
                        user_event_id,
                        next_sequence,
                        committed,
                    )
                    .await;
                }
            }
        }
        crate::conversation_policy::ConversationExecutionMode::Parallel => {
            let max_parallelism = policy.resource_limits.max_parallelism;
            let initial_history = history.clone();
            let generations = stream::iter(plan.speakers.into_iter().map(|speaker| {
                generate_conversation_speaker(
                    state,
                    &request,
                    &plan.scene_id,
                    initial_history.clone(),
                    speaker,
                    &cancellation,
                    turn_deadline,
                    root,
                    conversation_id,
                    &turn_id,
                )
            }))
            .buffered(max_parallelism)
            .collect::<Vec<_>>()
            .await;
            let mut generations = generations.into_iter();
            while let Some(generated) = generations.next() {
                let output = match generated {
                    Ok(output) => output,
                    Err(failure) => {
                        let dropped = generations
                            .map(|generated| match generated {
                                Ok(output) => UncommittedSpeakerObservation {
                                    participant_id: output.participant_id,
                                    outcome: crate::conversation_observability::ConversationSpeakerOutcome::GeneratedNotCommitted,
                                    speaker_latency_ms: output.latency_ms,
                                    recorded_output_tokens: Some(output.recorded_tokens),
                                },
                                Err(failure) => UncommittedSpeakerObservation {
                                    participant_id: failure.participant_id,
                                    outcome: crate::conversation_observability::ConversationSpeakerOutcome::Failed,
                                    speaker_latency_ms: failure.latency_ms,
                                    recorded_output_tokens: failure.recorded_tokens,
                                },
                            })
                            .collect::<Vec<_>>();
                        if !dropped.is_empty() {
                            let dropped_participants = dropped
                                .iter()
                                .map(|speaker| speaker.participant_id.as_str())
                                .collect::<Vec<_>>();
                            let dropped_recorded_tokens = dropped
                                .iter()
                                .filter_map(|speaker| speaker.recorded_output_tokens)
                                .map(u64::from)
                                .sum::<u64>();
                            tracing::warn!(
                                %conversation_id,
                                %turn_id,
                                failed_participant_id = %failure.participant_id,
                                code = failure.code,
                                ?dropped_participants,
                                dropped_recorded_tokens,
                                "parallel turn failed closed after later speaker outputs were generated and billed but not committed"
                            );
                        }
                        return commit_turn_terminal(
                            service,
                            conversation_id,
                            turn_id,
                            failure.event_kind,
                            Some(failure.participant_id.clone()),
                            failure.code,
                            TurnTerminalObservability {
                                speaker_latency_ms: Some(failure.latency_ms),
                                recorded_output_tokens: failure.recorded_tokens,
                                uncommitted_speakers: dropped,
                                ..TurnTerminalObservability::default()
                            },
                            user_event_id,
                            next_sequence,
                            committed,
                        )
                        .await;
                    }
                };
                if let Some(participant_id) = commit_speaker_output(
                    &service,
                    conversation_id,
                    &turn_id,
                    &user_event_id,
                    output,
                    &mut next_sequence,
                    &mut committed,
                    &mut history,
                    context_budget,
                    registration,
                )
                .await?
                {
                    return commit_turn_terminal(
                        service,
                        conversation_id,
                        turn_id,
                        crate::conversation_turn::TURN_FAILED,
                        Some(participant_id),
                        "context_budget_exceeded",
                        TurnTerminalObservability::default(),
                        user_event_id,
                        next_sequence,
                        committed,
                    )
                    .await;
                }
            }
        }
    }

    if cancellation.is_cancelled() {
        return commit_turn_terminal(
            service,
            conversation_id,
            turn_id,
            crate::conversation_turn::TURN_CANCELLED,
            None,
            "turn_cancelled",
            TurnTerminalObservability::default(),
            user_event_id,
            next_sequence,
            committed,
        )
        .await;
    }
    let completed = service
        .append_event_locked_async(
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: crate::conversation_turn::TURN_COMPLETED.to_string(),
                actor_id: None,
                causation_id: Some(user_event_id),
                correlation_id: Some(turn_id.clone()),
                payload: serde_json::json!({
                    "message_count": committed
                        .iter()
                        .filter(|event| event.kind == "message.created")
                        .count(),
                }),
                extensions: BTreeMap::new(),
                expected_next_sequence: Some(next_sequence),
            },
        )
        .await?;
    next_sequence += 1;
    committed.push(completed);
    Ok(ConversationTurnOutcome {
        turn_id,
        status: ConversationTurnStatus::Completed,
        lifecycle_state: crate::conversation_turn::ConversationTurnLifecycleState::Completed,
        events: committed,
        next_sequence,
        failure: None,
    })
}

struct GeneratedConversationSpeaker {
    participant_id: String,
    character_id: String,
    content: String,
    live_state: Option<serde_json::Value>,
    chunks: serde_json::Value,
    recorded_tokens: u32,
    latency_ms: u64,
}

struct ConversationSpeakerFailure {
    participant_id: String,
    event_kind: &'static str,
    code: &'static str,
    latency_ms: u64,
    recorded_tokens: Option<u32>,
}

#[derive(serde::Serialize)]
struct UncommittedSpeakerObservation {
    participant_id: String,
    outcome: crate::conversation_observability::ConversationSpeakerOutcome,
    speaker_latency_ms: u64,
    recorded_output_tokens: Option<u32>,
}

#[derive(serde::Serialize)]
struct TurnTerminalObservability {
    schema_version: u32,
    speaker_latency_ms: Option<u64>,
    recorded_output_tokens: Option<u32>,
    uncommitted_speakers: Vec<UncommittedSpeakerObservation>,
}

impl TurnTerminalObservability {
    fn from_failure(failure: &ConversationSpeakerFailure) -> Self {
        Self {
            speaker_latency_ms: Some(failure.latency_ms),
            recorded_output_tokens: failure.recorded_tokens,
            ..Self::default()
        }
    }
}

impl Default for TurnTerminalObservability {
    fn default() -> Self {
        Self {
            schema_version:
                crate::conversation_observability::CONVERSATION_OBSERVABILITY_SCHEMA_VERSION,
            speaker_latency_ms: None,
            recorded_output_tokens: None,
            uncommitted_speakers: Vec::new(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn generate_conversation_speaker(
    state: &Arc<DaemonState>,
    request: &ConversationTurnRequest,
    scene_id: &str,
    history: Vec<crate::adapter::ChatMessage>,
    speaker: crate::conversation_policy::ConversationSpeaker,
    cancellation: &tokio_util::sync::CancellationToken,
    turn_deadline: tokio::time::Instant,
    root: &std::path::Path,
    conversation_id: SessionId,
    turn_id: &str,
) -> Result<GeneratedConversationSpeaker, ConversationSpeakerFailure> {
    let speaker_started = tokio::time::Instant::now();
    let participant_id = speaker.participant_id;
    let character_id = speaker.resource_id;
    let mut payload = request.base.clone();
    payload.user_id = request.user_id.as_ref().map(ToString::to_string);
    payload.scene_id = Some(scene_id.to_string());
    payload.messages_history = None;
    let pipeline = match crate::chat_pipeline::prepare_scene_participant_pipeline(
        &payload,
        state,
        &character_id,
        history,
    ) {
        Ok(pipeline) => pipeline,
        Err(error) => {
            tracing::error!(
                %conversation_id,
                %turn_id,
                %participant_id,
                %error,
                "conversation turn preparation failed after user event commit"
            );
            return Err(ConversationSpeakerFailure {
                participant_id,
                event_kind: crate::conversation_turn::TURN_FAILED,
                code: "generation_preparation_failed",
                latency_ms: elapsed_millis(speaker_started),
                recorded_tokens: None,
            });
        }
    };
    let result = tokio::select! {
        biased;
        _ = cancellation.cancelled() => {
            tracing::info!(
                %conversation_id,
                %turn_id,
                %participant_id,
                "conversation turn explicitly cancelled"
            );
            return Err(ConversationSpeakerFailure {
                participant_id,
                event_kind: crate::conversation_turn::TURN_CANCELLED,
                code: "turn_cancelled",
                latency_ms: elapsed_millis(speaker_started),
                recorded_tokens: None,
            });
        }
        timed = tokio::time::timeout_at(
            turn_deadline,
            crate::chat_pipeline::run_generation_step(pipeline),
        ) => match timed {
            Ok(result) => result,
            Err(_) => {
                tracing::error!(
                    %conversation_id,
                    %turn_id,
                    %participant_id,
                    timeout_secs = CONVERSATION_TURN_TIMEOUT.as_secs(),
                    "conversation turn timed out after user event commit"
                );
                return Err(ConversationSpeakerFailure {
                    participant_id,
                    event_kind: crate::conversation_turn::TURN_FAILED,
                    code: "turn_timeout",
                    latency_ms: elapsed_millis(speaker_started),
                    recorded_tokens: None,
                });
            }
        }
    };
    let recorded_tokens =
        crate::volume_store::estimate_tokens(&result.raw_acc).min(u32::MAX as usize) as u32;
    crate::quota::record_tokens(root, recorded_tokens);
    if cancellation.is_cancelled() {
        return Err(ConversationSpeakerFailure {
            participant_id,
            event_kind: crate::conversation_turn::TURN_CANCELLED,
            code: "turn_cancelled",
            latency_ms: elapsed_millis(speaker_started),
            recorded_tokens: Some(recorded_tokens),
        });
    }
    if let Some(error) = result.error {
        tracing::error!(
            %conversation_id,
            %turn_id,
            %participant_id,
            %error,
            "conversation participant generation failed after user event commit"
        );
        return Err(ConversationSpeakerFailure {
            participant_id,
            event_kind: crate::conversation_turn::TURN_FAILED,
            code: "generation_failed",
            latency_ms: elapsed_millis(speaker_started),
            recorded_tokens: Some(recorded_tokens),
        });
    }
    let (content, live_state) =
        crate::chat_pipeline::extract_conversation_state(&result.cleaned_acc);
    if content.trim().is_empty() {
        tracing::error!(
            %conversation_id,
            %turn_id,
            %participant_id,
            "conversation participant returned an empty generation after user event commit"
        );
        return Err(ConversationSpeakerFailure {
            participant_id,
            event_kind: crate::conversation_turn::TURN_FAILED,
            code: "empty_generation",
            latency_ms: elapsed_millis(speaker_started),
            recorded_tokens: Some(recorded_tokens),
        });
    }
    let mut unpacker = crate::xml_unpacker::StreamingXmlUnpacker::new();
    let mut chunks = unpacker.process_chunk(&content);
    chunks.extend(unpacker.finish());
    let chunks = serde_json::to_value(chunks).map_err(|error| {
        tracing::error!(
            %conversation_id,
            %turn_id,
            %participant_id,
            %error,
            "conversation response chunk serialization failed"
        );
        ConversationSpeakerFailure {
            participant_id: participant_id.clone(),
            event_kind: crate::conversation_turn::TURN_FAILED,
            code: "response_serialization_failed",
            latency_ms: elapsed_millis(speaker_started),
            recorded_tokens: Some(recorded_tokens),
        }
    })?;
    Ok(GeneratedConversationSpeaker {
        participant_id,
        character_id,
        content,
        live_state,
        chunks,
        recorded_tokens,
        latency_ms: elapsed_millis(speaker_started),
    })
}

fn elapsed_millis(started: tokio::time::Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[allow(clippy::too_many_arguments)]
async fn commit_speaker_output(
    service: &ConversationService,
    conversation_id: SessionId,
    turn_id: &str,
    user_event_id: &str,
    output: GeneratedConversationSpeaker,
    next_sequence: &mut u64,
    committed: &mut Vec<crate::conversation::ConversationEvent>,
    history: &mut Vec<crate::adapter::ChatMessage>,
    context_budget: crate::conversation_context::ConversationContextBudget,
    registration: &crate::conversation_turn::ActiveTurnRegistration,
) -> Result<Option<String>, AirpError> {
    let assistant_event = service
        .append_event_locked_async(
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: "message.created".to_string(),
                actor_id: Some(output.participant_id.clone()),
                causation_id: Some(user_event_id.to_string()),
                correlation_id: Some(turn_id.to_string()),
                payload: serde_json::json!({
                    "role": "assistant",
                    "content": &output.content,
                    "resource": {"kind": "character", "id": &output.character_id},
                    "chunks": &output.chunks,
                    "state": &output.live_state,
                    "observability": {
                        "schema_version": crate::conversation_observability::CONVERSATION_OBSERVABILITY_SCHEMA_VERSION,
                        "speaker_latency_ms": output.latency_ms,
                        "recorded_output_tokens": output.recorded_tokens,
                    },
                }),
                extensions: BTreeMap::new(),
                expected_next_sequence: Some(*next_sequence),
            },
        )
        .await?;
    *next_sequence += 1;
    committed.push(assistant_event);
    if let Err(error) = crate::conversation_context::push_bounded_context_message(
        history,
        crate::adapter::ChatMessage {
            role: crate::adapter::MessageRole::Assistant,
            content: format!("[{}] {}", output.participant_id, output.content),
        },
        context_budget,
    ) {
        tracing::error!(
            %conversation_id,
            %turn_id,
            participant_id = %output.participant_id,
            %error,
            "conversation assistant message exceeds context budget"
        );
        return Ok(Some(output.participant_id));
    }
    registration.publish(
        crate::conversation_turn::ConversationTurnLifecycleState::Running,
        committed,
    );
    Ok(None)
}

fn validate_turn_base(request: &ConversationTurnRequest) -> Result<(), AirpError> {
    let base = &request.base;
    if request.user_actor_id.trim().is_empty() || base.message.trim().is_empty() {
        return Err(AirpError::BadRequest(
            "user_actor_id and base.message must not be empty".to_string(),
        ));
    }
    if base.character_id.is_some()
        || base.character_card_id.is_some()
        || base.lorebook_path.is_some()
        || base.messages_history.is_some()
        || base.session_id.is_some()
        || base.scene_id.is_some()
        || base.user_id.is_some()
        || !base.swipe_candidates.is_empty()
        || base.branch_from.is_some()
    {
        return Err(AirpError::BadRequest(
            "conversation scope, history, and legacy branch controls are Engine-owned".to_string(),
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn commit_turn_terminal(
    service: ConversationService,
    conversation_id: SessionId,
    turn_id: String,
    event_kind: &str,
    participant_id: Option<String>,
    code: &str,
    observability: TurnTerminalObservability,
    causation_id: String,
    next_sequence: u64,
    mut committed: Vec<crate::conversation::ConversationEvent>,
) -> Result<ConversationTurnOutcome, AirpError> {
    let lifecycle_state = match event_kind {
        crate::conversation_turn::TURN_FAILED => {
            crate::conversation_turn::ConversationTurnLifecycleState::Failed
        }
        crate::conversation_turn::TURN_CANCELLED => {
            crate::conversation_turn::ConversationTurnLifecycleState::Cancelled
        }
        crate::conversation_turn::TURN_UNKNOWN_COMMIT => {
            crate::conversation_turn::ConversationTurnLifecycleState::UnknownCommit
        }
        _ => {
            return Err(AirpError::Internal(format!(
                "unsupported terminal turn event kind: {event_kind}"
            )))
        }
    };
    let commit_state = turn_commit_state(&committed);
    let terminal = service
        .append_event_locked_async(
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: event_kind.to_string(),
                actor_id: None,
                causation_id: Some(causation_id),
                correlation_id: Some(turn_id.clone()),
                payload: serde_json::json!({
                    "code": code,
                    "participant_id": participant_id,
                    "commit_state": commit_state,
                    "observability": observability,
                }),
                extensions: BTreeMap::new(),
                expected_next_sequence: Some(next_sequence),
            },
        )
        .await?;
    committed.push(terminal);
    Ok(ConversationTurnOutcome {
        turn_id,
        status: ConversationTurnStatus::PartiallyCommitted,
        lifecycle_state,
        events: committed,
        next_sequence: next_sequence + 1,
        failure: Some(ConversationTurnFailure {
            schema_version:
                crate::conversation_observability::CONVERSATION_TURN_ERROR_SCHEMA_VERSION,
            code: code.to_string(),
            participant_id,
            recovery: crate::conversation_observability::recovery_for_code(code),
        }),
    })
}

fn turn_commit_state(events: &[crate::conversation::ConversationEvent]) -> &'static str {
    if events.iter().any(|event| event.kind == "message.created") {
        "partially_committed"
    } else {
        "no_message_committed"
    }
}

fn validate_turn_id(turn_id: &str) -> Result<(), AirpError> {
    if crate::ulid::is_valid_id(turn_id) {
        Ok(())
    } else {
        Err(AirpError::BadRequest(
            "turn_id must be a valid durable Engine ID".to_string(),
        ))
    }
}

async fn reconcile_unknown_commit(
    service: ConversationService,
    conversation_id: SessionId,
    snapshot: crate::conversation_turn::ConversationTurnSnapshot,
    expected_next_sequence: u64,
) -> Result<crate::conversation_turn::ConversationTurnSnapshot, AirpError> {
    let causation_id = snapshot
        .events
        .last()
        .map(|event| event.event_id.clone())
        .ok_or_else(|| AirpError::Internal("turn snapshot has no events".to_string()))?;
    let commit_state = turn_commit_state(&snapshot.events);
    let terminal = service
        .append_event_locked_async(
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: crate::conversation_turn::TURN_UNKNOWN_COMMIT.to_string(),
                actor_id: None,
                causation_id: Some(causation_id),
                correlation_id: Some(snapshot.turn_id.clone()),
                payload: serde_json::json!({
                    "code": "unknown_commit",
                    "commit_state": commit_state,
                    "recovery": "manual_retry_or_continue",
                }),
                extensions: BTreeMap::new(),
                expected_next_sequence: Some(expected_next_sequence),
            },
        )
        .await?;
    let mut events = snapshot.events;
    events.push(terminal);
    crate::conversation_turn::project_turn(&events, &snapshot.turn_id)?.ok_or_else(|| {
        AirpError::Internal("reconciled turn disappeared from its journal projection".to_string())
    })
}
