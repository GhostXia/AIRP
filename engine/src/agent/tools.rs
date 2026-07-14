//! Engine-authoritative Agent tool execution surface.
//!
//! 协调器（`AgentLoop`）在每一步选择"派生纯净 subagent / 调一个工具 / 收敛结束"。
//! 本模块定义工具抽象、注册表、capability/allowlist 门和内建 domain tools。
//!
//! ## 模块结构（#155 PR 3 之后）
//! - `tools.rs`（本文件）：facade。保留 [`Tool`] / [`ToolMeta`] / [`ToolResult`] /
//!   [`ToolSideEffect`] / [`ToolRegistry`] / [`EchoTool`] / [`default_registry`]
//!   契约，并对 `enhance_md_via_llm_shared` / `ENHANCE_ANALYSIS_SYSTEM_PROMPT`
//!   做最小 re-export（保持原 crate-private / public 调用路径）。
//! - `tools/params.rs`：跨 family 参数 helper（`pub(super)`，不外泄）。
//! - `tools/session.rs`：session family 5 工具，`pub(super) fn register` 集中注册。
//! - `tools/character.rs`：character family 3 工具，`pub(super) fn register` 集中注册。
//! - `tools/state_lorebook.rs`：state + lorebook family 6 工具，含 `read_lorebook_or_empty`。
//! - `tools/volume_context.rs`：volume seal + context bundle export family 2 工具。
//! - `tools/analysis.rs`：analysis enhance/apply family 2 工具 + 共享 LLM helper。
//! - `tools/tests/`：按 family 分组的测试子模块。
//!
//! ## 设计纪律（计划书 §2.1 第 4 条：工具最小授权）
//! - 每个工具带 [`ToolMeta`]：`readonly` / `mutate` / `destructive` / `append`。
//! - **破坏性工具默认 dry-run**：[`Tool::call`] 接收 `confirm: bool`，未确认时
//!   破坏性工具只回"将执行什么"的描述，不落副作用。M_AGENT-5 会补确认流。
//! - 工具入参/出参均为 `serde_json::Value`，零 schema 强制（呼应开放接入戒律）。

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::daemon::DaemonState;
use airp_state_protocol::Capability;

mod analysis;
mod character;
mod params;
mod session;
mod state_lorebook;
#[cfg(test)]
mod tests;
mod volume_context;

// #155 PR 3：analysis family 的共享资产经 facade 做最小 re-export，
// 保持原 `pub` / `pub(crate)` 调用路径不变（daemon decompose_handlers 等复用）。
pub(crate) use analysis::enhance_md_via_llm_shared;
pub use analysis::ENHANCE_ANALYSIS_SYSTEM_PROMPT;

/// 工具副作用分类（驱动 dry-run / 确认流 / 幂等去重）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSideEffect {
    /// 纯读，零副作用。
    Readonly,
    /// 写入/创建/更新，幂等或可安全重试。
    Mutate,
    /// 删除/覆盖/回滚，默认 dry-run，需显式确认。
    Destructive,
    /// 仅追加（JSONL append 等），幂等键去重适用。
    Append,
}

/// 工具元数据，对应 MCP `ToolAnnotations` 的子集。
#[derive(Debug, Clone, Serialize)]
pub struct ToolMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub side_effect: ToolSideEffect,
}

/// 工具执行结果。
#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    /// 工具返回值（任意 JSON）。
    pub output: Value,
    /// 是否为 dry-run（破坏性工具未确认时为 true）。
    pub dry_run: bool,
}

/// 工具 trait。`call` 是异步 boxed Future（动态分发足够，工具非热路径）。
pub trait Tool: Send + Sync {
    fn meta(&self) -> ToolMeta;

    /// 执行工具。
    ///
    /// `confirm` 对破坏性工具语义：`false` → dry-run（只描述将做什么，不落副作用）；
    /// `true` → 真执行。readonly / mutate / append 工具忽略 `confirm`。
    fn call(
        &self,
        params: Value,
        confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>>;
}

/// Tool registry assembled from built-ins; future MCP sources must pass the
/// same name-collision and authorization gates.
#[derive(Default)]
pub struct ToolRegistry {
    tools: std::collections::HashMap<&'static str, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册工具。重名视为注册表组装期的编程错误 → 返回 `Err`，绝不静默覆盖
    /// （旧实现 `insert` 会悄悄顶掉同名工具，掩盖冲突；issue #24）。
    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), AirpError> {
        let name = tool.meta().name;
        if self.tools.contains_key(name) {
            return Err(AirpError::Config(format!(
                "duplicate tool registration: {}",
                name
            )));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list(&self) -> Vec<ToolMeta> {
        let mut tools: Vec<_> = self.tools.values().map(|t| t.meta()).collect();
        tools.sort_by_key(|tool| tool.name);
        tools
    }

    /// Engine-authoritative gate. The model can only select registered tools,
    /// the caller must explicitly grant `call:tool`, and an optional allowlist
    /// can further reduce the surface for a run.
    pub fn allowed(
        &self,
        name: &str,
        capabilities: &[Capability],
        allowlist: Option<&[String]>,
    ) -> bool {
        self.tools.contains_key(name)
            && capabilities.contains(&Capability::CallTool)
            && allowlist.is_none_or(|allowed| allowed.iter().any(|item| item == name))
    }
}

// ── mock 工具：echo（M_AGENT-1 验收用）─────────────────────────────────────

/// Echo：原样回传入参，标注 dry_run=false。用于验证 loop → 工具 → 回灌闭环。
pub struct EchoTool;

impl Tool for EchoTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "echo",
            description:
                "M_AGENT-1 mock: returns its input verbatim. Verifies loop→tool→subagent wiring.",
            side_effect: ToolSideEffect::Readonly,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        Box::pin(async move {
            Ok(ToolResult {
                output: params,
                dry_run: false,
            })
        })
    }
}

/// Construct the built-in registry used by the structured Agent loop.
///
/// `state` 让 built-in 工具访问数据层（`data_root`）。echo 等无状态工具
/// 忽略它。调用方（`AgentLoop::new`）已有 `Arc<DaemonState>`，传入即可。
///
/// 注册顺序：echo → session family → character family → state/lorebook
/// → volume/context → analysis。family 内顺序由各 `register` fn 内部决定，
/// 但 `ToolRegistry::list` 最终按 name 字典序输出，故注册顺序不影响
/// `/v1/agent/tools` 响应。
pub fn default_registry(state: Arc<DaemonState>) -> ToolRegistry {
    // 内建工具集是编译期固定的、名字不重复的集合；若这里冒出重名，那是新增
    // 工具时的编程错误，应在启动时立刻炸出来，而非静默覆盖（issue #24）。
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(EchoTool))
        .expect("built-in tool name collision");
    // M_AGENT-2 第一批：会话类 5 工具。
    session::register(&mut reg, state.clone());
    // M_AGENT-2 第二批：角色类 3 工具（list/get/delete）。
    character::register(&mut reg, state.clone());
    // #155 PR 3：state + lorebook family 6 工具。
    state_lorebook::register(&mut reg, state.clone());
    // #155 PR 3：volume seal + context bundle export family 2 工具。
    volume_context::register(&mut reg, state.clone());
    // #155 PR 3：analysis enhance/apply family 2 工具。
    analysis::register(&mut reg, state);
    reg
}
