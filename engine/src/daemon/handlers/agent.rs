//! Agent HTTP handlers — agent run + tool catalog.
//!
//! #155 PR4：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；loop 逻辑在 `AgentLoop`，工具注册在 `agent::tools`。
//!
//! 端点：
//! - `POST /v1/agent/run` — 多步 loop 入口（SSE），quota 与 chat_completion 同路径
//! - `GET  /v1/agent/tools` — 列出内建工具元数据（19 工具，按名字字典序）

use super::*;
use axum::response::Sse;
use std::convert::Infallible;

/// M_AGENT-1: `POST /v1/agent/run` — 多步 loop 入口（SSE）。
///
/// 计划书 §4.3：`/v1/chat/completions` ≡ `max_steps=1` 的 `/v1/agent/run`。
/// 老客户端继续打 `/v1/chat/completions`（单回合）；要 agentic 的显式打此端点。
///
/// 复用 `AgentLoop::run`（协调器）；quota 检查与 chat_completion 同路径。
pub async fn agent_run(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(payload): Json<crate::agent::AgentRunRequest>,
) -> Result<
    Sse<impl futures_util::Stream<Item = Result<axum::response::sse::Event, Infallible>>>,
    AirpError,
> {
    // DX-3: quota check（与 chat_completion 同路径）
    let (quota_config, effective_root) = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        let quota = cfg.quota.clone();
        let root = crate::data_dir::resolve_effective_root(
            &state.data_root,
            payload.base.user_id.as_deref(),
        )?;
        (quota, root)
    };
    crate::quota::check_and_increment(&effective_root, &quota_config)?;

    let cancel = tokio_util::sync::CancellationToken::new();
    // 客户端断连 → drop SSE 流 → 我们不显式取消（M_AGENT-1 骨架）；
    // M_AGENT-5 会接 SSE 连接生命周期到 cancel token。
    let looper = crate::agent::AgentLoop::new(state);
    Ok(Sse::new(looper.run(payload, cancel)))
}

pub async fn list_agent_tools(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Json<Vec<crate::agent::tools::ToolMeta>> {
    Json(crate::agent::tools::default_registry(state).list())
}
