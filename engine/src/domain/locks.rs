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

/// #422: 清理 `CHARACTER_LOCKS` 中已删除 character 的 stale 条目。
///
/// `delete_character` durable 完成后调用。正在等待旧 Arc 的 waiter 拿到锁后
/// 操作已删除资源会 fail closed（NotFound）；新 caller 调 `character_lock` 会
/// 创建新 Arc 走正常 create 流程。与 `remove_deleted_session_lock` 同模式。
pub(super) fn remove_deleted_character_lock(character_id: &str) {
    let Some(lock_map) = CHARACTER_LOCKS.get() else {
        return;
    };
    let mut locks = lock_map.lock().unwrap_or_else(|p| p.into_inner());
    locks.remove(character_id);
}

/// #422: 清理 `STATE_LOCKS` 中已删除 character 的 stale 条目。
///
/// `delete_character` 删整个 `characters/{id}/` 目录（含 `state/live.json`），
/// 该 character 的 state lock 条目随之 stale。与 character lock 同时机清理。
pub(super) fn remove_deleted_state_lock(character_id: &str) {
    let Some(lock_map) = STATE_LOCKS.get() else {
        return;
    };
    let mut locks = lock_map.lock().unwrap_or_else(|p| p.into_inner());
    locks.remove(character_id);
}

/// #422: 清理 `PERSONA_LOCKS` 中已删除 persona 的 stale 条目。
///
/// `PersonaService::delete` durable 完成后调用。新 caller 调 `persona_lock`
/// 会创建新 Arc 走正常 create/lookup 流程。
pub(super) fn remove_deleted_persona_lock(user_id: &str) {
    let Some(lock_map) = PERSONA_LOCKS.get() else {
        return;
    };
    let mut locks = lock_map.lock().unwrap_or_else(|p| p.into_inner());
    locks.remove(user_id);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// #422: lock map 是 process-global static，测试间共享。用唯一 id 前缀
    /// 避免与其他测试用例冲突。验证清理后 `*_lock` 返回新 Arc（条目已移除）。
    fn unique_id(label: &str) -> String {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        format!(
            "#422-test-{label}-{}",
            COUNTER.fetch_add(1, Ordering::SeqCst)
        )
    }

    #[test]
    fn remove_deleted_character_lock_clears_entry() {
        let cid = unique_id("char");
        let arc_before = character_lock(&cid);
        remove_deleted_character_lock(&cid);
        let arc_after = character_lock(&cid);
        assert!(
            !Arc::ptr_eq(&arc_before, &arc_after),
            "character lock entry should be removed so a new Arc is created"
        );
    }

    #[test]
    fn remove_deleted_state_lock_clears_entry() {
        let cid = unique_id("state");
        let arc_before = state_lock(&cid);
        remove_deleted_state_lock(&cid);
        let arc_after = state_lock(&cid);
        assert!(
            !Arc::ptr_eq(&arc_before, &arc_after),
            "state lock entry should be removed so a new Arc is created"
        );
    }

    #[test]
    fn remove_deleted_persona_lock_clears_entry() {
        let uid = unique_id("persona");
        let arc_before = persona_lock(&uid);
        remove_deleted_persona_lock(&uid);
        let arc_after = persona_lock(&uid);
        assert!(
            !Arc::ptr_eq(&arc_before, &arc_after),
            "persona lock entry should be removed so a new Arc is created"
        );
    }

    #[test]
    fn remove_deleted_lock_is_noop_when_map_uninit() {
        // Map 未初始化时清理函数应安全 no-op（get() 返回 None）。
        // 无法保证 static 未初始化（其他测试可能已触发），但函数本身
        // 对已初始化 map 调用不存在 key 也是 no-op，这里验证不 panic。
        remove_deleted_character_lock("#422-test-nonexistent-char");
        remove_deleted_state_lock("#422-test-nonexistent-state");
        remove_deleted_persona_lock("#422-test-nonexistent-persona");
    }
}
