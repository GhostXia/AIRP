//! Per-resource lock maps used to serialize mutations across HTTP, Agent tools,
//! and chat pipelines.
//!
//! 这些锁表是 LOCK-ORDER-CONTRACT §1 的物理实现。获取顺序与嵌套规则见
//! `docs/LOCK-ORDER-CONTRACT.md` §3 R1–R6；运行时锁序追踪见 `lock_order` 模块。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::types::SessionId;

type SessionLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;
type CharacterLockMap = Mutex<HashMap<String, Arc<RwLock<()>>>>;
type StateLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;
type PersonaLockMap = Mutex<HashMap<String, Arc<Mutex<()>>>>;

static SESSION_LOCKS: OnceLock<SessionLockMap> = OnceLock::new();
static CHARACTER_LOCKS: OnceLock<CharacterLockMap> = OnceLock::new();
static STATE_LOCKS: OnceLock<StateLockMap> = OnceLock::new();
static PERSONA_LOCKS: OnceLock<PersonaLockMap> = OnceLock::new();

pub(crate) fn character_lock(character_id: &str) -> Arc<RwLock<()>> {
    // LOCK-ORDER: per-character 外层门控（R1）。获取 state_lock/session_lock 前必须先持此锁。
    // 合同：docs/LOCK-ORDER-CONTRACT.md §1.1 / §3 R1。
    let mut locks = CHARACTER_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    locks
        .entry(character_id.to_string())
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone()
}

/// Per-session state lock. Keyed on `character_id` (when `session_id` is
/// `None`) or `character_id/session_id`, used to serialize all mutations to
/// `session/current.md` and other per-session state files.
/// `pub(crate)` so sibling modules (agent::tools::npc / plot / world_event)
/// can participate in the same serialization contract when calling
/// `volume_store::append_to_current`, preventing concurrent appends from
/// interleaving narrative content in `current.md`.
pub(crate) fn session_lock(character_id: &str, session_id: Option<&SessionId>) -> Arc<Mutex<()>> {
    // LOCK-ORDER: per-session 叙事文件串行化。与 state_lock 唯一合法嵌套方向为
    // session → state（仅 advance_plot 经 StateService::mutate），反向禁止（R2）。
    // 合同：docs/LOCK-ORDER-CONTRACT.md §1.1 / §3 R2 / §2.3。
    let key = match session_id {
        Some(session_id) => format!("{character_id}/{session_id}"),
        None => character_id.to_string(),
    };
    let mut locks = SESSION_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    locks
        .entry(key)
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

pub(super) fn remove_deleted_session_lock(character_id: &str, session_id: &SessionId) {
    let Some(lock_map) = SESSION_LOCKS.get() else {
        return;
    };
    let key = format!("{character_id}/{session_id}");
    let mut locks = lock_map.lock().unwrap_or_else(|p| p.into_inner());
    // The tombstone is durable before this runs, so every waiter or future
    // caller will fail closed even if it holds/creates a different lock Arc.
    locks.remove(&key);
}

/// Per-character state lock. Keyed on `character_id`, used to serialize all
/// mutations to `state/live.json` and other per-character state files
/// (e.g. `world_events.json`). `pub(crate)` so sibling modules
/// (agent::tools::world_event) can participate in the same serialization
/// contract without re-implementing the lock map.
pub(crate) fn state_lock(character_id: &str) -> Arc<Mutex<()>> {
    // LOCK-ORDER: per-character 状态文件串行化。与 session_lock 唯一合法嵌套方向为
    // session → state（仅 advance_plot），反向禁止（R2）。trigger_world_event /
    // advance_clock 采用两段临界区，绝不嵌套。
    // 合同：docs/LOCK-ORDER-CONTRACT.md §1.1 / §3 R2 / §2.4 / §2.5。
    let mut locks = STATE_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    locks
        .entry(character_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Per-user persona lock（串行化 persona 写入与 revision bump）。
///
/// LOCK-ORDER: 独立锁族，按 `user_id` key，不与 character 锁族嵌套。
/// 合同：docs/LOCK-ORDER-CONTRACT.md §1.4 / §3 R5。
pub(super) fn persona_lock(user_id: &str) -> Arc<Mutex<()>> {
    let mut locks = PERSONA_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    locks
        .entry(user_id.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}
