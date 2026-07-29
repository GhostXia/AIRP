//! UI-independent Conversation HTTP API.

use crate::conversation::{
    effective_conversation_root, AppendConversationEventRequest, ConversationEventsQuery,
    ConversationScopeQuery, ConversationService, ConversationTurnFailure, ConversationTurnOutcome,
    ConversationTurnRequest, ConversationTurnStatus, CreateConversationRequest,
    CreateSceneConversationRequest,
};
use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::types::SessionId;
use axum::{extract::Query, Json};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const CONVERSATION_TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// Create a generic Engine-owned Conversation manifest.
pub(in crate::daemon) async fn create_conversation_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Json<crate::conversation::ConversationManifest>, AirpError> {
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    Ok(Json(ConversationService::new(root).create(request)?))
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
    Ok(Json(ConversationService::new(root).create(create)?))
}

/// List readable Conversation manifests within the selected user scope.
pub(in crate::daemon) async fn list_conversations_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Query(query): Query<ConversationScopeQuery>,
) -> Result<Json<Vec<crate::conversation::ConversationManifest>>, AirpError> {
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    Ok(Json(ConversationService::new(root).list()?))
}

/// Load one Conversation manifest from the selected user scope.
pub(in crate::daemon) async fn get_conversation_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
    Query(query): Query<ConversationScopeQuery>,
) -> Result<Json<crate::conversation::ConversationManifest>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    let root = effective_conversation_root(&state.data_root, query.user_id.as_ref())?;
    Ok(Json(ConversationService::new(root).get(conversation_id)?))
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
    Ok(Json(ConversationService::new(root).events(
        conversation_id,
        query.limit,
        query.before.as_deref(),
    )?))
}

/// Discover registered Engine policy descriptors.
pub(in crate::daemon) async fn list_conversation_policies_endpoint(
) -> Json<Vec<crate::conversation_policy::ConversationPolicyDescriptor>> {
    Json(crate::conversation_policy::builtin_conversation_policy_registry().list())
}

/// Execute and durably record one bounded Engine-owned Conversation turn.
pub(in crate::daemon) async fn execute_conversation_turn_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(conversation_id): axum::extract::Path<String>,
    Json(request): Json<ConversationTurnRequest>,
) -> Result<Json<ConversationTurnOutcome>, AirpError> {
    let conversation_id = SessionId::parse(&conversation_id)?;
    validate_turn_base(&request)?;
    let root = effective_conversation_root(&state.data_root, request.user_id.as_ref())?;
    let service = ConversationService::new(&root);
    let _write_guard = service.acquire_write(conversation_id).await;
    let manifest = service.get(conversation_id)?;
    let plan = crate::conversation_policy::builtin_conversation_policy_registry()
        .plan_turn(&manifest, &request.user_actor_id)?;
    if plan.speakers.len() > crate::conversation_policy::MAX_CONVERSATION_SPEAKERS_PER_TURN {
        return Err(AirpError::BadRequest(format!(
            "conversation turn planned {} speakers; maximum is {}",
            plan.speakers.len(),
            crate::conversation_policy::MAX_CONVERSATION_SPEAKERS_PER_TURN
        )));
    }
    let provider_call_count = u32::try_from(plan.speakers.len()).map_err(|_| {
        AirpError::BadRequest("conversation speaker count is too large".to_string())
    })?;
    let quota = state
        .config
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .quota
        .clone();
    crate::quota::check_and_increment_by(&root, &quota, provider_call_count)?;

    let turn_id = crate::ulid::new_id();
    let history_service = service.clone();
    let history_manifest = manifest.clone();
    let mut history = tokio::task::spawn_blocking(move || {
        let events = history_service.all_events(conversation_id)?;
        Ok::<_, AirpError>(
            crate::conversation_projection::project_conversation_messages(
                &history_manifest,
                &events,
            )
            .into_chat_messages(),
        )
    })
    .await
    .map_err(|error| AirpError::Internal(format!("conversation history task failed: {error}")))??;
    let mut committed = Vec::new();
    let user_event = append_event_locked_blocking(
        service.clone(),
        conversation_id,
        AppendConversationEventRequest {
            user_id: None,
            kind: "message.created".to_string(),
            actor_id: Some(request.user_actor_id.clone()),
            causation_id: None,
            correlation_id: Some(turn_id.clone()),
            payload: serde_json::json!({
                "role": "user",
                "content": request.base.message,
            }),
            extensions: request.extensions.clone(),
            expected_next_sequence: Some(request.expected_next_sequence),
        },
    )
    .await?;
    history.push(crate::adapter::ChatMessage {
        role: crate::adapter::MessageRole::User,
        content: request.base.message.clone(),
    });
    let mut next_sequence = user_event.sequence + 1;
    let user_event_id = user_event.event_id.clone();
    committed.push(user_event);
    let turn_deadline = tokio::time::Instant::now() + CONVERSATION_TURN_TIMEOUT;

    for speaker in plan.speakers {
        let participant_id = speaker.participant_id;
        let character_id = speaker.resource_id;
        let mut payload = request.base.clone();
        payload.user_id = request.user_id.as_ref().map(ToString::to_string);
        payload.scene_id = Some(plan.scene_id.clone());
        payload.messages_history = None;
        let pipeline = match crate::chat_pipeline::prepare_scene_participant_pipeline(
            &payload,
            &state,
            &character_id,
            history.clone(),
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
                return Ok(Json(
                    commit_turn_failure(
                        service.clone(),
                        conversation_id,
                        turn_id,
                        Some(participant_id),
                        "generation_preparation_failed",
                        user_event_id,
                        next_sequence,
                        committed,
                    )
                    .await?,
                ));
            }
        };
        let result = match tokio::time::timeout_at(
            turn_deadline,
            crate::chat_pipeline::run_generation_step(pipeline),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => {
                tracing::error!(
                    %conversation_id,
                    %turn_id,
                    %participant_id,
                    timeout_secs = CONVERSATION_TURN_TIMEOUT.as_secs(),
                    "conversation turn timed out after user event commit"
                );
                return Ok(Json(
                    commit_turn_failure(
                        service.clone(),
                        conversation_id,
                        turn_id,
                        Some(participant_id),
                        "turn_timeout",
                        user_event_id,
                        next_sequence,
                        committed,
                    )
                    .await?,
                ));
            }
        };
        crate::quota::record_tokens(
            &root,
            crate::volume_store::estimate_tokens(&result.raw_acc).min(u32::MAX as usize) as u32,
        );
        if let Some(error) = result.error {
            tracing::error!(
                %conversation_id,
                %turn_id,
                %participant_id,
                %error,
                "conversation participant generation failed after user event commit"
            );
            return Ok(Json(
                commit_turn_failure(
                    service.clone(),
                    conversation_id,
                    turn_id,
                    Some(participant_id),
                    "generation_failed",
                    user_event_id,
                    next_sequence,
                    committed,
                )
                .await?,
            ));
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
            return Ok(Json(
                commit_turn_failure(
                    service.clone(),
                    conversation_id,
                    turn_id,
                    Some(participant_id),
                    "empty_generation",
                    user_event_id,
                    next_sequence,
                    committed,
                )
                .await?,
            ));
        }
        let mut unpacker = crate::xml_unpacker::StreamingXmlUnpacker::new();
        let mut chunks = unpacker.process_chunk(&content);
        chunks.extend(unpacker.finish());
        let chunks = serde_json::to_value(chunks)?;
        let assistant_event = append_event_locked_blocking(
            service.clone(),
            conversation_id,
            AppendConversationEventRequest {
                user_id: None,
                kind: "message.created".to_string(),
                actor_id: Some(participant_id.clone()),
                causation_id: Some(user_event_id.clone()),
                correlation_id: Some(turn_id.clone()),
                payload: serde_json::json!({
                    "role": "assistant",
                    "content": content,
                    "resource": {"kind": "character", "id": character_id},
                    "chunks": chunks,
                    "state": live_state,
                }),
                extensions: BTreeMap::new(),
                expected_next_sequence: Some(next_sequence),
            },
        )
        .await?;
        next_sequence += 1;
        history.push(crate::adapter::ChatMessage {
            role: crate::adapter::MessageRole::Assistant,
            content: format!("[{participant_id}] {content}"),
        });
        committed.push(assistant_event);
    }

    let completed = append_event_locked_blocking(
        service.clone(),
        conversation_id,
        AppendConversationEventRequest {
            user_id: None,
            kind: "turn.completed".to_string(),
            actor_id: None,
            causation_id: Some(user_event_id),
            correlation_id: Some(turn_id.clone()),
            payload: serde_json::json!({
                "message_count": committed.len(),
            }),
            extensions: BTreeMap::new(),
            expected_next_sequence: Some(next_sequence),
        },
    )
    .await?;
    next_sequence += 1;
    committed.push(completed);
    Ok(Json(ConversationTurnOutcome {
        turn_id,
        status: ConversationTurnStatus::Completed,
        events: committed,
        next_sequence,
        failure: None,
    }))
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
async fn commit_turn_failure(
    service: ConversationService,
    conversation_id: SessionId,
    turn_id: String,
    participant_id: Option<String>,
    code: &str,
    causation_id: String,
    next_sequence: u64,
    mut committed: Vec<crate::conversation::ConversationEvent>,
) -> Result<ConversationTurnOutcome, AirpError> {
    let failed = append_event_locked_blocking(
        service,
        conversation_id,
        AppendConversationEventRequest {
            user_id: None,
            kind: "turn.failed".to_string(),
            actor_id: None,
            causation_id: Some(causation_id),
            correlation_id: Some(turn_id.clone()),
            payload: serde_json::json!({
                "code": code,
                "participant_id": participant_id,
                "commit_state": "partially_committed",
            }),
            extensions: BTreeMap::new(),
            expected_next_sequence: Some(next_sequence),
        },
    )
    .await?;
    committed.push(failed);
    Ok(ConversationTurnOutcome {
        turn_id,
        status: ConversationTurnStatus::PartiallyCommitted,
        events: committed,
        next_sequence: next_sequence + 1,
        failure: Some(ConversationTurnFailure {
            code: code.to_string(),
            participant_id,
        }),
    })
}

async fn append_event_locked_blocking(
    service: ConversationService,
    conversation_id: SessionId,
    request: AppendConversationEventRequest,
) -> Result<crate::conversation::ConversationEvent, AirpError> {
    tokio::task::spawn_blocking(move || service.append_event_locked(conversation_id, request))
        .await
        .map_err(|error| {
            AirpError::Internal(format!("conversation journal task failed: {error}"))
        })?
}
