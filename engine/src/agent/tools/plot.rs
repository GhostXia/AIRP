//! Plot family: Agent 驱动的剧情推进（3.4）。
//!
//! 工具清单：
//! - `advance_plot`：根据当前状态/伏笔/节奏主动推进剧情（mutate）
//! - `get_plot_status`：获取当前剧情进度和悬挂线索（readonly）
//!
//! 与封卷系统联动：封卷时评估剧情进度，生成"下卷悬念/方向"。
//!
//! 并发纪律（PR #272 审计修复 + CodeRabbit 跟进）：
//! - `advance_plot` 对 live.json 的 `plot_history` 写入走
//!   [`StateService::mutate`]，与 `update_relationship` /
//!   `update_character_state` 共享 `state_lock(character_id)`，
//!   杜绝 read-modify-write 丢更新；并复用 #115 Phase 2e revision 合同
//!   （原子写 + history.jsonl + revisions/{n}/ 快照）。
//! - current.md 仍走 `volume_store::append_to_current`，但调用前显式持有
//!   `session_lock(character_id, session_id)`，与 `npc_action` /
//!   `trigger_world_event` / `seal_volume` 共享同一把 per-session 锁，
//!   防止并发追加在 current.md 中交错混合叙事内容。
//! - `get_plot_status` 对 live.json 读取走 [`StateService::read`]，与写入
//!   共享同一把 `state_lock`，避免读到半写状态。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::domain::{session_lock, StateService};
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
        confirm: bool,
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

            // #281: dry-run 模式——未确认时返回预览，不落盘
            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "dry_run": true,
                        "would_inject": format!("[剧情推进: {}] {}", plot_type, development),
                        "character_id": cid.as_str(),
                        "session_id": sid.as_ref().map(|s| s.to_string()),
                    }),
                    dry_run: true,
                });
            }

            // 注入剧情推进到 session 的 current.md
            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

            // 持有 session_lock 直到 append_to_current + memory revision commit
            // 完成，与 npc_action / trigger_world_event / seal_volume 共享
            // 同一把 per-session 锁，防止并发追加在 current.md 中交错。
            let session_boundary = session_lock(cid.as_str(), sid.as_ref());
            let _session_guard = session_boundary.lock().expect("session lock poisoned");

            let entry = format!("\n[剧情推进: {}] {}\n", plot_type, development);

            crate::volume_store::append_to_current(&session_dir, &entry)?;

            // 通过 StateService::mutate 串行化 plot_history 写入：
            // 1) 与 update_relationship / update_character_state 共享 state_lock(character_id)；
            // 2) parse 失败返回 AirpError::Internal，而非静默吞错；
            // 3) 复用 #115 Phase 2e revision 合同。
            //
            // 防御性类型检查（Gemini #1 跟进）：旧版若 `live` 非 Object 会 panic
            // （`live["plot_history"]` indexing 在非 Object 上 panic）；若
            // `plot_history` 字段已存在但非 Array，`as_array_mut()` 返回 None
            // 时 push 被静默跳过，导致更新丢失却仍返回 Ok。改为显式
            // `as_object_mut` + `entry` + `as_array_mut` + `ok_or_else(Internal)`，
            // 任何类型错乱都上抛错误而非 panic 或静默丢更新。
            let snapshot = StateService::new(&state.data_root).mutate(&cid, |live| {
                let live_obj = live.as_object_mut().ok_or_else(|| {
                    AirpError::Internal("live state is not a JSON object".to_string())
                })?;
                let history = live_obj
                    .entry("plot_history")
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .ok_or_else(|| {
                        AirpError::Internal("plot_history field is not a JSON array".to_string())
                    })?;
                history.push(serde_json::json!({
                    "type": plot_type,
                    "development": development,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }));
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
