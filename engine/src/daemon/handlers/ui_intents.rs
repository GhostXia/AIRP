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
    error::AirpError,
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
    if target.widget_type != "core.chat" {
        return Err(AirpError::BadRequest(format!(
            "widget type {} has no first-party intent executor",
            target.widget_type
        )));
    }

    let character_id = CharacterId::new(target.scope.character_id().to_owned())?;
    let session_id = SessionId::parse(target.scope.session_id())?;
    let user_id = target.scope.user_id().map(ToOwned::to_owned);
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, user_id.as_deref())?;
    let session_exists = tokio::task::spawn_blocking({
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
    .is_some();
    if !session_exists {
        return Err(AirpError::NotFound(format!(
            "session {session_id} for character {character_id}"
        )));
    }
    match request.name.as_str() {
        "chat.send" => {
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
        "chat.regen" => regen_chat(
            State(state),
            Json(RegenRequest {
                character_id,
                session_id: Some(session_id),
                user_id,
            }),
        )
        .await
        .map(IntoResponse::into_response),
        "chat.continue" => continue_chat(
            State(state),
            Json(ContinueRequest {
                character_id,
                session_id: Some(session_id),
                user_id,
            }),
        )
        .await
        .map(IntoResponse::into_response),
        "chat.stop" => {
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
        "chat.swipe" => {
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
        "chat.loadMore" => {
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
        _ => Err(AirpError::BadRequest(format!(
            "unsupported core.chat intent: {}",
            request.name
        ))),
    }
}

fn decode_params<T: for<'de> Deserialize<'de>>(params: Option<Value>) -> Result<T, AirpError> {
    serde_json::from_value(params.unwrap_or_else(|| Value::Object(Default::default())))
        .map_err(|error| AirpError::BadRequest(format!("invalid intent params: {error}")))
}
