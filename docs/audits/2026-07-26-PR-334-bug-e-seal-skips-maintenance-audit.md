# PR #334 独立审计 — Bug E（封卷跳过周期维护）

> **审计主体**：GLM-5.2 审计代理（本会话独立执行）
> **审计时间**：2026-07-26
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：PR #334（`fix/bug-e-seal-skips-maintenance`，head `3bb96ef`，1 commit）
> **变更性质**：Bug E 修复——`run_finalize` 中封卷轮次跳过周期维护（2 文件，+136/-1）
> **结论**：**PASS**（无阻塞项，无 CodeRabbit 阻塞意见）

---

## 0. 审计来源与独立性声明

- **审计 LLM 模型**：GLM-5.2（本会话驱动模型，纯文本，未执行视觉审查——本 PR 仅改 engine Rust 代码，无 WebUI 改动，无需视觉审查）
- **独立性**：本报告未附和 CodeRabbit 的 review，亦未照搬 PR 描述。所有结论均基于本代理独立阅读 `3bb96ef` head 的源码、独立比对 `volume_manager.rs` 中 `run_seal_flow` 与 `run_maintenance` 的 `index.md` 写路径所得。
- **质疑历史**：本审计对"维护与封卷为何原本允许并发"提出查证——旧代码将维护触发条件仅基于 `turn_count % interval == 0`，未考虑本轮是否触发封卷。封卷是低频事件，且维护触发间隔通常 ≥10 轮，两者同时命中的概率低，未在生产中暴露。但并发写 `index.md` 的 read-modify-write 丢更新是确定性风险，修复正确。

---

## 1. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| 工作区状态 | `git status` + `git log --oneline origin/main..HEAD` | head `3bb96ef`，1 commit on `fix/bug-e-seal-skips-maintenance` |
| diff 内容 | `git diff --stat origin/main..HEAD` | 2 文件（finalize.rs +9/-1、tests.rs +133），与 PR 视图一致 |
| CI 状态 | `gh pr view 334 --json statusCheckRollup` | Rust lint/test/doc/UI/WebUI/Production topology 全 SUCCESS；CodeRabbit SUCCESS |
| mergeStateStatus | `gh pr view 334` | CLEAN（审计报告 commit 推送前） |
| 修复逻辑 | 读 `engine/src/chat_pipeline/finalize.rs:139-152` | ✓ 维护触发条件新增 `!should_seal`，封卷时跳过 |
| 封卷跳过测试 | 读 `tests_bug_e_seal_skips_maintenance::finalize_skips_maintenance_when_seal_triggered` | ✓ 封卷时维护被跳过，Alpha 不被晋升 |
| 控制组测试 | 读 `tests_bug_e_seal_skips_maintenance::finalize_runs_maintenance_when_no_seal` | ✓ 无封卷时维护正常执行，Alpha 被晋升 |
| index.md 写冲突验证 | 读 `engine/src/volume_manager.rs:219/280`（seal）+ `430/470`（maintenance） | ✓ 两者都对 `index.md` 做 read-modify-write，并发会丢更新 |

---

## 2. 修复正确性分析

### 2.1 旧代码的并发风险

`run_finalize` 的 volume side-effects 阶段：
- `run_seal_flow`（若 `should_seal=true`）：对 `index.md` 做 read-modify-write（`volume_manager.rs:219/280`）
- `run_maintenance`（若 `turn_count % interval == 0`）：对 `index.md` 做 read-modify-write（`volume_manager.rs:430/470`）

两者通过 `join_set.spawn` 并发执行（`finalize.rs:148`），无锁保护 `index.md`。并发时后写者覆盖先写者的 diff，导致：
- 卷索引丢失（seal 写入的卷边界被 maintenance 覆盖）
- 跨卷实体晋升丢失（maintenance 计算的晋升被 seal 覆盖）

### 2.2 修复方式

```rust
if !should_seal && turn_count > 0 && turn_count % interval == 0 {
    // 仅非封卷轮次执行维护
}
```

封卷时跳过维护，维护推迟到下一个非封卷轮次触发。

### 2.3 修复语义权衡

- **封卷优先**：封卷是本轮的主操作（用户主动触发的卷过渡），维护是辅助操作（周期性整理）。封卷失败的影响大于维护推迟。
- **维护自愈**：维护推迟一轮后，下轮 `turn_count % interval == 0` 仍可能成立（若 interval=1）或推迟到下个 interval。实体晋升只是延后，不会永久丢失。
- **不跳过封卷**：若反向选择（封卷跳过、维护优先），则用户的封卷操作被丢弃——不可接受。

修复选择正确：封卷优先，维护推迟。

---

## 3. 测试覆盖评估

### 3.1 测试策略

测试通过 `maintenance_interval=1` 确保每轮都触发维护条件，然后：
- 封卷测试：`<卷评估 封存="true"/>` → 维护应被跳过 → Alpha 不被晋升
- 控制组：普通正文 → 维护应执行 → Alpha 被晋升

两个测试共用 `seed_session_with_promotable_entity`（3 卷都含 Alpha，满足晋升阈值 3）。

### 3.2 测试隔离

- 封卷测试用 `sessions/seal_skip_main/s1`
- 控制组用 `sessions/no_seal_main/s1`
- 两者 data_root 独立（tempdir），无锁争用

### 3.3 测试充分性

- ✓ 封卷跳过验证（核心修复点）
- ✓ 控制组对照（证明"跳过"是修复的精确行为，非环境问题）
- ✓ `character_id=None` 跳过消息追加，仅激活 volume side-effects 路径

---

## 4. 结论

**PASS**——修复逻辑正确，测试覆盖充分（封卷跳过 + 控制组对照），CI 全绿，CodeRabbit 无阻塞项。

无非阻塞遗留项。

---

## 5. 审计来源 LLM 声明

- **审计 LLM 模型**：GLM-5.2
- **审计类型**：纯文本审计（无视觉审查需求——本 PR 仅改 engine Rust 代码）
- **审计时间**：2026-07-26
- **审计 agent 版本**：本会话 GLM-5.2 审计代理
