//! Plot family: Agent 驱动的剧情推进（3.4）。
//!
//! 工具清单：
//! - `advance_plot`：根据当前状态/伏笔/节奏主动推进剧情（mutate）
//! - `get_plot_status`：获取当前剧情进度和悬挂线索（readonly）
//!
//! 与封卷系统联动：封卷时评估剧情进度，生成"下卷悬念/方向"。
//!
//! 并发纪律（PR #272 审计修复）：
//! - `advance_plot` 对 live.json 的 `plot_history` 写入走
//!   [`StateService::mutate`]，与 `update_relationship` /
//!   `update_character_state` 共享 `state_lock(character_id)`，
//!   杜绝 read-modify-write 丢更新；并复用 #115 Phase 2e revision 合同
//!   （原子写 + history.jsonl + revisions/{n}/ 快照）。
//! - current.md 仍走 `volume_store::append_to_current`（session lock 串行化）。
//! - `get_plot_status` 对 live.json 读取走 [`StateService::read`]，与写入
//!   共享同一把 `state_lock`，避免读到半写状态。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::domain::StateService;
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// `advance_plot`：推进剧情。
struct AdvancePlotTool {
    state: Arc<DaemonState>,
}

impl Tool for AdvancePlotTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "advance_plot",
            description: "Advance the plot by introducing a new development, resolving a subplot, or escalating tension.",
            side_effect: ToolSideEffect::Mutate,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let cid = required_character_id(&params)?;
            let development = params
                .get("development")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("development is required".to_string()))?;
            let plot_type = params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("progression");
            let sid = optional_session_id(&params)?;

            // 注入剧情推进到 session 的 current.md
            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

            let entry = format!("\n[剧情推进: {}] {}\n", plot_type, development);

            crate::volume_store::append_to_current(&session_dir, &entry)?;

            // 通过 StateService::mutate 串行化 plot_history 写入：
            // 1) 与 update_relationship / update_character_state 共享 state_lock(character_id)；
            // 2) parse 失败返回 AirpError::Internal，而非静默吞错；
            // 3) 复用 #115 Phase 2e revision 合同。
            let snapshot = StateService::new(&state.data_root).mutate(&cid, |live| {
                if live.get("plot_history").is_none() {
                    live["plot_history"] = Value::Array(Vec::new());
                }
                if let Some(history) = live["plot_history"].as_array_mut() {
                    history.push(serde_json::json!({
                        "type": plot_type,
                        "development": development,
                        "timestamp": chrono::Utc::now().to_rfc3339()
                    }));
                }
                Ok(())
            })?;

            Ok(ToolResult {
                output: serde_json::json!({
                    "success": true,
                    "type": plot_type,
                    "development": development,
                    "revision": snapshot.revision
                }),
                dry_run: false,
            })
        })
    }
}

/// `get_plot_status`：获取剧情状态。
struct GetPlotStatusTool {
    state: Arc<DaemonState>,
}

impl Tool for GetPlotStatusTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_plot_status",
            description: "Get the current plot progress, including recent developments and pending plotlines.",
            side_effect: ToolSideEffect::Readonly,
        }
    }

    fn call(
        &self,
        params: Value,
        _confirm: bool,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, AirpError>> + Send + '_>> {
        let state = self.state.clone();
        Box::pin(async move {
            let cid = required_character_id(&params)?;

            // 通过 StateService::read 读取 live.json：
            // 1) 与写入共享 state_lock(character_id)，避免读到半写状态；
            // 2) parse 失败返回 AirpError::Internal，而非旧版 unwrap_or(empty) 静默吞错；
            // 3) 文件不存在时返回空对象，行为与原版一致。
            let live_state = StateService::new(&state.data_root).read(&cid)?;

            let plot_history = live_state
                .get("plot_history")
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));

            // 读取 index.md 中的悬挂线索
            let sid = optional_session_id(&params)?;
            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;
            let index_content = crate::volume_store::read_index(&session_dir).unwrap_or_default();

            // 提取悬挂线索段
            let pending_clues = extract_section(&index_content, "悬挂线索");

            Ok(ToolResult {
                output: serde_json::json!({
                    "plot_history": plot_history,
                    "pending_clues": pending_clues
                }),
                dry_run: false,
            })
        })
    }
}

/// 从 markdown 中提取指定 section 的内容。
fn extract_section(content: &str, section_name: &str) -> String {
    let mut result = String::new();
    let mut in_section = false;

    for line in content.lines() {
        if line.starts_with("## ") {
            if line.contains(section_name) {
                in_section = true;
                continue;
            } else if in_section {
                break;
            }
        }
        if in_section {
            result.push_str(line);
            result.push('\n');
        }
    }

    result.trim().to_string()
}

pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(AdvancePlotTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetPlotStatusTool { state }))
        .expect(COLLISION);
}
