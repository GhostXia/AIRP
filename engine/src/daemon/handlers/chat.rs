//! Chat HTTP handlers — history / rollback / regen / completion.
//!
//! #155 PR4：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；SSE 流由 `chat_pipeline` 产出。
//!
//! 端点：
//! - `POST /v1/chat/history` — 读聊天历史（cursor 分页或 legacy 全量）
//! - `POST /v1/chat/rollback` — 回滚到指定 message_index 或 message_id
//! - `POST /v1/chat/regen` — 删除最后一条 assistant 消息以供重新生成
//! - `POST /v1/chat/completions` — SSE 流式补全（quota 前置检查）

use super::*;
use crate::chat_pipeline;
use crate::chat_store::ChatLog;
use crate::daemon::types::{ChatCompletionRequest, HistoryQuery, RegenRequest, RollbackRequest};
use crate::domain::ChatService;
use axum::response::Sse;
use std::convert::Infallible;

/// POST /v1/chat/history — get chat history for a character
pub async fn get_chat_history(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(query): Json<HistoryQuery>,
) -> Result<Json<serde_json::Value>, AirpError> {
    // #37 cursor 分页：传 limit/before 走窗口；不传 → 全量（向后兼容旧客户端）。
    if query.limit.is_some() || query.before.is_some() {
        let window = ChatService::new(&state.data_root).history_window(
            &query.character_id,
            query.session_id.as_ref(),
            query.limit,
            query.before.as_deref(),
        )?;
        return Ok(Json(serde_json::to_value(window)?));
    }
    // legacy 全量返回必须保留 ChatLog 的既有响应形状。
    let log = ChatService::new(&state.data_root)
        .history(&query.character_id, query.session_id.as_ref())?;
    Ok(Json(serde_json::to_value(log)?))
}

/// POST /v1/chat/rollback — rollback to a specific message index
pub async fn rollback_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<RollbackRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    // #37：message_id / message_index 二选一校验。
    if let Err(msg) = req.validate_rollback_target() {
        return Err(AirpError::BadRequest(msg));
    }
    let service = ChatService::new(&state.data_root);
    let (log, _) = match (req.message_index, req.message_id.as_deref()) {
        (Some(idx), None) => service.rollback(&req.character_id, req.session_id.as_ref(), idx)?,
        (None, Some(id)) => {
            service.rollback_to_id(&req.character_id, req.session_id.as_ref(), id)?
        }
        // validate_rollback_target 已挡住二义与都空，这里不可达。
        _ => {
            return Err(AirpError::BadRequest(
                "rollback target invariant violated".into(),
            ))
        }
    };
    Ok(Json(log))
}

/// POST /v1/chat/regen — delete last assistant message for regeneration
pub async fn regen_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<RegenRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    let log =
        ChatService::new(&state.data_root).regen(&req.character_id, req.session_id.as_ref())?;
    Ok(Json(log))
}

pub async fn chat_completion(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AirpError,
> {
    // DX-3: quota check (before any expensive work; resolves same effective_root as pipeline)
    let (quota_config, effective_root) = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        let quota = cfg.quota.clone();
        let root =
            crate::data_dir::resolve_effective_root(&state.data_root, payload.user_id.as_deref())?;
        (quota, root)
    };
    crate::quota::check_and_increment(&effective_root, &quota_config)?;

    let pipeline = chat_pipeline::prepare_pipeline(&payload, &state)?;
    Ok(Sse::new(chat_pipeline::build_sse_stream(pipeline)))
}
