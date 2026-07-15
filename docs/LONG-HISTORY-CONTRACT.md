# Durable message / history 合同

> 状态：PR #124（domain/API/persistence）与 PR #125（WebUI window）已实现；本文是合同与剩余边界入口。
>
> 日期：2026-07-12
>
> 实现基线：PR #124/#125；2026-07-15 在 `main@1f3e6ed` 复核，合同未被后续 handler/session 身份拆分改变。
>
> 关联 issue：#37（durable message-id contract）、#122（WebUI 窗口化）。
>
> 已交付：durable ID、legacy deterministic ID、完整历史保留、cursor、rollback-by-ID、50 条 WebUI 窗口、加载更早、增量 DOM 与滚动保持。

## 1. 实施前现状（历史审计结论）

- `engine/src/adapter.rs::ChatMessage` 只有 `role` + `content`，**无稳定 ID**。
- 消息身份完全靠 **`Vec` 数组索引**：`rollback_to(index)`、`delete_last_n`、`recent(n)`、`RollbackRequest.message_index`、WebUI `msgs.forEach((m, i) => …)`、rollback UI 让用户手输 `prompt('message index')`。
- `MAX_MESSAGES = 1000`（`chat_store.rs:16`）在 `append` 里对超量消息 `drain(..drop)` + 整体重写——**是物理删除，不是窗口化**。被丢的消息从 jsonl 消失，UI 永久看不到，且只对 append 路径生效，`load_or_create` 不裁剪（1000 是"活跃存储上限"，非"保留上限"）。
- legacy 迁移是 lazy 且非确定性：`load_or_create_for_session` 每次加载若旧文件存在就迁移、迁移后删旧文件；重建 meta 用 `Uuid::new_v4()`——若删旧失败、多次加载会产生不同 `session_id`。当前测试未覆盖"多次加载产生相同 ID"。
- `HistoryQuery` 只有 `character_id` + `session_id`，无 `limit`/`cursor`/`before`；history 全量返回 `Vec<ChatMessage>`。
- 并发原语：`ChatService::with_session` 用 `character_lock` (RwLock read) + `session_lock` (Mutex) 串行化同一 session 的 append/rollback/regen/delete——**正确的单写者仲裁，PR 1 必须保留**，durable ID 不能绕过它。
- `scope_session_id` 已在 HTTP 响应暴露（#85 O1），但 `ChatLog.session_id` 是内部 UUID（meta 里），与 scope session id 不同——命名易混。
- 神圣不变式 `subagent_context_has_no_orchestrator_noise`（`engine/src/agent/mod.rs:563`）独立于 chat_store，PR 1 不触碰。
- API 向后兼容现状：A6 测试已断言"不传 session_id 走 legacy per-character log"、"传 session_id 走 scoped log"——durable ID 必须延续此契约。

## 2. 合同决策

### 2.1 durable message ID 格式

**决策：新消息使用 `m{uuid-v4-simple}`，legacy 消息使用 scope-bound deterministic ID；两者均为 opaque、无 `/`/`~`/空格。**

- 形如 `m550e8400e29b41d4a716446655440000`。`m` 前缀是人眼可辨前缀，区分 user/assistant 不靠 ID。
- ID 只承担身份，不承担顺序；写入顺序由 jsonl / `message_ids` 数组位置定义。仓库已有 UUIDv4 依赖，其并发唯一性优于自制时钟和进程 nonce。
- 不复用 Tauri 侧的 `u{n}`/`a{n}` 临时 id 格式（issue #37 历史评论已指出它们不含 `/`/`~` 但非持久）——durable ID 是新空间，不与 Phase 1 短命 state id 混。
- 不把 role 编码进 ID（`u…`/`a…`）：role 可变（未来编辑/swipe），ID 不应变；role 已是 `ChatMessage.role` 字段。
- JSON Pointer 安全：新 ID 只含 `m` + 十六进制字符，legacy ID 使用 Crockford base32，均无 `/`/`~`。

**实现：** 复用 workspace 已有 `uuid` crate，不新增依赖。legacy 派生逻辑只用于没有持久化 ID 的旧消息，并以 character/session scope + index 保持稳定。

### 2.2 legacy 无 ID JSONL 的稳定兼容 / 迁移

**决策：lazy + deterministic 派生，不批量迁移、不删旧文件。**

- 旧 jsonl 行反序列化为 `StoredMessage` 时 `id` 字段 `#[serde(default)]` → `None`。
- `load_or_create_for_session` 读完 messages 后，对 `None` 的消息按逻辑消息位置与 character/session scope 确定性派生；空 JSONL 行不参与位置计算，索引固定按 `u64` 编码，移动 data root 不改变 ID。
  - 关键：**同一 legacy fixture 多次加载 → 同一 salt + 同一 index → 同一派生 ID**。坐实"legacy fixture 多次加载产生相同 ID"验收。
  - 派生 ID 只在内存 `ChatLog.message_ids` 里补，**不写回 jsonl**（守 lazy + 不删旧文件原则；避免迁移半态）。新 append 的消息写 UUIDv4-backed ID。
- 当前 `load_or_create_for_session` 的迁移逻辑有两个 bug 必须修：
  1. `meta` 丢失时 `Uuid::new_v4()` 重建 `session_id` → 改为**确定性派生**（hash `character_id` + scope 或 `character_id` + "legacy"），保证多次加载一致。
  2. legacy JSON / pre-CF2 迁移后删旧文件，若删失败下次加载会重复迁移 → 迁移成功后删旧失败不致命（新位置已写），但 `session_id` 必须稳定。改确定性派生后即使重复迁移也同 ID。
- **不强制迁移**：旧 jsonl 永远可读，派生 ID 永远稳。只有新写入带持久化 ID。这是"允许小步迁移"的落地。

### 2.3 cursor 语义

**决策：`before` cursor = 某条消息的 durable ID，返回该 ID **严格之前**的消息（更早的），按时间正序排列，limit 上界。**

- `HistoryQuery` 增字段（全部 `Option`，旧客户端不传仍全量返回）：
  - `limit: Option<usize>` — 本次最多返回多少条（默认 50；上界 200，超过 clamp）。
  - `before: Option<String>` — cursor；只接受本 session 已知 durable ID；ID 不属于本 session → `BadRequest`（**cursor 不能跨 character/session 使用**）。
- 返回新增 `HistoryMeta`（附在 `ChatLog` 响应或独立字段）：
  - `has_more: bool` — 是否还有更早消息（cursor 之前还有消息）。
  - `oldest_id: Option<String>` — 当前返回窗口里最老消息的 ID，供前端下次作 `before`。
  - `total: usize` — session 消息总数（含未加载），让前端显示"X / N"。
- 不用 `offset`/`skip`：offset 在 append/rollback 后漂移。ID 是稳定锚。
- cursor 校验：`before` 必须命中当前 session 的某条 durable ID（含 legacy 派生 ID），否则 `BadRequest("cursor not in this session")`。坐实"cursor 不能跨 character/session"。
- 排序：返回的 vec 按写入时间正序（最老到最新）；前端 prepend 更早消息时自然得到完整时间线。

### 2.4 rollback-by-ID

**决策：新增 `rollback_to_id(message_id)`，保留旧 `rollback_to(index)` 向后兼容。**

- `RollbackRequest` 增 `message_id: Option<String>`：
  - 传 `message_id` → 走 ID 寻址：在 `messages` 里找该 durable ID 的位置，`rollback_to(that_index)`。ID 不存在 → `BadRequest`。
  - 不传 `message_id`（旧客户端）→ 走 `message_index`，行为不变。
  - 同时传两个 → `BadRequest`（显式二义）。
- **不删 `message_index`**：向后兼容期保留；WebUI 已在 PR #125 切到 `message_id`。
- ID 寻址仍走 `with_session` 串行化，与并发 append 不产生半态。

### 2.5 MAX_MESSAGES 与完整历史保留

**决策：MAX_MESSAGES 不再物理删除消息；它是近期上下文的默认上限，与持久化保留解耦。**

- 当前 `append` 的 `drain(..drop)` + 整体重写 = 永久删除。改为：
  - **持久化和领域态保留全部**：jsonl append-only，不 drain。`MAX_MESSAGES` 只作为调用 `recent` 时的近期上下文限制，不影响 `ChatLog` 或 jsonl。
  - 实现取舍：PR 1 的 `ChatLog.messages` load 全量且不 drain；模型上下文通过 `recent(n)` 裁剪。这样分页、回滚、删除和 save 都基于完整历史，不会由内存窗口误覆盖旧数据。后续再以流式分页读取降低全量 load 成本。
  - 这样 `history` API 能 serve 任意历史（含超 MAX_MESSAGES 的），cursor 能加载更早。
- **不用 prompt 上下文上限作为永久删除用户历史的理由**（工作纪律第 4 条）：orchestrator 注入的 recent N 是"本轮喂模型"的窗口，与"用户历史真相"分离。真相在 jsonl 全留；窗口是注入时的裁剪。
- `MAX_MESSAGES` 暂保留为近期上下文的默认限制名称，避免无关 churn；它不再触发存储删除。

### 2.6 API 向后兼容周期

**决策：所有新增请求字段 `Option`，旧端点和旧请求形状保持兼容；移除 legacy index 前必须另行版本化决策。**

- `ChatMessage` 保持 wire-compatible 的 `role` + `content`；`ChatLog.message_ids` 与其等长并行，legacy 派生或新 UUID 均在该数组中暴露。
- `StoredMessage`（jsonl 行）`id: Option<String>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`——旧 jsonl 无 id 行仍能解析，新行写真实 id。
- `HistoryQuery` / `RollbackRequest` 新增字段全 `Option`。
- HTTP 路径不变：`/v1/chat/history`、`/v1/chat/rollback`、`/v1/chat/regen`。无新端点。
- 不传新字段的旧客户端：
  - `history` 不传 `limit`/`before` → 保持 legacy `ChatLog` 全量响应；WebUI 显式传 `limit=50` 使用窗口响应。
  - `rollback` 不传 `message_id` → 走 `message_index`。
- `message_index` 当前仍是兼容合同；不得仅凭本文假定具体 minor release 删除时间。

## 3. 不变量（backend contract 必须有测试覆盖）

1. **ID 唯一**：同一 session 内任意两条消息 durable ID 不同。
2. **ID 稳定**：消息落盘后，重启 / 多次 `load_or_create` → 同一消息同一 ID。
3. **Legacy 派生稳定**：同一 legacy fixture 多次加载 → 同一派生 ID。
4. **并行数组一致**：`messages.len() == message_ids.len() == message_timestamps.len()`，同一 session 的 `message_ids` 无重复。
5. **cursor session-scoped**：`before` 跨 session → `BadRequest`。
6. **rollback-by-ID 与 rollback-by-index 在同一位置等价**：`rollback_to_id(id_at_index_k) == rollback_to(k)`。
7. **并发 append/rollback 不半态**：`with_session` 串行化保留；并发调用要么全成功要么全失败，无部分写入。
8. **MAX_MESSAGES 不删持久化**：append 超量后，jsonl 物理仍含全部消息；`history` 能加载超量部分。
9. **神圣不变式绿**：`subagent_context_has_no_orchestrator_noise` 不破。

## 4. 验收清单

- 新消息 ID 持久化且跨重启稳定（`append` → reload → ID 不变）。
- legacy fixture 多次加载产生相同 ID（含 meta 丢失重建场景）。
- `history` 带 `limit`/`before` 返回正确窗口、无重复无遗漏；`has_more`/`oldest_id`/`total` 正确。
- cursor 跨 character/session → `BadRequest`。
- `rollback` 传 `message_id` 正确截断；传 `message_index` 仍工作；同传 → `BadRequest`。
- 旧请求（不传新字段）在兼容期继续工作：`history` 全量、`rollback` 走 index。
- 并发 append/rollback 不产生半态（`with_session` 串行化保留）。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全绿。
- `cargo test -p airp-core subagent_context_has_no_orchestrator_noise` 绿。
- `node webui/smoke.mjs` 当前为 67 checks / 0 failures，覆盖 cursor、durable IDs、rollback-by-ID 与三轮 SSE 增量读取。

## 5. WebUI 实施结果（PR #125）与剩余项

- 已完成：首屏 `limit=50`、`before=oldest_id` 加载更早、durable-ID 节点复用、stale response guard、prepend 滚动保持、键盘可达的 rollback-by-ID。
- 已完成：engine-truth smoke 扩到 67/67；真实浏览器验证 50/54 → 54/54 且视口保持，production topology gate 另覆盖 system-Chrome 重启恢复与取消流。
- 已完成：按 frontend-design 与最新 Web Interface Guidelines 补 focus-visible、reduced-motion、touch、localized counts 和 `content-visibility`。
- 剩余：WebUI/Tauri 的 10k/100k 性能采样、内存上界与真正虚拟列表；因此 #122 不关闭。
- 剩余：regen 仍按“最后一条 assistant”语义执行，不需要伪造 message ID 参数；branch/swipe/edit 属 #37 后续。

## 6. 显式不做与剩余边界

- 不改 WebUI DOM 结构、不迁 React/Vue、不增第二套 UI。
- 不改 Tauri/Vue `ui/` 任何代码。
- 不删 `message_index`（兼容期保留）。
- 不新增消息 ID 依赖。
- 不改 `chat_pipeline` 的装配序或注入逻辑（`orchestrator`）。
- 不触碰神圣不变式。
- 不做 volume / lorebook / persona / preset 相关改动。
