# Engine Conversation 合同

> 状态：v1 基础、策略注册表、`airp.scene.round_robin.v1` 回合执行与可重建 message projection 已实现；旧 chat/scene 迁移仍待后续切片
>
> 适用范围：`engine/src/conversation.rs`、`engine/src/conversation_projection.rs` 与 `/v1/conversations*`

## 1. 定位

`Conversation` 是 Engine 一级资源，不属于 WebUI、Tauri、某个角色或某个场景。角色单聊、多人场景、Council、外部 Agent 和未来交互模式可以复用同一身份与事件合同；各 UI 只是读取、提交意图和显示投影，不拥有持久化真相或调度权。

“开放扩展”在 v1 中具体表示：

- manifest、event 和 event payload 均有显式 schema version；
- 参与者类型、资源类型、策略 ID 和事件 kind 是开放字符串；
- 未被 Core 理解的领域数据进入 namespaced `extensions`，可无损保存；
- Core 只强制身份、顺序、隔离和持久化不变量，不替具体 adapter 判断业务成员资格或事件语义；
- 新领域优先新增 adapter、policy 或 projection，不修改旧 UI 以反向塑造 Core。

开放字符串不表示执行任意代码。策略、工具和外部 adapter 仍必须经过 Engine registry、capability、资源上界和安全校验。

## 2. 身份与目录

Conversation ID 复用经验证的 UUID `SessionId` 格式，但它是独立资源身份，不等于角色命名 session：

```text
<effective-root>/conversations/{conversation_id}/
├── manifest.json
├── events.jsonl
└── journal_state.json
```

- 单用户请求的 effective root 是 `data/`；
- 带 `user_id` 的请求使用 `data/users/{user_id}/`，不同用户不能互相列出或读取 conversation；
- `manifest.json` 是不可变初始身份与配置；
- `events.jsonl` 是运行变化的唯一真相；
- `journal_state.json` 只是经过日志长度、尾事件 ID 和 sequence 复核的加速缓存。缺失、不一致或损坏时必须扫描事件日志恢复，不能覆盖事件真相。

## 3. Manifest v1

稳定字段：

- `schema_version`
- `conversation_id`
- `title`
- `participants[]`
- `resources[]`
- `orchestration`
- `extensions`
- `created_at`

Participant 使用 conversation 内稳定的 `participant_id`，另带开放 `kind` 和可选 resource ref。Resource ref 使用开放 `kind + id + revision`，避免把角色、场景、Agent 或外部身份硬编码成互斥字段。

同一 manifest 内 `participant_id` 必须唯一。Core 不要求 actor 永远出现在初始 participants：动态加入、外部 Agent 与工具代理的授权由具体策略或 adapter 判断，并通过事件留下审计轨迹。

未知顶层字段 fail-closed；向前扩展必须进入 namespaced `extensions`，例如 `airp.scene.v1` 或 `vendor.example`。这让旧读取器不会静默忽略具有强语义的新字段。

## 4. Event journal v1

每条 JSONL 记录至少包含：

- `schema_version`
- `event_id`：JSON Pointer 安全的 durable ID
- `conversation_id`
- `sequence`：从 0 开始严格连续
- `kind`：开放事件类型
- `actor_id`
- `causation_id` / `correlation_id`
- `payload`
- `extensions`
- `occurred_at`

写入在 per-conversation 单写者边界内完成：

1. 从已验证 cache 取得下一 sequence；cache 失配则扫描 journal；
2. 可选 `expected_next_sequence` 做乐观并发检查，不符返回 `409 Conflict`；
3. 把完整 JSON 对象和换行组成单一 buffer；
4. `write_all` 后 `sync_data`；
5. 最后更新加速 cache。cache 更新失败不回滚已经 durable 的事件。

事件 kind 和 payload 由领域 adapter 解释。Core 不把未知 kind 丢弃或强制转成消息。

## 5. 窗口读取与性能

`GET /v1/conversations/{id}/events` 支持：

- `limit`：默认 50，上界 200；
- `before`：返回该 durable event ID 严格之前的事件；
- `has_more`、`oldest_id`、`total`、`next_sequence`。

cursor 必须属于当前 conversation，跨 conversation 使用返回 `400`。读取会扫描 journal 以验证连续性和计算总数，但只保留请求窗口，内存占用为 `O(limit)`；正常追加通过已验证 tail cache 恢复 sequence，不随完整历史线性扫描。

后续若引入索引，索引只能是可重建加速结构，不能替代 JSONL 真相或改变 cursor 语义。

## 6. Message projection v1

`conversation_projection` 是 Engine-owned 的纯投影模块。`ConversationService::message_projection` 只读取当前 conversation 的 immutable manifest 与完整 event journal；不会读取 WebUI 状态、实时角色卡、scene 文件或其他可变资源。输出带 `schema_version = 1`，相同 manifest 与 events 必须产生逐字段相同的结果。

投影语义：

- 只解释 `message.created`；其他 event（包括未知 kind）保留在 journal 中，但不进入 message view，并计入 `ignored_non_message_event_count`。
- `message.created.payload` 必须包含字符串 `content` 与显式 `role: "user" | "assistant"`，且 event 必须带非空 `actor_id`；不满足时不猜测，跳过并计入 `ignored_invalid_message_count`。
- role 是消息发生时写入 event 的快照，不从 participant 当前 `kind` 或资源状态反推。因此同一 participant 在不同时点的 role 变化不会重写旧消息。
- manifest 中不存在的 actor 只要 payload 合法仍保留归属，并计入 `unresolved_actor_count`；未知 actor 不会被静默猜成 user 或 assistant。
- 多个 participant 即使引用同一角色资源，仍按各自稳定 `actor_id` 保持消息归属。
- 角色卡、scene 或外部资源后续修改/删除不会改变历史投影；manifest snapshot 与 journal 足以重建。
- 转换为 provider history 时，assistant 消息使用 `[actor_id] content` 保持多 speaker 可辨识，user 消息保持原 content。

projection 是 derived view，不落盘为第二真相。统计字段仅解释本次重建结果，删除后可由相同输入恢复。

## 7. HTTP API

| Method | Path | 当前语义 |
|---|---|---|
| `POST` | `/v1/conversations` | 创建通用 conversation |
| `GET` | `/v1/conversations` | 列出 effective root 下的 manifests |
| `GET` | `/v1/conversations/{id}` | 读取 manifest |
| `POST` | `/v1/conversations/{id}/events` | 追加一个开放事件 |
| `GET` | `/v1/conversations/{id}/events` | cursor 窗口读取 |
| `POST` | `/v1/conversations/{id}/turns` | 由 Engine 执行并持久化一个场景回合 |
| `GET` | `/v1/conversation-policies` | 列出已注册策略及其配置 schema |
| `POST` | `/v1/scenes/{scene_id}/conversations` | 把 scene 快照成通用 conversation |

scene adapter 会：

- 把 scene characters 转成 `character:{id}` participant；
- 把 scene 记录为 resource ref；
- 保留调用方新增的其他 participant 类型；
- 缺省引用 `airp.scene.round_robin.v1` 策略。

执行路径通过 Engine `ConversationPolicyRegistry` 解析 manifest 中的开放 policy ID。存储允许未知 ID 无损往返，但执行只接受已注册策略；重复注册、空 ID 和未知策略均 fail-closed。每个策略公开带 `schema_version` 的 descriptor 与配置 schema，因此客户端可以发现能力，但不能自行定义调度语义。

`POST /v1/conversations/{id}/turns` 当前内置注册 `airp.scene.round_robin.v1`，并按 manifest 中角色 participant 的稳定顺序执行：

1. 在整轮共享的异步 conversation 写边界内校验 `expected_next_sequence`；
2. 先持久化调用者归属明确的 `message.created`；
3. 从权威 event journal 投影历史，Engine 逐角色组装 prompt 并调用 provider；
4. 每个角色结果立即以其 participant ID 持久化，后续角色可以看到本轮先前角色的输出；
5. 成功写入 `turn.completed`；provider 或组装失败则写入 `turn.failed`，返回 `partially_committed`，不伪造回滚或泄漏上游错误正文。

participant 总量不设产品级上限，但单回合 speaker plan 最多执行 16 个 provider 调用，并在写入用户事件前原子预留对应的请求配额。整轮 provider 调用共享 120 秒绝对 deadline；超时会取消当前调用并 durable 写入 `turn.failed`。历史扫描、解析和 journal durable append 在 blocking pool 中执行，不占用 async runtime worker。

客户端不能在 turn 请求中注入 `scene_id`、`session_id`、`character_id`、history 或 legacy branch/swipe 控制。provider、model、preset、persona 和采样参数继续复用既有 chat pipeline 合同，因此 UI 只是能力调用方，不决定 Engine 的作用域、历史或调度。

## 8. 兼容边界

- 旧 `/v1/chat/completions`、`/v1/chat/*`、`/v1/sessions/*` 和角色目录布局不变；
- 未提供 `session_id` 的 legacy 角色聊天继续使用原角色级 history/memory；
- 既有 scene prompt 与 scene memory 不自动迁移；
- 现有 WebUI “多人场景”没有权威共享历史，不能把其多个角色轮询自动导入为真实 conversation；
- 迁移只能显式执行，并生成可读报告；没有可靠消息归属的数据不得猜测 speaker。

## 9. 后续实现门

1. Policy registry 后续：在现有注册/解析/descriptor 边界上增加经过 capability 校验的外部策略装载，以及可配置停止条件；不得让任意配置变成代码执行。
2. Projection 后续：在已交付的 message/history projection 上增加运行状态与审计视图；所有投影继续保持可重建，不成为第二真相。
3. Turn executor 后续：增加取消协议、未知提交状态恢复、可配置停止条件，以及不改变 journal 真相的长上下文压缩策略。
4. Compatibility adapters：让角色 chat、scene/group 和 Council 逐步消费通用内核；旧 API 在兼容期保持响应形状。
5. Recovery/export：纳入统一备份 manifest、完整性校验与恢复演练。

新的 Conversation 场景回合已经由 Engine 闭合；既有 WebUI/legacy scene 路径尚未迁移，因此不得宣称所有旧群聊入口已经闭合。
