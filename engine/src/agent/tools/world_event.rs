//! World event family: 世界事件触发器（3.1）。
//!
//! 工具清单：
//! - `trigger_world_event`：触发预设世界事件，注入到叙事上下文（mutate）
//! - `list_world_events`：列出角色可用的世界事件（readonly）
//!
//! 事件定义存储在 `characters/{id}/world_events.json`。
//! 事件注入走 volume_store::append_to_current（不新增注入路径）。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// 世界事件定义。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorldEvent {
    pub id: String,
    pub name: String,
    pub description: String,
    /// 触发关键词（任一命中即可触发）。
    #[serde(default)]
    pub trigger_keywords: Vec<String>,
    /// 最小触发轮次。
    #[serde(default)]
    pub min_turn: Option<u32>,
    /// 事件内容（注入到叙事上下文）。
    pub content: String,
    /// 是否已触发。
    #[serde(default)]
    pub triggered: bool,
}

fn world_events_path(data_root: &std::path::Path, character_id: &str) -> std::path::PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("world_events.json")
}

fn load_world_events(
    data_root: &std::path::Path,
    character_id: &str,
) -> Result<Vec<WorldEvent>, AirpError> {
    let path = world_events_path(data_root, character_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let events: Vec<WorldEvent> = serde_json::from_str(&content)?;
            Ok(events)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(AirpError::from(e)),
    }
}

fn save_world_events(
    data_root: &std::path::Path,
    character_id: &str,
    events: &[WorldEvent],
) -> Result<(), AirpError> {
    let path = world_events_path(data_root, character_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_string_pretty(events)?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// `trigger_world_event`：触发一个世界事件。
struct TriggerWorldEventTool {
    state: Arc<DaemonState>,
}

impl Tool for TriggerWorldEventTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "trigger_world_event",
            description: "Trigger a world event by ID. The event content will be injected into the narrative context.",
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
            let event_id = params
                .get("event_id")
                .and_then(Value::as_str)
                .ok_or_else(|| AirpError::BadRequest("event_id is required".to_string()))?;
            let sid = optional_session_id(&params)?;

            let mut events = load_world_events(&state.data_root, cid.as_str())?;
            let event_idx = events
                .iter()
                .position(|e| e.id == event_id)
                .ok_or_else(|| AirpError::NotFound(format!("event {} not found", event_id)))?;

            if events[event_idx].triggered {
                return Ok(ToolResult {
                    output: serde_json::json!({
                        "success": false,
                        "message": "event already triggered"
                    }),
                    dry_run: false,
                });
            }

            let event = events[event_idx].clone();

            // 注入事件内容到 session 的 current.md
            let session_dir = crate::data_dir::resolve_session_dir(
                &state.data_root,
                cid.as_str(),
                sid.as_ref(),
            )?;
            crate::volume_store::append_to_current(
                &session_dir,
                &format!("\n[世界事件: {}]\n{}\n", event.name, event.content),
            )?;

            // 标记为已触发
            events[event_idx].triggered = true;
            save_world_events(&state.data_root, cid.as_str(), &events)?;

            Ok(ToolResult {
                output: serde_json::json!({
                    "success": true,
                    "event": event
                }),
                dry_run: false,
            })
        })
    }
}

/// `list_world_events`：列出角色的世界事件。
struct ListWorldEventsTool {
    state: Arc<DaemonState>,
}

impl Tool for ListWorldEventsTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "list_world_events",
            description: "List all world events for a character.",
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
            let events = load_world_events(&state.data_root, cid.as_str())?;
            let out: Vec<Value> = events
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "id": e.id,
                        "name": e.name,
                        "description": e.description,
                        "triggered": e.triggered
                    })
                })
                .collect();
            Ok(ToolResult {
                output: Value::Array(out),
                dry_run: false,
            })
        })
    }
}

pub(super) fn register(reg: &mut ToolRegistry, state: Arc<DaemonState>) {
    const COLLISION: &str = "built-in tool name collision";
    reg.register(Box::new(TriggerWorldEventTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(ListWorldEventsTool { state }))
        .expect(COLLISION);
}
