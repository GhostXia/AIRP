//! Strictly authenticated, read-only Engine Surface projection endpoints.

use std::{convert::Infallible, sync::Arc, time::Duration};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{sse::Event, sse::KeepAlive, sse::Sse, IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    daemon::DaemonState,
    domain::{ChatService, StateService},
    error::AirpError,
    types::{CharacterId, SessionId, UserId},
    ui_surface::{
        SessionSurfaceProps, SurfaceCursor, SurfaceEvent, SurfaceMessage, SurfaceReplay,
        SurfaceScope,
    },
};

const SURFACE_POLL_INTERVAL: Duration = Duration::from_secs(1);
const MAX_WIDGET_PROPS_BYTES: usize = 196_608;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct SurfaceQuery {
    character_id: CharacterId,
    user_id: Option<UserId>,
}

#[derive(Debug, Serialize)]
struct SurfaceSnapshotResponse {
    cursor: String,
    snapshot: airp_state_protocol::SurfaceSnapshot,
}

pub(in crate::daemon) async fn get_surface_snapshot(
    State(state): State<Arc<DaemonState>>,
    Path(session_id): Path<SessionId>,
    Query(query): Query<SurfaceQuery>,
) -> Response {
    if !surface_auth_is_configured(&state) {
        return surface_auth_unavailable();
    }
    let effective_root = effective_root(&state, query.user_id.as_ref());
    let user_id = query.user_id.as_ref().map(ToString::to_string);
    match refresh_surface(
        state.clone(),
        effective_root.clone(),
        query.character_id.clone(),
        session_id,
        user_id,
    )
    .await
    {
        Ok(event) => match event.message {
            SurfaceMessage::Snapshot(snapshot) => Json(SurfaceSnapshotResponse {
                cursor: event.cursor.to_string(),
                snapshot,
            })
            .into_response(),
            SurfaceMessage::Patch(_) => AirpError::Internal(
                "current Surface projection unexpectedly returned a patch".to_string(),
            )
            .into_response(),
        },
        Err(error) => error.into_response(),
    }
}

pub(in crate::daemon) async fn get_surface_events(
    State(state): State<Arc<DaemonState>>,
    Path(session_id): Path<SessionId>,
    Query(query): Query<SurfaceQuery>,
    headers: HeaderMap,
) -> Response {
    if !surface_auth_is_configured(&state) {
        return surface_auth_unavailable();
    }
    let effective_root = effective_root(&state, query.user_id.as_ref());
    let user_id = query.user_id.as_ref().map(ToString::to_string);
    let current = match refresh_surface(
        state.clone(),
        effective_root.clone(),
        query.character_id.clone(),
        session_id,
        user_id.clone(),
    )
    .await
    {
        Ok(event) => event,
        Err(error) => return error.into_response(),
    };
    let scope = SurfaceScope::for_user(
        &effective_root,
        query.character_id.to_string(),
        session_id.to_string(),
        user_id.clone(),
    );
    let requested = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok())
        .map(SurfaceCursor::from_opaque);
    let initial = {
        let registry = state.ui_surfaces.lock().unwrap_or_else(|p| p.into_inner());
        match registry.replay(&scope, requested.as_ref()) {
            Ok(replay) => replay_events(replay),
            Err(crate::ui_surface::SurfaceRegistryError::UnknownScope) => {
                vec![current.clone()]
            }
            Err(error) => {
                return AirpError::Internal(format!("Surface replay failed: {error}"))
                    .into_response()
            }
        }
    };
    let mut last_cursor = initial
        .last()
        .map(|event| event.cursor.clone())
        .unwrap_or(current.cursor);
    let character_id = query.character_id;
    let mut shutdown = state.shutdown.subscribe();
    let stream = async_stream::stream! {
        if *shutdown.borrow() {
            return;
        }
        for event in initial {
            yield Ok::<Event, Infallible>(to_sse_event(&event));
        }
        let mut interval = tokio::time::interval(SURFACE_POLL_INTERVAL);
        interval.tick().await;
        loop {
            tokio::select! {
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                    continue;
                }
                _ = interval.tick() => {}
            }
            let refreshed = match refresh_surface(
                state.clone(),
                effective_root.clone(),
                character_id.clone(),
                session_id,
                user_id.clone(),
            ).await {
                Ok(event) => event,
                Err(error) => {
                    tracing::warn!(error = %error, "Surface projection polling stopped");
                    break;
                }
            };
            let replay = {
                let registry = state.ui_surfaces.lock().unwrap_or_else(|p| p.into_inner());
                registry.replay(&scope, Some(&last_cursor))
            };
            let events = match replay {
                Ok(replay) => replay_events(replay),
                Err(crate::ui_surface::SurfaceRegistryError::UnknownScope) => vec![refreshed],
                Err(error) => {
                    tracing::warn!(error = %error, "Surface replay polling stopped");
                    break;
                }
            };
            for event in events {
                last_cursor = event.cursor.clone();
                yield Ok::<Event, Infallible>(to_sse_event(&event));
            }
        }
    };

    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response()
}

async fn refresh_surface(
    state: Arc<DaemonState>,
    effective_root: std::path::PathBuf,
    character_id: CharacterId,
    session_id: SessionId,
    user_id: Option<String>,
) -> Result<SurfaceEvent, AirpError> {
    tokio::task::spawn_blocking(move || {
        refresh_surface_blocking(&state, &effective_root, &character_id, &session_id, user_id)
    })
    .await
    .map_err(|error| AirpError::Internal(format!("Surface projection task failed: {error}")))?
}

fn refresh_surface_blocking(
    state: &DaemonState,
    effective_root: &std::path::Path,
    character_id: &CharacterId,
    session_id: &SessionId,
    user_id: Option<String>,
) -> Result<SurfaceEvent, AirpError> {
    let props = project_session(state, effective_root, character_id, session_id)?;
    let scope = SurfaceScope::for_user(
        effective_root,
        character_id.to_string(),
        session_id.to_string(),
        user_id,
    );
    let mut registry = state.ui_surfaces.lock().unwrap_or_else(|p| p.into_inner());
    registry
        .publish(scope.clone(), props)
        .map_err(|error| AirpError::Internal(format!("Surface publish failed: {error}")))?;
    registry
        .current(&scope)
        .map_err(|error| AirpError::Internal(format!("Surface snapshot failed: {error}")))
}

fn project_session(
    state: &DaemonState,
    effective_root: &std::path::Path,
    character_id: &CharacterId,
    session_id: &SessionId,
) -> Result<SessionSurfaceProps, AirpError> {
    let session_dir = crate::data_dir::resolve_session_dir_read_only(
        effective_root,
        character_id.as_str(),
        Some(session_id),
    )?
    .ok_or_else(|| {
        AirpError::NotFound(format!("session {session_id} for character {character_id}"))
    })?;
    let mut chat = ChatService::new(effective_root)
        .history_window_read_only(character_id, Some(session_id), Some(50), None)?
        .map(serde_json::to_value)
        .transpose()?
        .unwrap_or_else(|| json!({"messages": [], "message_ids": [], "total": 0}));
    if let Some(chat) = chat.as_object_mut() {
        chat.insert(
            "context".into(),
            json!({
                "character_id": character_id,
                "session_id": session_id,
            }),
        );
    }
    let memory = json!({
        "content": crate::memory::read_resident_memory(&session_dir)?,
        "capacity_chars": crate::memory::ResidentMemoryConfig::default().capacity_chars,
    });
    let character_state = StateService::new(effective_root).read(character_id)?;
    let live_activity = serde_json::to_value(state.session_coordinators.status(
        effective_root,
        character_id,
        Some(session_id),
    ))?;
    let activity = match crate::ui_activity::read_window(&session_dir) {
        Ok(recent_failures) => json!({
            "live": live_activity,
            "recent_failures": recent_failures,
        }),
        Err(error) => {
            tracing::warn!(%error, "UI activity receipts are unavailable");
            json!({
                "live": live_activity,
                "recent_failures": {"unavailable": true},
            })
        }
    };
    Ok(SessionSurfaceProps {
        chat: bounded_props(chat),
        memory: bounded_props(memory),
        character_state: bounded_props(character_state),
        activity: bounded_props(activity),
    })
}

fn effective_root(state: &DaemonState, user_id: Option<&UserId>) -> std::path::PathBuf {
    match user_id {
        Some(user_id) => state.data_root.join("users").join(user_id.as_str()),
        None => state.data_root.clone(),
    }
}

fn bounded_props(value: Value) -> Value {
    match serde_json::to_vec(&value) {
        Ok(encoded) if encoded.len() <= MAX_WIDGET_PROPS_BYTES => value,
        Ok(encoded) => json!({"truncated": true, "original_bytes": encoded.len()}),
        Err(_) => json!({"unavailable": true}),
    }
}

fn replay_events(replay: SurfaceReplay) -> Vec<SurfaceEvent> {
    match replay {
        SurfaceReplay::Snapshot(event) => vec![event],
        SurfaceReplay::Events(events) => events,
    }
}

fn to_sse_event(event: &SurfaceEvent) -> Event {
    let base = Event::default().id(event.cursor.as_str());
    match &event.message {
        SurfaceMessage::Snapshot(snapshot) => base
            .event("snapshot")
            .json_data(snapshot)
            .unwrap_or_else(|_| Event::default().event("error").data("serialization_failed")),
        SurfaceMessage::Patch(patch) => base
            .event("patch")
            .json_data(patch)
            .unwrap_or_else(|_| Event::default().event("error").data("serialization_failed")),
    }
}

fn surface_auth_is_configured(state: &DaemonState) -> bool {
    state
        .read_config()
        .access_api_key
        .as_deref()
        .is_some_and(|key| !key.is_empty())
}

fn surface_auth_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({
            "error": {
                "code": "surface_auth_unavailable",
                "message": "Surface API requires daemon bearer authentication",
                "recovery": "configure_access_key"
            }
        })),
    )
        .into_response()
}
