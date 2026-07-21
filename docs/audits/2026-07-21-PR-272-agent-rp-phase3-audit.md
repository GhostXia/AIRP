# PR #272 独立审计报告 — 阶段三 Agent RP 差异化（World Events/NPC/Plot）

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-21
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#272 feat: 阶段三 - Agent RP 差异化 (World Events/NPC/Plot)](https://github.com/GhostXia/AIRP/pull/272)
- **分支**：`feat/phase3-agent-rp` → `main`
- **base commit**：`2e7b23e docs: mark first-chat golden path as default implementation baseline`
- **PR 原始范围**：6 files, +602/-11（3 新工具模块 + 3 既有点改）
- **审计后范围**：9 files, +1130/-13（含审计修复 + 端到端测试）
- **审计 commits**（本分支追加）：
  - `32b8419` audit(PR-272): 修复并发竞态 + revision 合同绕过 (A1/A2/A3)
  - `940b20e` audit(PR-272): 新增 9 个端到端测试覆盖审计修复点
  - `9559931` audit(PR-272): cargo fmt --all

## 1. 范围与背景

PR #272 落实 RP 能力增强工程阶段三，新增 6 个 Agent 工具：

| 工具 | family | side_effect | 数据落点 |
|---|---|---|---|
| `trigger_world_event` | world_event | Mutate | `characters/{id}/world_events.json` + `session/current.md` |
| `list_world_events` | world_event | Readonly | 读取 `world_events.json` |
| `npc_action` | npc | Mutate | `session/current.md` |
| `update_relationship` | npc | Mutate | `state/live.json` `relationships` 字段 |
| `advance_plot` | plot | Mutate | `state/live.json` `plot_history` 字段 + `session/current.md` |
| `get_plot_status` | plot | Readonly | 读取 `state/live.json` + `session/index.md` |

工具总数：21 → 27。新增模块 `engine/src/agent/tools/{world_event,npc,plot}.rs`，
经 `default_registry` 集中注册。

本审计按 §Audit Agent Charter 三原则独立执行：
1. 不附和开发 agent 的结论，以"我会不会这样写"为判据；
2. 可提出自己的设计意见，哪怕与既定路线相悖；
3. 可质疑历史决策并主动查证（读源码、跑测试）。

## 2. 独立证据

### 2.1 原始 PR diff 核查（base `2e7b23e` .. head before audit）

| 文件 | 改动 | 性质 |
|---|---|---|
| `engine/src/agent/tools/world_event.rs` | +196 | 新增 |
| `engine/src/agent/tools/npc.rs` | +166 | 新增 |
| `engine/src/agent/tools/plot.rs` | +189 | 新增 |
| `engine/src/agent/tools.rs` | +11/-1 | 注册 3 个 family |
| `engine/src/agent/tools/tests/registry.rs` | +38/-8 | 快照扩到 27 工具 |
| `engine/src/daemon/tests/catalog.rs` | +2/-2 | catalog 测试更新 |

原始 PR 无端到端测试覆盖新工具的并发、revision 合同、幂等等关键不变式。

### 2.2 阻塞问题独立复现

#### A1 — JSON 解析错误被静默吞掉（数据完整性）

原始 `load_world_events` 实现（审计前）：

```rust
let events: Vec<WorldEvent> =
    serde_json::from_str(&content).unwrap_or(Default::default());
```

`unwrap_or(Default::default())` 把任何 JSON 解析错误（文件损坏、schema 漂移、
并发半写）都吞成"空列表"。下游 `trigger_world_event` 会把"空列表"当成
"无已触发事件"，对已触发过的事件重复注入到 `current.md`，造成对话污染。

独立复现：手写一段损坏 JSON `[{ "id":` 放入 `world_events.json`，原实现
返回 `[]` 而非报错——确认错误被静默吞掉。

#### A2 — 三个 mutate 工具的 read-modify-write 无锁（并发竞态）

原始实现：

- `update_relationship`：`std::fs::read_to_string(live.json)` → 修改
  `relationships` → `std::fs::write` 覆写
- `advance_plot`：同上模式，修改 `plot_history`
- `trigger_world_event`：`load_world_events` → 检查 `triggered` 标记 →
  注入 + 标记 → `save_world_events`

三者均未持有 `state_lock` / `character_lock`。`domain.rs` 已存在
`StateService::write` 走 `state_lock` + `character_lock` + `replace_file`
+ `history.jsonl` + `revisions/{n}/` 的完整 #115 Phase 2e 合同，但原 PR
未复用，直接裸 `fs::read`/`fs::write`。

独立复现：用 `futures_util::future::join_all` 并发 5 个
`update_relationship` + 5 个 `advance_plot`，原实现会丢失部分更新
（`relationships` 或 `plot_history` 条目数 < 5）。详见审计新增测试
`concurrent_update_relationship_and_advance_plot_do_not_lose_updates`。

#### A3 — 绕过 #115 Phase 2e revision 合同（数据合同破坏）

原始实现直接 `std::fs::write` 写 `live.json` / `world_events.json`：
- 不走 `data_dir::replace_file` 原子写（tmp + rename + fsync parent），
  读者可能看到半写状态；
- 不 append `state/history.jsonl`，丢失 revision 历史；
- 不创建 `revisions/{content_revision}/` 不可变快照，破坏 #115 合同的
  "任意 revision 可回放"承诺。

`StateService::write` 已有完整合同实现，原 PR 未复用。

### 2.3 审计修复独立验证

#### 修复 A1 — 传播解析错误

`load_world_events` 改为：

```rust
serde_json::from_slice::<Vec<WorldEvent>>(&bytes).map_err(|e| {
    AirpError::Internal(format!(
        "failed to parse world_events.json for {}: {e}",
        cid.as_str()
    ))
})?
```

下游 `trigger_world_event` 拿到 `Err` 直接返回，不再误判"无已触发事件"。

#### 修复 A2 — 串行化 read-modify-write

新增 `StateService::mutate`（`domain.rs` L834）：

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
    let _character_guard = character.read().expect("character lock poisoned");
    let state_boundary = state_lock(character_id.as_str());
    let _state_guard = state_boundary.lock().expect("state lock poisoned");
    // ... read live.json -> mutate(&mut value) -> commit_state_under_lock
}
```

- `update_relationship` / `advance_plot` 改走 `StateService::mutate`，
  与 `StateService::write` 共享同一把 `state_lock` + `character_lock`。
- `trigger_world_event` 对 `world_events.json` 的 check-then-act 临界区
  也显式持有 `state_lock(cid.as_str())`，与 live.json 写入共享同一把锁
  （`state_lock` 在 `domain.rs` 中改为 `pub(crate)`，允许 sibling 模块
  参与同一串行化合同）。
- `state_lock` 是 per-character 的，不同角色之间不互斥。

#### 修复 A3 — 接入 revision 合同

- `StateService::mutate` 内部调用新提取的 `commit_state_under_lock`
  helper（`domain.rs` L888），与 `StateService::write` 共享：
  `data_dir::replace_file` 原子写 + `history.jsonl` append +
  `revisions/{content_revision}/state.json` 不可变快照。
- `save_world_events` 改用 `crate::data_dir::replace_file(&path, &content)?`
  原子写（不再裸 `fs::write`）。

#### 端到端测试覆盖（9 个新测试，全部通过）

`engine/src/agent/tools/tests/agent_rp_phase3.rs`：

| 测试 | 覆盖点 |
|---|---|
| `update_relationship_writes_live_json_with_revision_contract` | live.json + history.jsonl + revisions/1/state.json 三件套 |
| `advance_plot_appends_plot_history_under_revision_contract` | current.md 注入 + live.json plot_history + revision |
| `get_plot_status_returns_history_and_pending_clues` | readonly 读取 live.json + index.md |
| `trigger_world_event_injects_and_marks_triggered` | 事件注入 + triggered 标记 |
| `trigger_world_event_is_idempotent_for_already_triggered` | **幂等性**：已触发事件不重复注入 |
| `trigger_world_event_unknown_id_returns_not_found` | 边界：未知 event_id |
| `list_world_events_reflects_triggered_state` | list 输出反映 triggered 状态 |
| `npc_action_appends_to_current_md` | NPC 行动注入 current.md |
| `concurrent_update_relationship_and_advance_plot_do_not_lose_updates` | **并发**：5+5 并发调用，state_lock 串行化，无丢更新 |

### 2.4 本地验证全绿

| 门禁 | 命令 | 结果 |
|---|---|---|
| fmt | `cargo fmt --check` | clean |
| clippy | `cargo clippy -p airp-core --all-targets -- -D warnings` | 0 warnings |
| lib tests | `cargo test --workspace --lib` | 775 passed, 0 failed, 1 ignored |
| integration tests | `cargo test --workspace --tests` | 全部通过，0 failed |
| rustdoc | `cargo doc --workspace --no-deps` | clean |
| 新增测试 | `cargo test -p airp-core --lib agent::tools::tests::agent_rp_phase3` | 9/9 passed |
| 既有 chat_store | `cargo test -p airp-core --lib chat_store::tests` | 24/24 passed |

### 2.5 "3 个 chat_store 失败"的归因独立核查

审计过程中观察到 `cargo test --workspace --tests` 在某次运行中报 3 个
`chat_store::tests` 失败（`active_path_indices_legacy_linear_fallback` /
`children_of_case_insensitive_match` / `resolve_active_leaf_case_insensitive_match`）。

独立查证：
- `git log --oneline -1 engine/src/chat_store.rs`（在 pr-270 分支）->
  `dbe67ef feat: 阶段一 - 对话体验补全`，即 PR #270 的提交。
- PR #272 的 base 是 `2e7b23e`，**早于** `dbe67ef`，PR #272 不含
  `chat_store.rs` 改动。
- 在 pr-272 分支上单独跑 `cargo test -p airp-core --lib chat_store::tests`
  -> **24/24 passed**。

结论：3 个失败是 pr-270 分支的预存问题，与 PR #272 无关。审计不就
pr-270 的失败提出意见，留待 pr-270 自身审计处理。

## 3. 独立意见（按 §Audit Agent Charter 第 2 条）

### 3.1 同意"复用 `volume_store::append_to_current` 不新增注入路径"的设计

PR 描述明确"事件注入走 `volume_store::append_to_current`（不新增注入路径）"。
本审计同意此约束：避免在 `current.md` 之外另开注入入口，保持 session 记忆
单一写入边界。`npc_action` / `advance_plot` / `trigger_world_event` 都
遵守此约束。

### 3.2 关于 `state_lock` 改为 `pub(crate)` 的取舍

审计修复将 `domain.rs` 的 `state_lock` 从 `fn`（private）改为
`pub(crate) fn`，让 `agent::tools::world_event` 能参与同一串行化合同。

本审计认为此取舍合理：
- `pub(crate)` 不暴露给外部 crate，不污染 public API；
- 替代方案（在 `domain.rs` 内新增 `world_events` 专用锁或把
  `trigger_world_event` 的临界区逻辑搬到 `domain.rs`）会引入更重的
  耦合，且 `world_events.json` 本质上就是 per-character state 的一部分，
  共享 `state_lock` 语义正确。
- 但应明确：`pub(crate)` 是 crate 内部的串行化合同，不是 public
  stability 承诺。已在 `state_lock` 文档注释中说明。

### 3.3 关于 `world_events.json` 未接入 revision 合同的留白

审计修复让 `world_events.json` 改用 `data_dir::replace_file` 原子写，
但**未**接入 #115 Phase 2e 的完整 revision 合同（`history.jsonl` +
`revisions/{n}/` 快照）。原因：接入需要新增 `AssetKind::WorldEvents`
枚举变体 + revision manifest 路径，超出本审计的"最小修复"范围。

本审计认为此留白可接受：
- `world_events.json` 是低频写入的事件配置（不是高频对话状态），
  原子写已解决半写问题；
- revision 合同的"可回放"承诺对 `world_events.json` 的价值低于对
  `live.json`（后者每轮对话都可能变更）；
- 接入应作为独立 PR 处理，避免本审计修复的 scope 膨胀。

留作非阻塞跟进项 F-1。

### 3.4 关于 `npc_action` 未持 `session_lock` 的留白

`npc_action` 通过 `volume_store::append_to_current` 写 `session/current.md`，
但未持 `session_lock`。若并发 `seal_volume` 正在归档 `current.md`，
`append_to_current` 的原子 append 仍可能落在已被 `seal_volume` 清空的
文件上，导致追加丢失。

本审计认为此风险低：
- `append_to_current` 走 `OpenOptions::append(true)` 原子追加，
  不会被 `seal_volume` 的 `fs::rename` / `fs::write` 截断；
- `seal_volume` 是 Destructive 工具，需 `confirm=true`，正常使用场景
  不会与 `npc_action` 并发；
- `session_lock` 当前是 `fn`（private），改为 `pub(crate)` 的成本与
  `state_lock` 相同，但收益更低。

留作非阻塞跟进项 F-3，建议在 `npc_action` 文档中注明此约束。

### 3.5 关于审计修复扩大 PR scope 的透明性

审计修复在 `domain.rs` 新增 `StateService::read` / `StateService::mutate`
/ `commit_state_under_lock`，扩大了 PR 的 scope（从 6 files -> 9 files）。
本审计认为此扩大是必要的：
- A2/A3 的修复无法在不改 `domain.rs` 的前提下完成（工具层无法自行
  串行化 `state_lock`）；
- 新增方法是 `pub`（非 `pub(crate)`），但 `StateService` 本身已是
  public type，新增方法是自然的 API 扩展；
- 所有新增方法都有 doc comment + 端到端测试覆盖。

建议 PR 描述更新"变更摘要"以反映审计后的实际 scope。

## 4. 风险评估

| 风险 | 评级 | 说明 |
|---|---|---|
| 并发丢更新 | 低（修复后） | A2 已通过 `state_lock` 串行化 + 并发测试覆盖 |
| 数据合同破坏 | 低（修复后） | A3 已通过 `StateService::mutate` / `replace_file` 接入 revision 合同 |
| 解析错误静默 | 低（修复后） | A1 已改为传播 `AirpError::Internal` |
| `pub(crate) state_lock` 滥用 | 低 | 文档注释明确"crate 内部串行化合同"语义 |
| `world_events.json` revision 缺失 | 中 | F-1 跟进；当前原子写已解决半写 |
| `npc_action` vs `seal_volume` 并发 | 低 | F-3 跟进；`append_to_current` 原子追加 + Destructive confirm 门 |
| scope 膨胀 | 低 | 审计修复必要且最小化，有测试覆盖 |

## 5. 阻塞项

**无（审计修复后）**。

A1 / A2 / A3 三个阻塞项已在 commit `32b8419` 修复，端到端测试在
commit `940b20e` 覆盖，本地全部门禁绿。

## 6. 非阻塞 / 后续可追踪项

| 编号 | 内容 | 建议 |
|---|---|---|
| 272-F-1（非阻塞） | `world_events.json` 未接入 #115 Phase 2e revision 合同（缺 `AssetKind::WorldEvents` + manifest） | 独立 PR 处理；当前 `replace_file` 原子写已解决半写 |
| 272-F-2（非阻塞） | 6 个新工具中 `update_relationship` / `advance_plot` 标为 `Mutate`，未走 `confirm` dry-run 流 | 工具分类合理（非 Destructive）；可在 M_AGENT-5 确认流统一设计时复查 |
| 272-F-3（非阻塞） | `npc_action` 写 `current.md` 未持 `session_lock`，与 `seal_volume` 并发时理论上可能丢追加 | 低风险（原子 append + Destructive confirm 门）；建议在 `npc_action` 文档注明此约束 |
| 272-F-4（非阻塞） | PR 描述"变更摘要"未反映审计后的实际 scope（9 files） | 建议在 push 后更新 PR 描述，注明审计修复的 3 个 commit |

## 7. 审计结论

**通过（PASS，审计修复后无阻塞项）**。

PR #272 原始实现存在 3 个阻塞问题（A1 数据完整性 / A2 并发竞态 /
A3 revision 合同绕过），审计已在 commit `32b8419` + `940b20e` +
`9559931` 修复并覆盖端到端测试。本地全部门禁绿（fmt / clippy / lib
775+1 ignored / integration / rustdoc / 9 新增测试 / chat_store 24/24）。

3 个 `chat_store::tests` 在 pr-270 分支的失败经独立核查为预存问题，
与 PR #272 无关。

4 个非阻塞跟进项（F-1 ~ F-4）建议在 PR 合并后写入 GitHub issue
（按根 `AGENTS.md` §审计遗留项处理 时序约束：PR 合并后提交）。

可合并。

## 8. Refs

- 根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- 根 `AGENTS.md` §审计遗留项处理（PR 合并后写入 GitHub issue）
- #115 Phase 2e revision 合同（`StateService::write` 的 `replace_file` + `history.jsonl` + `revisions/{n}/` 三件套）
- `engine/src/domain.rs` L76 `pub(crate) fn state_lock` / L802 `StateService::read` / L834 `StateService::mutate` / L888 `commit_state_under_lock`
- `engine/src/agent/tools/{world_event,npc,plot}.rs` 审计修复
- `engine/src/agent/tools/tests/agent_rp_phase3.rs` 9 个端到端测试
