//! NPC family: 多角色编排增强（3.3）。
//!
//! 工具清单：
//! - `npc_action`：让 NPC 执行独立行动并影响世界状态（mutate）
//! - `update_relationship`：更新角色间关系（mutate）
//!
//! NPC 行动结果写入 session 的 current.md，关系矩阵存储在 state/live.json。
//!
//! 并发纪律（PR #272 审计修复 + CodeRabbit 跟进）：
//! - `update_relationship` 走 [`StateService::mutate`]，复用 #115 Phase 2e
//!   的 revision 合同（原子写 + history.jsonl + revisions/{n}/ 快照），
//!   并与 `update_character_state` / `advance_plot` 共享同一把
//!   `state_lock(character_id)`，杜绝 read-modify-write 丢更新。
//! - `npc_action` 在调用 `volume_store::append_to_current` 前显式持有
//!   `session_lock(character_id, session_id)`，与 `advance_plot` /
//!   `trigger_world_event` / `seal_volume` 共享同一把 per-session 锁，
//!   防止并发追加在 `current.md` 中交错混合叙事内容。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::domain::{session_lock, StateService};
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
            let result = params.get("result").and_then(Value::as_str).unwrap_or("");
            let sid = optional_session_id(&params)?;

            // 注入 NPC 行动到 session 的 current.md
            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

            // 持有 session_lock 直到 append_to_current + memory revision commit
            // 完成，与 advance_plot / trigger_world_event / seal_volume 共享
            // 同一把 per-session 锁，防止并发追加在 current.md 中交错。
            let session_boundary = session_lock(cid.as_str(), sid.as_ref());
            let _session_guard = session_boundary.lock().expect("session lock poisoned");

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
        confirm: bool,
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

            // #281: dry-run 模式——未确认时返回预览，不落盘
            if !confirm {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "dry_run": true,
                        "would_update": {
                            "from": from_char,
                            "to": to_char,
                            "relation_type": relation_type,
                            "intensity": intensity,
                        },
                        "character_id": cid.as_str(),
                    }),
                    dry_run: true,
                });
            }

            // 通过 StateService::mutate 串行化 read-modify-write：
            // 1) 与 advance_plot / update_character_state 共享 state_lock(character_id)，
            //    避免互相覆盖；
            // 2) parse 失败返回 AirpError::Internal，而非旧版 unwrap_or(empty) 静默吞错；
            // 3) 复用 #115 Phase 2e revision 合同：data_dir::replace_file 原子写 +
            //    history.jsonl append + revisions/{n}/ 不可变快照。
            //
            // 防御性类型检查（Gemini #2 跟进）：若 live.json 损坏（非 Object，
            // 或 relationships 字段类型错乱），旧版 `live["relationships"][&key] =`
            // 在 serde_json 中会 panic（非 Object 上 indexing panic）。改为
            // `as_object_mut` + `entry` + `ok_or_else(Internal)`，让损坏 JSON
            // 上抛错误而非 panic daemon。
            let snapshot = StateService::new(&state.data_root).mutate(&cid, |live| {
                let live_obj = live.as_object_mut().ok_or_else(|| {
                    AirpError::Internal("live state is not a JSON object".to_string())
                })?;
                let relationships = live_obj
                    .entry("relationships")
                    .or_insert_with(|| Value::Object(Default::default()))
                    .as_object_mut()
                    .ok_or_else(|| {
                        AirpError::Internal("relationships field is not a JSON object".to_string())
                    })?;
                let key = format!("{}->{}", from_char, to_char);
                relationships.insert(
                    key,
                    serde_json::json!({
                        "type": relation_type,
                        "intensity": intensity
                    }),
                );
                Ok(())
            })?;

            Ok(ToolResult {
                output: serde_json::json!({
                    "success": true,
                    "from": from_char,
                    "to": to_char,
                    "relation_type": relation_type,
                    "intensity": intensity,
                    "revision": snapshot.revision
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
