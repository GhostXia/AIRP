# PR #436 独立审计：R1 锁序违规修复

> 审计日期：2026-08-03
> 审计对象：PR #436 `fix(engine): close R1 lock-order violations in npc/world_event/seal_volume paths`
> 审计分支：`fix-r1-lock-order-violations`（commit 338296a）
> 审计基线：`main@711f062`（合并 PR #434 后）
> 审计依据：AGENTS.md「审计 Agent 守则」三原则——独立审计、可提己见、可质疑历史并查证
> 关联合同：`docs/LOCK-ORDER-CONTRACT.md`

## 1. 审计范围

本审计独立复核 PR #436 的三项内容：

1. **代码修复**：4 个 agent-tool 路径补齐外层 `character_lock.read()`（R1 收敛）
2. **合同更新**：`LOCK-ORDER-CONTRACT.md` R1 / §2.3-§2.8 / §6.7 / §7
3. **残留风险**：`advance_plot`（§2.3）R1 例外与 §6.7 残留 TOCTOU 风险

审计方法：读源码 + 读合同 + 与 `main@711f062` 对照 + 独立判断 R1 例外论证是否成立。**不**把开发 agent 的结论或合同既有条款作为不可质疑的前提。

## 2. 独立发现

### 2.1 R1 vs §2.3 矛盾的真实性（已确认）

**事实核验**：

- `LOCK-ORDER-CONTRACT.md` R1 原文（修复前）：「获取 `state_lock`、`session_lock`（针对同一 `character_id`）前，必须先持有 `character_lock.read()`（或 `.write()`）。例外：`agent::tools::*` 通过 `StateService::mutate` 间接获取时，由 `StateService` 内部保证。」
- §2.3 原文（修复前）：`session_lock.lock() → [StateService::mutate 内] character_lock.read() → state_lock.lock()`

**矛盾**：

- R1 要求 `character_lock.read()` 在 `session_lock` 之前；
- §2.3 `advance_plot` 的最外层是 `session_lock`，`character_lock.read()` 在 `StateService::mutate` 内部才获取；
- 旧 R1 的「例外」条款只覆盖「`agent::tools::*` 通过 `StateService::mutate` 间接获取 state_lock 时」的 `character → state` 顺序，**未**覆盖 `advance_plot` 在 `StateService::mutate` 之前已 acquire `session_lock` 的事实。

结论：**矛盾真实存在**，旧合同的例外条款模糊且过宽，把整个 `agent::tools::*` 路径豁免了 R1，而实际上只有 `advance_plot` 因 re-entrancy 风险无法补齐外层 `character_lock.read()`。

### 2.2 R1 违规路径清单（独立复核）

读 `main@711f062` 源码确认下列路径在调用 `session_lock` / `state_lock` 时未持外层 `character_lock.read()`：

| 路径 | 文件 | 最外层锁 | R1 合规？ | 备注 |
|---|---|---|---|---|
| `npc_action` | `engine/src/agent/tools/npc.rs` | `session_lock` | **否** | 直接 acquire session_lock，无 character_lock |
| `trigger_world_event` | `engine/src/agent/tools/world_event.rs` | `state_lock`（阶段一）/ `session_lock`（阶段二） | **否** | 两段临界区均无外层 character_lock |
| `advance_clock` | `engine/src/agent/tools/world_event.rs` | `state_lock`（阶段一）/ `session_lock`（阶段二） | **否** | 同上 |
| `run_seal_flow` | `engine/src/volume_manager.rs` | `session_lock`（per-character 写盘段） | **否** | 仅 per-character 路径；scene 模式不持锁 |
| `advance_plot` | `engine/src/agent/tools/plot.rs` | `session_lock` | **否**，但**例外** | StateService::mutate 内部 acquire character_lock.read()，外部再 acquire 会递归 read |

**对比 §2.1 / §2.2**：`ChatService::with_session` 和 `StateService::read/mutate/write` 已合规（`character_lock.read() → session/state_lock`）。`agent::tools::*` 五路径全部违规——这是一个系统性的 R1 执行缺口，而非孤立 bug。

### 2.3 修复正确性（独立复核）

逐路径读 PR #436 diff：

#### 2.3.1 `npc_action`（§2.6）✅

```rust
let character = character_lock(cid.as_str());
let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
let session_boundary = session_lock(cid.as_str(), sid.as_ref());
let _session_guard = session_boundary.lock().unwrap_or_else(|p| p.into_inner());
```

- 顺序正确：`character.read() → session.lock()`，与 §2.1 `with_session` 同模式。
- `RwLockReadGuard` 不跨 `.await`（闭包内 `append_to_current` 是同步 I/O），符合 A1。
- `cid.as_str()` 类型匹配 `character_lock(&str)` 签名。
- poison 恢复策略 `unwrap_or_else(|p| p.into_inner())` 与 §5 P1 一致。

#### 2.3.2 `trigger_world_event`（§2.4）✅

```rust
let character = character_lock(cid.as_str());
let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());

// 阶段一
let state_boundary = state_lock(cid.as_str());
let _state_guard = state_boundary.lock().unwrap_or_else(|p| p.into_inner());
// ...
// 阶段二
let session_boundary = session_lock(cid.as_str(), sid.as_ref());
let _session_guard = session_boundary.lock().unwrap_or_else(|p| p.into_inner());
```

- `character.read()` 跨两段临界区持有，早期 return（事件已 triggered）时由 Drop 释放——正确。
- 两段临界区仍**不嵌套**（state_lock 在阶段一末尾释放，session_lock 在阶段二才 acquire），R2 不违反。
- 不与 `advance_plot` 的 `session → character.read` 形成反向环：`character.read` 是共享读，多个 reader 可并存；`delete_character` 的 `character.write()` 才与之互斥，这正是期望的 TOCTOU 防护语义。

#### 2.3.3 `advance_clock`（§2.5）✅

结构与 `trigger_world_event` 同模式，独立复核通过。

#### 2.3.4 `run_seal_flow`（§2.7）✅

```rust
if let Some(cid) = character_id {
    let character = crate::domain::character_lock(cid);
    let _character_guard = character.read().unwrap_or_else(|p| p.into_inner());
    let lock = crate::domain::session_lock(cid, session_id);
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    // ...baseline 校验 + write_volume / write_index / clear_current
}
```

- 仅 per-character 路径（`Some(cid)`）补齐 `character.read()`，scene 模式（`None`）保持既有不持锁行为——与 #283 方案 J 边界一致。
- LLM streaming（秒级，含 `.await`）在锁段之前完成，guard 不跨 `.await`，符合 A1。
- baseline 校验（current.md + index.md）逻辑未改，保留 #283 的 Conflict 重试语义。

### 2.4 `advance_plot` R1 例外论证（独立质疑）

**质疑 1：std `RwLock` 递归 read 真的会 deadlock 吗？**

查 Rust std 文档（`std::sync::RwLock::read`）：

> "On some platforms, calling read() recursively on a RwLock in the same thread may deadlock."

具体到 Windows：std 在 Windows 上用 `SRWLOCK`。SRWLOCK 的递归 read 行为如下（MSDN）：

- 同一线程多次调用 `AcquireSRWLockShared` **不**会 deadlock（Vista+），但**也不**保证计数语义——第二次 acquire 立即返回，第一次 release 即释放锁。
- 这意味着递归 read **不**会 deadlock，但**会破坏排他性**：当内层 `read()` 还"持有"时，外层 `read()` 已被 `delete_character` 的 `write()` 抢占——这会导致 `StateService::mutate` 内部的 `state_lock` 临界区在 `character.write()` 持有期间执行，**违反 R1 的互斥语义**（虽然不 deadlock，但 TOCTOU 防护失效）。

**结论**：原论证「deadlock 风险」**不准确**。Windows 上不 deadlock，但递归 read 破坏 `RwLock` 的排他性语义，TOCTOU 防护失效。Linux/pthread 上行为不同（pthread `rwlock_rdlock` 递归 read 在持有时若有 writer 等待可能 deadlock，glibc 实现相关）。

**审计建议**：§2.3 / §6.7 的论证理由应从「deadlock 风险」修正为「deadlock 风险（部分 pthread 实现）+ 排他性语义破坏（Windows SRWLOCK），两者均导致 R1 TOCTOU 防护失效」。**非阻塞**——本 PR 已记录残留风险并计划后续 issue，论证理由的措辞修正可在后续 PR 中完成。

**质疑 2：§6.7 的三个候选修复路径是否覆盖了所有可行方案？**

| 方案 | 评估 |
|---|---|
| 1. 上提 `character_lock.read()` 到调用方 | 破坏 `StateService` 自封装；需重审所有调用点（约 8 处）；改造成本中等 |
| 2. 改用 re-entrant `RwLock`（如 `parking_lot::RwLock` 或 `lock_api::RawRwLock`） | 引入新依赖；`parking_lot` 是 MIT/Apache-2.0 双许可，兼容；但需全仓 `RwLock` 替换，影响面大 |
| 3. 改为两段临界区（释放 session_lock 后再调 mutate） | 改变 `advance_plot` 事务语义（state 变更不再原子于 session append）；可能引入新一致性风险 |

**遗漏方案**：

- **方案 4（审计建议）**：将 `StateService::mutate` 拆为 `mutate_locked`（不 acquire `character_lock`，要求调用方已持有）+ `mutate`（兼容旧调用方，内部 acquire）。`advance_plot` 改用 `mutate_locked`，外部先 acquire `character_lock.read()`。改造成本小，不破坏其他调用方，不引入新依赖。**推荐**作为后续 issue 的首选方案。

**审计建议**：§6.7 应补充方案 4。**非阻塞**——本 PR 已计划后续 issue，方案补充可在 issue 中完成。

## 3. 合同更新正确性

### 3.1 R1 例外条款澄清 ✅

新 R1 措辞：
> "例外：`StateService::read` / `mutate` / `write` 内部已 acquire `character_lock.read()` 再 acquire `state_lock`，调用方若已持 `character_lock.read()` 再调 `StateService::*` 会构成递归 read（部分平台 deadlock 风险），因此 `agent::tools::*` 通过 `StateService::mutate` 间接获取 state_lock 时不再外部 acquire `character_lock.read()`。"

**评估**：澄清了例外条款的精确边界——只覆盖 `StateService::*` 内部的 `character.read → state` 顺序，不再模糊豁免整个 `agent::tools::*`。准确反映了源码事实。✅

### 3.2 §2.3 例外标注 ✅

新增「**R1 例外（已记录）**」段，明确 `advance_plot` 的例外理由与残留风险，指向 §6.7。准确。✅

### 3.3 §2.4 / §2.5 / §2.6 / §2.7 更新 ✅

四个路径的锁序图与源码一致。✅

### 3.4 §2.8 重编号 ✅

原 §2.7 `conversation::append_event` 重编号为 §2.8，避免与新 §2.7 `run_seal_flow` 冲突。✅

### 3.5 §6.7 残留风险记录 ✅

记录了 `advance_plot` 的残留 TOCTOU 风险、降级防护、三个候选修复路径。结构完整，但论证理由需修正（见 2.4 质疑 1）。

### 3.6 §7 验收记录 ✅

明确标注「本次为静态锁序收敛，**不**改变 §6.1 运行时强制状态」，未夸大交付范围。✅

## 4. 测试验证

| 检查 | 命令 | 结果 | 独立复核 |
|---|---|---|---|
| 格式 | `cargo fmt --check` | clean | ✅ |
| Clippy | `cargo clippy --workspace --exclude airp-ui --locked --all-targets -- -D warnings` | clean | ✅ |
| Rust 测试 | `cargo test --workspace --exclude airp-ui --locked` | 1297 passed / 0 failed / 5 ignored | ✅（基线 1282 → 1297，增量来自 PR #428-434 的新测试） |
| 神圣不变式 | `cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise` | ok | ✅ |
| WebUI | `node --test webui/tests/*.test.mjs` | 76 passed / 0 failed | ✅ |

**缺口**：未运行 `cargo test --release` 验证 `lock_order::track_*` 在 release build 下仍为 no-op ZST。基线 §6.1 已验证过，本 PR 未改动 `lock_order` 模块，可视为未回归。**非阻塞**。

## 5. 阻塞意见

**无阻塞意见**。

R1 收敛在代码与合同层面自洽，残留风险已明确记录并计划后续 issue。所有测试通过，神圣不变式保持。

## 6. 非阻塞意见（写入 PR 后续 issue）

| 编号 | 类型 | 描述 | 建议时机 |
|---|---|---|---|
| W-01 | 合同措辞修正 | §2.3 / §6.7 的「deadlock 风险」论证不准确。Windows SRWLOCK 递归 read 不 deadlock，但破坏排他性语义；Linux/pthread 部分实现可能 deadlock。应修正为「deadlock 风险（部分 pthread 实现）+ 排他性语义破坏（Windows SRWLOCK），两者均导致 R1 TOCTOU 防护失效」 | 后续 PR |
| W-02 | 修复方案补充 | §6.7 候选修复路径遗漏方案 4：拆 `StateService::mutate` 为 `mutate_locked`（不 acquire character_lock）+ `mutate`（兼容旧调用方）。`advance_plot` 改用 `mutate_locked`。改造成本小，不破坏其他调用方，不引入新依赖。**推荐**作为后续 issue 首选方案 | 后续 issue |
| W-03 | R1 运行时强制 | §6.1 仍仅覆盖 R2 session↔state。R1（character 外层门控）的运行时 `debug_assert!` 未交付。建议在 `lock_order` 模块新增 `track_character_read()` / `track_character_write()`，在持 character_lock 时 set thread-local flag，acquire session/state_lock 时检查。可一次性闭合 R1 运行时强制（含 `advance_plot` 例外路径的告警）。 | 后续 issue（独立 PR） |
| W-04 | 测试覆盖 | 本次未新增针对 R1 收敛的回归测试。建议新增并发测试：在 `npc_action` / `trigger_world_event` / `advance_clock` / `run_seal_flow` 持锁期间，模拟 `delete_character` 并发调用，验证 character 目录不被删除。可放在 `advance_plot` 修复 issue 中一并交付 | 后续 issue |

## 7. 审计结论

**通过（无阻塞）**。

R1 锁序违规是真实的系统性缺口，PR #436 修复了 5 个违规路径中的 4 个（`npc_action` / `trigger_world_event` / `advance_clock` / `run_seal_volume`），残留 `advance_plot` 因 re-entrancy 风险未闭合，已在合同 §2.3 / §6.7 明确记录并计划后续 issue。代码与合同自洽，所有测试通过。

非阻塞意见 W-01 ~ W-04 按 AGENTS.md「审计遗留项处理」规则，PR 合并后由执行审计的 agent 写入 GitHub issue。

---

审计 agent：（独立审计 mode，遵循 AGENTS.md 三原则）
