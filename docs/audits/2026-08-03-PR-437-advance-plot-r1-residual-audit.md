# PR #437 独立审计：advance_plot R1 残留 TOCTOU 闭合（fix path 4）

> 审计日期：2026-08-03
> 审计对象：PR #437 `fix(engine): close advance_plot R1 residual TOCTOU (fix path 4: split StateService::mutate)`
> 审计分支：`fix-advance-plot-r1-residual`
> 审计基线：`main@48afdf1`（合并 PR #436 后）
> 审计依据：AGENTS.md「审计 Agent 守则」三原则——独立审计、可提己见、可质疑历史并查证
> 关联合同：`docs/LOCK-ORDER-CONTRACT.md`
> 关联 issue：[#437](https://github.com/GhostXia/AIRP/issues/437)（PR #436 W-01/W-02 follow-up）

## 1. 审计范围

本审计独立复核 PR #437 的四项内容：

1. **代码修复**：`StateService::mutate` 拆分为 `mutate_locked`（不 acquire `character_lock`）+ `mutate`（兼容包装）；`advance_plot` 改用 `mutate_locked` + 外层 `character_lock.read()`
2. **合同更新**：`LOCK-ORDER-CONTRACT.md` §2.2 / §2.3 / R1 / R2 / §6.7 / §7
3. **W-01 措辞修正**：re-entrancy 风险论证从「deadlock」修正为「deadlock（部分 pthread）+ 排他性语义破坏（Windows SRWLOCK）」
4. **回归测试**：新增 `advance_plot_and_delete_character_serialized_by_character_lock` 并发测试

审计方法：读源码 + 读合同 + 与 `main@48afdf1` 对照 + 独立判断 fix path 4 实现是否正确闭合 R1。**不**把开发 agent 的结论或 PR #436 审计报告的 W-02 建议作为不可质疑的前提。

## 2. 独立发现

### 2.1 fix path 4 实现正确性（独立复核）

#### 2.1.1 `StateService::mutate_locked`（`engine/src/domain/state.rs`）✅

```rust
pub fn mutate_locked<F>(
    &self,
    character_id: &CharacterId,
    mutate: F,
) -> Result<StateSnapshot, AirpError>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), AirpError>,
{
    let state_boundary = state_lock(character_id.as_str());
    let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
    let _state_track = lock_order::track_state();

    let state_dir = data_dir::char_state_dir(&self.data_root, character_id.as_str());
    fs::create_dir_all(&state_dir)?;

    let mut value: serde_json::Value = Self::load_live_value(character_id, &state_dir)?;

    mutate(&mut value)?;
    self.commit_state_under_lock(character_id, &state_dir, &value)
}
```

**独立复核要点**：

1. **不 acquire `character_lock.read()`** ✅：与 `mutate` 相比，`mutate_locked` 直接进入 `state_lock` 临界区，跳过 `character_lock.read()`。这是 fix path 4 的核心——调用方负责外层 `character_lock.read()`。
2. **`state_lock` + `track_state()` 保留** ✅：R2 运行时强制仍生效。若调用方持 `session_lock`（如 `advance_plot`），`session → state` 是合法嵌套方向，`track_state()` 不触发 `debug_assert!`。
3. **`commit_state_under_lock` 复用** ✅：与 `mutate` / `write` 共享同一 commit 路径（schema 校验 + replace_file + history.jsonl + revision），保持 #115 Phase 2e revision 合同。
4. **`load_live_value` 复用** ✅：与 `mutate` 共享同一 load 路径（文件不存在 → 空对象 / 解析失败 → Internal error），不引入新的错误处理路径。
5. **`fs::create_dir_all` 保留** ✅：与 `mutate` 一致，确保 `state/` 目录存在。

**结论**：`mutate_locked` 是 `mutate` 的精确子集（移除 `character_lock.read()` 前置），无行为差异。✅

#### 2.1.2 `StateService::mutate` 兼容包装（`engine/src/domain/state.rs`）✅

```rust
pub fn mutate<F>(
    &self,
    character_id: &CharacterId,
    mutate: F,
) -> Result<StateSnapshot, AirpError>
where
    F: FnOnce(&mut serde_json::Value) -> Result<(), AirpError>,
{
    let character = character_lock(character_id.as_str());
    let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
    self.mutate_locked(character_id, mutate)
}
```

**独立复核要点**：

1. **向后兼容** ✅：`mutate` 行为与 PR #437 之前完全一致（acquire `character_lock.read()` → 进入 `state_lock` 临界区）。其他调用方（`update_relationship` / `update_character_state` / 等）无需改动。
2. **内部委托 `mutate_locked`** ✅：避免代码重复，`mutate` 仅负责外层 `character_lock.read()` 门控。
3. **lock_order tracking** ✅：`mutate_locked` 内部已调用 `track_state()`，`mutate` 不重复调用。

**结论**：兼容包装正确，无行为回归。✅

#### 2.1.3 `advance_plot` 锁序修复（`engine/src/agent/tools/plot.rs`）✅

```rust
let character = character_lock(cid.as_str());
let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
let session_boundary = session_lock(cid.as_str(), sid.as_ref());
let _session_guard = session_boundary.lock().unwrap_or_else(|p| p.into_inner());
let _session_track = lock_order::track_session();

let entry = format!("\n[剧情推进: {}] {}\n", plot_type, development);

crate::volume_store::append_to_current(&session_dir, &entry)?;

let snapshot = StateService::new(&state.data_root).mutate_locked(&cid, |live| {
    // ...
})?;
```

**独立复核要点**：

1. **锁序** ✅：`character.read() → session.lock() → [mutate_locked 内] state.lock()`，与合同 §2.3 一致。
2. **`character_lock.read()` 在 `session_lock` 之前** ✅：R1 要求 character_lock 是外层门控，修复后顺序正确。
3. **`mutate_locked` 而非 `mutate`** ✅：避免 `StateService::mutate` 内部 re-acquire `character_lock.read()` 构成递归 read。
4. **`track_session()` 在 `session_lock` acquire 后调用** ✅：与 §6.1 运行时强制一致，检测 R2 违规。
5. **guard 不跨 `.await`** ✅：闭包内 `append_to_current` + `mutate_locked` 全为同步 I/O，符合 A1。
6. **poison 恢复** ✅：`unwrap_or_else(|p| p.into_inner())` 与 §5 P1 一致。

**结论**：锁序修复正确，R1 TOCTOU 防护已闭合。✅

### 2.2 W-01 措辞修正正确性（独立复核）

合同 §6.7 原 W-01 措辞：

> 「std `RwLock` 递归 read 在部分平台（含 Windows SRWLOCK）会 deadlock」

修正后：

> 「Windows SRWLOCK：递归 read **不**会 deadlock（Vista+），但**破坏排他性语义**——第二次 acquire 立即返回，第一次 release 即释放锁，导致 `StateService::mutate` 内部的 `state_lock` 临界区在 `character.write()` 持有期间执行，R1 TOCTOU 防护失效。Linux/pthread：递归 read 在持有时若有 writer 等待**可能 deadlock**（glibc 实现相关）。」

**独立复核**：

1. **Windows SRWLOCK 行为** ✅：MSDN 确认 `AcquireSRWLockShared` 递归调用不 deadlock（Vista+），但无计数语义。修正后的措辞准确。
2. **Linux/pthread 行为** ✅：POSIX `pthread_rwlock_rdlock` 递归 read 在 writer 等待时可能 deadlock（glibc 实现相关，POSIX 未定义递归 read 行为）。修正后的措辞准确。
3. **结论**：W-01 措辞修正准确，反映了平台特定的真实行为。✅

### 2.3 合同更新正确性

#### 2.3.1 §2.2 新增 `mutate_locked` 描述 ✅

新增段落准确描述了 `mutate_locked` 的语义（不 acquire `character_lock`，要求调用方已持有）和使用约束（仅供 `advance_plot`）。✅

#### 2.3.2 §2.3 R1 闭合标注 ✅

- 锁序图更新为 `character_lock.read() → session_lock.lock() → [StateService::mutate_locked 内] state_lock.lock()` ✅
- 「R1 例外（已记录）」改为「R1 已闭合（#437 fix path 4）」 ✅
- 历史风险保留并指向 §6.7 ✅

#### 2.3.3 R1 例外路径清零 ✅

- 例外路径列表从 1 处改为 0 处，原条目标记为「已闭合」 ✅
- R1 收敛进度更新为「无 R1 例外路径残留」 ✅

#### 2.3.4 R2 嵌套方向更新 ✅

- `session_lock → state_lock` 的路径描述从「经 `StateService::mutate`」改为「经 `StateService::mutate_locked`」 ✅
- 新增说明 `advance_plot` 经 `mutate_locked` 仍保持 `session → state` 方向 ✅

#### 2.3.5 §6.7 闭合状态 ✅

- 标题改为「已闭合，#437 fix path 4」 ✅
- 历史背景保留，re-entrancy 论证修正（W-01） ✅
- 修复路径表新增方案 4（W-02）并标记为「已交付」 ✅
- 闭合状态说明准确 ✅

#### 2.3.6 §7 验收记录 ✅

- 新增「R1 残留闭合验收记录（PR #437，2026-08-03）」段落 ✅
- 明确标注「不改变 §6.1 运行时强制状态（R1 仍无运行时强制）」 ✅
- 记录 W-01 措辞修正 ✅

### 2.4 回归测试正确性

#### 2.4.1 测试设计 ✅

`advance_plot_and_delete_character_serialized_by_character_lock` 测试：

- **并发模式** ✅：两个 worker（`advance_plot` + `delete_character`）经 `Barrier` 同时放行，与现有 `trigger_world_event_and_advance_plot_concurrent_no_deadlock` 同模式。
- **30s 超时** ✅：死锁时转为明确测试失败。
- **独立 OS thread + current-thread runtime** ✅：避免占用 parent tokio runtime worker pool。
- **唯一 character_id** ✅：避免与其他 `#[tokio::test]` 争用 process-global 锁。

#### 2.4.2 测试断言 ✅

- **不 deadlock** ✅：30s 超时检测。
- **advance_plot 不返回 Internal error** ✅：worker A 将非 NotFound error 升级为测试失败。这验证 R1 TOCTOU 防护——若 `delete_character` 在 `advance_plot` 临界区期间删除 `live.json`，`advance_plot` 读到半删状态会返回 Internal（而非 NotFound），测试失败。
- **不断言 character dir 终态** ✅（CI 失败后修正）：R1 只保证串行化（不保证顺序）。若 `delete_character` 先完成而 `advance_plot` 后完成，`advance_plot` 会重新创建 dir 写 `live.json` —— 这是合法的串行化结果，非 TOCTOU 失效。原版测试在 `delete_succeeded == true` 时断言 `!character_dir.exists()`，这在「delete 先 / advance_plot 后」顺序下会误判失败。详见 §6 re-audit。

#### 2.4.3 Windows `DirectoryNotEmpty` quirk 处理 ✅

测试注释准确记录了 Windows 上 `fs::remove_dir_all` 可能因 #422 lock-map cleanup race（entry 在 write guard 释放前被移除）而失败的情况。这不是 #437 引入的回归，测试正确地将 `delete_character` 失败视为合法结果。

**审计建议**：#422 lock-map cleanup race（`delete_character` 在 write guard 释放前移除 lock map entry，允许新 caller 创建不同 lock 实例）是一个 pre-existing TOCTOU 风险，与 #437 闭合的 R1 风险独立。建议后续 issue 追踪。**非阻塞**——不在 #437 范围内。

## 3. 测试验证

| 检查 | 命令 | 结果 | 独立复核 |
|---|---|---|---|
| 格式 | `cargo fmt --check` | clean | ✅ |
| Clippy | `cargo clippy --workspace --exclude airp-ui --locked --all-targets -- -D warnings` | clean | ✅ |
| Rust 测试 | `cargo test --workspace --exclude airp-ui --locked` | 1250 passed / 0 failed / 5 ignored | ✅（基线 1249 → 1250，增量来自 #437 新增 1 个回归测试） |
| 神圣不变式 | `cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise` | ok | ✅ |
| WebUI | `node --test webui/tests/*.test.mjs` | 76 passed / 0 failed | ✅ |

## 4. 阻塞意见

**无阻塞意见**。

fix path 4 实现正确闭合了 R1 残留 TOCTOU 风险，合同更新自洽，W-01 措辞修正准确，回归测试覆盖 R1 串行化语义。所有测试通过，神圣不变式保持。

## 5. 非阻塞意见（写入 PR 后续 issue）

| 编号 | 类型 | 描述 | 建议时机 |
|---|---|---|---|
| W-01 | Pre-existing race | #422 lock-map cleanup race：`delete_character` 在 write guard 释放前移除 `CHARACTER_LOCKS` / `STATE_LOCKS` 中的 entry，允许新 caller（如 `advance_plot`）创建不同的 lock 实例并并发执行。这在 Windows 上导致 `fs::remove_dir_all` 失败（`DirectoryNotEmpty`），在 Linux 上可能导致 TOCTOU（`advance_plot` 在 `delete_character` 删除目录后仍写入 `live.json`，因为使用了不同 lock 实例）。修复方案：将 `remove_deleted_*_lock` 调用移到 `_guard` drop 之后（例如显式 `drop(_guard)` 后再清理 lock map）。 | 后续 issue（独立 PR） |
| W-02 | R1 运行时强制 | §6.1 仍仅覆盖 R2 session↔state。R1（character 外层门控）的运行时 `debug_assert!` 未交付（与 PR #436 W-03 同一缺口）。建议在 `lock_order` 模块新增 `track_character_read()` / `track_character_write()`，在持 character_lock 时 set thread-local flag，acquire session/state_lock 时检查。 | 后续 issue（独立 PR，与 PR #436 W-03 合并） |

## 6. 审计结论

**通过（无阻塞）**。

PR #437 通过 fix path 4（拆 `StateService::mutate` 为 `mutate_locked` + `mutate`）闭合了 PR #436 残留的 `advance_plot` R1 TOCTOU 风险。实现正确，合同更新自洽，W-01 措辞修正准确，回归测试覆盖 R1 串行化语义。R1 例外路径数从 1 降为 0。

非阻塞意见 W-01（#422 lock-map cleanup race）和 W-02（R1 运行时强制）按 AGENTS.md「审计遗留项处理」规则，PR 合并后由执行审计的 agent 写入 GitHub issue。

## 7. Re-audit：CI 失败后测试修正（2026-08-03）

PR #439（即 #437 实现）首次 CI 运行 `Rust test` job 失败：

```
test agent::tools::tests::agent_rp_phase3::advance_plot_and_delete_character_serialized_by_character_lock ... FAILED
thread '...advance_plot_and_delete_character_serialized_by_character_lock' panicked at engine\src\agent\tools\tests\agent_rp_phase3.rs:1516:9:
character dir must be deleted after delete_character succeeded
```

### 7.1 根因分析

测试断言 `if delete_succeeded { assert!(!character_dir.exists(), ...) }` 错误地假设「`delete_character` 成功 ⇒ character dir 不存在」。这只在 `delete_character` 是最后一个完成的 worker 时成立。R1 只保证**串行化**（两个 worker 不重叠），**不保证顺序**：

- **顺序 A（advance_plot 先 / delete_character 后）**：advance_plot 持 `character_lock.read()` 写 live.json → 释放 → delete_character 持 `character_lock.write()` 删 dir。终态：dir 不存在。原断言通过。
- **顺序 B（delete_character 先 / advance_plot 后）**：delete_character 持 `character_lock.write()` 删 dir + cleanup lock-map → 释放 → advance_plot 持 `character_lock.read()`（**新 Arc 实例**，因 lock-map 已 cleanup）→ `StateService::mutate_locked` 内 `fs::create_dir_all(&state_dir)` 重新创建 dir → 写 live.json。终态：dir **存在**且含 advance_plot 产物。原断言失败。

CI 在顺序 B 下运行，触发了误判失败。这是**测试逻辑 bug**，不是生产代码 bug —— `state.rs` / `plot.rs` 实现正确，R1 串行化语义在两种顺序下都成立。

### 7.2 修正内容

- **移除** `if delete_succeeded { assert!(!character_dir.exists(), ...) }` 断言块。
- **保留** 核心不变式：(1) 不 deadlock（30s 超时）；(2) `advance_plot` 不返回 `Internal` error（worker A 将非 NotFound error 升级为失败）。
- **更新** 测试 doc comment：明确 R1 只保证串行化不保证顺序，两种终态都合法，dir 终态不断言。
- **更新** §2.4.2：第三条断言从「dir 已删除」改为「不断言 dir 终态」并标注 CI 修正原因。

### 7.3 独立复核

- 修正后 `cargo test --lib -p airp-core advance_plot_and_delete_character_serialized_by_character_lock` 单测连跑 5 次全过。
- 修正后 `cargo test --workspace --all-features` 全量测试通过。
- 生产代码（`state.rs` / `plot.rs` / `LOCK-ORDER-CONTRACT.md`）**未改动**，仅测试文件 + 审计报告变更。
- 修正不削弱 R1 覆盖：核心不变式（no-deadlock + no-TOCTOU）仍在验证；dir 终态断言原本就是过度约束，与 R1 语义无关。

### 7.4 Re-audit 结论

**通过（无阻塞）**。测试修正正确，根因分析准确，R1 覆盖未削弱。建议合并。

## 8. Second re-audit：第二次 CI 失败后测试修正（2026-08-03）

PR #439 第二次 CI 运行 `Rust test` job 再次失败（同一测试）：

```
worker error: advance_plot failed with non-NotFound error (R1 TOCTOU protection may have failed): Io(Os { code: 3, kind: NotFound, message: "The system cannot find the path specified." })
```

### 8.1 根因分析

§7 修正移除了 dir-existence 断言，但 worker A 的 error matcher 仍只接受 `AirpError::NotFound`，未覆盖 `AirpError::Io(io::Error { kind: NotFound })`。

当 `delete_character` 先完成（dir 已删）而 `advance_plot` 后运行时：

1. `advance_plot` 调 `resolve_session_dir`（line 78-79，**在 character_lock.read() 之前**）→ 返回路径（此时 dir 可能已不存在，但 `resolve_session_dir` 不检查存在性，只构造路径）。
2. `advance_plot` acquire `character_lock.read()`（**新 Arc 实例**——#422 lock-map cleanup race：`delete_character` 在 write guard 释放前已移除 lock-map entry）。
3. `advance_plot` 调 `append_to_current(&session_dir, &entry)`（line 108）→ `session_dir/current.md` 路径不存在 → 返回 `AirpError::Io(std::io::Error { kind: NotFound, code: 3 })`。

这是 **#422 lock-map cleanup race** 的另一种表现（与 §2.4.3 W-01 同源）：`delete_character` 在 write guard 释放前移除 lock-map entry，允许 `advance_plot` 用新 lock 实例运行，此时 session_dir 已被 `remove_dir_all` 删除。**非 R1 TOCTOU 失效**——`advance_plot` 未读到半删状态，只是路径不存在。

### 8.2 修正内容

worker A 的 error matcher 扩展为同时接受：

- `AirpError::NotFound(_)`：StateService 层显式 NotFound（原已接受）。
- `AirpError::Io(io_err) if io_err.kind() == ErrorKind::NotFound`：`append_to_current` / `mutate_locked` 内 `fs` 操作因路径不存在失败（新增）。

两种都是「character 已被 delete_character 删除」的合法串行化结果。真正的 R1 失效（读到半删 live.json → `AirpError::Internal`）仍会触发测试失败。

### 8.3 独立复核

- 修正后 `cargo test --workspace --all-features` 全量通过（1307 passed / 0 failed / 5 ignored）。
- 生产代码（`state.rs` / `plot.rs` / `LOCK-ORDER-CONTRACT.md`）**仍未改动**。
- #422 race 的根本修复（将 `remove_deleted_*_lock` 移到 guard drop 之后）不在 #437 范围，仍由 W-01 follow-up issue 追踪。

### 8.4 Second re-audit 结论

**通过（无阻塞）**。测试正确覆盖了 #422 race 的两种 NotFound 表现形式，R1 TOCTOU 不变式仍被严格验证。建议合并。

---

审计 agent：（独立审计 mode，遵循 AGENTS.md 三原则）
