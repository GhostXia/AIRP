//! World event family: 世界事件触发器（3.1）。
//!
//! 工具清单：
//! - `trigger_world_event`：触发预设世界事件，注入到叙事上下文（mutate）
//! - `list_world_events`：列出角色可用的世界事件（readonly）
//!
//! 事件定义存储在 `characters/{id}/world_events.json`。
//! 事件注入走 volume_store::append_to_current（不新增注入路径）。
//!
//! 并发纪律（PR #272 审计修复 + CodeRabbit 跟进）：
//! - `trigger_world_event` 的 check-then-act（读 `triggered` → 注入 → 标记）
//!   原本无锁，并发触发同一 event_id 会双重注入 current.md。现在整段
//!   临界区持有 `state_lock(character_id)`，与 live.json 写入共享同一把
//!   锁，保证触发原子性。
//! - 事件注入到 current.md 走 `volume_store::append_to_current`，调用前
//!   显式持有 `session_lock(character_id, session_id)`，与 `npc_action` /
//!   `advance_plot` / `seal_volume` 共享同一把 per-session 锁，防止并发
//!   追加在 current.md 中交错混合叙事内容。
//! - `save_world_events` 改用 `data_dir::replace_file` 原子写，避免半写
//!   状态被其他读者看到；并在写入前 `fsync` 父目录（`replace_file` 内置）。
//! - `load_world_events` 的 JSON parse 错误原本通过 `?` 上抛（行为正确），
//!   本审计未改动其错误传播策略，仅修复写路径。
//!
//! 注：world_events.json 当前未接入 #115 Phase 2e revision 合同（缺少
//! AssetKind::WorldEvents 枚举与 revision 目录约定）。该缺失属于设计
//! 扩展项，已记入审计报告遗留项，不阻塞本 PR。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::domain::{session_lock, state_lock};
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
    /// Phase 2.3: 时间触发条件——当 world_clock >= time_trigger 时自动触发。
    #[serde(default)]
    pub time_trigger: Option<u64>,
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
    let content = serde_json::to_vec_pretty(events)?;
    // 原子写：替换旧版 std::fs::write，避免半写状态被并发 reader 看到。
    // data_dir::replace_file 内部走 tmp + rename + fsync(parent)。
    crate::data_dir::replace_file(&path, &content)?;
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

            // 持有 state_lock(character_id) 直到所有 mutation 完成：
            // 1) 防止两个并发 trigger_world_event(event_id=X) 都通过 `triggered=false`
            //    检查后各自注入 + 标记，导致 current.md 出现两份事件内容；
            // 2) 与 update_relationship / advance_plot 共享同一把锁，避免
            //    live.json 与 world_events.json 的写乱序影响叙事一致性。
            let state_boundary = state_lock(cid.as_str());
            let _state_guard = state_boundary.lock().expect("state lock poisoned");

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
            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

            // 持有 session_lock 直到 append_to_current + memory revision commit
            // 完成，与 npc_action / advance_plot / seal_volume 共享同一把
            // per-session 锁，防止并发追加在 current.md 中交错混合叙事内容。
            let session_boundary = session_lock(cid.as_str(), sid.as_ref());
            let _session_guard = session_boundary.lock().expect("session lock poisoned");

            crate::volume_store::append_to_current(
                &session_dir,
                &format!("\n[世界事件: {}]\n{}\n", event.name, event.content),
            )?;

            // 标记为已触发（save_world_events 已改用 data_dir::replace_file 原子写）
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
    reg.register(Box::new(ListWorldEventsTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(AdvanceClockTool {
        state: state.clone(),
    }))
    .expect(COLLISION);
    reg.register(Box::new(GetClockTool { state }))
        .expect(COLLISION);
}

// ── Phase 2.3: 世界时钟 ────────────────────────────────────────────────────────

/// 世界时钟配置与状态。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorldClock {
    /// 当前时间（抽象时间单位，从 0 开始）。
    pub current_time: u64,
    /// 每轮自动推进的时间单位。
    pub advance_per_turn: u64,
    /// 时间单位描述（用于显示，如 "hour", "day"）。
    pub time_unit: String,
    /// 可选的显示格式（如 "第{day}天 {hour}:00"）。
    #[serde(default)]
    pub display_format: Option<String>,
}

impl Default for WorldClock {
    fn default() -> Self {
        Self {
            current_time: 0,
            advance_per_turn: 1,
            time_unit: "hour".to_string(),
            display_format: None,
        }
    }
}

impl WorldClock {
    /// 生成人类可读的时间显示。
    pub fn display(&self) -> String {
        if let Some(ref fmt) = self.display_format {
            // 简单模板替换
            let hours_per_day = 24u64;
            let day = self.current_time / hours_per_day + 1;
            let hour = self.current_time % hours_per_day;
            fmt.replace("{day}", &day.to_string())
                .replace("{hour}", &format!("{:02}", hour))
                .replace("{time}", &self.current_time.to_string())
        } else {
            format!("T+{} {}", self.current_time, self.time_unit)
        }
    }
}

fn world_clock_path(data_root: &std::path::Path, character_id: &str) -> std::path::PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("world_clock.json")
}

/// 读取世界时钟。不存在返回默认值。
pub fn load_world_clock(
    data_root: &std::path::Path,
    character_id: &str,
) -> Result<WorldClock, AirpError> {
    let path = world_clock_path(data_root, character_id);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let clock: WorldClock = serde_json::from_str(&content)?;
            Ok(clock)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(WorldClock::default()),
        Err(e) => Err(AirpError::from(e)),
    }
}

/// 保存世界时钟。
pub fn save_world_clock(
    data_root: &std::path::Path,
    character_id: &str,
    clock: &WorldClock,
) -> Result<(), AirpError> {
    let path = world_clock_path(data_root, character_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = serde_json::to_vec_pretty(clock)?;
    crate::data_dir::replace_file(&path, &content)?;
    Ok(())
}

/// 推进时钟并检查时间触发事件。返回触发的事件列表。
pub fn advance_and_check_triggers(
    data_root: &std::path::Path,
    character_id: &str,
    session_dir: &std::path::Path,
    advance_by: Option<u64>,
) -> Result<(WorldClock, Vec<WorldEvent>), AirpError> {
    let mut clock = load_world_clock(data_root, character_id)?;
    let advance = advance_by.unwrap_or(clock.advance_per_turn);
    // 溢出检查：避免 unchecked addition 导致 u64 wrap-around
    clock.current_time = clock
        .current_time
        .checked_add(advance)
        .ok_or_else(|| AirpError::BadRequest("clock advance overflow".to_string()))?;
    save_world_clock(data_root, character_id, &clock)?;

    // 检查时间触发事件
    let mut events = load_world_events(data_root, character_id)?;
    let mut triggered_events = Vec::new();
    for event in events.iter_mut() {
        if !event.triggered {
            if let Some(time_trigger) = event.time_trigger {
                if clock.current_time >= time_trigger {
                    // 先注入事件内容到 current.md，成功后再标记 triggered
                    crate::volume_store::append_to_current(
                        session_dir,
                        &format!("\n[世界事件: {}]\n{}\n", event.name, event.content),
                    )?;
                    event.triggered = true;
                    triggered_events.push(event.clone());
                }
            }
        }
    }
    if !triggered_events.is_empty() {
        save_world_events(data_root, character_id, &events)?;
    }
    Ok((clock, triggered_events))
}

/// `advance_clock`：推进世界时钟并检查时间触发事件。
struct AdvanceClockTool {
    state: Arc<DaemonState>,
}

impl Tool for AdvanceClockTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "advance_clock",
            description: "Advance the world clock by N time units (default: advance_per_turn). Checks and triggers any time-based world events.",
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
            let sid = optional_session_id(&params)?;
            let advance_by = params.get("advance_by").and_then(Value::as_u64);

            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

            let state_boundary = state_lock(cid.as_str());
            let _state_guard = state_boundary.lock().expect("state lock poisoned");

            let (clock, triggered) = advance_and_check_triggers(
                &state.data_root,
                cid.as_str(),
                &session_dir,
                advance_by,
            )?;

            Ok(ToolResult {
                output: serde_json::json!({
                    "current_time": clock.current_time,
                    "display": clock.display(),
                    "time_unit": clock.time_unit,
                    "triggered_events": triggered.iter().map(|e| &e.name).collect::<Vec<_>>(),
                }),
                dry_run: false,
            })
        })
    }
}

/// `get_clock`：读取当前世界时钟状态。
struct GetClockTool {
    state: Arc<DaemonState>,
}

impl Tool for GetClockTool {
    fn meta(&self) -> ToolMeta {
        ToolMeta {
            name: "get_clock",
            description: "Get the current world clock state for a character.",
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
            let clock = load_world_clock(&state.data_root, cid.as_str())?;
            Ok(ToolResult {
                output: serde_json::json!({
                    "current_time": clock.current_time,
                    "display": clock.display(),
                    "time_unit": clock.time_unit,
                    "advance_per_turn": clock.advance_per_turn,
                }),
                dry_run: false,
            })
        })
    }
}
