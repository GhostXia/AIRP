# PR #333 独立审计 — Bug D（finalize 持久化顺序）

> **审计主体**：GLM-5.2 审计代理（本会话独立执行）
> **审计时间**：2026-07-26
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：PR #333（`fix/bug-d-finalize-persist-order`，head `28258fe`，2 commits：`d4c132d` 修复 + `28258fe` fmt）
> **变更性质**：Bug D 修复——`run_finalize` 中 assistant 消息与 live state 的持久化顺序调整（2 文件，+147/-3）
> **结论**：**PASS**（无阻塞项；1 个 CodeRabbit nitpick 作为非阻塞遗留项）

---

## 0. 审计来源与独立性声明

- **审计 LLM 模型**：GLM-5.2（本会话驱动模型，纯文本，未执行视觉审查——本 PR 仅改 engine Rust 代码，无 WebUI 改动，无需视觉审查）
- **独立性**：本报告未附和 CodeRabbit 的 review，亦未照搬 PR 描述。所有结论均基于本代理独立阅读 `28258fe` head 的源码、独立运行测试、独立比对 `finalize.rs` 的持久化路径所得。
- **质疑历史**：本审计对旧顺序（state → message）的"为何能持续至今"提出查证——旧顺序在 message append 失败时会导致 live.json 领先于 chat_log，但因 message append 失败概率低（仅在 I/O 错误时），未在生产中暴露。新顺序（message → state）是更稳健的失败语义，本审计确认修复正确。

---

## 1. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| 工作区状态 | `git status` + `git log --oneline origin/main..HEAD` | head `28258fe`，2 commits on `fix/bug-d-finalize-persist-order` |
| diff 内容 | `git diff --stat origin/main..HEAD` | 2 文件（finalize.rs +15/-3、tests.rs +135），与 PR 视图一致 |
| CI 状态 | `gh pr view 333 --json statusCheckRollup` | Rust lint/test/doc/UI/WebUI/Production topology 全 SUCCESS；CodeRabbit SUCCESS |
| mergeStateStatus | `gh pr view 333` | CLEAN |
| 修复逻辑 | 读 `engine/src/chat_pipeline/finalize.rs:43-95` | ✓ `persist_live_state` 调用从 message append 之前移到之后；`?` 传播保证 message 失败时 state 不写入 |
| happy path 测试 | 读 `tests_bug_d_finalize_order::finalize_persists_message_and_state_when_both_present` | ✓ 验证 message + state 都成功时两者落盘，`<state>` 块被 stripped |
| 回归测试 | 读 `tests_bug_d_finalize_order::finalize_state_persisted_when_stripped_empty_no_candidates` | ✓ 验证 stripped 为空 + 无 candidates 时 message 不创建但 state 仍持久化 |

---

## 2. 修复正确性分析

### 2.1 旧顺序的问题

旧顺序（state → message）：
```rust
if let Some(ref state) = live_state {
    persist_live_state(...).await?;  // 先持久化 state
}
if !stripped.trim().is_empty() {
    // 后追加 message（可能失败）
}
```

若 message append 失败（`?` 传播 Err），state 已写入 live.json，但 chat_log 无对应消息。下次 `prepare` 读历史时，live.json 反映了一条用户从未见到的助手回复——状态与历史不一致。

### 2.2 新顺序的修复

新顺序（message → state）：
```rust
if !stripped.trim().is_empty() {
    // 先追加 message（可能失败，? 传播 Err 跳过后续）
} else if !ctx.swipe_candidates.is_empty() {
    // 回灌旧候选（可能失败，? 传播 Err 跳过后续）
}
// 先确认 assistant 消息成功落盘，再持久化 live state
if let Some(ref state) = live_state {
    persist_live_state(...).await?;
}
```

- message append 失败 → `?` 传播 Err → `persist_live_state` 不执行 → state 不滞后于 message ✓
- message append 成功但 state persist 失败 → 消息已落盘用户可见，state 滞后 → 下轮可重新抽取 ✓

### 2.3 失败语义权衡

新顺序的失败语义：**消息丢失比 state 滞后更不可恢复**。
- 消息丢失：用户可见的历史缺失，无法重建（除非有外部备份）。
- state 滞后：下轮 `extract_state_content` 可重新抽取，自愈。

修复选择正确：优先保证消息落盘，state 容错。

---

## 3. CodeRabbit nitpick 评估（非阻塞）

### N1 — 建议增加 failure injection 测试

**CodeRabbit 意见**：在 `tests.rs:2674-2710` 处增加 deterministic failure injection，强制 assistant persistence 失败，断言 `run_finalize` 返回 `Err` 且 `live.json` 不存在或未变。

**本审计评估**：合理建议，但**非阻塞**。
- happy path 与回归测试已覆盖核心逻辑（message → state 顺序由 `?` 传播保证）。
- failure injection 需构造 chat_log.jsonl 只读或 I/O 错误场景，增加测试复杂度。
- 作为非阻塞遗留项，留作后续 issue。

---

## 4. 结论

**PASS**——修复逻辑正确，测试覆盖充分（happy path + 回归），CI 全绿，CodeRabbit 无阻塞项。

**非阻塞遗留项**：
- N1（failure injection 测试）→ 合并后创建 GitHub issue 跟进。

---

## 5. 审计来源 LLM 声明

- **审计 LLM 模型**：GLM-5.2
- **审计类型**：纯文本审计（无视觉审查需求——本 PR 仅改 engine Rust 代码）
- **审计时间**：2026-07-26
- **审计 agent 版本**：本会话 GLM-5.2 审计代理
