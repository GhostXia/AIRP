//! Session HTTP handlers — list / create / delete named sessions.
//!
//! #155 PR4：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；domain 逻辑在 `ChatService`。
//!
//! 端点：
//! - `GET    /v1/sessions/:character_id` — 列出该角色所有命名会话
//! - `POST   /v1/sessions/:character_id` — 创建新命名会话，返回 session id
//! - `DELETE /v1/sessions/:character_id/:session_id` — 删除命名会话目录

use crate::daemon::DaemonState;
use crate::domain::ChatService;
use crate::error::AirpError;
use crate::types::{CharacterId, SessionId, UserId};
use axum::{extract::Query, Json};
use serde::Deserialize;
use std::sync::Arc;

/// GET /v1/sessions/:character_id — list all named sessions for a character.
///
/// `ChatService::list_sessions` 是同步文件 IO；在 async handler 中用
/// `spawn_blocking` 包装避免阻塞 tokio worker 线程（#433）。
pub(in crate::daemon) async fn list_sessions_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    Query(query): Query<SessionScopeQuery>,
) -> Result<Json<Vec<SessionId>>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let data_root = query.user_id.map_or_else(
        || state.data_root.clone(),
        |user_id| state.data_root.join("users").join(user_id.as_str()),
    );
    let sessions =
        tokio::task::spawn_blocking(move || ChatService::new(&data_root).list_sessions(&cid))
            .await
            .map_err(|e| AirpError::Internal(format!("list_sessions join failed: {e}")))??;
    Ok(Json(sessions))
}

/// POST /v1/sessions/:character_id — create a new named session, return its ID.
///
/// `ChatService::create_session` 是同步文件 IO；在 async handler 中用
/// `spawn_blocking` 包装避免阻塞 tokio worker 线程（#433）。
#[derive(Debug, Deserialize)]
pub(in crate::daemon) struct SessionScopeQuery {
    user_id: Option<UserId>,
}

#[derive(Debug, Deserialize)]
pub(in crate::daemon) struct CreateSessionQuery {
    session_id: Option<String>,
    user_id: Option<UserId>,
}

#[derive(Debug, Deserialize)]
pub(in crate::daemon) struct DeleteSessionQuery {
    #[serde(default)]
    force: bool,
    user_id: Option<UserId>,
}

pub(in crate::daemon) async fn create_session_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    Query(query): Query<CreateSessionQuery>,
) -> Result<Json<SessionId>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let data_root = query.user_id.map_or_else(
        || state.data_root.clone(),
        |user_id| state.data_root.join("users").join(user_id.as_str()),
    );
    let requested_sid = query
        .session_id
        .map(|sid| SessionId::parse(&sid))
        .transpose()?;
    let sid = tokio::task::spawn_blocking(move || {
        let service = ChatService::new(&data_root);
        match requested_sid {
            Some(sid) => {
                service.create_session_with_id(&cid, &sid)?;
                Ok(sid)
            }
            None => service.create_session(&cid),
        }
    })
    .await
    .map_err(|e| AirpError::Internal(format!("create_session join failed: {e}")))??;
    Ok(Json(sid))
}

/// DELETE /v1/sessions/:character_id/:session_id — 删除一个命名会话目录。
/// #35：destructive，调用方负责确认。返回 `{deleted, character_id, status, pre_delete_backup_id?}`。
/// 会话不存在 → 404。
///
/// #342 E-P2-1：默认创建 `PreDelete` scoped backup，让删除可恢复。
/// `?force=true` 跳过 pre-delete backup（advanced / testing）。
///
/// `delete_session` 内部执行同步文件 IO（pre-delete backup + `remove_dir_all`），
/// 整段搬到 `spawn_blocking` 避免阻塞 tokio worker 线程。参考
/// `delete_character_endpoint` 与 `restore_backup_endpoint` 的同模式。
pub(in crate::daemon) async fn delete_session_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((character_id, session_id)): axum::extract::Path<(String, String)>,
    axum::extract::Query(params): axum::extract::Query<DeleteSessionQuery>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let sid = SessionId::parse(&session_id)?;
    let data_root = params.user_id.map_or_else(
        || state.data_root.clone(),
        |user_id| state.data_root.join("users").join(user_id.as_str()),
    );
    let force = params.force;
    let cid_str = cid.as_str().to_string();
    let sid_str = sid.to_string();
    let backup_id = tokio::task::spawn_blocking(move || {
        ChatService::new(&data_root).delete_session(&cid, &sid, force)
    })
    .await
    .map_err(|e| AirpError::Internal(format!("delete_session join failed: {e}")))??;
    Ok(Json(serde_json::json!({
        "deleted": sid_str,
        "character_id": cid_str,
        "status": "ok",
        "pre_delete_backup_id": backup_id,
    })))
}
