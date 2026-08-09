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

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestCharacterLockKind {
    Read,
    Write,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TestCharacterLockPhase {
    Acquired,
    Released,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TestCharacterLockEvent {
    pub(crate) character_id: String,
    pub(crate) kind: TestCharacterLockKind,
    pub(crate) phase: TestCharacterLockPhase,
}

#[cfg(test)]
struct TestCharacterLockProbe {
    character_id: String,
    kind: TestCharacterLockKind,
}

#[cfg(test)]
impl Drop for TestCharacterLockProbe {
    fn drop(&mut self) {
        record_test_character_lock_event(
            &self.character_id,
            self.kind,
            TestCharacterLockPhase::Released,
        );
    }
}

#[cfg(test)]
pub(crate) struct TestCharacterReadGuard<'a> {
    guard: Option<std::sync::RwLockReadGuard<'a, ()>>,
    probe: Option<TestCharacterLockProbe>,
}

#[cfg(test)]
impl Drop for TestCharacterReadGuard<'_> {
    fn drop(&mut self) {
        // Record release while the real guard is still held. A writer can only
        // report acquisition after this event, because the underlying RwLock
        // remains held until the next statement.
        drop(self.probe.take());
        drop(self.guard.take());
    }
}

#[cfg(test)]
pub(crate) struct TestCharacterWriteGuard<'a> {
    guard: Option<std::sync::RwLockWriteGuard<'a, ()>>,
    probe: Option<TestCharacterLockProbe>,
}

#[cfg(test)]
impl Drop for TestCharacterWriteGuard<'_> {
    fn drop(&mut self) {
        drop(self.probe.take());
        drop(self.guard.take());
    }
}

#[cfg(test)]
static TEST_CHARACTER_LOCK_EVENTS: OnceLock<Mutex<Vec<TestCharacterLockEvent>>> = OnceLock::new();

#[cfg(test)]
fn record_test_character_lock_event(
    character_id: &str,
    kind: TestCharacterLockKind,
    phase: TestCharacterLockPhase,
) {
    TEST_CHARACTER_LOCK_EVENTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .push(TestCharacterLockEvent {
            character_id: character_id.to_string(),
            kind,
            phase,
        });
}

#[cfg(test)]
pub(crate) fn take_test_character_lock_events(character_id: &str) -> Vec<TestCharacterLockEvent> {
    let mut events = TEST_CHARACTER_LOCK_EVENTS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let selected = events
        .iter()
        .filter(|event| event.character_id == character_id)
        .cloned()
        .collect();
    events.retain(|event| event.character_id != character_id);
    selected
}

#[cfg(test)]
struct TestCharacterLockGateControl {
    read_acquired: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    read_release: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
    write_attempted: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

#[cfg(test)]
pub(crate) struct TestCharacterLockGate {
    character_id: String,
    release: Option<tokio::sync::oneshot::Sender<()>>,
}

#[cfg(test)]
static TEST_CHARACTER_LOCK_GATES: OnceLock<
    Mutex<HashMap<String, Arc<TestCharacterLockGateControl>>>,
> = OnceLock::new();

#[cfg(test)]
pub(crate) fn install_test_character_lock_gate(
    character_id: &str,
) -> (
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Receiver<()>,
    TestCharacterLockGate,
) {
    let (read_acquired_tx, read_acquired_rx) = tokio::sync::oneshot::channel();
    let (read_release_tx, read_release_rx) = tokio::sync::oneshot::channel();
    let (write_attempted_tx, write_attempted_rx) = tokio::sync::oneshot::channel();
    let control = Arc::new(TestCharacterLockGateControl {
        read_acquired: Mutex::new(Some(read_acquired_tx)),
        read_release: Mutex::new(Some(read_release_rx)),
        write_attempted: Mutex::new(Some(write_attempted_tx)),
    });
    TEST_CHARACTER_LOCK_GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(character_id.to_string(), control);
    (
        read_acquired_rx,
        write_attempted_rx,
        TestCharacterLockGate {
            character_id: character_id.to_string(),
            release: Some(read_release_tx),
        },
    )
}

#[cfg(test)]
impl Drop for TestCharacterLockGate {
    fn drop(&mut self) {
        // Always unblock a read helper if the test unwinds before its normal
        // release point. The control entry is removed after the sender is
        // dropped so no later test can accidentally reuse this gate.
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(gates) = TEST_CHARACTER_LOCK_GATES.get() {
            gates
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(&self.character_id);
        }
    }
}

#[cfg(test)]
fn test_character_lock_gate_for(character_id: &str) -> Option<Arc<TestCharacterLockGateControl>> {
    TEST_CHARACTER_LOCK_GATES.get().and_then(|gates| {
        gates
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(character_id)
            .cloned()
    })
}

#[cfg(test)]
pub(crate) fn test_character_read_guard<'a>(
    character_id: &str,
    lock: &'a Arc<RwLock<()>>,
) -> TestCharacterReadGuard<'a> {
    let guard = lock.read().unwrap_or_else(|poisoned| poisoned.into_inner());
    record_test_character_lock_event(
        character_id,
        TestCharacterLockKind::Read,
        TestCharacterLockPhase::Acquired,
    );
    if let Some(control) = test_character_lock_gate_for(character_id) {
        let _ = control
            .read_acquired
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .and_then(|sender| sender.send(()).ok());
        if let Some(receiver) = control
            .read_release
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
        {
            let _ = receiver.blocking_recv();
        }
    }
    TestCharacterReadGuard {
        guard: Some(guard),
        probe: Some(TestCharacterLockProbe {
            character_id: character_id.to_string(),
            kind: TestCharacterLockKind::Read,
        }),
    }
}

#[cfg(test)]
pub(crate) fn test_character_write_guard<'a>(
    character_id: &str,
    lock: &'a Arc<RwLock<()>>,
) -> TestCharacterWriteGuard<'a> {
    if let Some(control) = test_character_lock_gate_for(character_id) {
        let _ = control
            .write_attempted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
            .and_then(|sender| sender.send(()).ok());
    }
    let guard = lock
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    record_test_character_lock_event(
        character_id,
        TestCharacterLockKind::Write,
        TestCharacterLockPhase::Acquired,
    );
    TestCharacterWriteGuard {
        guard: Some(guard),
        probe: Some(TestCharacterLockProbe {
            character_id: character_id.to_string(),
            kind: TestCharacterLockKind::Write,
        }),
    }
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

/// #435: 清理 `SESSION_LOCKS` 中 default-session 的 stale 条目。
///
/// `session_lock(character_id, None)` 的键为 `character_id` 本身（无 `/session_id`
/// 后缀），`remove_deleted_session_lock` 无法触达。`delete_character` 删除整个
/// character 目录后必须单独清理此条目，避免内存泄漏与 ghost default-session lock。
pub(super) fn remove_deleted_default_session_lock(character_id: &str) {
    let Some(lock_map) = SESSION_LOCKS.get() else {
        return;
    };
    let mut locks = lock_map.lock().unwrap_or_else(|p| p.into_inner());
    locks.remove(character_id);
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

    /// #435: `delete_character` 删除整个 `characters/{id}/` 目录（含所有 sessions），
    /// 需批量清理该 character 下所有 session 的 SESSION_LOCKS 条目，避免内存泄漏
    /// 和 ghost session lock 复用。本测试验证逐个调用 `remove_deleted_session_lock`
    ///（`delete_character` 的实际清理路径）后，所有 session lock 条目均被移除。
    #[test]
    fn remove_deleted_session_locks_for_character_bulk_cleanup() {
        let cid = unique_id("bulk-char");
        let sid1 = SessionId::new();
        let sid2 = SessionId::new();
        let sid3 = SessionId::new();

        // 模拟 3 个活跃 session 的 lock（delete_character 前的状态）
        let arc1_before = session_lock(&cid, Some(&sid1));
        let arc2_before = session_lock(&cid, Some(&sid2));
        let arc3_before = session_lock(&cid, Some(&sid3));

        // delete_character 的清理路径：遍历已知 sessions 逐个清理
        for sid in [&sid1, &sid2, &sid3] {
            remove_deleted_session_lock(&cid, sid);
        }

        // 所有 session lock 条目应被移除：再次获取返回新 Arc（非 ptr_eq）
        let arc1_after = session_lock(&cid, Some(&sid1));
        let arc2_after = session_lock(&cid, Some(&sid2));
        let arc3_after = session_lock(&cid, Some(&sid3));

        assert!(
            !Arc::ptr_eq(&arc1_before, &arc1_after),
            "session lock for sid1 should be removed"
        );
        assert!(
            !Arc::ptr_eq(&arc2_before, &arc2_after),
            "session lock for sid2 should be removed"
        );
        assert!(
            !Arc::ptr_eq(&arc3_before, &arc3_after),
            "session lock for sid3 should be removed"
        );

        // 清理测试产生的条目，避免污染全局 lock map
        for sid in [&sid1, &sid2, &sid3] {
            remove_deleted_session_lock(&cid, sid);
        }
        remove_deleted_character_lock(&cid);
    }

    /// #435: `session_lock(cid, None)` 的键为 `cid` 本身，`remove_deleted_session_lock`
    /// 触达不到。`delete_character` 必须额外调 `remove_deleted_default_session_lock`，
    /// 否则 default-session lock 残留为 ghost，新 caller 复用旧 Arc 绕过串行化。
    #[test]
    fn remove_deleted_default_session_lock_cleanup() {
        let cid = unique_id("default-sess-char");

        // 创建 default-session lock
        let arc_before = session_lock(&cid, None);

        // delete_character 的清理路径
        remove_deleted_default_session_lock(&cid);

        // 再次获取应返回新 Arc（条目已移除）
        let arc_after = session_lock(&cid, None);
        assert!(
            !Arc::ptr_eq(&arc_before, &arc_after),
            "default-session lock should be removed"
        );

        // 清理测试产生的条目
        remove_deleted_default_session_lock(&cid);
        remove_deleted_character_lock(&cid);
    }

    /// #443: 正向对照——未调用 cleanup 时，多次获取同 key 返回同一 Arc。
    ///
    /// `remove_deleted_*_lock_clears_entry` 系列断言 cleanup 后 `!Arc::ptr_eq`，
    /// 但若 lock map 实现每次都创建新 Arc，`!Arc::ptr_eq` 永远成立、断言无意义。
    /// 本测试提供正向基线：未 cleanup 时 `Arc::ptr_eq` 必须成立，证明 cleanup 后
    /// 的 `!Arc::ptr_eq` 确实由 entry 移除导致，而非其他副作用。
    ///
    /// 覆盖 4 种 lock（character / state / persona / named session），与 issue
    /// #443 验收标准「同模式覆盖 delete_session / delete_persona」对齐。
    #[test]
    fn same_arc_returned_when_cleanup_not_called() {
        let cid = unique_id("pos-char");
        let uid = unique_id("pos-persona");
        let sid = SessionId::new();

        // character lock：两次获取应同一 Arc
        let char_a = character_lock(&cid);
        let char_b = character_lock(&cid);
        assert!(
            Arc::ptr_eq(&char_a, &char_b),
            "character lock should reuse entry when not cleaned up"
        );

        // state lock
        let state_a = state_lock(&cid);
        let state_b = state_lock(&cid);
        assert!(
            Arc::ptr_eq(&state_a, &state_b),
            "state lock should reuse entry when not cleaned up"
        );

        // persona lock
        let persona_a = persona_lock(&uid);
        let persona_b = persona_lock(&uid);
        assert!(
            Arc::ptr_eq(&persona_a, &persona_b),
            "persona lock should reuse entry when not cleaned up"
        );

        // named session lock
        let sess_a = session_lock(&cid, Some(&sid));
        let sess_b = session_lock(&cid, Some(&sid));
        assert!(
            Arc::ptr_eq(&sess_a, &sess_b),
            "session lock should reuse entry when not cleaned up"
        );

        // 清理测试产生的条目
        remove_deleted_character_lock(&cid);
        remove_deleted_state_lock(&cid);
        remove_deleted_persona_lock(&uid);
        remove_deleted_session_lock(&cid, &sid);
    }
}
