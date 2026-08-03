# AIRP 锁序合同

> 创建：2026-08-02，`main@830426e`（E-P1-2，对应 [#381](https://github.com/GhostXia/AIRP/issues/381) E-P1-2、[#284](https://github.com/GhostXia/AIRP/issues/284)、[#220](https://github.com/GhostXia/AIRP/issues/220)）
> 真理顺序：当前源码 > 本文 > `284-PER-SESSION-INFLIGHT-MUTEX-DESIGN.md` §6 > 历史审计。

本文是 AIRP 进程内锁的**当前合同**，不是「全仓绝无死锁」声明。它把分散在 `284-PER-SESSION-INFLIGHT-MUTEX-DESIGN.md` §6、`docs/audits/2026-07-26-PR-335-bug-f-deadlock-audit.md`、`docs/audits/2026-07-26-PR-338-bug-b-advance-clock-session-lock-audit.md` 和源码注释里的锁序结论收敛为一份可审计的入口。

「已核验」表示本文作者已读源码确认；「未核验/已知缺口」表示尚未在源码层闭合。

## 1. 锁清单

按粒度分组。所有锁均为进程内、单 daemon 实例（AGENTS.md）；跨进程安全是调用方责任，本合同不覆盖。

### 1.1 per-character 资源锁（`engine/src/domain.rs`）

| 锁 | 类型 | key | 用途 |
|---|---|---|---|
| `CHARACTER_LOCKS` | `RwLock<()>` | `character_id` | per-character 外层门控；读多写少 |
| `STATE_LOCKS` | `Mutex<()>` | `character_id` | 串行化 `state/live.json`、`world_clock.json`、`world_events.json` 等 per-character 状态文件 |
| `SESSION_LOCKS` | `Mutex<()>` | `character_id` 或 `character_id/session_id` | 串行化 `session/current.md` 等 per-session 叙事文件；无 `session_id` 时退化为 per-character |
| `PERSONA_LOCKS` | `Mutex<()>` | `user_id` | 串行化 persona 写入与 revision bump |

均经 `OnceLock<Mutex<HashMap<…, Arc<…>>>>` 懒创建；`session_lock` 在 session 删除后由 `remove_deleted_session_lock` 主动剔除条目。

### 1.2 Conversation 双锁（`engine/src/conversation.rs`）

| 锁 | 类型 | key | 用途 |
|---|---|---|---|
| `CONVERSATION_LOCKS` | `tokio::sync::Mutex<()>` | `(data_root, conversation_id)` | 串行化 conversation append/event 序号与 manifest 写入 |
| `CONVERSATION_IO_LOCKS` | `tokio::sync::Mutex<()>` | `(data_root, conversation_id)` | 串行化 journal 文件 I/O；可单独持有（如 `context_projection`） |

使用 `Weak<tokio::sync::Mutex<()>>` registry，`scoped_conversation_lock` 内做 `retain` 回收。

### 1.3 Scene advisory 锁（`engine/src/scene.rs`）

| 锁 | 类型 | key | 用途 |
|---|---|---|---|
| `SCENE_WRITE_LOCKS` | `Mutex<()>`（std） | `(root, scene_id)` | scene 写入 advisory；非 OS 锁 |

### 1.4 Registry / 派生数据锁

| 锁 | 文件 | 类型 | 用途 |
|---|---|---|---|
| `ACTIVE_TURNS` | `conversation_turn.rs` | `Mutex<HashMap>` | conversation active turn 注册表 |
| `MEMORY_MUTATION_LOCKS` | `memory/mod.rs` | `Lazy<Mutex<HashMap>>` | per-session_dir 记忆 mutation |
| `USER_MODEL_LOCKS` | `memory/user_model.rs` | `Lazy<Mutex<HashMap>>` | per-user 用户模型写入 |
| `DRIFT_LOCKS` | `style/drift.rs` | `Lazy<Mutex<HashMap>>` | per-character 风格漂移 |
| FTS connection cache | `memory/fts.rs` | `Mutex<HashMap>` | FTS 连接缓存（poison → error，不 recover） |

### 1.5 全局 utility 锁

| 锁 | 文件 | 类型 | 用途 |
|---|---|---|---|
| `COMMIT_LOCK` | `revision/atomic.rs` | `Mutex<()>`（std） | 串行化所有 asset 的 revision commit（TOCTOU 防护） |
| `QUOTA_LOCK` | `quota.rs` | `Mutex<()>`（std） | 串行化 quota 记录 |
| `PRESET_WRITE_LOCK` | `orchestrator/preset.rs` | `Lazy<Mutex<()>>`（std） | preset 写入 |
| `PRESET_IMPORT_LOCK` | `daemon/handlers/presets.rs` | `OnceLock<Mutex<()>>`（std） | preset 导入 |
| `INDEX_LOCK` | `image_gen.rs` | `tokio::sync::Mutex<()>` | image gen 索引 |
| `ENV_LOCK` | `config.rs`、`daemon/handlers/characters.rs` | `Mutex<()>` / `tokio::sync::Mutex<()>` | 进程 env reload 串行化 |
| `DaemonState.provider_routing_update` / `plugin_tools_update` / `settings_update.transaction` | `daemon/mod.rs` | `tokio::sync::Mutex<()>` | per-DaemonState 配置热更新串行化 |
| `BACKUP_LOCK` | `backup/snapshot.rs` | `Mutex<()>`（std） | 串行化 backup vs backup / backup vs restore（#342 E-P2-1，PR #445 引入） |

## 2. 已核验的嵌套路径

下列路径已读源码确认锁序。任何新路径若与下列不一致，必须先更新本合同再合入。

### 2.1 `ChatService::with_session`（`domain.rs`）

```text
character_lock.read()  →  session_lock.lock()
```

- 同步 std 锁；闭包内为纯同步 I/O，不跨 `.await`。
- 同时持两把锁直到 `operation()` 返回。

### 2.2 `StateService::read` / `mutate` / `mutate_locked` / `write`（`domain.rs`）

```text
character_lock.read()  →  state_lock.lock()
```

- 同步 std 锁；闭包内为纯同步 I/O，不跨 `.await`。
- `mutate` 在持锁期间执行 `load → closure → schema 校验 → replace_file → history.jsonl append → revision commit`。
- `mutate_locked`（#437 fix path 4 新增）：与 `mutate` 行为一致，但**不** acquire `character_lock.read()`，要求调用方已持有 `character_lock.read()`（或 `.write()`）作为外层门控。仅供 `advance_plot`（§2.3）使用以避免 `StateService::mutate` 内部 acquire 与外层 acquire 构成递归 read；其他调用方应继续使用 `mutate`。

### 2.3 `agent::tools::plot::advance_plot`（`plot.rs`）—— 唯一合法 session→state 嵌套，R1 已闭合（#437 fix path 4）

```text
character_lock.read()  →  session_lock.lock()  →  [StateService::mutate_locked 内]  state_lock.lock()
```

- 同步 std 锁；async fn 体内不 `.await`（同步 I/O 阻塞 tokio worker——已知 debt，见 §6.2）。
- **这是 session_lock 与 state_lock 同时持有的唯一已核验路径**，方向固定为 session → state。
- 由 Bug F 修复（PR #335）后，无反向路径与之成环。
- **R1 已闭合（#437 fix path 4）**：`StateService::mutate` 拆为 `mutate_locked`（不 acquire `character_lock.read()`，要求调用方已持有）+ `mutate`（兼容包装，内部 acquire 后调 `mutate_locked`）。`advance_plot` 改用 `mutate_locked`，外层先 acquire `character_lock.read()`，再 acquire `session_lock`，最后调 `mutate_locked` 进入 `state_lock` 临界区。这消除了旧版「`StateService::mutate` 内部 acquire `character_lock.read()` 构成递归 read」的风险，使外层 `character_lock.read()` 成为合法 R1 门控，防止 `delete_character`（持 `character.write()`）在 `advance_plot` 临界区期间删除 character 顶层目录（含 `live.json`）。
- **历史风险（已闭合，记录于 §6.7）**：PR #436 之前，本路径外层为 `session_lock` 而非 `character_lock.read()`，存在 `delete_character` 并发删除 character 顶层目录的 TOCTOU 风险。PR #436 因 `StateService::mutate` 的 re-entrancy 风险未能闭合，记录为 §6.7 残留风险；#437 通过 fix path 4 闭合。

### 2.4 `agent::tools::world_event::trigger_world_event`（`world_event.rs`，Bug F 修复后 + R1 收敛）

外层 `character_lock.read()` 跨两段临界区持有；两段临界区**绝不嵌套**：

```text
character_lock.read()  →  阶段一: state_lock.lock()           → 释放
                      →  阶段二: session_lock.lock()         → 释放
```

- 同一调用任意时刻只持一把内层锁（state 或 session），`character_lock.read()` 共享读不阻塞其他 reader。
- Bug F（PR #335）就是消除旧版 state→session 与 `advance_plot` session→state 的锁序倒置死锁。
- R1 收敛（本 PR）：新增外层 `character_lock.read()`，防止 `delete_character` 在事件标记 / append 期间删除 character 目录（TOCTOU）。早期 return（事件已 triggered）时 guard 由 Drop 自动释放。

### 2.5 `agent::tools::world_event::advance_clock`（`world_event.rs`，Bug B 修复后 + R1 收敛）

与 §2.4 同模式：

```text
character_lock.read()  →  阶段一: state_lock.lock()           → 释放
                      →  阶段二: session_lock.lock()         → 释放（仅当 content_buf 非空）
```

- `advance_and_check_triggers` 在阶段一内完成 clock 推进 + 事件标记 + `save_world_events`。
- Bug B（PR #338）修复了旧版阶段二无 `session_lock` 导致的 `current.md` 并发交错。
- R1 收敛（本 PR）：新增外层 `character_lock.read()`，与 `trigger_world_event` 同模式，防止 `delete_character` 在时钟推进 / 事件 append 期间删除 character 目录。

### 2.6 `agent::tools::npc::npc_action`（`npc.rs`，R1 收敛）

```text
character_lock.read()  →  session_lock.lock()
```

- 单内层锁，不与 state_lock 嵌套。
- R1 收敛（本 PR）：新增外层 `character_lock.read()`，与 `with_session` 同模式，防止 `delete_character` 在 append 期间删除 session 目录（TOCTOU）。

### 2.7 `volume_manager::run_seal_flow`（`volume_manager.rs`，#283 方案 J + R1 收敛）

LLM streaming（秒级）不持锁；写盘段持 `character.read + session_lock` + baseline 校验：

```text
[LLM streaming 不持锁]
character_lock.read()  →  session_lock.lock()  →  baseline 校验  →  write_volume / write_index / clear_current
```

- 仅 per-character 路径（`character_id = Some`）持锁；scene 模式（`character_id = None`）保持既有行为不持锁。
- guard 不跨 `.await`（write_volume / write_index / clear_current 全为 sync），因此 std `Mutex` / `RwLock` 合法。
- 双 baseline 校验（current.md + index.md）防止并发 `npc_action` / `advance_plot` 的 append 或 `run_maintenance` 的跨卷实体晋升被静默覆盖；校验失败返回 `Conflict`，调用方可重试。
- R1 收敛（本 PR）：新增外层 `character_lock.read()`，防止 `delete_character` 在 seal 写盘期间删除 character 目录（TOCTOU）。

### 2.8 `conversation::append_event`（`conversation.rs`）

```text
conversation_lock.lock().await  →  conversation_io_lock.lock().await  →  spawn_blocking(I/O)
```

- tokio::sync::Mutex；`conversation_io_lock` 在 `spawn_blocking` 外持有，确保 journal I/O 串行。
- `append_event_locked_async` 是唯一同时持两把锁的入口；`context_projection` 只持 `conversation_io_lock`。

### 2.9 `delete_character` / `delete_session` → `create_backup`（`domain/chat.rs`，#342 E-P2-1，PR #445）

`delete_character` 持 `character.write()` 后调用 `create_backup`（后者 acquire `BACKUP_LOCK`）：

```text
character_lock.write()  →  [create_backup 内] BACKUP_LOCK.lock()
```

`delete_session` 持 `character.read() + session_lock` 后调用 `create_backup`：

```text
character_lock.read()  →  session_lock.lock()  →  [create_backup 内] BACKUP_LOCK.lock()
```

- `BACKUP_LOCK` 是叶锁（R4），不与 `character_lock` / `session_lock` 反向嵌套。
- `BACKUP_LOCK` 为 `std::sync::Mutex`，调用方（`delete_character_endpoint` / `delete_session_endpoint` / `delete_character` agent tool / `restore_backup_endpoint`）通过 `tokio::task::spawn_blocking` 包装 sync I/O，避免阻塞 tokio worker（A1）。
- `restore_backup` 内部调 `create_backup_locked`（非 `create_backup`）以避免 `std::sync::Mutex` 不可重入死锁：`restore_backup` 已持 `BACKUP_LOCK`，再调 `create_backup` 会重入同一 `Mutex` 死锁；split 后 `create_backup_locked` 假设锁已持有，不再 acquire。
- **残留风险（W-02，#447）**：`restore_backup` 的 swap 阶段（`swap_full_data_root` / `swap_scoped_subtree`）仅持 `BACKUP_LOCK`，不持任何 `character_lock`，可与并发 `append_to_current` / `StateService::mutate` 竞态。v1 缓解：用户在维护窗口执行 restore（无活跃 session）。

## 3. 锁序规则

### R1：`character_lock` 是 per-character 外层门控

获取 `state_lock`、`session_lock`（针对同一 `character_id`）前，必须先持有 `character_lock.read()`（或 `.write()`）。例外：`StateService::read` / `mutate` / `write` 内部已 acquire `character_lock.read()` 再 acquire `state_lock`，调用方若已持 `character_lock.read()` 再调 `StateService::*` 会构成递归 read（详见 §6.7 历史风险论证），因此 `agent::tools::*` 通过 `StateService::mutate` 间接获取 state_lock 时不再外部 acquire `character_lock.read()`。`advance_plot`（§2.3）通过 `StateService::mutate_locked` 变体（不 acquire `character_lock`）显式在外层 acquire `character_lock.read()`，已闭合 R1。

**已记录的 R1 例外路径**（0 处，#437 闭合后无例外）：

- ~~§2.3 `advance_plot`：外层为 `session_lock`（非 `character_lock.read()`），因 `StateService::mutate` 内部 acquire `character_lock.read()` 会构成递归 read。~~ **已闭合（#437 fix path 4）**：`StateService::mutate` 拆为 `mutate_locked`（不 acquire `character_lock`）+ `mutate`（兼容包装）；`advance_plot` 改用 `mutate_locked`，外层 `character_lock.read()` 已补齐。

**R1 收敛进度**：§2.3 `advance_plot`、§2.4 `trigger_world_event`、§2.5 `advance_clock`、§2.6 `npc_action`、§2.7 `run_seal_flow` 已全部补齐外层 `character_lock.read()`，无 R1 例外路径残留。

### R2：`session_lock` 与 `state_lock` 的唯一合法嵌套方向

```text
session_lock  →  state_lock   （仅 advance_plot，经 StateService::mutate_locked）
```

- 反向（`state_lock → session_lock`）**禁止**。
- 若同一调用需同时变更 state 与 session 叙事文件，必须采用 §2.4/§2.5 的两段临界区模式：先释放 `state_lock`，再获取 `session_lock`。
- 两段临界区之间的中间状态（state 已持久化、内容未 append）由调用方明确定义 fail 行为，不得静默累积。
- `advance_plot` 经 `StateService::mutate_locked` 进入 `state_lock` 临界区（#437 fix path 4 后），仍保持 `session → state` 嵌套方向，R2 不违反。

### R3：`conversation_lock` → `conversation_io_lock` 是唯一合法 conversation 嵌套

- `conversation_io_lock` 可单独持有（`context_projection`、`all_events_locked_async`）。
- 反向（持 `conversation_io_lock` 时获取 `conversation_lock`）**禁止**。

### R4：全局 utility 锁是叶锁

`COMMIT_LOCK`、`QUOTA_LOCK`、`PRESET_WRITE_LOCK`、`PRESET_IMPORT_LOCK`、`INDEX_LOCK`、`ENV_LOCK`、`DaemonState.*_update`、`BACKUP_LOCK` 持有期间**不得**获取任何 per-character / per-conversation / per-session 资源锁。它们必须是临界区的最内层。

例外审计：`StateService::mutate` 在持 `character_lock.read + state_lock` 期间调用 `commit_revision`，后者获取 `COMMIT_LOCK`——这是 `character.read → state → COMMIT_LOCK` 的合法外→内序列，`COMMIT_LOCK` 仍为最内层叶锁，不违反 R4。

### R5：`persona_lock`、`scene_write_lock`、registry 锁独立

- `persona_lock` 按 `user_id` key，不与 character 锁族嵌套（已核验路径）。
- `scene_write_lock` advisory，不与资源锁嵌套（已核验路径）。
- `MEMORY_MUTATION_LOCKS`、`USER_MODEL_LOCKS`、`DRIFT_LOCKS`、`ACTIVE_TURNS` 为各自资源的串行化锁，未发现与 §1.1/§1.2 锁互嵌套；新路径若需嵌套必须先更新本合同。

### R6：禁止反向获取 coordinator/owner

参见 `284-PER-SESSION-INFLIGHT-MUTEX-DESIGN.md` §6：持有 `session_lock`、`state_lock` 或 agent tool 内部锁时，不得反向获取 session coordinator / generation owner token；同一 task 不得重入同一 session coordinator。该规则的源码级实现尚未交付（#284 方案 N/O 在途），本合同仅记录约束。

## 4. 异步安全约束

### A1：std `Mutex`/`RwLock` guard 不得跨 `.await`

所有 `std::sync::Mutex` / `RwLock` guard 必须在同一同步作用域内释放。`agent::tools::*` 的 async fn 体内使用 std 锁时，临界区必须是纯同步代码（含同步 `fs` I/O），不得 `.await`。

### A2：`tokio::sync::Mutex` 可跨 `.await`，但不得跨 `spawn_blocking`

`conversation_lock`、`conversation_io_lock`、`INDEX_LOCK`、`DaemonState.*_update` 可跨 `.await` 持有。`spawn_blocking` 闭包内不得持有任何 `tokio::sync::Mutex` guard（Send 边界 + 阻塞语义不兼容）。

### A3：锁内同步 I/O 是已知 debt，不是合同违反

`agent::tools::plot::advance_plot`、`npc_action`、`trigger_world_event`、`advance_clock` 在 std 锁内做同步 `fs` I/O，阻塞 tokio worker。这是 CURRENT-BASELINE §2.1.4 记录的结构性 debt（#284/#381 E-P0-4/5），由方案 O 收敛，本合同不要求立即消除，但禁止新增此类路径。

## 5. Poison 恢复策略

### P1：默认 silent recover

`std::sync::Mutex` / `RwLock` poison 默认用 `unwrap_or_else(|p| p.into_inner())` 恢复，继续服务。理由：daemon 单进程前台运行，poison 表示前一次临界区 panic，crash 整个 daemon 比继续服务更危险。

涉及：`domain.rs`、`conversation.rs`、`conversation_policy.rs`、`conversation_turn.rs`、`memory/mod.rs`、`memory/user_model.rs`、`style/drift.rs`、`quota.rs`、`orchestrator/preset.rs`、`agent/tools/npc.rs`、`agent/tools/plot.rs`、`agent/tools/world_event.rs`、`daemon/mod.rs`、`config.rs`（部分）。

### P2：FTS connection cache poison → error（例外）

`memory/fts.rs` 的 `Mutex<Connection>` poison 返回 `AirpError::Internal`，不 recover。理由：Connection poison 通常伴随 SQLite 内部状态损坏，继续用该连接可能写坏 FTS 索引；上层会重建连接。

### P3：config RwLock poison 统一为 recover + warn（已交付）

`DaemonState.config: RwLock<MutableConfig>` 的 poison 处理曾不一致：

- `daemon/*`：`unwrap_or_else(|e| e.into_inner())`（recover，无日志）
- `agent/*`、`chat_pipeline/*`、`daemon/handlers/{dialogue_gen,image_gen,settings,style}.rs`：`.map_err(|_| AirpError::Internal("config lock poisoned"))`（error）
- `daemon/mod.rs::health_handler`：`.unwrap()`（panic）

2026-08-02 PR 收敛为统一 `DaemonState::read_config()` / `write_config()` helper，poison 时 `tracing::warn!` + `into_inner()` 恢复（P4 模式）。13 处调用点已全部替换。

### P4：memory mutation lock recover + warn

`memory/mod.rs` 在 recover 时额外 `tracing::warn!("…lock was poisoned; recovering")`。这是 P1 的增强版（recover + 可观测），推荐用于新路径。

## 6. 已知缺口与 follow-up

### 6.1 运行时锁序强制：已交付（R1 + R2，全路径）

`domain.rs` 新增 `lock_order` 模块（`#[cfg(debug_assertions)]` thread-local 栈 + RAII `Guard`；`#[cfg(not(debug_assertions))]` 零成本 no-op），覆盖 R1（character 外层门控）与 R2（session↔state 嵌套方向）的运行时检测：

- **R1 强制**（#438 W-04，2026-08-03 交付）：
  - `track_character_read()` / `track_character_write()` 记录 `character_lock` 持有状态，无 violation 检查（character 是最外层门控，无前置要求）。
  - `track_session()` / `track_state()` 在调用时检查 HELD 栈是否含 `CharacterRead` 或 `CharacterWrite`；不含则 `debug_assert!` panic（R1 违反：session/state 必须由 character 外层门控）。
  - 覆盖所有 4 条 agent tool 路径（`advance_plot` / `trigger_world_event` / `advance_clock` / `npc_action`）+ `volume_manager::run_seal_flow` + `StateService::read`/`mutate`/`write` + `LorebookService::read`/`write` + `ChatService::with_session`/`delete_session`/`delete_character`。
- **R2 强制**（既有）：
  - `track_session()` 在持 `state_lock` 时获取 `session_lock` 触发 `debug_assert!`（state→session 禁止，Bug F 类死锁回归）。
  - `track_state()` 无 R2 检查（session→state 是 R2 唯一合法嵌套方向）。
- **R1 回归测试**（#438 W-04，2026-08-03 交付）：
  - 4 条并发测试：`advance_plot` / `trigger_world_event` / `advance_clock` / `npc_action` 各与 `delete_character` 经 `Barrier` 同时放行，30s 超时检测死锁。关键不变式：tool 不应返回 `Internal` error（那表示读到半删 live.json / world_events.json / world_clock.json，R1 TOCTOU 防护失效）。
  - 15 条 `lock_order` 单测覆盖 R1/R2 合法路径、违反路径、Drop 语义、两段临界区模式。
- release build（`--release`）下 `track_*` 返回 ZST `Guard`，零开销（§7）。
- **未覆盖**：R3（conversation 双锁）、R6（coordinator 反向获取）尚未有运行时强制。guard 不跨 `.await`（§4 A1），thread-local 栈仅在同步作用域内有效。

### 6.2 std 锁内同步 I/O（结构性 debt）

`agent::tools::*` 在 std 锁内做 `fs` I/O 阻塞 tokio worker，由 #284 方案 O 收敛。本合同禁止新增此类路径，但不动既有 5 处（`advance_plot`、`npc_action`、`trigger_world_event`、`advance_clock`、`StateService::*`）。

### 6.3 config RwLock poison 策略不一致（已交付）

见 §5 P3。2026-08-02 PR 已统一为 `DaemonState::read_config()` / `write_config()` helper（recover + warn，P4 模式），13 处调用点全部替换。

### 6.4 FTS poison 策略与默认分叉（已加强注释）

见 §5 P2。是有意为之。2026-08-02 PR 在 `memory/fts.rs` 模块级文档补充了 error 策略的理由（SQLite Connection poison 伴随内部状态损坏，上层会重建连接）。

### 6.5 跨进程安全

本合同只覆盖进程内。AIRP daemon 单进程前台运行（AGENTS.md），无跨进程 writer；若未来引入多进程，必须重新审计 `COMMIT_LOCK`、`session_lock` 等的跨进程语义。

### 6.6 agent tool 直接 `fs` 写路径

`agent::tools::*` 部分路径绕过 shared service 直接 `replace_file` / `fs` 写（#381 E-P1-3 / #160）。这些路径若同时持 `session_lock` / `state_lock`，仍受本合同约束；若不持锁，则不在本合同覆盖范围，由 #381 E-P1-3 单独收敛。

### 6.7 `advance_plot` R1 例外残留 TOCTOU 风险（**已闭合，#437 fix path 4**）

**历史背景（PR #436 时记录）**：§2.3 `advance_plot` 因 `StateService::mutate` 内部 acquire `character_lock.read()` 的 re-entrancy 风险，未在 PR #436 补齐外层 `character_lock.read()`。当时 TOCTOU 防护降级为：

- `advance_plot` 持 `session_lock` 期间，`delete_character` 不会清空 session 目录（`delete_character` 不持 `session_lock`，但会持 `character.write()`）。
- 但 `delete_character` 仍可能并发删除 character 顶层目录（含 `live.json` / `world_clock.json`），与 `advance_plot` 调用 `StateService::mutate` 读 `live.json` 形成竞态。

**re-entrancy 风险的准确论证（W-01 措辞修正，2026-08-03 独立审计）**：

原论证「std `RwLock` 递归 read 在部分平台（含 Windows SRWLOCK）会 deadlock」**不准确**。准确事实如下：

- **Windows SRWLOCK**：同一线程多次调用 `AcquireSRWLockShared` **不**会 deadlock（Vista+），但**也不**保证计数语义——第二次 acquire 立即返回，第一次 release 即释放锁。这**破坏排他性语义**：当内层 `read()` 还"持有"时，外层 `read()` 已被 `delete_character` 的 `write()` 抢占——导致 `StateService::mutate` 内部的 `state_lock` 临界区在 `character.write()` 持有期间执行，**违反 R1 的互斥语义**（虽不 deadlock，但 TOCTOU 防护失效）。
- **Linux/pthread**：`pthread_rwlock_rdlock` 递归 read 在持有时若有 writer 等待**可能 deadlock**（glibc 实现相关）。

正确措辞：「deadlock 风险（部分 pthread 实现）+ 排他性语义破坏（Windows SRWLOCK），两者均导致 R1 TOCTOU 防护失效」。

**修复路径（#437 选用方案 4）**：

| 方案 | 评估 | 状态 |
|---|---|---|
| 1. 上提 `character_lock.read()` 到调用方 | 破坏 `StateService` 自封装；需重审所有调用点（约 8 处）；改造成本中等 | 未选 |
| 2. 改用 re-entrant `RwLock`（`parking_lot` / `lock_api`） | 引入新依赖；需全仓 `RwLock` 替换，影响面大 | 未选 |
| 3. 改为两段临界区（释放 `session_lock` 后再调 mutate） | 改变 `advance_plot` 事务语义（state 变更不再原子于 session append）；可能引入新一致性风险 | 未选 |
| **4. 拆 `StateService::mutate` 为 `mutate_locked` + `mutate`（#437 选用）** | 改造成本小，不破坏其他调用方，不引入新依赖；`advance_plot` 改用 `mutate_locked`，外部 acquire `character_lock.read()` | **已交付（#437）** |

**闭合状态**：#437 通过 fix path 4 闭合本风险。`StateService::mutate` 拆为 `mutate_locked`（不 acquire `character_lock.read()`，要求调用方已持有）+ `mutate`（兼容包装，内部 acquire 后调 `mutate_locked`）。`advance_plot` 改用 `mutate_locked`，外层先 acquire `character_lock.read()` 再 acquire `session_lock`，最后调 `mutate_locked` 进入 `state_lock` 临界区。re-entrancy 风险消除，R1 TOCTOU 防护恢复，character 顶层目录（含 `live.json`）的并发删除被 `character_lock.read()` 外层门控阻断。本节保留作为历史记录，不再代表当前风险。

**R1 运行时强制 + 回归测试（#438 W-04，2026-08-03 交付）**：在 #437 静态闭合基础上，#438 补齐运行时强制（§6.1）与 4 条并发回归测试（`advance_plot` / `trigger_world_event` / `advance_clock` / `npc_action` 各与 `delete_character` 经 `Barrier` 并发，30s 超时检测死锁 + Internal error 检测 TOCTOU）。任意 PR 若回退 R1 fix（如移除 `character_lock.read()` 外层 acquire），`debug_assert!` 会在 CI debug build 立即 panic，回归测试也会失败。

### 6.8 lock-map cleanup race（**已闭合，#440**）

**历史背景**：PR #434 为修复 #422（stale lock-map 条目无界增长）引入 `remove_deleted_*_lock` cleanup 代码。但 cleanup 调用时机错误——在 `delete_character` / `delete_session` / `delete_persona` 的 write guard 释放**之前**调用，导致 race：

1. `delete_character` 持 `character.write()`（旧 Arc A）期间，`data_dir::delete_character` 删除目录。
2. `remove_deleted_character_lock` 移除 map entry（旧 Arc A 仍被 `_guard` 持有，但 map 已无 entry）。
3. 新 caller（如 `advance_plot`）调 `character_lock(cid)` → map 无 entry → 创建新 Arc B → acquire `read()` 立即成功（Arc B 无 contention）。
4. 新 caller 进入临界区，`StateService::mutate_locked` 内 `fs::create_dir_all` 重新创建 dir → 写 `live.json`。**race**：dir 被「复活」，`delete_character` 已返回 `Ok(())` 但 dir 存在。

**影响**：
- Windows：`fs::remove_dir_all` 在 `delete_character` 持 write guard 期间被并发文件创建打败 → `DirectoryNotEmpty`（PR #439 CI 中观察到）。
- Linux / 通用：TOCTOU——`advance_plot` 用新 Arc 绕过 write guard，复活已删 dir 的部分文件（半删状态）。

**关键区分**：这**不是** R1 TOCTOU 失效——R1（`character_lock.read()` 外层门控）在 #437 后已闭合。问题是 lock-map cleanup 过早移除 entry，使新 caller 用新 Arc 绕过 R1 锁（用新 Arc 而非旧 Arc，不与 write guard 互斥）。

**修复**（#440，2026-08-03 交付）：将 `remove_deleted_*_lock` 调用移到 write guard 显式 `drop` **之后**：

```rust
// delete_character / delete_session / delete_persona 同模式
let _guard = character.write().unwrap_or_else(|p| p.into_inner());
let result = data_dir::delete_character(&self.data_root, character_id);
drop(_guard);  // 显式释放 write guard
if result.is_ok() {
    remove_deleted_character_lock(character_id.as_str());  // cleanup 在 guard drop 之后
}
```

**闭合状态**：修复后新 caller 在 cleanup 前拿到旧 Arc（与 write guard 互斥，看到 dir 已删除 → `NotFound` fail-closed），cleanup 后拿到新 Arc（`delete_character` 已完全完成，合法串行化）。race 闭合由 drop 顺序静态保证，R1 回归测试间接覆盖（修复前 CI 偶发 `Io(NotFound)` / `DirectoryNotEmpty`，修复后稳定通过）。

**未覆盖**：Arc 指针相等性测试（`arc_ptr_eq` 模式）未实现（W-03 非阻塞 follow-up）。race 闭合由 drop 顺序静态保证，不依赖运行时检测。

## 7. 验收

本合同不引入代码改动时（docs-only PR）：

- 所有「已核验」路径必须与 `main@<commit>` 源码一致；
- 所有「已知缺口」必须在 §6 列出，不得隐藏；
- 不宣称「全仓绝无死锁」；
- 不宣称运行时强制已交付。

引入代码改动时（如 §6.1 的 thread-local tracker）：

- 必须新增测试覆盖每个 `debug_assert!` 触发点；
- 必须在 release build (`--release`) 下零开销；
- 必须在本合同 §6.1 更新状态为「已交付（部分路径）」或「已交付（全路径）」。

§6.1 验收记录（2026-08-02）：6 个单测覆盖唯一 `debug_assert!` 触发点（`track_session` 持 state 时）+ 合法方向 + Drop 释放 + 两段临界区；`cargo check --release` 通过（no-op ZST `Guard`）；§6.1 状态已更新为「已交付（部分路径）」（仅 R2 session↔state，不含 R1/R3/R6）。本地 `cargo test --workspace` 1301 passed / 0 failed，WebUI 76 passed / 0 failed，神圣不变式 `subagent_context_has_no_orchestrator_noise` 通过。

R1 收敛验收记录（PR #436，2026-08-03）：§2.4 `trigger_world_event` / §2.5 `advance_clock` / §2.6 `npc_action` / §2.7 `run_seal_flow` 四个路径已补齐外层 `character_lock.read()`；§2.3 `advance_plot` 因 `StateService::mutate` re-entrancy 风险未闭合，残留风险与修复路径记录于 §6.7。本次为静态锁序收敛，**不**改变 §6.1 运行时强制状态（R1 仍无运行时强制，仅 R2 session↔state 有 `debug_assert!`）。`cargo test --workspace --exclude airp-ui --locked` 通过（数字见 PR 描述）。

R1 残留闭合验收记录（PR #439，closes issue #437，2026-08-03）：§2.3 `advance_plot` 通过 fix path 4（拆 `StateService::mutate` 为 `mutate_locked` + `mutate`）闭合 R1。外层 `character_lock.read()` 已补齐，re-entrancy 风险消除，§6.7 残留 TOCTOU 风险已闭合。R1 例外路径数：0。本次为静态锁序收敛 + `StateService` API 拆分（新增 `mutate_locked` 方法），**不**改变 §6.1 运行时强制状态（R1 仍无运行时强制，仅 R2 session↔state 有 `debug_assert!`；R1 运行时强制见 §6.1 W-03 follow-up）。同时修正 W-01 措辞（§6.7 re-entrancy 论证从「deadlock 风险」改为「deadlock 风险（部分 pthread 实现）+ 排他性语义破坏（Windows SRWLOCK）」）。`cargo test --workspace --exclude airp-ui --locked` 通过（数字见 PR 描述）。

R1 运行时强制 + 回归测试验收记录（closes #438 W-04，2026-08-03）：§6.1 运行时强制从「仅 R2」扩展到「R1 + R2 全路径」。新增 `track_character_read()` / `track_character_write()` 标记 character 外层门控；`track_session()` / `track_state()` 增补 R1 `debug_assert!`（无 character 时 panic）。修复 `LorebookService::write` 漏调 `track_character_read()` 的遗漏（R1 强制上线即捕获）。新增 4 条并发回归测试（`advance_plot` / `trigger_world_event` / `advance_clock` / `npc_action` 各与 `delete_character` 经 `Barrier` 并发，30s 超时检测死锁 + Internal error 检测 TOCTOU）+ 9 条 R1 单测（合法路径 + 违反 panic + Drop 语义）。本地 `cargo test -p airp-core --lib --locked` 1262 passed / 0 failed / 5 ignored；`cargo test -p airp-core --tests --locked` 全绿；WebUI 98 passed / 0 failed；神圣不变式 `subagent_context_has_no_orchestrator_noise` 通过。§6.1 状态更新为「已交付（R1 + R2，全路径）」。

lock-map cleanup race 修复验收记录（closes #440，2026-08-03）：§6.8 新增。`delete_character` / `delete_session` / `delete_persona` 的 `remove_deleted_*_lock` 调用移到 write guard 显式 `drop` 之后。修复闭合 PR #434 引入的 cleanup race（#422 修复的副作用）：修复前新 caller 在 cleanup 后拿到新 Arc 绕过 write guard（Windows `DirectoryNotEmpty` / Linux TOCTOU dir 复活）；修复后新 caller 在 cleanup 前拿到旧 Arc 与 write guard 互斥（fail-closed `NotFound`），cleanup 后 `delete_*` 已完全完成（合法串行化）。race 闭合由 drop 顺序静态保证，R1 回归测试间接覆盖。本次为 cleanup 时机修复，**不**改变 §6.1 运行时强制状态。`cargo test -p airp-core --lib --locked` 1262 passed / 0 failed / 5 ignored（同 #438 验收，未新增测试）。

BACKUP_LOCK 锁序合同补全验收记录（closes #446 W-01，2026-08-03，docs-only）：§1.5 全局 utility 锁清单新增 `BACKUP_LOCK`；R4 叶锁规则列举 `BACKUP_LOCK`；§2.9 新增 `delete_character` / `delete_session` → `create_backup` 嵌套路径（`character_lock.write/read → session_lock → BACKUP_LOCK`，外→内合法序列）。本次为 docs-only 合同补全，**不**引入代码改动，**不**改变 §6.1 运行时强制状态。残留风险 W-02（`restore_backup` swap 阶段不持 character_lock，#447）记录于 §2.9，v1 缓解为维护窗口执行 restore。

## 8. 关联

- [#284](https://github.com/GhostXia/AIRP/issues/284)：per-session in-flight mutex 设计；方案 N/O 在途。
- [#220](https://github.com/GhostXia/AIRP/issues/220)：持久化/lock 遗留；PR #416 已收敛 mutex poison recovery。
- [#381](https://github.com/GhostXia/AIRP/issues/381) E-P1-2：本合同对应 issue。
- [#440](https://github.com/GhostXia/AIRP/issues/440)：lock-map cleanup race（已闭合，§6.8）。
- `284-PER-SESSION-INFLIGHT-MUTEX-DESIGN.md` §6：本合同的子集，限定 coordinator 路径。
- `docs/audits/2026-07-26-PR-335-bug-f-deadlock-audit.md`：Bug F 锁序倒置死锁审计。
- `docs/audits/2026-07-26-PR-338-bug-b-advance-clock-session-lock-audit.md`：Bug B `advance_clock` session 锁审计。
- `CURRENT-BASELINE.md` §2.1.4：锁模型分裂结构性事实。
