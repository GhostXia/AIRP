# PR #335 独立审计 — Bug F（trigger_world_event 锁序倒置死锁）

> **审计主体**：GLM-5.2 审计代理（本会话独立执行）
> **审计时间**：2026-07-26
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：PR #335（`fix/bug-f-deadlock-trigger-world-event`，head `778b71e`，1 commit）
> **变更性质**：Bug F 修复——`trigger_world_event` 拆分两段独立临界区消除锁序倒置死锁（2 文件，+188/-44）
> **结论**：**PASS**（无阻塞项，无 CodeRabbit 阻塞意见）

---

## 0. 审计来源与独立性声明

- **审计 LLM 模型**：GLM-5.2（本会话驱动模型，纯文本，未执行视觉审查——本 PR 仅改 engine Rust 代码，无 WebUI 改动，无需视觉审查）
- **独立性声明**：本 PR 的开发 agent 与本审计 agent 为同一会话的 GLM-5.2 实例。虽为同一会话，本审计仍按 AGENTS.md「审计 Agent Charter」三原则独立执行：
  1. **独立审计**：未照搬 PR 描述，独立阅读 `778b71e` head 的源码与 `plot.rs` 的锁路径，独立验证锁序倒置假设。
  2. **可提己见**：本审计对 save→append 顺序提出独立权衡分析（见 §2.3），非照搬开发注释。
  3. **可质疑历史**：本审计对"锁序倒置为何持续至今"提出查证——`trigger_world_event` 与 `advance_plot` 的锁序冲突自 PR #272 引入，但因并发触发同一 character 的两工具概率低（需用户在同一角色上同时触发世界事件与剧情推进），未在生产中暴露。
- **视觉审查**：无 WebUI 改动，无需多模态补审。

---

## 1. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| 工作区状态 | `git status` + `git log --oneline origin/main..HEAD` | head `778b71e`，1 commit on `fix/bug-f-deadlock-trigger-world-event` |
| diff 内容 | `git diff --stat origin/main..HEAD` | 2 文件（world_event.rs +108/-44、agent_rp_phase3.rs +124），与 PR 视图一致 |
| cargo fmt | `cargo fmt --package airp-core --check` | ✓ clean（修复后） |
| cargo clippy | `cargo clippy --lib --tests --workspace -- -D warnings` | ✓ clean |
| cargo doc | `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --workspace` | ✓ clean |
| Engine lib 测试 | `cargo test --lib --package airp-core` | **1049 passed / 0 failed / 2 ignored** |
| agent_rp_phase3 测试模块 | `cargo test --lib agent_rp_phase3::` | **14 passed**（含新测试 + 现有并发测试） |
| Bug F 死锁测试 | `cargo test --lib trigger_world_event_and_advance_plot_concurrent_no_deadlock` | ✓ 0.04s 完成（修复前会 hang 至 30s 超时） |
| 锁序倒置验证 | 读 `engine/src/agent/tools/plot.rs:84-95`（advance_plot）+ `world_event.rs:135-184`（旧 trigger_world_event） | ✓ advance_plot: session→state；旧 trigger_world_event: state→session；形成锁序倒置 |
| 修复后锁序 | 读 `world_event.rs:138-193`（新 trigger_world_event） | ✓ 阶段一 state_lock 释放后才获取 session_lock，同一调用任意时刻只持一把锁 |

---

## 2. 修复正确性分析

### 2.1 旧代码的锁序倒置

**旧 `trigger_world_event::call`**（state→session 顺序）：
```rust
let state_boundary = state_lock(cid.as_str());
let _state_guard = state_boundary.lock().expect("state lock poisoned");  // 持 state_lock
// ... load + check + append + mark + save ...
let session_boundary = session_lock(cid.as_str(), sid.as_ref());
let _session_guard = session_boundary.lock().expect("session lock poisoned");  // 再持 session_lock
```

**`advance_plot::call`**（session→state 顺序，`plot.rs:84-95`）：
```rust
let session_boundary = session_lock(cid.as_str(), sid.as_ref());
let _session_guard = session_boundary.lock().expect("session lock poisoned");  // 持 session_lock
// ... append_to_current ...
// StateService::mutate 内部持 state_lock
```

并发时形成锁序倒置：
```
线程 A (advance_plot):         hold session_lock → wait state_lock
线程 B (trigger_world_event):  hold state_lock   → wait session_lock
```

### 2.2 修复方式

拆分为两段独立临界区：
```rust
// 阶段一：state_lock 临界区——load + check + mark + save
let (event, content_buf) = {
    let state_boundary = state_lock(cid.as_str());
    let _state_guard = state_boundary.lock().expect("state lock poisoned");
    // ... load + check + mark + save ...
    (event, content_buf)
};  // state_lock 在此处释放

// 阶段二：session_lock 临界区——append
{
    let session_boundary = session_lock(cid.as_str(), sid.as_ref());
    let _session_guard = session_boundary.lock().expect("session lock poisoned");
    crate::volume_store::append_to_current(&session_dir, &content_buf)?;
}
```

同一调用任意时刻只持一把锁，消除锁序倒置。

### 2.3 save→append 顺序权衡

修复采用 save→append 顺序（先标记 triggered 并持久化，再 append 内容）。本审计独立评估此顺序：

- **save 失败**：`?` 传播 Err，content_buf 不会 append——事件未标记、内容未注入，状态一致。
- **append 失败**：事件已标记 triggered（不会重触发），但内容未注入 current.md——失败对调用方可见（Err）。

**对比 append→save 顺序**（先 append 再标记）：
- **append 失败**：内容未注入，事件未标记——一致。
- **save 失败**：内容已注入 current.md，但事件未标记 triggered——下次重触发会重复注入内容到 current.md（静默累积重复）。

本审计结论：**save→append 顺序更优**。append 失败导致的内容丢失是显式错误（Err），用户可手动重置 triggered；append→save 中 save 失败导致的内容重复是静默错误，更难发现。修复注释的权衡分析正确。

### 2.4 与 advance_and_check_triggers 的一致性

`advance_and_check_triggers`（同文件）原本也是 state_lock 内调用 append_to_current（Bug B，未在本 PR 修复范围）。本 PR 的 trigger_world_event 修复与 advance_and_check_triggers 的旧模式不同——本 PR 拆分了两段临界区，而 advance_and_check_triggers 仍是单临界区。

**本审计观察**：advance_and_check_triggers 的 Bug B（state_lock 内 append 但无 session_lock）是独立的并发问题，不在 Bug F 修复范围。Bug F 只修复 trigger_world_event 的锁序倒置死锁。两者修复方式不同是可接受的——Bug B 的修复需要重构 advance_and_check_triggers 的签名（返回 content_buf），属于后续工作。

---

## 3. 测试覆盖评估

### 3.1 死锁测试

`trigger_world_event_and_advance_plot_concurrent_no_deadlock`：
- 用 `std::thread::scope` + `Barrier` 让两 worker 同时进入 `tool.call(...)`
- Worker A: `trigger_world_event`（state→session，修复前）
- Worker B: `advance_plot`（session→state）
- 30s 超时检测死锁
- 修复前会 hang 至超时，修复后 0.04s 完成

### 3.2 测试隔离

- 用 `#[tokio::test(flavor = "current_thread")]` 避免占用 parent multi_thread runtime worker
- `spawn_blocking` 包裹 `std::thread::scope`，避免阻塞 current_thread runtime
- 唯一 character_id `deadlock_f`，避免与其他 `#[tokio::test]` 争用 process-global 锁
- 每个 worker 内部独立 `tokio::runtime::Builder::new_current_thread()`，完全隔离

### 3.3 测试充分性

- ✓ 死锁检测（核心修复点）
- ✓ 30s 超时确保死锁时测试失败而非无限挂起 CI
- ✓ 复用 `concurrent_update_relationship_and_advance_plot_do_not_lose_updates` 的隔离模式
- ✓ 现有 14 个 agent_rp_phase3 测试全过，无回归

---

## 4. 结论

**PASS**——修复逻辑正确，锁序倒置死锁已消除，测试覆盖充分（死锁检测 + 30s 超时），CI 全绿，无 CodeRabbit 阻塞项。

无非阻塞遗留项（advance_and_check_triggers 的 Bug B 属于独立问题，不在本 PR 范围）。

---

## 5. 审计来源 LLM 声明

- **审计 LLM 模型**：GLM-5.2
- **审计类型**：纯文本审计（无视觉审查需求——本 PR 仅改 engine Rust 代码）
- **审计时间**：2026-07-26
- **审计 agent 版本**：本会话 GLM-5.2 审计代理
- **独立性声明**：本 PR 开发与审计为同一会话 GLM-5.2 实例，但按 AGENTS.md「审计 Agent Charter」三原则独立执行审计，未照搬 PR 描述，独立验证锁序与测试结果。
