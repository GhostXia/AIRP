//! Memory handlers: 记忆可见性 API（2.4）。
//!
//! - GET /v1/memory/resident - 读取 resident memory
//! - PUT /v1/memory/resident - 更新 resident memory
//! - GET /v1/memory/user-model - 读取用户模型
//! - PUT /v1/memory/user-model - 更新用户模型

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};

// ── Request/Response types ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ResidentMemoryQuery {
    pub character_id: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ResidentMemoryResponse {
    pub content: String,
    pub char_count: usize,
    pub capacity: usize,
}

#[derive(Debug, Deserialize)]
pub struct UpdateResidentMemoryRequest {
    pub character_id: String,
    pub session_id: Option<String>,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct UserModelQuery {
    pub user_id: String,
}

#[derive(Debug, Serialize)]
pub struct UserModelResponse {
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateUserModelRequest {
    pub user_id: String,
    pub content: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /v1/memory/resident?character_id=...&session_id=...
pub async fn get_resident_memory(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Query(query): axum::extract::Query<ResidentMemoryQuery>,
) -> impl IntoResponse {
    let result = (|| -> Result<ResidentMemoryResponse, AirpError> {
        let cid = CharacterId::new(&query.character_id)?;
        let sid = query
            .session_id
            .as_ref()
            .map(|s| SessionId::parse(s))
            .transpose()?;

        let session_dir =
            crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

        let content = crate::memory::read_resident_memory(&session_dir)?;
        let config = crate::memory::ResidentMemoryConfig::default();

        Ok(ResidentMemoryResponse {
            char_count: content.chars().count(),
            content,
            capacity: config.capacity_chars,
        })
    })();

    match result {
        Ok(resp) => (StatusCode::OK, Json(serde_json::to_value(resp).unwrap())).into_response(),
        Err(e) => e.into_response(),
    }
}

/// PUT /v1/memory/resident
pub async fn update_resident_memory(
    State(state): State<Arc<DaemonState>>,
    Json(payload): Json<UpdateResidentMemoryRequest>,
) -> impl IntoResponse {
    let result = (|| -> Result<(), AirpError> {
        let cid = CharacterId::new(&payload.character_id)?;
        let sid = payload
            .session_id
            .as_ref()
            .map(|s| SessionId::parse(s))
            .transpose()?;

        let session_dir =
            crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

        crate::memory::write_resident_memory(&session_dir, &payload.content)
    })();

    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "success": true })),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}

/// GET /v1/memory/user-model?user_id=...
pub async fn get_user_model(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Query(query): axum::extract::Query<UserModelQuery>,
) -> impl IntoResponse {
    let content = crate::memory::read_user_model(&state.data_root, &query.user_id)
        .unwrap_or_default();

    (
        StatusCode::OK,
        Json(serde_json::to_value(UserModelResponse { content }).unwrap()),
    )
}

/// PUT /v1/memory/user-model
pub async fn update_user_model(
    State(state): State<Arc<DaemonState>>,
    Json(payload): Json<UpdateUserModelRequest>,
) -> impl IntoResponse {
    let result = crate::memory::write_user_model(&state.data_root, &payload.user_id, &payload.content);

    match result {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "success": true })),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}
