//! NPC family: 多角色编排增强（3.3）。
//!
//! 工具清单：
//! - `npc_action`：让 NPC 执行独立行动并影响世界状态（mutate）
//! - `update_relationship`：更新角色间关系（mutate）
//!
//! NPC 行动结果写入 session 的 current.md，关系矩阵存储在 state/live.json。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// `npc_action`：让 NPC 执行独立行动。
struct NpcActionTool {
    state: Arc<DaemonState>,
}

impl Tool for NpcActionTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "npc_action",
            description: "Execute an NPC autonomous action. The action result will be injected into the narrative context.",
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
            let npc_name = params
                .get("npc_name")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("npc_name is required".to_string()))?;
            let action = params
                .get("action")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("action is required".to_string()))?;
            let result = params
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("");
            let sid = optional_session_id(&params)?;

            // 注入 NPC 行动到 session 的 current.md
            let session_dir = crate::data_dir::resolve_session_dir(
                &state.data_root,
                cid.as_str(),
                sid.as_ref(),
            )?;

            let mut entry = format!("\n[NPC行动: {}] {}\n", npc_name, action);
            if !result.is_empty() {
                entry.push_str(&format!("结果: {}\n", result));
            }

            crate::volume_store::append_to_current(&session_dir, &entry)?;

            Ok(ToolResult {
                output: serde_json::json!({
                    "success": true,
                    "npc_name": npc_name,
                    "action": action,
                    "result": result
                }),
                dry_run: false,
            })
        })
    }
}

/// `update_relationship`：更新角色间关系。
struct UpdateRelationshipTool {
    state: Arc<DaemonState>,
}

impl Tool for UpdateRelationshipTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "update_relationship",
            description: "Update the relationship between two characters. Stores in state/live.json relationships matrix.",
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
            let from_char = params
                .get("from")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("from is required".to_string()))?;
            let to_char = params
                .get("to")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("to is required".to_string()))?;
            let relation_type = params
                .get("relation_type")
                .and_then(Value::as_str)
                .unwrap_or("neutral");
            let intensity = params
                .get("intensity")
                .and_then(Value::as_f64)
                .unwrap_or(0.5);

            // 读取现有 state
            let state_dir = crate::data_dir::char_state_dir(&state.data_root, cid.as_str());
            let live_path = state_dir.join("live.json");
            let mut live_state: Value = match std::fs::read_to_string(&live_path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or(Value::Object(Default::default())),
                Err(_) => Value::Object(Default::default()),
            };

            // 确保 relationships 字段存在
            if live_state.get("relationships").is_none() {
                live_state["relationships"] = Value::Object(Default::default());
            }

            // 更新关系
            let key = format!("{}->{}", from_char, to_char);
            live_state["relationships"][&key] = serde_json::json!({
                "type": relation_type,
                "intensity": intensity
            });

            // 写回
            std::fs::create_dir_all(&state_dir)?;
            std::fs::write(&live_path, serde_json::to_string_pretty(&live_state)?)?;

            Ok(ToolResult {
                output: serde_json::json!({
                    "success": true,
                    "from": from_char,
                    "to": to_char,
                    "relation_type": relation_type,
                    "intensity": intensity
                }),
                dry_run: false,
            })
        })
    }
}

pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(NpcActionTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(UpdateRelationshipTool { state }))
        .expect(COLLISION);
}
