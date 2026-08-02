//! 运行时锁序追踪（LOCK-ORDER-CONTRACT §6.1）。
//!
//! 在 debug build 下用 thread-local 栈记录当前线程持有的 `session_lock` /
//! `state_lock`，检测 R2 禁止的 `state → session` 反向获取（Bug F 类死锁回归）。
//! `session → state`（仅 `advance_plot` 经 `StateService::mutate`）是 R2 唯一合法
//! 嵌套方向，不触发。
//!
//! release build (`--release`) 下 `track_*` 返回零成本 no-op `Guard`，满足
//! LOCK-ORDER-CONTRACT §7「release build 零开销」。
//!
//! 约束：std `Mutex` guard 不得跨 `.await`（§4 A1），因此 thread-local 栈只在一
//! 个同步作用域内有效；async fn 临界区是纯同步代码，guard 在作用域末尾 Drop。
//!
//! 合同：docs/LOCK-ORDER-CONTRACT.md §6.1 / §3 R2 / §4 A1 / §7。

// ── debug build ─────────────────────────────────────────────────────────────
#[cfg(debug_assertions)]
use std::cell::RefCell;

#[cfg(debug_assertions)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Session,
    State,
}

#[cfg(debug_assertions)]
thread_local! {
    static HELD: RefCell<Vec<Kind>> = const { RefCell::new(Vec::new()) };
}

/// RAII guard：构造时 push，Drop 时 pop。debug-only。
#[cfg(debug_assertions)]
#[must_use = "Guard tracks a held lock until dropped; bind it to a named variable for the whole critical section"]
pub(crate) struct Guard(Option<Kind>);

#[cfg(debug_assertions)]
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(kind) = self.0.take() {
            HELD.with(|held| {
                let mut held = held.borrow_mut();
                // LIFO 弹出栈顶匹配项；guard 顺序错乱时也安全移除一项。
                if let Some(pos) = held.iter().rposition(|k| *k == kind) {
                    held.remove(pos);
                }
            });
        }
    }
}

/// 记录已持有 `session_lock`。R2：持 `state_lock` 时获取 `session_lock`
/// 禁止（`state → session`），触发 `debug_assert!`。
#[cfg(debug_assertions)]
pub(crate) fn track_session() -> Guard {
    let violation = HELD.with(|held| held.borrow().contains(&Kind::State));
    debug_assert!(
        !violation,
        "LOCK-ORDER R2 violation: acquiring session_lock while state_lock held \
         (state→session forbidden; see docs/LOCK-ORDER-CONTRACT.md §3 R2)"
    );
    HELD.with(|held| held.borrow_mut().push(Kind::Session));
    Guard(Some(Kind::Session))
}

/// 记录已持有 `state_lock`。R2：`session → state` 合法（`advance_plot`），
/// 无 violation；`state` 单独持有也合法。
#[cfg(debug_assertions)]
pub(crate) fn track_state() -> Guard {
    HELD.with(|held| held.borrow_mut().push(Kind::State));
    Guard(Some(Kind::State))
}

#[cfg(all(test, debug_assertions))]
fn holds_session() -> bool {
    HELD.with(|held| held.borrow().contains(&Kind::Session))
}

#[cfg(all(test, debug_assertions))]
fn holds_state() -> bool {
    HELD.with(|held| held.borrow().contains(&Kind::State))
}

#[cfg(all(test, debug_assertions))]
fn reset() {
    HELD.with(|held| held.borrow_mut().clear());
}

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::*;

    #[test]
    fn session_alone_ok() {
        reset();
        let _g = track_session();
        assert!(holds_session());
        assert!(!holds_state());
    }

    #[test]
    fn state_alone_ok() {
        reset();
        let _g = track_state();
        assert!(holds_state());
        assert!(!holds_session());
    }

    /// R2：session → state 是唯一合法嵌套方向（advance_plot 经
    /// StateService::mutate）。同时持有时不应触发。
    #[test]
    fn session_then_state_legal_no_panic() {
        reset();
        let _s = track_session();
        let _t = track_state();
        assert!(holds_session());
        assert!(holds_state());
    }

    /// R2：state → session 禁止（Bug F 类锁序倒置死锁）。
    /// `track_session` 在持 state_lock 时必须 `debug_assert!` panic。
    #[test]
    fn state_then_session_panics() {
        reset();
        let _t = track_state();
        let result = std::panic::catch_unwind(track_session);
        assert!(
            result.is_err(),
            "track_session must panic when state_lock held (state→session forbidden by R2)"
        );
        // panic 发生在 push 之前，栈仍含 State。
        assert!(holds_state());
        assert!(!holds_session());
    }

    #[test]
    fn drop_releases_held() {
        reset();
        {
            let _g = track_session();
            assert!(holds_session());
        }
        assert!(!holds_session());
        assert!(!holds_state());
    }

    /// 两段临界区模式（trigger_world_event / advance_clock）：
    /// state_lock 释放后再获取 session_lock，合法，不应 panic。
    #[test]
    fn state_released_then_session_ok() {
        reset();
        {
            let _t = track_state();
            assert!(holds_state());
        }
        let _s = track_session();
        assert!(holds_session());
        assert!(!holds_state());
    }
}

// ── release build（零成本 no-op）─────────────────────────────────────────────
/// release build 零成本 no-op。`track_*` 不做任何检查，Guard 为 ZST。
#[cfg(not(debug_assertions))]
#[must_use = "Guard tracks a held lock until dropped; bind it to a named variable for the whole critical section"]
pub(crate) struct Guard;

#[cfg(not(debug_assertions))]
#[inline]
pub(crate) fn track_session() -> Guard {
    Guard
}

#[cfg(not(debug_assertions))]
#[inline]
pub(crate) fn track_state() -> Guard {
    Guard
}
