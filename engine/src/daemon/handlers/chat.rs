//! Chat HTTP handlers — history / rollback / regen / continue / delete / completion.
//!
//! #155 PR4：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；SSE 流由 `chat_pipeline` 产出。
//!
//! 端点：
//! - `POST /v1/chat/history` — 读聊天历史（cursor 分页或 legacy 全量）
//! - `POST /v1/chat/rollback` — 回滚到指定 message_index 或 message_id
//! - `POST /v1/chat/regen` — 对最后一条 assistant 生成新候选 (SSE)
//! - `POST /v1/chat/continue` — 继续生成，追加到最后一条 assistant 消息 (SSE)
//! - `POST /v1/chat/delete` — 删除单条消息
//! - `POST /v1/chat/completions` — SSE 流式补全（quota 前置检查）

use crate::chat_pipeline;
use crate::chat_store::ChatLog;
use crate::daemon::types::{
    CancelGenerationRequest, ChatCompletionRequest, ContinueRequest, DeleteMessageRequest,
    EditMessageRequest, HistoryQuery, RegenRequest, RollbackRequest, SessionStateQuery,
    SwipeRequest, SwitchBranchRequest,
};
use crate::daemon::DaemonState;
use crate::domain::ChatService;
use crate::error::AirpError;
use crate::session_coordinator::{SessionCommand, SessionCoordinatorStatus};
use axum::{response::Sse, Json};
use std::convert::Infallible;
use std::sync::Arc;

/// POST /v1/chat/history — get chat history for a character
///
/// `ChatService::history_window` / `history` 是同步文件 IO；在 async handler 中用
/// `spawn_blocking` 包装避免阻塞 tokio worker 线程（#433）。
pub(in crate::daemon) async fn get_chat_history(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(query): Json<HistoryQuery>,
) -> Result<Json<serde_json::Value>, AirpError> {
    // DX-1：与同族 mutation handler 对齐，按 user_id 解析 effective root，
    // 避免多用户隔离下 history 读取与 mutation 写入落在不同数据根。
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, query.user_id.as_deref())?;
    // #37 cursor 分页：传 limit/before 走窗口；不传 → 全量（向后兼容旧客户端）。
    if query.limit.is_some() || query.before.is_some() {
        let window = tokio::task::spawn_blocking({
            let effective_root = effective_root.clone();
            let character_id = query.character_id.clone();
            let session_id = query.session_id;
            let limit = query.limit;
            let before = query.before.clone();
            move || {
                ChatService::new(&effective_root).history_window(
                    &character_id,
                    session_id.as_ref(),
                    limit,
                    before.as_deref(),
                )
            }
        })
        .await
        .map_err(|e| AirpError::Internal(format!("history_window join failed: {e}")))??;
        return Ok(Json(serde_json::to_value(window)?));
    }
    // legacy 全量返回必须保留 ChatLog 的既有响应形状。
    let log = tokio::task::spawn_blocking({
        let character_id = query.character_id.clone();
        let session_id = query.session_id;
        move || ChatService::new(&effective_root).history(&character_id, session_id.as_ref())
    })
    .await
    .map_err(|e| AirpError::Internal(format!("history join failed: {e}")))??;
    Ok(Json(serde_json::to_value(log)?))
}

/// POST /v1/chat/session-state — observe the Coordinator without creating an idle entry.
pub(in crate::daemon) async fn get_chat_session_state(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(query): Json<SessionStateQuery>,
) -> Result<Json<SessionCoordinatorStatus>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, query.user_id.as_deref())?;
    Ok(Json(state.session_coordinators.status(
        &effective_root,
        &query.character_id,
        query.session_id.as_ref(),
    )))
}

/// POST /v1/chat/cancel — cooperatively cancel one exact active generation.
pub(in crate::daemon) async fn cancel_chat_generation(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<CancelGenerationRequest>,
) -> Result<Json<SessionCoordinatorStatus>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let status = state.session_coordinators.cancel_generation(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        &req.generation_id,
    )?;
    Ok(Json(status))
}

/// POST /v1/chat/rollback — rollback to a specific message index
///
/// #433: ChatService 的 rollback / rollback_to_id 是同步文件 IO，在 async
/// handler 中用 `spawn_blocking` 包装避免阻塞 tokio worker 线程。lease
/// 仍在 async 上下文持有，保证 IO 完成前不会被释放。
pub(in crate::daemon) async fn rollback_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<RollbackRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    // #37：message_id / message_index 二选一校验。
    if let Err(msg) = req.validate_rollback_target() {
        return Err(AirpError::BadRequest(msg));
    }
    // DX-1：与同族 mutation handler（regen/continue/delete/swipe/edit/branch 等）对齐：
    // effective root = resolve_effective_root(daemon_root, user_id)。Coordinator
    // lease key 以 effective root 为前缀，因此 history 读取与 rollback 写入必须
    // 解析到同一个根，否则多用户隔离下 lease key 不一致。user_id 省略/空串时
    // resolve_effective_root 原样返回 daemon root，旧请求行为不变。
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let _operation = state.session_coordinators.try_submit(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        SessionCommand::Rollback,
    )?;
    let character_id = req.character_id.clone();
    let session_id = req.session_id;
    let message_index = req.message_index;
    let message_id = req.message_id.clone();
    let (log, _) = tokio::task::spawn_blocking(move || -> Result<_, AirpError> {
        let service = ChatService::new(&effective_root);
        match (message_index, message_id.as_deref()) {
            (Some(idx), None) => service.rollback(&character_id, session_id.as_ref(), idx),
            (None, Some(id)) => service.rollback_to_id(&character_id, session_id.as_ref(), id),
            // validate_rollback_target 已挡住二义与都空，这里不可达。
            _ => Err(AirpError::BadRequest(
                "rollback target invariant violated".into(),
            )),
        }
    })
    .await
    .map_err(|e| AirpError::Internal(format!("rollback join failed: {e}")))??;
    Ok(Json(log))
}

/// POST /v1/chat/regen — stream a new candidate for the active assistant message (SSE)
pub(in crate::daemon) async fn regen_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<RegenRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AirpError,
> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let operation = state.session_coordinators.try_submit(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        SessionCommand::Regen,
    )?;
    // DX-3: quota check (same gate as chat_completion).
    let quota_config = {
        let cfg = state.read_config();
        cfg.quota.clone()
    };
    crate::quota::check_and_increment(&effective_root, &quota_config)?;

    // 1. Capture the active assistant without mutating durable history.
    let snapshot = ChatService::new(&effective_root).regen_snapshot(
        &req.character_id,
        req.session_id.as_ref(),
        operation.generation_id().to_string(),
    )?;

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
        swipe_candidates: Vec::new(),
        branch_from: None,
    };
    let mut pipeline = chat_pipeline::prepare_regen_pipeline(&payload, &state, snapshot)?;
    pipeline.finalizer.session_operation_lease = Some(operation);
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
    let operation = state.session_coordinators.try_submit(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        SessionCommand::Continue,
    )?;
    let quota_config = {
        let cfg = state.read_config();
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
        branch_from: None,
    };
    let mut pipeline = chat_pipeline::prepare_continue_pipeline(&payload, &state)?;
    pipeline.finalizer.session_operation_lease = Some(operation);
    Ok(Sse::new(chat_pipeline::build_sse_stream(pipeline)))
}

/// POST /v1/chat/delete — delete a single message by durable ID
///
/// #433: ChatService::delete_message 是同步文件 IO，用 `spawn_blocking` 包装。
pub(in crate::daemon) async fn delete_message(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<DeleteMessageRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let _operation = state.session_coordinators.try_submit(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        SessionCommand::DeleteMessage,
    )?;
    let character_id = req.character_id.clone();
    let session_id = req.session_id;
    let message_id = req.message_id.clone();
    let log = tokio::task::spawn_blocking(move || {
        ChatService::new(&effective_root).delete_message(
            &character_id,
            session_id.as_ref(),
            &message_id,
        )
    })
    .await
    .map_err(|e| AirpError::Internal(format!("delete_message join failed: {e}")))??;
    Ok(Json(log))
}

/// POST /v1/chat/swipe — #249 Swipe：切换指定消息的激活候选。
///
/// #252 D3：返回 `SwipeResponse` 增量响应，不再回完整 `ChatLog`。
///
/// #433: ChatService::switch_swipe 是同步文件 IO，用 `spawn_blocking` 包装。
pub(in crate::daemon) async fn swipe_chat(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<SwipeRequest>,
) -> Result<Json<crate::domain::SwipeResponse>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let _operation = state.session_coordinators.try_submit(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        SessionCommand::Swipe,
    )?;
    let character_id = req.character_id.clone();
    let session_id = req.session_id;
    let message_id = req.message_id.clone();
    let index = req.index;
    let resp = tokio::task::spawn_blocking(move || {
        ChatService::new(&effective_root).switch_swipe(
            &character_id,
            session_id.as_ref(),
            &message_id,
            index,
        )
    })
    .await
    .map_err(|e| AirpError::Internal(format!("swipe join failed: {e}")))??;
    // #252 H.3：swipe 可审计性——记录 trace 事件。
    // regen/continue 通过 quota::check_and_increment 间接留下审计痕迹；
    // swipe 不走 quota，此处显式记录以保持 mutation 审计一致性。
    // CodeRabbit nitpick: 加入 user_id 便于多租户场景审计追溯。
    tracing::info!(
        character_id = %req.character_id,
        session_id = ?req.session_id,
        message_id = %req.message_id,
        new_index = req.index,
        user_id = ?req.user_id,
        "swipe switched"
    );
    Ok(Json(resp))
}

/// PUT /v1/chat/message — edit a user message's content by durable ID.
///
/// #433: ChatService::edit_message 是同步文件 IO，用 `spawn_blocking` 包装。
pub(in crate::daemon) async fn edit_message(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<EditMessageRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let _operation = state.session_coordinators.try_submit(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        SessionCommand::EditMessage,
    )?;
    let character_id = req.character_id.clone();
    let session_id = req.session_id;
    let message_id = req.message_id.clone();
    let content = req.content.clone();
    let log = tokio::task::spawn_blocking(move || {
        ChatService::new(&effective_root).edit_message(
            &character_id,
            session_id.as_ref(),
            &message_id,
            &content,
        )
    })
    .await
    .map_err(|e| AirpError::Internal(format!("edit_message join failed: {e}")))??;
    tracing::info!(
        character_id = %req.character_id,
        session_id = ?req.session_id,
        message_id = %req.message_id,
        user_id = ?req.user_id,
        "message edited"
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
        let cfg = state.read_config();
        let quota = cfg.quota.clone();
        let root =
            crate::data_dir::resolve_effective_root(&state.data_root, payload.user_id.as_deref())?;
        (quota, root)
    };
    let operation = payload
        .character_id
        .as_ref()
        .map(|character_id| {
            state.session_coordinators.try_submit(
                &effective_root,
                character_id,
                payload.session_id.as_ref(),
                SessionCommand::Completion,
            )
        })
        .transpose()?;
    crate::quota::check_and_increment(&effective_root, &quota_config)?;
    let mut pipeline = chat_pipeline::prepare_pipeline(&payload, &state)?;
    pipeline.finalizer.session_operation_lease = operation;
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

/// POST /v1/chat/branch/switch — switch the active branch to a target leaf.
///
/// #433: ChatService::switch_branch 是同步文件 IO，用 `spawn_blocking` 包装。
pub(in crate::daemon) async fn switch_branch(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<SwitchBranchRequest>,
) -> Result<Json<ChatLog>, AirpError> {
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, req.user_id.as_deref())?;
    let _operation = state.session_coordinators.try_submit(
        &effective_root,
        &req.character_id,
        req.session_id.as_ref(),
        SessionCommand::SwitchBranch,
    )?;
    let character_id = req.character_id.clone();
    let session_id = req.session_id;
    let target_leaf_id = req.target_leaf_id.clone();
    let log = tokio::task::spawn_blocking(move || {
        ChatService::new(&effective_root).switch_branch(
            &character_id,
            session_id.as_ref(),
            &target_leaf_id,
        )
    })
    .await
    .map_err(|e| AirpError::Internal(format!("switch_branch join failed: {e}")))??;
    tracing::info!(
        character_id = %req.character_id,
        session_id = ?req.session_id,
        target_leaf_id = %req.target_leaf_id,
        user_id = ?req.user_id,
        "branch switched"
    );
    Ok(Json(log))
}
