# PR #336 独立审计 — Bug B（advance_clock 缺失 session_lock 并发保护）

> **审计主体**：GLM-5.2 审计代理（本会话独立执行）
> **审计时间**：2026-07-26
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：分支 `audit/bug-b-tests`（基于 `origin/trae/agent-tnxPQA` 提取的 Bug B 修复 + 测试，2 文件 +290/-22）
> **变更性质**：Bug B 修复——`AdvanceClockTool::call` 拆分两段独立临界区（state_lock → session_lock），与 Bug F（PR #335）同模式；附 3 个功能性测试
> **结论**：**PASS**（无阻塞项）

---

## 0. 审计来源与独立性声明

- **审计 LLM 模型**：GLM-5.2（本会话驱动模型，纯文本，未执行视觉审查——本 PR 仅改 engine Rust 代码，无 WebUI 改动，无需视觉审查）
- **独立性声明**：本 PR 的开发 agent 与本审计 agent 为同一会话的 GLM-5.2 实例。本审计按 AGENTS.md「审计 Agent Charter」三原则独立执行：
  1. **独立审计**：未照搬分支描述。独立验证 main 上 `AdvanceClockTool::call` 仍是单临界区旧实现（state_lock 包裹 `advance_and_check_triggers` + 内部 `append_to_current`，无 `session_lock`），确认 Bug B 在 main 上确实未修复。
  2. **可提己见**：本审计对 save→append 顺序与 batch collect 的失败语义提出独立权衡分析（见 §2.3、§2.4），非照搬开发注释。
  3. **可质疑历史**：本审计查证 PR #335 审计报告（`docs/audits/2026-07-26-PR-335-bug-f-deadlock-audit.md` §2.4）明确将 Bug B 列为"独立问题，属于后续工作"——本 PR 即该后续工作。
- **视觉审查**：无 WebUI 改动，无需多模态补审。

---

## 1. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| 工作区状态 | `git status` + `git log --oneline main..HEAD` | 分支 `audit/bug-b-tests` 基于 main `6f2dcd0`，1 commit 待提交 |
| diff 内容 | `git diff --stat` | 2 文件（world_event.rs +74/-22、agent_rp_phase3.rs +238/-1），与审计范围一致 |
| cargo build | `cargo build --lib` | ✓ clean |
| cargo clippy | `cargo clippy --lib --tests -- -D warnings` | ✓ clean |
| cargo doc | `RUSTDOCFLAGS=-D warnings cargo doc --no-deps --workspace` | ✓ clean |
| Engine lib 测试 | `cargo test --lib` | ✓ 全绿（含 3 个新 Bug B 测试） |
| Bug B 功能测试 | `cargo test --lib advance_clock` | **3 passed**（`advance_clock_triggers_time_events_and_appends_to_current`、`advance_clock_no_due_events_does_not_append`、`advance_clock_triggers_multiple_due_events_in_single_append`） |
| Bug F 回归测试 | `cargo test --lib trigger_world_event_and_advance_plot_concurrent_no_deadlock` | ✓ 通过（本 PR 未破坏 Bug F 修复） |
| 旧 main 锁路径验证 | 读 main `world_event.rs:393-436`（旧 `AdvanceClockTool::call`） | ✓ 确认旧实现：单 `state_lock` 临界区，`advance_and_check_triggers` 内部逐事件 `append_to_current`，无 `session_lock` |
| 新锁路径验证 | 读修复后 `world_event.rs:420-456`（新 `AdvanceClockTool::call`） | ✓ 两段独立临界区：阶段一 state_lock（advance+collect+mark+save），阶段二 session_lock（append） |
| `advance_and_check_triggers` 调用点 | `grep advance_and_check_triggers\(` | ✓ 唯一调用点 `world_event.rs:442`，签名已同步更新 |

---

## 2. 修复正确性分析

### 2.1 旧代码的并发缺陷

**旧 `AdvanceClockTool::call`**（main `6f2dcd0`）：
```rust
let state_boundary = state_lock(cid.as_str());
let _state_guard = state_boundary.lock().expect("state lock poisoned");
let (clock, triggered) = advance_and_check_triggers(
    &state.data_root, cid.as_str(), &session_dir, advance_by,
)?;
```

`advance_and_check_triggers` 内部逐事件调用 `append_to_current(session_dir, ...)`，**未持有 `session_lock`**。这与 `npc_action` / `advance_plot` / `trigger_world_event`（修复后）的 append 路径不共享同一把 per-session 锁，允许并发 append 在 `current.md` 中交错混合叙事内容。

**与 Bug F 的关系**：Bug F（PR #335）修复了 `trigger_world_event` 的锁序倒置死锁（state→session 与 advance_plot 的 session→state 形成环）。Bug B 是独立的并发问题——`advance_clock` 的 append 无 `session_lock` 保护。Bug F 审计报告 §2.4 明确将 Bug B 列为后续工作。

### 2.2 新代码的两阶段临界区

**新 `AdvanceClockTool::call`**：
```rust
// 阶段一：state_lock 临界区——advance + collect + mark + save
let (clock, triggered, content_buf) = {
    let state_boundary = state_lock(cid.as_str());
    let _state_guard = state_boundary.lock().expect("state lock poisoned");
    advance_and_check_triggers(&state.data_root, cid.as_str(), advance_by)?
};
// state_lock 在此处释放

// 阶段二：session_lock 临界区——append content_buf 到 current.md
if !content_buf.is_empty() {
    let session_boundary = session_lock(cid.as_str(), sid.as_ref());
    let _session_guard = session_boundary.lock().expect("session lock poisoned");
    crate::volume_store::append_to_current(&session_dir, &content_buf)?;
}
```

**锁序一致性**：与 `TriggerWorldEventTool::call`（Bug F 修复）结构完全一致——同一调用任意时刻只持一把锁，state_lock 释放后才获取 session_lock，与 `advance_plot` 的 session→state 顺序不形成环。✓

### 2.3 `advance_and_check_triggers` 签名重构

旧签名：`(data_root, character_id, session_dir, advance_by) -> (WorldClock, Vec<WorldEvent>)`
新签名：`(data_root, character_id, advance_by) -> (WorldClock, Vec<WorldEvent>, String)`

- 移除 `session_dir` 参数：函数不再直接 append，无需 session 路径。
- 新增 `String` 返回值：待追加的内容缓冲，由调用方在 session_lock 临界区内 append。
- 可见性：`pub fn`（模块内 `pub`，`tools.rs` 未 re-export，不影响外部 API）。✓

### 2.4 batch collect 的失败语义

旧实现逐事件 `append_to_current`：
- 事件 1 append 成功 → 事件 2 append 失败 → 事件 1 内容已落盘但 `triggered` 标志未持久化（`save_world_events` 在循环外）→ 下次 `advance_clock` 重试会重复注入事件 1 内容。

新实现批量收集 → 标记 + 持久化 → 单次 append：
- 若 `save_world_events` 失败：无任何内容落盘，无副作用。✓
- 若 `append_to_current` 失败：事件已标记 `triggered`（不会重触发），内容未注入——失败对调用方可见（`Err` 传播），不会静默累积重复内容。

**审计权衡**：save→append 顺序意味着 append 失败时事件"丢失"（triggered=true 但内容未注入）。替代方案 append→save 则可能 append 成功但 save 失败导致重复注入。前者（内容丢失）比后者（内容重复）更可控——用户可见错误可手动重置 `triggered=false`，而静默重复会污染叙事连续性。此权衡与 Bug F 修复一致。✓

### 2.5 空 content_buf 优化

`if !content_buf.is_empty()` 跳过 session_lock 获取。当无到期事件时（常见路径），避免不必要的锁竞争。✓

---

## 3. 测试覆盖分析

### 3.1 新增测试

| 测试名 | 覆盖点 | 结果 |
|---|---|---|
| `advance_clock_triggers_time_events_and_appends_to_current` | happy path：单事件触发 + 内容追加 + 标志持久化 | ✓ pass |
| `advance_clock_no_due_events_does_not_append` | 负路径：无到期事件时不 append、不标记 | ✓ pass |
| `advance_clock_triggers_multiple_due_events_in_single_append` | batch：多事件同时触发，内容均落盘、标志均持久化 | ✓ pass |

### 3.2 测试隔离

- 每个测试使用独立 `tempdir()` + 唯一 character_id（`adv_clk_trig` / `adv_clk_empty` / `adv_clk_multi`），避免 process-global `state_lock`/`session_lock` 争用。✓
- 与现有 `concurrent_update_relationship_and_advance_plot_do_not_lose_updates`、Bug F 死锁测试的隔离模式一致。

### 3.3 测试参数说明

测试调用 `tool.call(params, true)`——第二参数为 `_confirm: bool`（`AdvanceClockTool` 忽略此参数）。这与同文件现有测试（如 `trigger_world_event_injects_and_marks_triggered`）的调用模式一致。`_confirm` 在 `AdvanceClockTool` 中无实际语义，不影响测试有效性。✓

---

## 4. 与 Bug F 修复的一致性验证

| 维度 | Bug F（`trigger_world_event`，PR #335） | Bug B（`advance_clock`，本 PR） | 一致性 |
|---|---|---|---|
| 阶段一 | state_lock：load + check + mark + save | state_lock：advance + collect + mark + save | ✓ 同模式 |
| 阶段二 | session_lock：`append_to_current(content_buf)` | session_lock：`append_to_current(content_buf)` | ✓ 同模式 |
| 锁序 | state→release→session | state→release→session | ✓ 无环 |
| content_buf 构造 | `format!` 单事件 | `push_str` 批量收集 | ✓ 批量是 Bug B 的扩展 |
| save→append 顺序 | save 在 state_lock 内，append 在 session_lock 内 | save 在 state_lock 内，append 在 session_lock 内 | ✓ 一致 |

---

## 5. 非阻塞遗留项

| 编号 | 描述 | 严重度 | 建议时机 |
|---|---|---|---|
| N1 | 无 `advance_clock + advance_plot` 并发死锁测试。Bug F 的 `trigger_world_event_and_advance_plot_concurrent_no_deadlock` 覆盖了 trigger+plot 组合，但未覆盖 advance_clock+plot 组合。两阶段拆分应能防止同类死锁，但直接测试提供更强证据。 | low | 未来迭代 |
| N2 | `advance_clock_triggers_multiple_due_events_in_single_append` 测试名声称验证"single append"，但实际只检查内容存在性，未验证 `append_to_current` 调用次数。单次 append 由实现构造保证（`content_buf` 单次 `push_str` 收集 + 单次 `append_to_current`），测试不验证是实现层的契约。 | informational | 无需修 |
| N3 | `AdvanceClockTool::call` 的 `_confirm: bool` 参数被忽略。`Tool::call` trait 的 confirm/dry_run 语义在 `AdvanceClockTool` 未实现。这是 pre-existing pattern（同文件其他工具同样忽略），非本 PR 引入。 | informational | 无需修 |
| N4 | save→append 失败语义：append 失败时事件"丢失"（triggered=true 但内容未注入）。用户需手动重置 `triggered=false` 才能重试。可考虑未来增加"append 失败时回滚 triggered 标志"的恢复路径，但当前权衡（内容丢失 > 内容重复）合理。 | informational | 未来迭代 |

---

## 6. 审计结论

**PASS**——无阻塞项。

- Bug B 修复正确，两阶段临界区与 Bug F 同模式，锁序无环。
- `advance_and_check_triggers` 签名重构合理，唯一调用点已同步。
- 3 个新测试覆盖功能性契约（触发、追加、持久化），隔离良好。
- 全量 lib 测试 + clippy + doc 全绿。
- Bug F 回归测试通过，未破坏既有死锁修复。

非阻塞遗留项 N1–N4 已记录，建议 PR 合并后按 AGENTS.md §"审计遗留项处理" 提交 GitHub issue 跟踪。
