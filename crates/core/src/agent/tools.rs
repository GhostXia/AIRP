//! M_AGENT-1: Tool execution surface — minimal skeleton.
//!
//! 协调器（`AgentLoop`）在每一步选择"派生纯净 subagent / 调一个工具 / 收敛结束"。
//! 本模块定义工具的抽象（[`Tool`] trait）、注册表（[`ToolRegistry`]）以及一个
//! 用于 M_AGENT-1 验收的 mock 工具 `echo`。
//!
//! ## 设计纪律（计划书 §2.1 第 4 条：工具最小授权）
//! - 每个工具带 [`ToolMeta`]：`readonly` / `mutate` / `destructive` / `append`。
//! - **破坏性工具默认 dry-run**：[`Tool::call`] 接收 `confirm: bool`，未确认时
//!   破坏性工具只回"将执行什么"的描述，不落副作用。M_AGENT-5 会补确认流。
//! - 工具入参/出参均为 `serde_json::Value`，零 schema 强制（呼应开放接入戒律）。
//!
//! M_AGENT-2 会把 Core 已有进程内数据操作（chat_store / volume_* / orchestrator /
//! scene / preset_regex / png_parser）包成 built-in 工具；M_AGENT-3 会合并 MCP
//! upstream 工具。本骨架仅含 echo，验证 loop → 工具 → subagent 闭环。

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;

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

/// 工具注册表。M_AGENT-1 仅 mock；M_AGENT-2 注入 built-in；M_AGENT-3 合并 MCP upstream。
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<&'static str, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        let name = tool.meta().name;
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list(&self) -> Vec<ToolMeta> {
        self.tools.values().map(|t| t.meta()).collect()
    }

    /// 是否授权调用该工具（M_AGENT-5 补 allowlist；M_AGENT-1 全放行）。
    pub fn allowed(&self, _name: &str) -> bool {
        true
    }
}

// ── mock 工具：echo（M_AGENT-1 验收用）─────────────────────────────────────

/// Echo：原样回传入参，标注 dry_run=false。用于验证 loop → 工具 → 回灌闭环。
pub struct EchoTool;

impl Tool for EchoTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "echo",
            description: "M_AGENT-1 mock: returns its input verbatim. Verifies loop→tool→subagent wiring.",
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

/// 构造 M_AGENT-1 骨架默认注册表（仅 echo）。
pub fn default_registry() -> ToolRegistry {
    let mut reg = ToolRegistry::new();
    reg.register(Box::new(EchoTool));
    reg
}
