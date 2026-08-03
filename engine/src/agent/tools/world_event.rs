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
//!   追加在 current.md 中交错混合叙事内容。
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
                .ok_or_else(|| AirpError::BadRequest("event_id is required".to_string()))?;
            let sid = optional_session_id(&params)?;

            // session_dir 解析（无锁，仅路径计算 + 目录确保），与 advance_clock 同模式
            // 在两段临界区外完成，避免在锁内做无关 I/O。
            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

            // character_lock.read() 跨两段临界区持有（R1 外层门控），防止
            // delete_character 在事件标记 / append 期间删除 character 目录。
            // RwLock read 共享，不阻塞其他 reader / 不与 advance_plot 的
            // session→character.read 形成反向环（character.read 是共享读）。
            // 早期 return（事件已 triggered）时 guard 由 Drop 自动释放。
            //
            // LOCK-ORDER: character.read → [阶段一 state] → [阶段二 session]（§2.4 / R1 / R2）。
            // 合同：docs/LOCK-ORDER-CONTRACT.md §2.4 / §3 R1 / §3 R2 / §4 A1 / §4 A3。
            let character = character_lock(cid.as_str());
            let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());

            // 阶段一：state_lock 临界区——load + check + mark + save。
            // 返回 (event, content_buf) 给阶段二使用。若事件已 triggered，
            // 直接返回 success:false（无需 append）。
            //
            // 审计 Bug F（死锁）修复：旧实现在此函数内同时持有 state_lock 与
            // session_lock（state → session 顺序），与 `advance_plot` 的
            // session → state 顺序（plot.rs 持 session_lock 后经
            // StateService::mutate 持 state_lock）形成锁序倒置死锁：
            //   线程 A (advance_plot):    hold session_lock → wait state_lock
            //   线程 B (trigger_world_event): hold state_lock → wait session_lock
            // 两者并发时永久阻塞。修复方式与 advance_and_check_triggers 一致：
            // 拆分为两段独立临界区，state_lock 在阶段一末尾释放，阶段二才获取
            // session_lock，同一调用任意时刻只持有一把锁。
            //
            // LOCK-ORDER: 两段临界区（§2.4 / R2）。阶段一持 state_lock，阶段二持 session_lock，
            // 绝不嵌套。与 advance_plot 的 session→state 单向嵌套不形成环。
            // 合同：docs/LOCK-ORDER-CONTRACT.md §2.4 / §3 R2 / §4 A1 / §4 A3。
            let (event, content_buf) = {
                let state_boundary = state_lock(cid.as_str());
                let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
                let _state_track = lock_order::track_state();

                let svc = WorldEventService::new(&state.data_root);
                let mut events = svc.load_events(cid.as_str())?;
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

                // 先标记 triggered 并持久化（在 state_lock 内），再构造 content_buf
                // 交给阶段二的 session_lock 临界区 append。
                // 顺序权衡（save → append）：若 append 失败，事件已标记 triggered
                // （不会重触发），内容未注入——失败对调用方可见（Err），不会静默累积
                // 重复内容。内容丢失比静默重复更可控：用户可见错误，可手动重置 triggered。
                events[event_idx].triggered = true;
                svc.save_events(cid.as_str(), &events)?;

                let content_buf = format!("\n[世界事件: {}]\n{}\n", event.name, event.content);
                (event, content_buf)
            };
            // state_lock 在此处释放，避免与 session_lock 形成锁序倒置死锁。

            // 阶段二：session_lock 临界区——append 内容到 current.md。
            // 与 npc_action / advance_plot / seal_volume 共享同一把 per-session 锁，
            // 防止并发追加在 current.md 中交错混合叙事内容。
            {
                let session_boundary = session_lock(cid.as_str(), sid.as_ref());
                let _session_guard = session_boundary.lock().unwrap_or_else(|p| p.into_inner());
                let _session_track = lock_order::track_session();
                crate::volume_store::append_to_current(&session_dir, &content_buf)?;
            }

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
            let events = WorldEventService::new(&state.data_root).load_events(cid.as_str())?;
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

            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

            // character_lock.read() 跨两段临界区持有（R1 外层门控），与
            // trigger_world_event 同模式。防止 delete_character 在时钟推进 /
            // 事件 append 期间删除 character 目录。
            //
            // LOCK-ORDER: character.read → [阶段一 state] → [阶段二 session]（§2.5 / R1 / R2）。
            // 合同：docs/LOCK-ORDER-CONTRACT.md §2.5 / §3 R1 / §3 R2 / §4 A1 / §4 A3。
            let character = character_lock(cid.as_str());
            let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());

            // 阶段一：推进时钟 + 收集/标记到期事件（state_lock 临界区）。
            // 持有 state_lock 直到 save_world_events 完成，与
            // update_relationship / advance_plot / trigger_world_event 共享
            // 同一把锁，杜绝 world_clock.json / world_events.json 的
            // read-modify-write 丢更新。
            //
            // LOCK-ORDER: 两段临界区（§2.5 / R2），与 trigger_world_event 同模式。
            // 阶段一持 state_lock，阶段二持 session_lock，绝不嵌套。
            // 合同：docs/LOCK-ORDER-CONTRACT.md §2.5 / §3 R2 / §4 A1 / §4 A3。
            let (clock, triggered, content_buf) = {
                let state_boundary = state_lock(cid.as_str());
                let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
                let _state_track = lock_order::track_state();
                WorldEventService::new(&state.data_root)
                    .advance_and_check_triggers(cid.as_str(), advance_by)?
            };
            // state_lock 在此处释放，避免与 session_lock 形成锁序倒置死锁。

            // 阶段二：将到期事件内容追加到 current.md（session_lock 临界区）。
            // 与 npc_action / advance_plot / trigger_world_event 共享同一把
            // per-session 锁，防止并发追加在 current.md 中交错混合叙事内容。
            // 审计 Bug B 修复：旧实现未持有 session_lock 就调用
            // append_to_current，允许并发 npc_action / advance_plot 的 append
            // 与此处的 append 在 current.md 中交错。
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
            let clock = WorldEventService::new(&state.data_root).load_clock(cid.as_str())?;
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
