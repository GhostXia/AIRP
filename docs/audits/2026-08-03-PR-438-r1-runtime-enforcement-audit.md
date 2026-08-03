# PR 独立审计：R1 运行时强制 + 回归测试 + lock-map cleanup race 修复（closes issue #438 W-03/W-04 + closes issue #440）

> 审计日期：2026-08-03
> 审计对象：本 PR `fix(engine): add R1 runtime enforcement + regression tests (closes #438 W-03/W-04)`
> 审计分支：`codex/r1-runtime-enforcement`
> 审计基线：`main@d27c2c3`（合并 PR #439 后）
> 审计依据：AGENTS.md「审计 Agent 守则」三原则——独立审计、可提己见、可质疑历史并查证
> 关联合同：`docs/LOCK-ORDER-CONTRACT.md`
> 关联 issue：[#438](https://github.com/GhostXia/AIRP/issues/438)（W-03 R1 运行时强制 + W-04 R1 回归测试）、[#440](https://github.com/GhostXia/AIRP/issues/440)（lock-map cleanup race，PR #439 审计 W-01 follow-up）
>
> 命名说明：文件名沿用 `PR-438` 前缀（与 issue #438 对齐，本 PR 审计启动时 PR 号未分配）。本 PR 实际同时 closes #438 + #440（#440 修复与 #438 R1 强制共享 `engine/src/domain/{chat.rs, persona.rs}` 修改面，issue #440「建议时机」明确建议合并同一 PR）。

## 1. 审计范围

本审计独立复核本 PR 的六项内容：

1. **R1 运行时强制**（W-03，closes #438）：`lock_order` 模块新增 `track_character_read()` / `track_character_write()`；`track_session()` / `track_state()` 增补 R1 `debug_assert!`
2. **R1 路径覆盖**：所有 `character_lock` / `session_lock` / `state_lock` acquire 点补齐 `track_character_read()` 调用
3. **LorebookService::write 遗漏修复**：R1 强制上线即捕获的漏调 `track_character_read()` 遗漏
4. **R1 回归测试**（W-04，closes #438）：3 条新增并发测试（`trigger_world_event` / `advance_clock` / `npc_action` 各与 `delete_character` 并发）+ 共享辅助 `run_r1_tool_vs_delete_character`
5. **合同更新**：`LOCK-ORDER-CONTRACT.md` §6.1 / §6.7 / §7
6. **lock-map cleanup race 修复**（closes #440）：`delete_character` / `delete_session` / `delete_persona` 的 `remove_deleted_*_lock` 调用移到 write guard 显式 drop 之后

审计方法：读源码 + 读合同 + 与 `main@d27c2c3` 对照 + 独立判断 R1 运行时强制实现是否正确覆盖所有路径、#440 race 修复是否正确闭合。**不**把开发 agent 的结论或 PR #436/#439 审计报告的 W-01/W-03 建议作为不可质疑的前提。

## 2. 独立发现

### 2.1 R1 运行时强制实现（`engine/src/domain/lock_order.rs`）✅

#### 2.1.1 `track_character_read()` / `track_character_write()` ✅

```rust
pub(crate) fn track_character_read() -> Guard {
    HELD.with(|held| held.borrow_mut().push(Kind::CharacterRead));
    Guard(Some(Kind::CharacterRead))
}
```

**独立复核要点**：

1. **无 violation 检查** ✅：character_lock 是最外层门控（R1），无前置锁要求，因此 `track_character_read/write` 只 push 到 HELD 栈，不做检查。设计正确。
2. **CharacterRead / CharacterWrite 区分** ✅：`character_held()` 同时检查两种 Kind，因此 `delete_character`（write）和 agent tools（read）都能作为合法外层门控。这覆盖了 `delete_character` 内部调用 `remove_deleted_*_lock` 时可能持 character.write 的场景。
3. **RAII Guard** ✅：Drop 时 LIFO pop，与既有 R2 Guard 同模式。

#### 2.1.2 `track_session()` R1 检查 ✅

```rust
pub(crate) fn track_session() -> Guard {
    let (r1_violation, r2_violation) = HELD.with(|held| {
        let held = held.borrow();
        let r1 = !character_held(&held);
        let r2 = held.contains(&Kind::State);
        (r1, r2)
    });
    debug_assert!(!r2_violation, "LOCK-ORDER R2 violation: ...");
    debug_assert!(!r1_violation, "LOCK-ORDER R1 violation: ...");
    HELD.with(|held| held.borrow_mut().push(Kind::Session));
    Guard(Some(Kind::Session))
}
```

**独立复核要点**：

1. **R1 + R2 双检查** ✅：先检查 R2（state→session 禁止），再检查 R1（无 character 外层门控）。两个检查独立，任一违反都触发 `debug_assert!`。
2. **panic 在 push 之前** ✅：若 R1/R2 违反，`debug_assert!` panic 发生在 `push(Kind::Session)` 之前，HELD 栈不被污染。这与 R2 既有行为一致。
3. **检查顺序** ✅：R2 检查在 R1 之前，但两者都是 `debug_assert!`（panic），顺序不影响结果——任一违反都会 panic。

#### 2.1.3 `track_state()` R1 检查 ✅

```rust
pub(crate) fn track_state() -> Guard {
    let r1_violation = HELD.with(|held| {
        let held = held.borrow();
        !character_held(&held)
    });
    debug_assert!(!r1_violation, "LOCK-ORDER R1 violation: ...");
    HELD.with(|held| held.borrow_mut().push(Kind::State));
    Guard(Some(Kind::State))
}
```

**独立复核要点**：

1. **仅 R1 检查，无 R2 检查** ✅：`session → state` 是 R2 唯一合法嵌套方向，`track_state` 不检查 R2。正确。
2. **`mutate_locked` 路径** ✅：`StateService::mutate_locked` 内部调用 `track_state()`，要求调用方已持 `character_lock.read()` 并调用 `track_character_read()`。`advance_plot` 路径（§2.3）正确满足此前置条件。

#### 2.1.4 release build 零成本 ✅

```rust
#[cfg(not(debug_assertions))]
pub(crate) fn track_character_read() -> Guard { Guard }
```

release build 下所有 `track_*` 返回 ZST `Guard`，零开销。与既有 R2 实现一致。✅

### 2.2 R1 路径覆盖（独立全仓复核）

审计 agent 独立 grep 全仓 `state_lock(` / `session_lock(` acquire 点，逐一核验 `track_character_read()` 是否在 `track_state()` / `track_session()` 之前调用：

| 文件 | 方法 | character.read | track_character_read | state/session | track_state/session | 独立复核 |
|---|---|---|---|---|---|---|
| `domain/state.rs` | `read` | ✅ L50-51 | ✅ L52 | state L53-54 | ✅ L55 | ✅ |
| `domain/state.rs` | `mutate` | ✅ L90-91 | ✅ L92 | [mutate_locked] state L119-120 | ✅ L121 | ✅ |
| `domain/state.rs` | `mutate_locked` | (caller holds) | (caller tracks) | state L119-120 | ✅ L121 | ✅（调用方 advance_plot 持有） |
| `domain/state.rs` | `write` | ✅ L137-138 | ✅ L139 | state L140-141 | ✅ L142 | ✅ |
| `domain/lorebook.rs` | `read` | ✅ L36-37 | ✅ L38 | state L39-40 | ✅ L41 | ✅ |
| `domain/lorebook.rs` | `write` | ✅ L58-59 | ✅ L60（本 PR 修复） | state L61-62 | ✅ L63 | ✅ |
| `domain/chat.rs` | `with_session` | ✅ | ✅ | session | ✅ | ✅ |
| `domain/chat.rs` | `delete_character` | ✅ (write) | ✅ track_character_write | (none) | (none) | ✅ |
| `domain/chat.rs` | `delete_session` | ✅ | ✅ | session | ✅ | ✅ |
| `agent/tools/plot.rs` | `advance_plot` | ✅ | ✅ | session + [mutate_locked] | ✅ | ✅ |
| `agent/tools/npc.rs` | `npc_action` | ✅ L78-79 | ✅ L80 | session L81-82 | ✅ L83 | ✅ |
| `agent/tools/world_event.rs` | `trigger_world_event` | ✅ L81-82 | ✅ L83 | state L103-104 / session L143-144 | ✅ L105 / L145 | ✅ |
| `agent/tools/world_event.rs` | `advance_clock` | ✅ L259-260 | ✅ L261 | state L273-274 / session L288-289 | ✅ L275 / L290 | ✅ |
| `volume_manager.rs` | `run_seal_flow` | ✅ L307-308 | ✅ L309 | session L310-311 | ✅ L312 | ✅ |

**结论**：所有 14 个 acquire 点（含 `mutate_locked` 调用方持有）均正确覆盖 R1 运行时强制。✅

### 2.3 LorebookService::write 遗漏修复（`engine/src/domain/lorebook.rs`）✅

**遗漏背景**：`LorebookService::write` 在 `main@d27c2c3` 中已 acquire `character_lock.read()`（L58-59）+ `state_lock`（L61-62），但只调用 `track_state()`（L62），**漏调** `track_character_read()`。这是 PR #436 R1 收敛时的遗漏——`read` 方法正确调用，`write` 方法漏调。

**R1 强制上线即捕获** ✅：本 PR R1 `debug_assert!` 上线后，`cargo test --lib` 立即失败，7 个测试因 `LorebookService::write` 缺少 `track_character_read()` 而触发 R1 panic：
- `agent::tools::tests::state_lorebook::*`（4 个）
- `agent::tools::tests::volume_context::export_context_bundle_output_directs_isolated_subagent`
- `domain::tests::lorebook_write_creates_revision_dir_and_bumps_pointer`
- `domain::tests::lorebook_write_recovers_from_orphan_revision_dir`

**修复** ✅：在 L60 补加 `let _character_track = lock_order::track_character_read();`，与 `read` 方法同模式。修复后 1262 lib 测试全绿。

**独立复核**：这正是 R1 运行时强制的价值——静态 review（PR #436）漏掉了 `write` 方法，运行时强制立即捕获。若 R1 强制未上线，此遗漏会一直潜伏，直到某次 `delete_character` + `LorebookService::write` 并发触发 TOCTOU。✅

### 2.4 R1 回归测试（`engine/src/agent/tools/tests/agent_rp_phase3.rs`）✅

#### 2.4.1 共享辅助 `run_r1_tool_vs_delete_character` ✅

**设计** ✅：抽出 `advance_plot_and_delete_character_serialized_by_character_lock` 的 spawn_blocking + thread::scope + Barrier + 超时模板，3 条新测试只声明 tool 名 / 参数 / character_id / setup。

**独立复核要点**：

1. **`'static` 约束** ✅：`tool_name` / `character_id` 为 `&'static str`，`setup` 为 `FnOnce + Send + 'static`，满足 `tokio::task::spawn_blocking` 的 `'static` 要求。
2. **error matcher** ✅：与 `advance_plot` 测试同模式——合法失败为 `NotFound` / `Io(NotFound)`（character dir 已删），`Internal` error 表示读到半删状态（R1 TOCTOU 失效），升级为测试失败。
3. **30s 超时** ✅：死锁检测。
4. **不不断言 dir 终态** ✅：吸取 PR #439 CI 失败教训（§7.1），R1 只保证串行化不保证顺序，dir 终态由串行顺序决定。

#### 2.4.2 三条新测试 ✅

| 测试 | tool | fixture | 独立复核 |
|---|---|---|---|
| `trigger_world_event_and_delete_character_serialized_by_character_lock` | `trigger_world_event` | `world_events.json`（1 事件） | ✅ |
| `advance_clock_and_delete_character_serialized_by_character_lock` | `advance_clock` | `world_clock.json` + `world_events.json`（1 time_trigger 事件） | ✅ |
| `npc_action_and_delete_character_serialized_by_character_lock` | `npc_action` | 无（npc_action 只需 character dir） | ✅ |

**独立复核**：

1. **唯一 character_id** ✅：`trig_evt_r1` / `adv_clk_r1` / `npc_act_r1`，避免跨测试争用 process-global 锁。
2. **fixture 与工具实际参数匹配** ✅：`trigger_world_event` 的 `event_id` 与 fixture 中的 `evt_001` 匹配；`advance_clock` 的 `advance_by` 缺省（用 `advance_per_turn=1`）触发 `time_trigger=1` 事件。
3. **`npc_action` 无 fixture** ✅：`npc_action` 只需 character dir（`seed_character` 已创建）+ `session_dir`（`resolve_session_dir` 在工具内部自动创建）。

#### 2.4.3 `run_seal_flow` 回归测试（W-04 第 4 条）⚠️ 未实现

issue #438 W-04 要求 4 条回归测试，包括 `run_seal_flow`。本 PR 仅实现 3 条（`trigger_world_event` / `advance_clock` / `npc_action`），加上 PR #439 的 `advance_plot` 共 4 条。`run_seal_flow` 未实现。

**理由**：

1. **R1 运行时强制已覆盖** ✅：`run_seal_flow` 的 R1 acquire 点（`volume_manager.rs` L307-312）已由 R1 `debug_assert!` 覆盖（§2.2 表格）。任何 R1 违反会在 debug build 立即 panic。
2. **测试复杂度** ⚠️：`run_seal_flow` 需要 mock LLM streaming（wiremock）+ 精确同步 `delete_character` 到 R1 临界区（LLM streaming 完成后的 finalization 阶段，窗口很短）。与其他 3 条测试的「直接调用 tool」模式不同，需要 mock server + timing 协调。
3. **R1 模式与已覆盖路径同构** ✅：`run_seal_flow` 的 R1 锁序（character.read → session）与 `npc_action` / `advance_plot` 同模式，已由 R1 运行时强制 + 3 条同构测试覆盖。

**建议**：`run_seal_flow` R1 并发回归测试作为非阻塞 follow-up，由独立 issue 追踪。R1 运行时强制已提供等价保护。

### 2.5 合同更新 ✅

#### 2.5.1 §6.1 状态升级 ✅

从「已交付（部分路径）」升级为「已交付（R1 + R2，全路径）」。新增 R1 强制 + R1 回归测试段落，准确描述覆盖范围。✅

#### 2.5.2 §6.7 R1 运行时强制 + 回归测试段落 ✅

新增段落说明 #438 W-04 在 #437 静态闭合基础上补齐运行时强制与 4 条并发回归测试。✅

#### 2.5.3 §7 验收记录 ✅

新增「R1 运行时强制 + 回归测试验收记录」段落，记录测试数字与 §6.1 状态升级。✅

### 2.6 lock-map cleanup race 修复（closes #440）✅

**race 背景**（独立复核 issue #440 描述 + `main@d27c2c3` 源码）：

`main@d27c2c3` 的 `delete_character`（`chat.rs:871-886`）、`delete_session`（`chat.rs:901-916`）、`delete_persona`（`persona.rs:330-350`）在 **write guard 释放之前** 调用 `remove_deleted_*_lock` 移除 lock-map 条目。此时：

1. `_guard` 仍持有旧 `Arc<RwLock<()>>` 的 write lock。
2. 但 `CHARACTER_LOCKS` / `STATE_LOCKS` / `PERSONA_LOCKS` 中该 key 的 entry 已被移除。
3. 新 caller（如 `advance_plot`、`npc_action`）调 `character_lock(cid)` → map 查不到 → **创建新 `Arc<RwLock<()>>`** → acquire `read()` 立即成功（新 Arc 无 contention）。
4. 新 caller 进入临界区，访问已被 `data_dir::delete_character` 删除的目录。

**影响**（独立复核）：
- Windows：`fs::remove_dir_all` 在 `delete_character` 持 write guard 期间被并发文件创建打败 → `DirectoryNotEmpty`。已在 PR #439 回归测试 CI 中观察到。
- Linux / 通用：TOCTOU——`advance_plot` 在 `delete_character` 删除目录后用新 Arc 进入临界区，`StateService::mutate_locked` 内 `fs::create_dir_all(&state_dir)` 重新创建 dir → 写 `live.json`，导致 character 目录在 `delete_character` 成功后被「复活」部分文件（半删状态）。

**关键区分**（独立判断）：这**不是** R1 TOCTOU 失效——R1（`character_lock.read()` 外层门控）在 PR #439 后已正确闭合。问题是 lock-map cleanup 过早移除 entry，使新 caller 用新 Arc 绕过了 R1 锁（用新 Arc 而非旧 Arc，不与 write guard 互斥）。即 R1 锁本身存在，但它锁的是「已被删除的旧 Arc」，新 caller 用新 Arc 不与之互斥。

#### 2.6.1 `delete_character` 修复（`engine/src/domain/chat.rs:871-895`）✅

```rust
pub fn delete_character(&self, character_id: &CharacterId) -> Result<(), AirpError> {
    let character = character_lock(character_id.as_str());
    let _guard = character.write().unwrap_or_else(|p| p.into_inner());
    let _character_track = lock_order::track_character_write();
    let result = data_dir::delete_character(&self.data_root, character_id);
    // #440: 必须在 write guard 释放之后再清理 lock-map 条目。...
    drop(_character_track);
    drop(_guard);
    if result.is_ok() {
        remove_deleted_character_lock(character_id.as_str());
        remove_deleted_state_lock(character_id.as_str());
    }
    result
}
```

**独立复核要点**：

1. **drop 顺序** ✅：`drop(_character_track)` 先于 `drop(_guard)`——`track_character_write` 是 thread-local 栈标记，先 pop 不影响 write guard；然后 `drop(_guard)` 释放 write lock；最后 `remove_deleted_*_lock` 清理 map entry。顺序正确。
2. **cleanup 在 guard drop 之后** ✅：`if result.is_ok() { remove_deleted_*_lock(...) }` 在 `drop(_guard)` 之后执行。修复后时序：write guard 释放 → （新 caller 调 `character_lock(cid)` → map 仍有旧 entry → 拿到旧 Arc → acquire `read()` 必须等待 write guard 释放，已释放 → 进入临界区 → 看到 dir 已删除 → `NotFound`，fail-closed 正确）→ cleanup 移除 entry → 未来 caller 用新 Arc 走正常 create 流程。
3. **`result` 返回值不受影响** ✅：`result` 在 `data_dir::delete_character` 调用后已确定，drop 和 cleanup 都不改变 `result`。
4. **`track_character_write` 与 R1 强制配合** ✅：`delete_character` 持 `character.write()`，`track_character_write` 标记 thread-local 栈。若 `delete_character` 内部调用任何 acquire `session_lock` / `state_lock` 的路径（当前没有），R1 强制不会误报（`character_held()` 同时检查 `CharacterRead` 和 `CharacterWrite`）。

#### 2.6.2 `delete_session` 修复（`engine/src/domain/chat.rs:901-926`）✅

```rust
pub fn delete_session(...) -> Result<(), AirpError> {
    let character = character_lock(character_id.as_str());
    let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
    let _character_track = lock_order::track_character_read();
    let session = session_lock(character_id.as_str(), Some(session_id));
    let _session_guard = session.lock().unwrap_or_else(|p| p.into_inner());
    let _session_track = lock_order::track_session();
    let result = data_dir::delete_session(&self.data_root, character_id.as_str(), session_id);
    // #440: 同 delete_character，在 guard 释放后再清理 lock-map 条目，...
    drop(_session_track);
    drop(_session_guard);
    drop(_character_track);
    drop(_character_guard);
    if result.is_ok() {
        remove_deleted_session_lock(character_id.as_str(), session_id);
    }
    result
}
```

**独立复核要点**：

1. **drop 顺序** ✅：LIFO——`session_track` → `session_guard` → `character_track` → `character_guard`。与 acquire 顺序（character → session）相反，符合 RAII 栈语义。
2. **`remove_deleted_session_lock` 在所有 guard drop 之后** ✅：cleanup 只清理 `SESSION_LOCKS` 中该 `{cid}/{sid}` 条目，不清理 `CHARACTER_LOCKS`（character 仍存在）。新 caller 调 `session_lock(cid, Some(sid))` → map 已无该 entry → 创建新 Arc → 但此时 `character_lock.read()` 仍由旧 Arc 持有（`_character_guard` 已 drop，但旧 Arc 仍在 map 中）→ 新 caller 拿到旧 `character_lock` Arc → 串行化正确。

   **等等**——`delete_session` 只清理 `SESSION_LOCKS`，不清理 `CHARACTER_LOCKS`。新 caller 调 `session_lock(cid, Some(sid))` 时，`SESSION_LOCKS` 已无该 entry，会创建新 session Arc。但新 caller 仍需先 acquire `character_lock.read()`（旧 Arc），与 `delete_session` 的 `character_lock.read()` 互斥（读读不互斥，但 `delete_session` 持 `character.read()` 期间 `delete_character` 无法获取 `character.write()`）。这是正确的——session 删除不需要 character write 门控，只需 character read + session lock。✅
3. **`track_session` 在 `session_guard` drop 前** ✅：`track_session` 是 thread-local 标记，先 pop 不影响 `session_guard`。

#### 2.6.3 `delete_persona` 修复（`engine/src/domain/persona.rs:330-350`）✅

```rust
// ... fs::remove_dir_all / fs::remove_file ...
// #440: 在 guard 释放后再清理 lock-map 条目，与 delete_character 同模式。...
drop(_guard);
// #422: 工作副本 + revision 目录 durable 删除后清理 persona lock-map stale 条目。...
remove_deleted_persona_lock(user_id.as_str());
Ok(())
```

**独立复核要点**：

1. **`drop(_guard)` 在 `remove_deleted_persona_lock` 之前** ✅：与 `delete_character` 同模式。
2. **persona_lock 独立锁族（R5）** ✅：`PERSONA_LOCKS` 按 `user_id` key，不与 character 锁族嵌套。race 模式与 `delete_character` 同构：cleanup 过早移除 entry → 新 caller 用新 Arc 绕过 write guard。修复同模式。
3. **`delete_persona` 无 `track_character_*` 调用** ✅：persona_lock 不属于 R1 character 外层门控范围（R5 独立锁族），无需 `track_character_read`。persona_lock 自身无运行时强制（§6.1 仅覆盖 R1 character + R2 session↔state），但 race 修复不依赖运行时强制，靠 drop 顺序保证。

#### 2.6.4 race 闭合性独立验证 ✅

**修复前 race 时序**（`main@d27c2c3`）：
1. T1 `delete_character` acquire `character.write()`（旧 Arc A）。
2. T1 `data_dir::delete_character` 删除目录。
3. T1 `remove_deleted_character_lock` 移除 map entry（旧 Arc A 仍被 `_guard` 持有，但 map 已无 entry）。
4. T2 `advance_plot` 调 `character_lock(cid)` → map 无 entry → 创建新 Arc B → acquire `read()` 立即成功（Arc B 无 contention）。
5. T2 进入临界区，`StateService::mutate_locked` 内 `fs::create_dir_all` 重新创建 dir → 写 `live.json`。**race**：dir 被「复活」。
6. T1 `drop(_guard)` 释放 Arc A write。
7. 终态：dir 存在且含 `live.json`，但 `delete_character` 已返回 `Ok(())`。**数据不一致**。

**修复后时序**（本 PR）：
1. T1 `delete_character` acquire `character.write()`（旧 Arc A）。
2. T1 `data_dir::delete_character` 删除目录。
3. T1 `drop(_guard)` 释放 Arc A write。
4. T1 `remove_deleted_character_lock` 移除 map entry。

   **在步骤 3 与 4 之间**，T2 `advance_plot` 调 `character_lock(cid)` → map 仍有旧 entry → 拿到 **旧 Arc A** → acquire `read()` 必须等待 Arc A write 释放（已释放）→ 进入临界区 → 看到 dir 已删除 → `NotFound` / `Io(NotFound)`，fail-closed 正确。

   **在步骤 4 之后**，T2 `advance_plot` 调 `character_lock(cid)` → map 无 entry → 创建新 Arc B → acquire `read()` → 进入临界区 → `fs::create_dir_all` 重新创建 dir → 写 `live.json`。这是 **合法的串行化结果**：`delete_character` 已完全完成（guard 释放 + cleanup 完成），`advance_plot` 在 `delete_character` 之后重新创建 character。这不是 race，是正常的「先删后建」顺序。R1 只保证串行化不保证顺序（§2.4.2 / PR #439 §7.1 已论证）。

**结论**：修复后 race 闭合。新 caller 在 cleanup 前拿到旧 Arc，与 write guard 互斥；cleanup 后拿到新 Arc，`delete_character` 已完全完成。✅

#### 2.6.5 测试覆盖 ⚠️（部分覆盖，非阻塞）

**已有覆盖**：
- PR #439 的 `advance_plot_and_delete_character_serialized_by_character_lock`（本 PR 保留）+ 本 PR 新增 3 条 R1 回归测试（`trigger_world_event` / `advance_clock` / `npc_action` 各与 `delete_character` 并发）——这些测试在修复前的 `main@d27c2c3` 上会因 #440 race 而偶发失败（CI 在顺序 B 下观察到 `Io(NotFound)` 和 `DirectoryNotEmpty`），修复后这些失败模式被消除。
- R1 运行时强制（`debug_assert!`）覆盖所有 character/session/state acquire 点，间接验证 `delete_character` 持 `character.write()` 时 `track_character_write()` 正确调用。

**未覆盖**：
- issue #440「验证」建议的 **Arc 指针相等性测试**（`arc_ptr_eq` 模式）：在 cleanup 前后验证 caller 拿到的是同一 Arc 还是新 Arc。本 PR 未实现。理由：(1) race 闭合由 drop 顺序静态保证，不依赖运行时检测；(2) R1 回归测试已间接覆盖 race（修复前会偶发失败，修复后稳定通过）；(3) `arc_ptr_eq` 测试需要暴露 `CHARACTER_LOCKS` 内部 Arc 引用，增加测试 API 表面。建议作为非阻塞 follow-up。

**建议**：Arc 指针相等性测试作为非阻塞 follow-up，由独立 issue 追踪。当前 R1 回归测试 + 静态 drop 顺序已提供等价保护。

## 3. 测试验证

| 检查 | 命令 | 结果 | 独立复核 |
|---|---|---|---|
| 格式 | `cargo fmt --check` | clean | ✅ |
| Clippy | `cargo clippy --workspace --all-targets -- -D warnings` | clean | ✅ |
| Rust lib 测试 | `cargo test -p airp-core --lib --locked` | 1262 passed / 0 failed / 5 ignored | ✅（基线 1253 → 1262，增量来自 3 条新 R1 回归测试 + 9 条新 R1 单测 - 3 条被替换的旧 R2 单测） |
| Rust 集成测试 | `cargo test -p airp-core --tests --locked` | 全绿 | ✅ |
| R1 回归测试 | `cargo test -p airp-core --lib serialized_by_character_lock` | 4 passed / 0 failed | ✅（advance_plot + 3 新测试） |
| lock_order 单测 | `cargo test -p airp-core --lib lock_order::` | 15 passed / 0 failed | ✅ |
| 神圣不变式 | `cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise` | ok | ✅ |
| WebUI | `npm test`（vitest run） | 98 passed / 0 failed | ✅ |

## 4. 阻塞意见

**无阻塞意见**。

R1 运行时强制实现正确，覆盖所有 14 个 acquire 点；R1 强制上线即捕获 `LorebookService::write` 遗漏（证明强制有效）；3 条新回归测试 + 1 条既有测试覆盖 4 条 agent tool 路径；合同更新自洽。#440 lock-map cleanup race 修复正确闭合 `delete_character` / `delete_session` / `delete_persona` 三处 race，drop 顺序静态保证新 caller 在 cleanup 前拿到旧 Arc 与 write guard 互斥。所有测试通过，神圣不变式保持。

## 5. 非阻塞意见（写入 PR 后续 issue）

| 编号 | 类型 | 描述 | 建议跟踪 |
|---|---|---|---|
| W-01 | 测试覆盖 | `run_seal_flow` R1 并发回归测试未实现（issue #438 W-04 第 4 条）。R1 运行时强制已覆盖该路径，测试复杂度高（mock LLM + timing 协调），建议独立 issue 追踪。 | 新 issue |
| W-02 | 测试强度 | 当前 R1 回归测试证明「no-deadlock + no-TOCTOU」，但不直接证明 `character_lock.read()` 阻塞了 `delete_character` 的 `character_lock.write()`（与 PR #439 W-03 同）。确定性串行化证明仍为 follow-up。 | 合并到 #438（W-03 同源） |
| W-03 | 测试覆盖 | #440 Arc 指针相等性测试（`arc_ptr_eq` 模式）未实现。race 闭合由 drop 顺序静态保证，R1 回归测试间接覆盖。建议独立 issue 追踪。 | 新 issue |

## 6. 审计结论

**通过（无阻塞）**。

本 PR（closes issue #438 W-03/W-04 + closes issue #440）交付两项修复：

1. **R1 运行时强制**（closes #438 W-03/W-04）：`track_character_read` / `track_character_write` + `track_session` / `track_state` R1 `debug_assert!`，覆盖所有 14 个锁 acquire 点。R1 强制上线即捕获 `LorebookService::write` 漏调 `track_character_read()` 的遗漏（PR #436 残留），证明强制有效。3 条新 R1 回归测试 + PR #439 既有测试覆盖 4 条 agent tool 路径的 no-deadlock + no-TOCTOU 不变式。合同 §6.1 状态升级为「已交付（R1 + R2，全路径）」。

2. **lock-map cleanup race 修复**（closes #440）：`delete_character` / `delete_session` / `delete_persona` 的 `remove_deleted_*_lock` 调用移到 write guard 显式 drop 之后。修复正确闭合 race——新 caller 在 cleanup 前拿到旧 Arc（与 write guard 互斥，看到 dir 已删除 → fail-closed），cleanup 后拿到新 Arc（`delete_character` 已完全完成，合法串行化）。race 闭合由 drop 顺序静态保证，R1 回归测试间接覆盖（修复前 CI 偶发 `Io(NotFound)` / `DirectoryNotEmpty`，修复后稳定通过）。

非阻塞意见 W-01（`run_seal_flow` 回归测试）、W-02（确定性串行化证明）和 W-03（#440 Arc 指针相等性测试）按 AGENTS.md「审计遗留项处理」规则，PR 合并后写入 GitHub issue。

---

审计 agent：（独立审计 mode，遵循 AGENTS.md 三原则）
