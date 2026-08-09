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
//!   防止并发追加在 current.md 中交错混合叙事内容。整个同步临界区在
//!   `spawn_blocking` 中执行，避免同步锁/文件 I/O 占用 tokio worker。
//! - `get_plot_status` 对 live.json 读取走 [`StateService::read`]，与写入
//!   共享同一把 `state_lock`，避免读到半写状态。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::domain::{character_lock, lock_order, session_lock, StateService};
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
                .ok_or_else(|| AirpError::BadRequest("development is required".to_string()))?
                .to_string();
            let plot_type = params
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("progression")
                .to_string();
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

            // 所有同步路径解析、锁获取、文件 I/O 与 revision 提交都放入
            // blocking pool；std guard 不跨 await，锁序仍为
            // character.read → session → state（§2.3 / R1 / R2）。
            let data_root = state.data_root.clone();
            tokio::task::spawn_blocking(move || -> Result<ToolResult, AirpError> {
                let session_dir =
                    crate::data_dir::resolve_session_dir(&data_root, cid.as_str(), sid.as_ref())?;

                // #437 fix path 4：外层先持有 character_lock.read() 作为 per-character
                // 门控，StateService::mutate_locked 不再重复获取该锁。
                let character = character_lock(cid.as_str());
                let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
                let _character_track = lock_order::track_character_read();
                let session_boundary = session_lock(cid.as_str(), sid.as_ref());
                let _session_guard = session_boundary.lock().unwrap_or_else(|p| p.into_inner());
                let _session_track = lock_order::track_session();

                let entry = format!("\n[剧情推进: {}] {}\n", plot_type, development);
                crate::volume_store::append_to_current(&session_dir, &entry)?;

                let snapshot = StateService::new(&data_root).mutate_locked(&cid, |live| {
                    let live_obj = live.as_object_mut().ok_or_else(|| {
                        AirpError::Internal("live state is not a JSON object".to_string())
                    })?;
                    let history = live_obj
                        .entry("plot_history")
                        .or_insert_with(|| Value::Array(Vec::new()))
                        .as_array_mut()
                        .ok_or_else(|| {
                            AirpError::Internal(
                                "plot_history field is not a JSON array".to_string(),
                            )
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
            .await
            .map_err(|e| AirpError::Internal(format!("advance_plot blocking task failed: {e}")))?
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

            let sid = optional_session_id(&params)?;
            let data_root = state.data_root.clone();
            tokio::task::spawn_blocking(move || -> Result<ToolResult, AirpError> {
                // StateService/read_index 均为同步 I/O；将读取整体移出 tokio worker。
                let live_state = StateService::new(&data_root).read(&cid)?;
                let plot_history = live_state
                    .get("plot_history")
                    .cloned()
                    .unwrap_or(Value::Array(Vec::new()));
                let session_dir =
                    crate::data_dir::resolve_session_dir(&data_root, cid.as_str(), sid.as_ref())?;
                let index_content =
                    crate::volume_store::read_index(&session_dir).unwrap_or_default();
                let pending_clues = extract_section(&index_content, "悬挂线索");

                Ok(ToolResult {
                    output: serde_json::json!({
                        "plot_history": plot_history,
                        "pending_clues": pending_clues
                    }),
                    dry_run: false,
                })
            })
            .await
            .map_err(|e| {
                AirpError::Internal(format!("get_plot_status blocking task failed: {e}"))
            })?
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

// ── Phase 2.4: 剧情弧编辑器 ──────────────────────────────────────────────────
// PlotArc / PlotPhase 类型与 load/save 逻辑已提取至 `domain::plot::PlotService`
// （E-P1-3 slice 2）。本模块不再定义这些类型或直接写盘。
// daemon/handlers/plot.rs 通过 `crate::domain::{PlotArc, PlotService}` 调用。
// 边界：advance_plot 工具写 live.json 的 plot_history 仍走 StateService::mutate，
// 不经过 PlotService（PlotService 只管独立的 plot_arc.json 资产）。
