//! Trusted execution boundary for first-party Engine Surface widget intents.

use std::{collections::HashMap, sync::Arc};

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use serde_json::Value;

use crate::{
    daemon::{
        handlers::{chat_completion, continue_chat, get_chat_history, regen_chat, swipe_chat},
        types::{
            ChatCompletionRequest, ContinueRequest, HistoryQuery, RegenRequest, SwipeRequest,
            UserProfile,
        },
        DaemonState,
    },
    domain::{StateService, WorkspaceService},
    error::AirpError,
    session_coordinator::SessionCommand,
    types::{CharacterId, SessionId},
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct UiIntentRequest {
    surface_id: String,
    instance_id: String,
    name: String,
    params: Option<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SendParams {
    text: String,
    #[serde(default)]
    user_profile: Option<UserProfile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SwipeParams {
    message_id: String,
    index: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryParams {
    before: Option<String>,
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryReplaceParams {
    content: String,
    expected_content_hash: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterStatePatchParams {
    expected_revision: u64,
    patch: Vec<Value>,
}

pub(in crate::daemon) async fn dispatch_ui_intent(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<UiIntentRequest>,
) -> Response {
    if state
        .read_config()
        .access_api_key
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": {
                    "code": "ui_intent_auth_unavailable",
                    "message": "UI intent API requires daemon bearer authentication",
                    "recovery": "configure_access_key"
                }
            })),
        )
            .into_response();
    }
    match dispatch(state, request).await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    }
}

async fn dispatch(
    state: Arc<DaemonState>,
    request: UiIntentRequest,
) -> Result<Response, AirpError> {
    if request.surface_id.is_empty() || request.instance_id.is_empty() || request.name.is_empty() {
        return Err(AirpError::BadRequest(
            "surface_id, instance_id and name must not be empty".into(),
        ));
    }
    let target = {
        let registry = state
            .ui_surfaces
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        registry
            .resolve_intent_target(&request.surface_id, &request.instance_id)
            .map_err(|error| AirpError::BadRequest(format!("intent target rejected: {error}")))?
    };
    validate_intent_name(&target.widget_type, &request.name)?;

    let character_id = CharacterId::new(target.scope.character_id().to_owned())?;
    let session_id = SessionId::parse(target.scope.session_id())?;
    let user_id = target.scope.user_id().map(ToOwned::to_owned);
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, user_id.as_deref())?;
    let accepted_workspace_revision = target.workspace_revision.ok_or_else(|| {
        AirpError::BadRequest("intent target is not backed by a Workspace revision".into())
    })?;
    let workspace = tokio::task::spawn_blocking({
        let effective_root = effective_root.clone();
        move || WorkspaceService::new(effective_root).read()
    })
    .await
    .map_err(|error| AirpError::Internal(format!("intent Workspace check failed: {error}")))??;
    if workspace.revision != accepted_workspace_revision {
        return Err(AirpError::Conflict(
            "accepted Surface no longer matches the current Workspace revision".into(),
        ));
    }
    let workspace_widget = workspace
        .layout
        .widgets
        .iter()
        .find(|widget| widget.id == request.instance_id)
        .ok_or_else(|| {
            AirpError::Conflict(
                "intent Widget is no longer present in the current Workspace".into(),
            )
        })?;
    if workspace_widget.widget_type != target.widget_type {
        return Err(AirpError::Conflict(
            "intent Widget type no longer matches the current Workspace".into(),
        ));
    }
    let session_dir = tokio::task::spawn_blocking({
        let effective_root = effective_root.clone();
        let character_id = character_id.clone();
        move || {
            crate::data_dir::resolve_session_dir_read_only(
                &effective_root,
                character_id.as_str(),
                Some(&session_id),
            )
        }
    })
    .await
    .map_err(|error| AirpError::Internal(format!("intent scope check failed: {error}")))??
    .ok_or_else(|| {
        AirpError::NotFound(format!("session {session_id} for character {character_id}"))
    })?;
    match (target.widget_type.as_str(), request.name.as_str()) {
        ("core.chat", "chat.send") => {
            let params: SendParams = decode_params(request.params)?;
            if params.text.trim().is_empty() {
                return Err(AirpError::BadRequest(
                    "message text must not be empty".into(),
                ));
            }
            let payload = ChatCompletionRequest {
                character_id: Some(character_id),
                character_card_id: None,
                lorebook_path: None,
                user_profile: params.user_profile.unwrap_or(UserProfile {
                    name: "User".into(),
                    variables: HashMap::new(),
                }),
                message: params.text,
                messages_history: None,
                regex_filters: None,
                preset_id: None,
                enabled_presets: None,
                session_id: Some(session_id),
                provider: None,
                endpoint: None,
                api_key: None,
                model: None,
                temperature: None,
                max_tokens: None,
                scene_id: None,
                user_id,
                persona_id: None,
                swipe_candidates: Vec::new(),
                branch_from: None,
            };
            chat_completion(State(state), Json(payload))
                .await
                .map(IntoResponse::into_response)
        }
        ("core.chat", "chat.regen") => regen_chat(
            State(state),
            Json(RegenRequest {
                character_id,
                session_id: Some(session_id),
                user_id,
            }),
        )
        .await
        .map(IntoResponse::into_response),
        ("core.chat", "chat.continue") => continue_chat(
            State(state),
            Json(ContinueRequest {
                character_id,
                session_id: Some(session_id),
                user_id,
            }),
        )
        .await
        .map(IntoResponse::into_response),
        ("core.chat", "chat.stop") => {
            let status = state.session_coordinators.status(
                &effective_root,
                &character_id,
                Some(&session_id),
            );
            let generation_id = status
                .generation_id
                .ok_or_else(|| AirpError::Conflict("session has no active generation".into()))?;
            let cancelled = state.session_coordinators.cancel_generation(
                &effective_root,
                &character_id,
                Some(&session_id),
                &generation_id,
            )?;
            Ok(Json(cancelled).into_response())
        }
        ("core.chat", "chat.swipe") => {
            let params: SwipeParams = decode_params(request.params)?;
            swipe_chat(
                State(state),
                Json(SwipeRequest {
                    character_id,
                    session_id: Some(session_id),
                    message_id: params.message_id,
                    index: params.index,
                    user_id,
                }),
            )
            .await
            .map(IntoResponse::into_response)
        }
        ("core.chat", "chat.loadMore") => {
            let params: HistoryParams = decode_params(request.params)?;
            get_chat_history(
                State(state),
                Json(HistoryQuery {
                    character_id,
                    session_id: Some(session_id),
                    limit: Some(params.limit.unwrap_or(50)),
                    before: params.before,
                    user_id,
                }),
            )
            .await
            .map(IntoResponse::into_response)
        }
        ("core.memory", "memory.replace") => {
            let params: MemoryReplaceParams = decode_params(request.params)?;
            validate_sha256(&params.expected_content_hash)?;
            let capacity = crate::memory::ResidentMemoryConfig::default();
            let capacity_chars = capacity.capacity_chars;
            let content = params.content;
            let expected_content_hash = params.expected_content_hash;
            tokio::task::spawn_blocking({
                let session_dir = session_dir.clone();
                let content = content.clone();
                move || {
                    crate::memory::replace_resident_memory(
                        &session_dir,
                        &content,
                        &expected_content_hash,
                        &capacity,
                    )
                }
            })
            .await
            .map_err(|error| {
                AirpError::Internal(format!("memory replace task failed: {error}"))
            })??;
            Ok(Json(serde_json::json!({
                "content": content,
                "content_hash": crate::memory::resident_memory_content_hash(&content),
                "char_count": content.chars().count(),
                "capacity_chars": capacity_chars,
                "source": {
                    "kind": "resident_memory",
                    "scope": "session",
                    "character_id": character_id,
                    "session_id": session_id,
                }
            }))
            .into_response())
        }
        ("core.character-state", "characterState.patch") => {
            let params: CharacterStatePatchParams = decode_params(request.params)?;
            let patched = tokio::task::spawn_blocking({
                let state = state.clone();
                let effective_root = effective_root.clone();
                let character_id = character_id.clone();
                move || {
                    let _lease = state.session_coordinators.try_submit(
                        &effective_root,
                        &character_id,
                        Some(&session_id),
                        SessionCommand::AgentToolMutation,
                    )?;
                    StateService::new(&effective_root).patch(
                        &character_id,
                        params.expected_revision,
                        &params.patch,
                    )
                }
            })
            .await
            .map_err(|error| AirpError::Internal(format!("state patch task failed: {error}")))??;
            Ok(Json(serde_json::json!({
                "revision": patched.revision,
                "timestamp": patched.timestamp,
                "state": patched.state,
                "source": {
                    "kind": "character_state",
                    "scope": "character",
                    "character_id": character_id,
                }
            }))
            .into_response())
        }
        _ => Err(AirpError::Internal(
            "validated UI intent did not match its executor".to_string(),
        )),
    }
}

fn validate_intent_name(widget_type: &str, name: &str) -> Result<(), AirpError> {
    let accepted = match widget_type {
        "core.chat" => matches!(
            name,
            "chat.send"
                | "chat.regen"
                | "chat.continue"
                | "chat.stop"
                | "chat.swipe"
                | "chat.loadMore"
        ),
        "core.memory" => name == "memory.replace",
        "core.character-state" => name == "characterState.patch",
        _ => false,
    };
    if accepted {
        Ok(())
    } else {
        Err(AirpError::BadRequest(format!(
            "intent {name} is not accepted by widget type {widget_type}"
        )))
    }
}

fn decode_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> Result<T, AirpError> {
    serde_json::from_value(params.unwrap_or_else(|| Value::Object(Default::default())))
        .map_err(|error| AirpError::BadRequest(format!("invalid intent params: {error}")))
}

fn validate_sha256(value: &str) -> Result<(), AirpError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(AirpError::BadRequest(
            "expected_content_hash must be a lowercase SHA-256 digest".to_string(),
        ))
    }
}
