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
//! - `save_world_events` 改用 `data_dir::replace_file` 原子写，避免半写
//!   状态被其他读者看到；并在写入前 `fsync` 父目录（`replace_file` 内置）。
//! - `load_world_events` 的 JSON parse 错误原本通过 `?` 上抛（行为正确），
//!   本审计未改动其错误传播策略，仅修复写路径。
//!
//! 注：world_events.json 已接入 #115 Phase 2e revision 合同（#280）。
//! asset_dir = `characters/{id}/world_events/`，批准文件 = `world_events.json`。
//! 工作副本 `characters/{id}/world_events.json` 通过 `data_dir::replace_file` 原子写，
//! revision 快照通过 `commit_revision` 写入 `characters/{id}/world_events/revisions/{n}/`。

use super::params::{optional_session_id, required_character_id};
use super::*;
use crate::daemon::DaemonState;
use crate::domain::{session_lock, state_lock};
use crate::error::AirpError;
use crate::revision::atomic::{
    commit_revision, next_content_revision, read_current_revision, CommitOptions, StagedRevision,
};
use crate::revision::manifest::{AssetKind, AssetSource};
use serde_json::Value;
use std::future::Future;
use std::path::Path;
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

/// #280: world_events 的 revision asset_dir，与工作副本 `world_events.json` 分离。
/// 参考 SoulDrift 模式（`characters/{id}/soul_drift/`），避免 `characters/{id}/revisions/`
/// 与 character card 的 revisions 混淆。
fn world_events_asset_dir(data_root: &Path, character_id: &str) -> std::path::PathBuf {
    data_root
        .join("characters")
        .join(character_id)
        .join("world_events")
}

/// #280: 将 world_events 写入 revision 快照目录。
/// 参考 `commit_soul_drift_unlocked` 模式：构造 `StagedRevision` + `CommitOptions`，
/// 调用统一 `commit_revision` 入口。
fn commit_world_events_revision(
    data_root: &Path,
    character_id: &str,
    content: &[u8],
    source_kind: &str,
    parent_revision: Option<u64>,
) -> Result<u64, AirpError> {
    let asset_dir = world_events_asset_dir(data_root, character_id);
    let revision = next_content_revision(&asset_dir)?;
    let staged = StagedRevision {
        content_revision: revision,
        asset_kind: AssetKind::WorldEvents,
        asset_id: character_id.to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        source: AssetSource {
            source_kind: source_kind.to_string(),
            parent_revision,
            ..Default::default()
        },
        files: vec![("world_events.json".to_string(), content.to_vec())],
    };
    commit_revision(&staged, &CommitOptions::new(asset_dir))?;
    Ok(revision)
}

/// #280: legacy migration——若 `world_events.json` 存在但无 revision 快照，
/// 首次 commit 为 revision 1。参考 `ensure_legacy_revision_unlocked`（SoulDrift）。
/// 调用方已在 `state_lock` 临界区内，无需额外加锁。
///
/// 返回值：
/// - `Ok(Some(rev))`：已有 revision 或刚 migration 产生的 revision
/// - `Ok(None)`：无 legacy 文件，调用方应将 parent_revision 视为 None
fn ensure_legacy_world_events_revision(
    data_root: &Path,
    character_id: &str,
) -> Result<Option<u64>, AirpError> {
    let asset_dir = world_events_asset_dir(data_root, character_id);
    if let Some(existing) = read_current_revision(&asset_dir)? {
        return Ok(Some(existing));
    }
    let path = world_events_path(data_root, character_id);
    if !path.exists() {
        return Ok(None);
    }
    let legacy = std::fs::read(&path)?;
    let rev =
        commit_world_events_revision(data_root, character_id, &legacy, "legacy_migration", None)?;
    Ok(Some(rev))
}

/// 读取世界事件列表。不存在返回空 Vec。
///
/// Phase 4.5：本函数从私有提升为 `pub(crate)`，供 `timeline_export` 模块复用
/// （避免在 timeline_export 中重复实现 world_events.json 的读取与解析）。
pub(crate) fn load_world_events(
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

    // #280: 接入 revision 合同。顺序（CodeRabbit review 修正）：
    // 1. legacy migration 必须在 replace_file 之前，确保读到的是真正的 legacy 内容
    //    而非刚写入的新内容；返回值复用为 parent_revision，避免重复 read_current_revision
    // 2. commit_revision 写不可变快照（在暴露工作副本之前）
    // 3. replace_file 写工作副本（commit 成功后才暴露给并发 reader）
    //
    // 若 commit 失败，工作副本未被修改，不会产生"工作副本已更新但无 revision 快照"的不一致。
    let parent_revision = ensure_legacy_world_events_revision(data_root, character_id)?;
    commit_world_events_revision(
        data_root,
        character_id,
        &content,
        "tool_triggered",
        parent_revision,
    )?;

    // 原子写工作副本：替换旧版 std::fs::write，避免半写状态被并发 reader 看到。
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

            // session_dir 解析（无锁，仅路径计算 + 目录确保），与 advance_clock 同模式
            // 在两段临界区外完成，避免在锁内做无关 I/O。
            let session_dir =
                crate::data_dir::resolve_session_dir(&state.data_root, cid.as_str(), sid.as_ref())?;

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
            let (event, content_buf) = {
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

                // 先标记 triggered 并持久化（在 state_lock 内），再构造 content_buf
                // 交给阶段二的 session_lock 临界区 append。
                // 顺序权衡（save → append）：若 append 失败，事件已标记 triggered
                // （不会重触发），内容未注入——失败对调用方可见（Err），不会静默累积
                // 重复内容。内容丢失比静默重复更可控：用户可见错误，可手动重置 triggered。
                events[event_idx].triggered = true;
                save_world_events(&state.data_root, cid.as_str(), &events)?;

                let content_buf = format!("\n[世界事件: {}]\n{}\n", event.name, event.content);
                (event, content_buf)
            };
            // state_lock 在此处释放，避免与 session_lock 形成锁序倒置死锁。

            // 阶段二：session_lock 临界区——append 内容到 current.md。
            // 与 npc_action / advance_plot / seal_volume 共享同一把 per-session 锁，
            // 防止并发追加在 current.md 中交错混合叙事内容。
            {
                let session_boundary = session_lock(cid.as_str(), sid.as_ref());
                let _session_guard = session_boundary.lock().expect("session lock poisoned");
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

/// 推进时钟并检查时间触发事件。返回触发的事件列表与待追加到 `current.md`
/// 的内容缓冲。调用方负责在 `session_lock` 临界区内执行 `append_to_current`。
///
/// 不在此函数内执行 `append_to_current` 的原因：append 必须持有
/// `session_lock(character_id, session_id)`，而时钟/事件状态变更必须持有
/// `state_lock(character_id)`。若同一调用同时持有两把锁（state → session），
/// 会与 `advance_plot`（session → state，经 `StateService::mutate`）形成
/// 锁序倒置死锁。因此将状态变更（state_lock）与内容追加（session_lock）
/// 拆分到两个临界区，由调用方分别加锁。
pub fn advance_and_check_triggers(
    data_root: &std::path::Path,
    character_id: &str,
    advance_by: Option<u64>,
) -> Result<(WorldClock, Vec<WorldEvent>, String), AirpError> {
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

    // 先收集所有到期事件，构造单次追加内容。
    // 旧实现逐事件 append_to_current：若事件 1 append 成功但事件 2 append
    // 失败，事件 1 内容已落盘但 triggered 标志未持久化（save_world_events
    // 在循环外），下次 advance_clock 重试会重复注入事件 1 内容到 current.md。
    // 新实现：批量收集 → 标记 + 持久化 → 由调用方在 session_lock 下单次 append。
    // 顺序权衡（save → append）：若 append 失败，事件已标记 triggered（不会
    // 重触发），内容未注入——失败对调用方可见（Err），不会静默累积重复内容。
    let mut due_indices: Vec<usize> = Vec::new();
    let mut content_buf = String::new();
    for (idx, event) in events.iter().enumerate() {
        if !event.triggered {
            if let Some(time_trigger) = event.time_trigger {
                if clock.current_time >= time_trigger {
                    content_buf.push_str(&format!(
                        "\n[世界事件: {}]\n{}\n",
                        event.name, event.content
                    ));
                    due_indices.push(idx);
                }
            }
        }
    }

    if !due_indices.is_empty() {
        for &idx in &due_indices {
            events[idx].triggered = true;
            triggered_events.push(events[idx].clone());
        }
        save_world_events(data_root, character_id, &events)?;
    }
    Ok((clock, triggered_events, content_buf))
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

            // 阶段一：推进时钟 + 收集/标记到期事件（state_lock 临界区）。
            // 持有 state_lock 直到 save_world_events 完成，与
            // update_relationship / advance_plot / trigger_world_event 共享
            // 同一把锁，杜绝 world_clock.json / world_events.json 的
            // read-modify-write 丢更新。
            let (clock, triggered, content_buf) = {
                let state_boundary = state_lock(cid.as_str());
                let _state_guard = state_boundary.lock().expect("state lock poisoned");
                advance_and_check_triggers(&state.data_root, cid.as_str(), advance_by)?
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
                let _session_guard = session_boundary.lock().expect("session lock poisoned");
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
