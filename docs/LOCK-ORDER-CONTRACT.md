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

## 2. 已核验的嵌套路径

下列路径已读源码确认锁序。任何新路径若与下列不一致，必须先更新本合同再合入。

### 2.1 `ChatService::with_session`（`domain.rs`）

```text
character_lock.read()  →  session_lock.lock()
```

- 同步 std 锁；闭包内为纯同步 I/O，不跨 `.await`。
- 同时持两把锁直到 `operation()` 返回。

### 2.2 `StateService::read` / `mutate` / `write`（`domain.rs`）

```text
character_lock.read()  →  state_lock.lock()
```

- 同步 std 锁；闭包内为纯同步 I/O，不跨 `.await`。
- `mutate` 在持锁期间执行 `load → closure → schema 校验 → replace_file → history.jsonl append → revision commit`。

### 2.3 `agent::tools::plot::advance_plot`（`plot.rs`）—— 唯一合法 session→state 嵌套

```text
session_lock.lock()  →  [StateService::mutate 内]  character_lock.read()  →  state_lock.lock()
```

- 同步 std 锁；async fn 体内不 `.await`（同步 I/O 阻塞 tokio worker——已知 debt，见 §6.2）。
- **这是 session_lock 与 state_lock 同时持有的唯一已核验路径**，方向固定为 session → state。
- 由 Bug F 修复（PR #335）后，无反向路径与之成环。

### 2.4 `agent::tools::world_event::trigger_world_event`（`world_event.rs`，Bug F 修复后）

两段独立临界区，**绝不嵌套**：

```text
阶段一: state_lock.lock()           → 释放
阶段二: session_lock.lock()         → 释放
```

- 同一调用任意时刻只持一把锁。
- Bug F（PR #335）就是消除旧版 state→session 与 `advance_plot` session→state 的锁序倒置死锁。

### 2.5 `agent::tools::world_event::advance_clock`（`world_event.rs`，Bug B 修复后）

与 §2.4 同模式：

```text
阶段一: state_lock.lock()           → 释放
阶段二: session_lock.lock()         → 释放（仅当 content_buf 非空）
```

- `advance_and_check_triggers` 在阶段一内完成 clock 推进 + 事件标记 + `save_world_events`。
- Bug B（PR #338）修复了旧版阶段二无 `session_lock` 导致的 `current.md` 并发交错。

### 2.6 `agent::tools::npc::npc_action`（`npc.rs`）

```text
session_lock.lock()
```

- 单锁，不与 state_lock 嵌套。

### 2.7 `conversation::append_event`（`conversation.rs`）

```text
conversation_lock.lock().await  →  conversation_io_lock.lock().await  →  spawn_blocking(I/O)
```

- tokio::sync::Mutex；`conversation_io_lock` 在 `spawn_blocking` 外持有，确保 journal I/O 串行。
- `append_event_locked_async` 是唯一同时持两把锁的入口；`context_projection` 只持 `conversation_io_lock`。

## 3. 锁序规则

### R1：`character_lock` 是 per-character 外层门控

获取 `state_lock`、`session_lock`（针对同一 `character_id`）前，必须先持有 `character_lock.read()`（或 `.write()`）。例外：`agent::tools::*` 通过 `StateService::mutate` 间接获取时，由 `StateService` 内部保证。

### R2：`session_lock` 与 `state_lock` 的唯一合法嵌套方向

```text
session_lock  →  state_lock   （仅 advance_plot，经 StateService::mutate）
```

- 反向（`state_lock → session_lock`）**禁止**。
- 若同一调用需同时变更 state 与 session 叙事文件，必须采用 §2.4/§2.5 的两段临界区模式：先释放 `state_lock`，再获取 `session_lock`。
- 两段临界区之间的中间状态（state 已持久化、内容未 append）由调用方明确定义 fail 行为，不得静默累积。

### R3：`conversation_lock` → `conversation_io_lock` 是唯一合法 conversation 嵌套

- `conversation_io_lock` 可单独持有（`context_projection`、`all_events_locked_async`）。
- 反向（持 `conversation_io_lock` 时获取 `conversation_lock`）**禁止**。

### R4：全局 utility 锁是叶锁

`COMMIT_LOCK`、`QUOTA_LOCK`、`PRESET_WRITE_LOCK`、`PRESET_IMPORT_LOCK`、`INDEX_LOCK`、`ENV_LOCK`、`DaemonState.*_update` 持有期间**不得**获取任何 per-character / per-conversation / per-session 资源锁。它们必须是临界区的最内层。

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

### 6.1 运行时锁序强制：未交付

本合同是**文档合同**，由 code review 与审计按 R1–R6、A1–A3、P1–P4 校验。尚无 thread-local 锁序追踪或 `debug_assert!` 运行时检查。后续可考虑在 `domain.rs` 加 `ResourceLockGuard` RAII + thread-local depth counter，先覆盖 `advance_plot` / `trigger_world_event` / `advance_clock` 三个高风险路径。

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

## 8. 关联

- [#284](https://github.com/GhostXia/AIRP/issues/284)：per-session in-flight mutex 设计；方案 N/O 在途。
- [#220](https://github.com/GhostXia/AIRP/issues/220)：持久化/lock 遗留；PR #416 已收敛 mutex poison recovery。
- [#381](https://github.com/GhostXia/AIRP/issues/381) E-P1-2：本合同对应 issue。
- `284-PER-SESSION-INFLIGHT-MUTEX-DESIGN.md` §6：本合同的子集，限定 coordinator 路径。
- `docs/audits/2026-07-26-PR-335-bug-f-deadlock-audit.md`：Bug F 锁序倒置死锁审计。
- `docs/audits/2026-07-26-PR-338-bug-b-advance-clock-session-lock-audit.md`：Bug B `advance_clock` session 锁审计。
- `CURRENT-BASELINE.md` §2.1.4：锁模型分裂结构性事实。
