//! Plot family: Agent 驱动的剧情推进（3.4）。
//!
//! 工具清单：
//! - `advance_plot`：根据当前状态/伏笔/节奏主动推进剧情（mutate）
//! - `get_plot_status`：获取当前剧情进度和悬挂线索（readonly）
//!
//! 与封卷系统联动：封卷时评估剧情进度，生成"下卷悬念/方向"。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
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
            let session_dir = crate::data_dir::resolve_session_dir(
                &state.data_root,
                cid.as_str(),
                sid.as_ref(),
            )?;

            let entry = format!(
                "\n[剧情推进: {}] {}\n",
                plot_type, development
            );

            crate::volume_store::append_to_current(&session_dir, &entry)?;

            // 更新 state 中的 plot_progress
            let state_dir = crate::data_dir::char_state_dir(&state.data_root, cid.as_str());
            let live_path = state_dir.join("live.json");
            let mut live_state: Value = match std::fs::read_to_string(&live_path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or(Value::Object(Default::default())),
                Err(_) => Value::Object(Default::default()),
            };

            // 记录剧情推进历史
            if live_state.get("plot_history").is_none() {
                live_state["plot_history"] = Value::Array(Vec::new());
            }
            if let Some(history) = live_state["plot_history"].as_array_mut() {
                history.push(serde_json::json!({
                    "type": plot_type,
                    "development": development,
                    "timestamp": chrono::Utc::now().to_rfc3339()
                }));
            }

            std::fs::create_dir_all(&state_dir)?;
            std::fs::write(&live_path, serde_json::to_string_pretty(&live_state)?)?;

            Ok(ToolResult {
                output: serde_json::json!({
                    "success": true,
                    "type": plot_type,
                    "development": development
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

            // 读取 state
            let state_dir = crate::data_dir::char_state_dir(&state.data_root, cid.as_str());
            let live_path = state_dir.join("live.json");
            let live_state: Value = match std::fs::read_to_string(&live_path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or(Value::Object(Default::default())),
                Err(_) => Value::Object(Default::default()),
            };

            let plot_history = live_state
                .get("plot_history")
                .cloned()
                .unwrap_or(Value::Array(Vec::new()));

            // 读取 index.md 中的悬挂线索
            let sid = optional_session_id(&params)?;
            let session_dir = crate::data_dir::resolve_session_dir(
                &state.data_root,
                cid.as_str(),
                sid.as_ref(),
            )?;
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
