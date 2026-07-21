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
  - `660f250` audit(PR-272): 独立审计报告 + 修复总结
  - `fe2e9ae` audit(PR-272): CodeRabbit + Gemini 跟进修复（F-3 部分 + 防御性类型检查 + 测试并行死锁修复）
  - (本 commit) audit(PR-272): 修正 F-3"已解决"为"部分解决"并新增 F-5（`seal_volume` 未持 `session_lock`）；Nitpick 跟进跳过测试新增（race 真实但修复非最小）

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

#### 端到端测试覆盖（13 个新测试，全部通过）

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
| `update_relationship_returns_internal_when_live_json_is_not_object` | **Gemini #2**：live.json 非 Object 时返回 Internal 而非 panic |
| `update_relationship_returns_internal_when_relationships_field_is_wrong_type` | **Gemini #2**：relationships 字段类型错乱时返回 Internal |
| `advance_plot_returns_internal_when_live_json_is_not_object` | **Gemini #1**：live.json 非 Object 时返回 Internal 而非 panic |
| `advance_plot_returns_internal_when_plot_history_field_is_wrong_type` | **Gemini #1**：plot_history 字段类型错乱时返回 Internal |

### 2.4 本地验证全绿

| 门禁 | 命令 | 结果 |
|---|---|---|
| fmt | `cargo fmt --check` | clean |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings |
| lib tests | `cargo test --workspace --lib` | 772 passed, 0 failed, 1 ignored |
| integration tests | `cargo test --workspace --tests` | 29 passed, 0 failed |
| protocol tests | `cargo test -p airp-state-protocol` | 6 passed, 0 failed |
| ui tests | `cargo test -p airp-ui` | 9 passed, 0 failed |
| 新增测试 | `cargo test -p airp-core --lib agent::tools::tests::agent_rp_phase3` | 13/13 passed（默认 16 线程并行，0.21s） |
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

### 3.4 关于 `npc_action` 未持 `session_lock` 的留白（部分解决，见 F-5）

`npc_action` 通过 `volume_store::append_to_current` 写 `session/current.md`，
原审计版本未持 `session_lock`。若并发 `seal_volume` 正在归档 `current.md`，
`append_to_current` 的原子 append 仍可能落在已被 `seal_volume` 清空的
文件上，导致追加丢失。

本审计原认为此风险低，留作非阻塞跟进项 F-3。**CodeRabbit review 跟进后
此问题部分解决**：`session_lock` 已改为 `pub(crate)`，`npc_action` /
`advance_plot` / `trigger_world_event` 在调用 `append_to_current` 前都
显式持有 `session_lock(character_id, session_id)`。**但 `seal_volume`
（[volume_context.rs](../../engine/src/agent/tools/volume_context.rs)
`SealVolumeTool::call` L41-127）全程未获取 `session_lock`**，read_current
→ `run_seal_flow().await`（含 `clear_current`）的临界区无锁保护，
`npc_action` 的 append 仍可能落在 read 与 clear 之间被 clear 静默销毁。

本审计 §3.4 / F-3 原文"与 `seal_volume` 共享同一把 per-session 锁"不准确
——共享需要双方都获取锁。`seal_volume` 侧未持锁意味着 F-3 仅部分解决，
剩余竞态记为新跟进项 F-5。修复非最小：`session_lock` 当前是
`std::sync::Mutex`，guard 为 `!Send`，无法跨 `run_seal_flow().await`
持有；正确修复需迁移到 `tokio::sync::Mutex` 并改写 [domain.rs](../../engine/src/domain.rs)
`ChatService::with_session`（11 个 sync 调用点）和 `delete_session`
（sync 公共 API），属跨层架构变更，超出本 PR / Nitpick 跟进的最小修改
范围。详见 §6 F-5 与 §8.1。

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
| `npc_action` vs `seal_volume` 并发 | 中 | F-3 部分解决 + F-5 跟进；`npc_action`/`advance_plot`/`trigger_world_event` 已持 `session_lock` 互不交错，但 `seal_volume` 未持锁，clear_current 仍可能销毁并发 append |
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
| 272-F-3（**部分解决**） | ~~`npc_action` 写 `current.md` 未持 `session_lock`~~ | **CodeRabbit 跟进已部分修复**：`session_lock` 改为 `pub(crate)`，`npc_action` / `advance_plot` / `trigger_world_event` 均显式持有 `session_lock`。**但 `seal_volume` 仍未持 `session_lock`**，见 F-5。详见 §8.1 |
| 272-F-4（非阻塞） | PR 描述"变更摘要"未反映审计后的实际 scope（9 files） | 建议在 push 后更新 PR 描述，注明审计修复的 3 个 commit |
| 272-F-5（非阻塞，**新**） | **`seal_volume` 未持 `session_lock`，与 `npc_action` / `advance_plot` / `trigger_world_event` 仍有并发竞态**：`seal_volume` 在 [volume_context.rs](../../engine/src/agent/tools/volume_context.rs) `SealVolumeTool::call` 中 read_current → run_seal_flow(含 clear_current) 全程不持 `session_lock`，`npc_action` 的 append 可能落在 read 和 clear 之间被 clear 静默销毁。本审计 §3.4 / F-3 原文"与 `seal_volume` 共享同一把 per-session 锁"不准确——共享需要双方都获取锁。**修复非最小**：`session_lock` 当前是 `std::sync::Mutex`，guard 为 `!Send`，无法跨 `run_seal_flow().await` 持有；最小正确修复需将 `session_lock` 迁移到 `tokio::sync::Mutex`，但这会影响 [domain.rs](../../engine/src/domain.rs) `ChatService::with_session`（11 个 sync 调用点）和 `delete_session`（sync 公共 API，被 [daemon/handlers/sessions.rs](../../engine/src/daemon/handlers/sessions.rs#L46) 调用），属跨层架构变更，超出 Nitpick 跟进的最小修改范围。 | 独立 PR 处理：迁移 `session_lock` 到 `tokio::sync::Mutex` 并将受影响的 sync `ChatService` 方法改为 async；或重构 `run_seal_flow` 为 sync（`reqwest::blocking` + `spawn_blocking`）。同时加入本 Nitpick 建议的 `concurrent_npc_action_and_seal_volume_does_not_lose_npc_action` 回归测试。 |

## 7. 审计结论

**通过（PASS，审计修复后无阻塞项）**。

PR #272 原始实现存在 3 个阻塞问题（A1 数据完整性 / A2 并发竞态 /
A3 revision 合同绕过），审计已在 commit `32b8419` + `940b20e` +
`9559931` 修复并覆盖端到端测试。本地全部门禁绿（fmt / clippy / lib
772+1 ignored / integration 29 / protocol 6 / ui 9 / 13 新增测试 /
chat_store 24/24）。

3 个 `chat_store::tests` 在 pr-270 分支的失败经独立核查为预存问题，
与 PR #272 无关。

CodeRabbit + Gemini review 跟进修复已在 §9 详述。F-3（`npc_action`
未持 `session_lock`）**部分解决**：`npc_action` / `advance_plot` /
`trigger_world_event` 侧已持 `session_lock`，但 `seal_volume` 侧未持，
剩余竞态记为 F-5（见 §3.4 / §6）。Gemini #1/#2（live.json 损坏时 panic）
已通过防御性类型检查修复并覆盖 4 个新测试。

**Nitpick 跟进（本 commit）**：CodeRabbit Nitpick 建议在
`engine/src/agent/tools/tests/agent_rp_phase3.rs` L337-365 旁加
`concurrent_npc_action_and_seal_volume_does_not_lose_npc_action` 回归
测试（`std::thread::scope` + Barrier 模式）。经独立核证，Nitpick 描述的
竞态真实存在（`seal_volume` 在 [volume_context.rs](../../engine/src/agent/tools/volume_context.rs#L41-L127)
`SealVolumeTool::call` L41-127 全程未获取 `session_lock`），但建议的
"验证 NPC action 仍持久化"断言在 F-5 修复前会失败——`npc_action` 的
append 仍会被 `seal_volume` 的 `clear_current` 静默销毁。F-5 的正确修复
需将 `session_lock` 从 `std::sync::Mutex` 迁移到 `tokio::sync::Mutex`
（跨层架构变更，影响 11+ sync `ChatService` 调用点），超出 Nitpick 跟进
的最小修改范围。**按用户指示"Fix only still-valid issues, skip the rest
with a brief reason, keep changes minimal"，本审计跳过该测试新增**，
将其作为 F-5 修复 PR 的一部分（与 F-5 修复同步落地，确保测试断言可
通过）。F-5 已在 §6 记录并列入下方合并后 issue 清单。

4 个非阻塞跟进项（F-1 / F-2 / F-4 / F-5）建议在 PR 合并后写入 GitHub
issue（按根 `AGENTS.md` §审计遗留项处理 时序约束：PR 合并后提交）。F-3
部分解决，其未解决部分已并入 F-5，无需单独提 issue。

可合并。

## 8. CodeRabbit + Gemini Review 跟进修复

PR #272 push 后触发 CodeRabbit 和 Gemini review，分别提出 3 条和 2 条
actionable comment。本节记录所有跟进修复，均在"本 commit"中落地。

### 8.1 CodeRabbit 跟进 #1：`npc_action` 未持 `session_lock`（F-3 部分解决，剩余见 F-5）

**问题**：原审计修复让 `trigger_world_event` 持有 `session_lock`，但
`npc_action` 仍裸调 `volume_store::append_to_current`，与 `seal_volume` /
`advance_plot` 并发时可能在 `current.md` 中交错。

**修复**：
- `engine/src/domain.rs`：`session_lock` 从 `fn`（private）改为
  `pub(crate) fn`，附 doc comment 说明"crate 内部串行化合同"语义。
- `engine/src/agent/tools/npc.rs`：`npc_action` 在
  `append_to_current` 前显式 `session_lock(cid, sid).lock()`。
- `engine/src/agent/tools/world_event.rs`：`trigger_world_event` 在
  `append_to_current` 前显式 `session_lock(cid, sid).lock()`。
- `engine/src/agent/tools/plot.rs`：`advance_plot` 在
  `append_to_current` 前显式 `session_lock(cid, sid).lock()`。

**效果（部分）**：`npc_action` / `advance_plot` / `trigger_world_event`
三个写 `current.md` 的工具共享同一把 per-session 锁，互相之间不再交错。
**但 `seal_volume`（[volume_context.rs](../../engine/src/agent/tools/volume_context.rs#L41-L127)
`SealVolumeTool::call`）全程未获取 `session_lock`**，与上述三个工具之间
的并发竞态未消除——`seal_volume` 的 `clear_current` 仍可能销毁
`npc_action` 在 read 与 clear 之间 append 的内容。F-3 标记为**部分解决**，
剩余竞态记为新跟进项 F-5（见 §3.4 / §6）。F-5 修复需将 `session_lock`
迁移到 `tokio::sync::Mutex`（跨层架构变更），超出本 PR / Nitpick 跟进的
最小修改范围。

### 8.2 CodeRabbit 跟进 #2：`concurrent_*` 测试改用独立 OS thread

**问题**：原 `concurrent_update_relationship_and_advance_plot_do_not_lose_updates`
用 `futures_util::future::join_all` 在单一 tokio runtime 上并发 poll
10 个 task。但 `update_relationship` / `advance_plot` 的 future 内部全是
同步代码（`StateService::mutate` 同步持有 `state_lock` 不 yield），10 个
同步 task 会占满 runtime worker pool。

**修复**：改用 `std::thread::scope` + 共享 `std::sync::Barrier` + 每个
worker 内部独立 `tokio::runtime::Builder::new_current_thread().build()`。
worker OS thread 不占用任何 tokio runtime worker pool，独立 runtime 不
与 parent runtime 共享，无死锁可能。

### 8.3 Gemini 跟进 #1：`advance_plot` 在 live.json 损坏时 panic

**问题**：`advance_plot` 的 `mutate` 闭包用
`live["plot_history"].as_array_mut()` 取数组。若 `live` 不是 Object，
`live["plot_history"]` 在 serde_json 中会 panic（`Index::index` on
non-Object Value）。若 `plot_history` 字段类型错乱（非 Array），
`as_array_mut()` 返回 `None`，`unwrap()` 也会 panic。

**修复**：改为防御性类型检查：
```rust
let live_obj = live.as_object_mut()
    .ok_or_else(|| AirpError::Internal("live state is not a JSON object".to_string()))?;
let history = live_obj
    .entry("plot_history")
    .or_insert_with(|| Value::Array(Vec::new()))
    .as_array_mut()
    .ok_or_else(|| AirpError::Internal("plot_history field is not a JSON array".to_string()))?;
```

### 8.4 Gemini 跟进 #2：`update_relationship` 在 live.json 损坏时 panic

**问题**：`update_relationship` 的 `mutate` 闭包用
`live["relationships"][&key] = ...` 直接 indexing。若 `live` 不是
Object，indexing panic；若 `relationships` 字段类型错乱，
`live["relationships"][&key]` 也会 panic。

**修复**：同 §8.3，改为 `as_object_mut()` + `entry()` + `ok_or_else(Internal)`
防御性检查。

### 8.5 测试并行死锁修复

**问题**：CodeRabbit/Gemini 跟进修复后，13 个 `agent_rp_phase3` 测试在
默认 16 线程并行下 hang。根因：8 个测试共用 character_id `"alice"`，
process-global `state_lock` / `session_lock` 以 `character_id` 为 key，
多个测试争用同一把锁，结合独立 tokio runtime + `reqwest::Client::new()`
的内部线程，导致 OS 线程饥饿。

**修复**：每个测试用唯一 character_id（如 `upd_rel_basic` /
`adv_plot_basic` / `trig_evt_basic` / `npc_act_basic` /
`upd_rel_corrupt1` 等），消除跨测试锁争用。各测试的 `data_root` 本来
就独立（`tempdir()`），character_id 唯一化不影响测试隔离性。

**验证**：13/13 测试在默认 16 线程并行下 0.21s 全绿。

### 8.6 跟进修复验证

| 门禁 | 命令 | 结果 |
|---|---|---|
| fmt | `cargo fmt --check` | clean |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 0 warnings |
| 全量测试 | `cargo test --workspace` | 772 lib (1 ignored) + 29 integration + 6 protocol + 9 ui = 816 passed, 0 failed |
| phase3 测试 | `cargo test -p airp-core --lib agent::tools::tests::agent_rp_phase3` | 13/13 passed（16 线程并行，0.21s） |

## 9. Refs

- 根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- 根 `AGENTS.md` §审计遗留项处理（PR 合并后写入 GitHub issue）
- #115 Phase 2e revision 合同（`StateService::write` 的 `replace_file` + `history.jsonl` + `revisions/{n}/` 三件套）
- `engine/src/domain.rs` L42 `pub(crate) fn character_lock` / L52 `pub(crate) fn session_lock` / L76 `pub(crate) fn state_lock` / `StateService::read` / `StateService::mutate` / `commit_state_under_lock` / `load_live_value`
- `engine/src/agent/tools/{world_event,npc,plot}.rs` 审计修复 + CodeRabbit/Gemini 跟进修复
- `engine/src/agent/tools/tests/agent_rp_phase3.rs` 13 个端到端测试（9 原始 + 4 Gemini 跟进）
