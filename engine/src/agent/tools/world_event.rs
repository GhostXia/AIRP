//! World event family: 世界事件触发器（3.1）。
//!
//! 工具清单：
//! - `trigger_world_event`：触发预设世界事件，注入到叙事上下文（mutate）
//! - `list_world_events`：列出角色可用的世界事件（readonly）
//!
//! 事件定义存储在 `characters/{id}/world_events.json`。
//! 事件注入走 volume_store::append_to_current（不新增注入路径）。
//!
//! 并发纪律（PR #272 审计修复 + CodeRabbit 跟进 + Bug F 死锁修复）：
//! - `trigger_world_event` 的 check-then-act（读 `triggered` → 注入 → 标记）
//!   原本无锁，并发触发同一 event_id 会双重注入 current.md。修复方式为
//!   两段独立临界区：阶段一在 `state_lock(character_id)` 内 load + check +
//!   mark + save；阶段二在 `session_lock(character_id, session_id)` 内
//!   append。同一调用任意时刻只持有一把锁——这是 Bug F（死锁）修复的关键：
//!   旧实现同时持有 state_lock + session_lock（state→session 顺序）与
//!   `advance_plot` 的 session→state 顺序形成锁序倒置死锁。
//! - 事件注入到 current.md 走 `volume_store::append_to_current`，调用前
//!   显式持有 `session_lock(character_id, session_id)`，与 `npc_action` /
//!   `advance_plot` / `seal_volume` 共享同一把 per-session 锁，防止并发
//!   追加在 current.md 中交错混合叙事内容。同步锁与文件操作在
//!   `spawn_blocking` 中执行，避免占用 tokio worker。
//!
//! 注：world_events.json 已接入 #115 Phase 2e revision 合同（#280）。
//! asset_dir = `characters/{id}/world_events/`，批准文件 = `world_events.json`。
//!
//! E-P1-3 slice 1：写路径收口至 `domain::WorldEventService`，本模块不再直接
//! 调用 `replace_file` / `fs::write`。类型 `WorldEvent` / `WorldClock` 已迁移
//! 至 `domain::world_event`，经 `pub use` 重新导出保持 API 不变。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::domain::{character_lock, lock_order, session_lock, state_lock, WorldEventService};
use crate::error::AirpError;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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
                .ok_or_else(|| AirpError::BadRequest("event_id is required".to_string()))?
                .to_string();
            let sid = optional_session_id(&params)?;

            let data_root = state.data_root.clone();
            tokio::task::spawn_blocking(move || -> Result<ToolResult, AirpError> {
                let session_dir =
                    crate::data_dir::resolve_session_dir(&data_root, cid.as_str(), sid.as_ref())?;

                // character_lock.read() 跨两段临界区持有（R1）作为目录生命周期门控。
                // 整个同步流程在 blocking pool 中执行，std guards 不跨 await。
                let character = character_lock(cid.as_str());
                let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
                let _character_track = lock_order::track_character_read();

                // 阶段一：state_lock 内 load + check + mark + save；结束后才获取
                // session_lock，避免 state→session 与其他路径的 session→state 形成环。
                let (event, content_buf) = {
                    let state_boundary = state_lock(cid.as_str());
                    let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
                    let _state_track = lock_order::track_state();

                    let svc = WorldEventService::new(&data_root);
                    let mut events = svc.load_events(cid.as_str())?;
                    let event_idx =
                        events
                            .iter()
                            .position(|e| e.id == event_id)
                            .ok_or_else(|| {
                                AirpError::NotFound(format!("event {} not found", event_id))
                            })?;

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
                    events[event_idx].triggered = true;
                    svc.save_events(cid.as_str(), &events)?;
                    let content_buf = format!("\n[世界事件: {}]\n{}\n", event.name, event.content);
                    (event, content_buf)
                };

                // 阶段二：session_lock 内追加到 current.md。
                let session_boundary = session_lock(cid.as_str(), sid.as_ref());
                let _session_guard = session_boundary.lock().unwrap_or_else(|p| p.into_inner());
                let _session_track = lock_order::track_session();
                crate::volume_store::append_to_current(&session_dir, &content_buf)?;

                Ok(ToolResult {
                    output: serde_json::json!({
                        "success": true,
                        "event": event
                    }),
                    dry_run: false,
                })
            })
            .await
            .map_err(|e| {
                AirpError::Internal(format!("trigger_world_event blocking task failed: {e}"))
            })?
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
            let data_root = state.data_root.clone();
            tokio::task::spawn_blocking(move || -> Result<ToolResult, AirpError> {
                let events = WorldEventService::new(&data_root).load_events(cid.as_str())?;
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
            .await
            .map_err(|e| {
                AirpError::Internal(format!("list_world_events blocking task failed: {e}"))
            })?
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
// WorldClock 类型与 load/save/advance_and_check_triggers 逻辑已提取至
// `domain::world_event::WorldEventService`（E-P1-3 slice 1）。本模块仅保留
// Tool 入口，不再直接操作文件系统。

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

            let data_root = state.data_root.clone();
            tokio::task::spawn_blocking(move || -> Result<ToolResult, AirpError> {
                let session_dir =
                    crate::data_dir::resolve_session_dir(&data_root, cid.as_str(), sid.as_ref())?;

                // character_lock.read() 跨两段临界区持有（R1）；同步锁与文件 I/O
                // 均在 blocking pool 中执行，锁序为 character.read → [state] → [session]。
                let character = character_lock(cid.as_str());
                let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
                let _character_track = lock_order::track_character_read();

                let (clock, triggered, content_buf) = {
                    let state_boundary = state_lock(cid.as_str());
                    let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
                    let _state_track = lock_order::track_state();
                    WorldEventService::new(&data_root)
                        .advance_and_check_triggers(cid.as_str(), advance_by)?
                };

                if !content_buf.is_empty() {
                    let session_boundary = session_lock(cid.as_str(), sid.as_ref());
                    let _session_guard = session_boundary.lock().unwrap_or_else(|p| p.into_inner());
                    let _session_track = lock_order::track_session();
                    crate::volume_store::append_to_current(&session_dir, &content_buf)?;
                }

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
            .await
            .map_err(|e| AirpError::Internal(format!("advance_clock blocking task failed: {e}")))?
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
            let data_root = state.data_root.clone();
            tokio::task::spawn_blocking(move || -> Result<ToolResult, AirpError> {
                let clock = WorldEventService::new(&data_root).load_clock(cid.as_str())?;
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
            .await
            .map_err(|e| AirpError::Internal(format!("get_clock blocking task failed: {e}")))?
        })
    }
}
