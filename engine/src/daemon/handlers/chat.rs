//! Chat HTTP handlers — history / rollback / regen / continue / delete / completion.
//!
//! #155 PR4：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；SSE 流由 `chat_pipeline` 产出。
//!
//! 端点：
//! - `POST /v1/chat/history` — 读聊天历史（cursor 分页或 legacy 全量）
//! - `POST /v1/chat/rollback` — 回滚到指定 message_index 或 message_id
//! - `POST /v1/chat/regen` — 删除最后一条 assistant 消息并流式生成新响应 (SSE)
//! - `POST /v1/chat/continue` — 继续生成，追加到最后一条 assistant 消息 (SSE)
//! - `POST /v1/chat/delete` — 删除单条消息
//! - `POST /v1/chat/completions` — SSE 流式补全（quota 前置检查）

use crate::chat_pipeline;
use crate::chat_store::ChatLog;
use crate::daemon::types::{
    ChatCompletionRequest, ContinueRequest, DeleteMessageRequest, HistoryQuery, RegenRequest,
    RollbackRequest, SwipeRequest,
};
use crate::daemon::DaemonState;
use crate::domain::ChatService;
use crate::error::AirpError;
use axum::{response::Sse, Json};
use std::convert::Infallible;
use std::sync::Arc;

/// POST /v1/chat/history — get chat history for a character
pub(in crate::daemon) async fn get_chat_history(
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
pub(in crate::daemon) async fn rollback_chat(
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

/// POST /v1/chat/regen — delete last assistant message and stream a new response (SSE)
pub(in crate::daemon) async fn regen_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<RegenRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AirpError,
> {
    // DX-3: quota check (same gate as chat_completion).
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let quota_config = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        cfg.quota.clone()
    };
    crate::quota::check_and_increment(&effective_root, &quota_config)?;

    // 1. Delete the last assistant message and capture its candidates.
    let (_log, old_candidates) =
        ChatService::new(&effective_root).regen(&req.character_id, req.session_id.as_ref())?;

    // 2. Build a regen pipeline (no new user message, no timeline advancement).
    let payload = ChatCompletionRequest {
        character_id: Some(req.character_id),
        character_card_id: None,
        lorebook_path: None,
        user_profile: crate::daemon::types::UserProfile {
            name: String::new(),
            variables: std::collections::HashMap::new(),
        },
        message: String::new(),
        messages_history: None,
        regex_filters: None,
        preset_id: None,
        enabled_presets: None,
        session_id: req.session_id,
        provider: None,
        endpoint: None,
        api_key: None,
        model: None,
        temperature: None,
        max_tokens: None,
        scene_id: None,
        user_id: req.user_id,
        persona_id: None,
        // #249 Swipe：将旧候选传入 pipeline，finalizer 会追加新候选。
        swipe_candidates: old_candidates,
    };
    let pipeline = chat_pipeline::prepare_regen_pipeline(&payload, &state)?;
    Ok(Sse::new(chat_pipeline::build_sse_stream(pipeline)))
}

/// POST /v1/chat/continue — continue generating, appending to the last assistant message (SSE)
pub(in crate::daemon) async fn continue_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<ContinueRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AirpError,
> {
    // DX-3: quota check (same gate as chat_completion).
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let quota_config = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        cfg.quota.clone()
    };
    crate::quota::check_and_increment(&effective_root, &quota_config)?;

    let payload = ChatCompletionRequest {
        character_id: Some(req.character_id),
        character_card_id: None,
        lorebook_path: None,
        user_profile: crate::daemon::types::UserProfile {
            name: String::new(),
            variables: std::collections::HashMap::new(),
        },
        message: String::new(),
        messages_history: None,
        regex_filters: None,
        preset_id: None,
        enabled_presets: None,
        session_id: req.session_id,
        provider: None,
        endpoint: None,
        api_key: None,
        model: None,
        temperature: None,
        max_tokens: None,
        scene_id: None,
        user_id: req.user_id,
        persona_id: None,
        swipe_candidates: Vec::new(),
    };
    let pipeline = chat_pipeline::prepare_continue_pipeline(&payload, &state)?;
    Ok(Sse::new(chat_pipeline::build_sse_stream(pipeline)))
}

/// POST /v1/chat/delete — delete a single message by durable ID
pub(in crate::daemon) async fn delete_message(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<DeleteMessageRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let log = ChatService::new(&effective_root).delete_message(
        &req.character_id,
        req.session_id.as_ref(),
        &req.message_id,
    )?;
    Ok(Json(log))
}

/// POST /v1/chat/swipe — #249 Swipe：切换指定消息的激活候选。
pub(in crate::daemon) async fn swipe_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<SwipeRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let log = ChatService::new(&effective_root).switch_swipe(
        &req.character_id,
        req.session_id.as_ref(),
        &req.message_id,
        req.index,
    )?;
    // #252 H.3：swipe 可审计性——记录 trace 事件。
    // regen/continue 通过 quota::check_and_increment 间接留下审计痕迹；
    // swipe 不走 quota，此处显式记录以保持 mutation 审计一致性。
    tracing::info!(
        character_id = %req.character_id,
        session_id = ?req.session_id,
        message_id = %req.message_id,
        new_index = req.index,
        "swipe switched"
    );
    Ok(Json(log))
}

pub(in crate::daemon) async fn chat_completion(
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

/// POST /v1/chat/preview — assemble the exact bounded trace without provider calls or writes.
pub(in crate::daemon) async fn preview_chat_assembly(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(payload): Json<ChatCompletionRequest>,
) -> Result<Json<crate::orchestrator::trace::PromptAssemblyTrace>, AirpError> {
    let pipeline = chat_pipeline::preview_pipeline(&payload, &state)?;
    Ok(Json(pipeline.prompt_trace))
}
