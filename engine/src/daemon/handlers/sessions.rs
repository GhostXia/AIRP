//! Session HTTP handlers — list / create / delete named sessions.
//!
//! #155 PR4：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；domain 逻辑在 `ChatService`。
//!
//! 端点：
//! - `GET    /v1/sessions/:character_id` — 列出该角色所有命名会话
//! - `POST   /v1/sessions/:character_id` — 创建新命名会话，返回 session id
//! - `DELETE /v1/sessions/:character_id/:session_id` — 删除命名会话目录

use super::DaemonState;
use crate::domain::ChatService;
use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};
use axum::Json;
use std::sync::Arc;

/// GET /v1/sessions/:character_id — list all named sessions for a character.
pub(in crate::daemon) async fn list_sessions_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<Vec<SessionId>>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sessions = ChatService::new(&state.data_root).list_sessions(&cid)?;
    Ok(Json(sessions))
}

/// POST /v1/sessions/:character_id — create a new named session, return its ID.
pub(in crate::daemon) async fn create_session_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<SessionId>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sid = ChatService::new(&state.data_root).create_session(&cid)?;
    Ok(Json(sid))
}

/// DELETE /v1/sessions/:character_id/:session_id — 删除一个命名会话目录。
/// #35：destructive，调用方负责确认。返回 `{deleted, status}`。会话不存在 → 404。
pub(in crate::daemon) async fn delete_session_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((character_id, session_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sid = SessionId::parse(&session_id)?;
    ChatService::new(&state.data_root).delete_session(&cid, &sid)?;
    Ok(Json(serde_json::json!({
        "deleted": sid.to_string(),
        "character_id": cid.as_str(),
        "status": "ok"
    })))
}
