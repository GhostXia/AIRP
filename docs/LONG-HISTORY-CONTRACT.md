# Durable message / history 合同

> 状态：PR #1（domain / API / persistence）的权威设计入口。
>
> 日期：2026-07-12
>
> 范围：先于实现。本文由独立审计当前源码后写出，不把 issue #37/#122 或既往注释当作不可质疑前提。
>
> 关联 issue：#37（durable message-id contract）、#122（WebUI 窗口化）。
>
> 实施顺序：PR 1 只做 domain / HTTP / persistence / tests（本文）。PR 2 在 PR 1 合并后做 WebUI 分页、增量 DOM 和窗口化。

## 1. 现状真相（独立审计结论）

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
- `load_or_create_for_session` 读完 messages 后，对 `None` 的消息**就地按数组位置确定性派生**：`m{fixed_padding_of_index_and_session_salt}`。具体：`legacy_id = "m" + &keccak-ish(session_scope_salt + index)` 取前 25 字符。session_scope_salt = `scope_session_id` 或 `character_id`（legacy）的 hash 前缀。
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
- `rollback_preview` 同理增 `message_id` 路径。
- **不删 `message_index`**：向后兼容期保留（见 §2.7）。WebUI 在 PR 2 切到 `message_id`。
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

**决策：所有新增字段 `Option` + 旧端点不动 + 新端点不互斥。兼容期至少 2 个 minor release。**

- `ChatMessage` 保持 wire-compatible 的 `role` + `content`；`ChatLog.message_ids` 与其等长并行，legacy 派生或新 UUID 均在该数组中暴露。
- `StoredMessage`（jsonl 行）`id: Option<String>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`——旧 jsonl 无 id 行仍能解析，新行写真实 id。
- `HistoryQuery` / `RollbackRequest` 新增字段全 `Option`。
- HTTP 路径不变：`/v1/chat/history`、`/v1/chat/rollback`、`/v1/chat/regen`。无新端点。
- 不传新字段的旧客户端：
  - `history` 不传 `limit`/`before` → 全量返回（或 capped at `ACTIVE_HISTORY_WINDOW`，但保留 cursor meta）。**MVP 取全量返回**，PR 2 WebUI 切窗口后再默认 cap。
  - `rollback` 不传 `message_id` → 走 `message_index`。
- 弃用计划（文档声明，不在 PR 1 删）：`message_index` 在 PR 2 WebUI 切完后，下个 minor 标 `#[deprecated]`，再下个 minor 删。`before` cursor 与 `message_id` 永留。

## 3. 不变量（PR 1 必须有测试覆盖）

1. **ID 唯一**：同一 session 内任意两条消息 durable ID 不同。
2. **ID 稳定**：消息落盘后，重启 / 多次 `load_or_create` → 同一消息同一 ID。
3. **Legacy 派生稳定**：同一 legacy fixture 多次加载 → 同一派生 ID。
4. **`order`/`messages` 一致**：未来若引 id-keyed map + order 数组（PR 2 候选），`order.len() == messages.len()` 且无重复 ID。PR 1 暂用 `Vec`，不变量是 `messages.iter().map(|m| &m.id).all unique`。
5. **cursor session-scoped**：`before` 跨 session → `BadRequest`。
6. **rollback-by-ID 与 rollback-by-index 在同一位置等价**：`rollback_to_id(id_at_index_k) == rollback_to(k)`。
7. **并发 append/rollback 不半态**：`with_session` 串行化保留；并发调用要么全成功要么全失败，无部分写入。
8. **MAX_MESSAGES 不删持久化**：append 超量后，jsonl 物理仍含全部消息；`history` 能加载超量部分。
9. **神圣不变式绿**：`subagent_context_has_no_orchestrator_noise` 不破。

## 4. 验收清单（PR 1）

- 新消息 ID 持久化且跨重启稳定（`append` → reload → ID 不变）。
- legacy fixture 多次加载产生相同 ID（含 meta 丢失重建场景）。
- `history` 带 `limit`/`before` 返回正确窗口、无重复无遗漏；`has_more`/`oldest_id`/`total` 正确。
- cursor 跨 character/session → `BadRequest`。
- `rollback` 传 `message_id` 正确截断；传 `message_index` 仍工作；同传 → `BadRequest`。
- 旧请求（不传新字段）在兼容期继续工作：`history` 全量、`rollback` 走 index。
- 并发 append/rollback 不产生半态（`with_session` 串行化保留）。
- `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets -- -D warnings`、`cargo test --workspace` 全绿。
- `cargo test -p airp-core subagent_context_has_no_orchestrator_noise` 绿。
- `node webui/smoke.mjs` 56 checks / 0 failures（PR 1 不动 WebUI，但 smoke 必须仍绿——engine 新字段不破坏现有断言）。

## 5. PR 2 范围预告（不在 PR 1 做）

- WebUI `loadHistory` 改分页 + cursor：首屏 `limit=50`，滚到顶 `before=oldest_id` prepend 更早。
- 增量 DOM：`appendMsg` 按 durable ID 复用节点，refresh 不全量 `innerHTML=''` 重建。
- prepend 后滚动位置保持（记 `scrollHeight` → prepend → `scrollTop += delta`）。
- session/character 切换取消在飞 stream + 清 stale response（已有 `clearChatView`，补 stale guard）。
- regen/rollback 改用 `message_id`。
- 10k fixture 性能采样：首屏耗时、加载耗时、DOM 节点数。
- 真实浏览器 smoke：发送、刷新、加载更早、切换 session、regen、rollback。
- `webui/smoke.mjs` 增 cursor / window 断言，保持全绿。
- 用 `frontend-design` 做现状审查与交互设计，`web-design-guidelines` 做完成后可访问性 / UX 审计。

## 6. 显式不做（PR 1）

- 不改 WebUI DOM 结构、不迁 React/Vue、不增第二套 UI。
- 不改 Tauri/Vue `ui/` 任何代码。
- 不删 `message_index`（兼容期保留）。
- 不新增消息 ID 依赖。
- 不改 `chat_pipeline` 的装配序或注入逻辑（`orchestrator`）。
- 不触碰神圣不变式。
- 不做 volume / lorebook / persona / preset 相关改动。
