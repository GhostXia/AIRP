# Engine Conversation 合同

> 状态：v1 基础、策略注册表、`airp.scene.round_robin.v1` 回合执行、可重建 message projection 与 durable turn lifecycle 已实现；旧 chat/scene 迁移仍待后续切片
>
> 适用范围：`engine/src/conversation.rs`、`engine/src/conversation_projection.rs`、`engine/src/conversation_turn.rs` 与 `/v1/conversations*`

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

所有 Conversation manifest 与 journal 文件操作由 `ConversationService` 统一卸载到 blocking
worker；HTTP handler 和 turn executor 不直接执行同步文件 I/O。追加仍由
per-conversation 顺序锁串行化，不存在跨 conversation 全局写锁。独立的短时 journal
I/O 锁只覆盖实际 append/read/sync：读取窗口不会观察半条记录，也不会等待 turn 持有
顺序锁进行最长 120 秒的 provider 调用；immutable manifest 读取同样不延长 I/O 锁。

恢复遵循“完整事件优先、损坏尾部可截断”：

- `write_all` 中途失败留下的无换行尾部，重启后的下一次追加会截断到最后一条已验证事件；
- `sync_data` 返回失败属于 unknown-commit 边界；若完整 JSONL 记录在重启后仍可见，恢复扫描会保留它，调用方必须按实际 next sequence 继续；
- cache 写失败不影响 journal 真相；重启后扫描完整 journal 重建 sequence；
- 可被恢复性截断的只有两类尾部：无换行结尾的不完整尾部（无论是否有 cache），以及超出已验证 cache 边界的最后一条损坏记录。无可用 cache 时，带换行的损坏尾记录仍 fail-closed；cache 已确认范围内及中间记录损坏同样 fail-closed。

当前仍保持每个事件一次 `sync_data`。2026-07-29 在维护者 Windows/D 盘环境以 release
构建运行 64 次连续追加基准，命令和结果记录在本节下方；该单机微基准只用于判断本批
是否有证据降低 durability，不代表生产 SLO。现阶段没有跨设备、断电与尾延迟证据支持
批量 fsync，因此不降低资产安全边界。

```text
cargo test -p airp-core --lib conversation::tests::conversation_append_fsync_benchmark \
  --release --locked -- --ignored --exact --nocapture

# appends=64, elapsed_ms=338, mean_ms=5.288
```

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
| `GET` | `/v1/conversations/{id}/turns/{turn_id}` | 从 journal 重建回合生命周期；重启后悬空回合收敛为 `unknown_commit` |
| `POST` | `/v1/conversations/{id}/turns/{turn_id}/cancel` | 显式请求 Engine 协作式取消在途回合 |
| `GET` | `/v1/conversation-policies` | 列出已注册策略及其配置 schema |
| `POST` | `/v1/scenes/{scene_id}/conversations` | 把 scene 快照成通用 conversation |

scene adapter 会：

- 把 scene characters 转成 `character:{id}` participant；
- 把 scene 记录为 resource ref；
- 保留调用方新增的其他 participant 类型；
- 缺省引用 `airp.scene.round_robin.v1` 策略。

执行路径通过 Engine `ConversationPolicyRegistry` 解析 manifest 中的开放 policy ID。存储允许未知 ID 无损往返，但执行只接受已注册策略；重复注册、空 ID 和未知策略均 fail-closed。每个策略公开带 `schema_version` 的 descriptor 与配置 schema，因此客户端可以发现能力，但不能自行定义调度语义。

`POST /v1/conversations/{id}/turns` 当前内置注册 `airp.scene.round_robin.v1`，并按 manifest 中角色 participant 的稳定顺序执行：

1. 客户端可提供稳定 `turn_id`，其值必须符合 Engine durable ID（ULID）格式；非法 ID 在提交、状态查询和取消入口均返回 `400 Bad Request`。相同 ID 与相同语义请求只执行一次并重放原结果；相同 ID 的不同请求返回 `409 Conflict`。省略 ID 保留旧的一次性提交兼容路径；
2. 在整轮共享的异步 conversation 写边界内校验 `expected_next_sequence`，依次写入 `turn.accepted`、`turn.started`。request fingerprint 仅用于幂等冲突检测，覆盖业务语义，但排除 `turn_id`、provider endpoint 和 API credential；因此凭据轮换或等价传输地址变化不会把已完成回合误判为新语义；
3. 持久化调用者归属明确的 `message.created`，再从权威 event journal 投影历史，由 Engine 逐角色组装 prompt 并调用 provider；
4. 每个角色结果立即以其 participant ID 持久化，后续角色可以看到本轮先前角色的输出；
5. 成功写入 `turn.completed`；provider 或组装失败写入 `turn.failed`；显式取消写入 `turn.cancelled`；
6. 若进程重启后发现 `accepted` 或 `running` 而无终态，Engine 写入 `turn.unknown_commit`。这表示 provider 侧结果不可证明，不会自动重调 provider、删除已提交事件或伪造原子回滚。

精确生命周期为 `accepted → running → completed | failed | cancelled | unknown_commit`；`accepted` 也可在启动前直接进入 `failed | cancelled | unknown_commit`。原响应字段 `status=completed|partially_committed` 为兼容保留，调用方应使用新增的 `lifecycle_state` 区分失败、取消和未知提交。

取消是 Engine endpoint 驱动的显式协作协议。HTTP 客户端断线或丢弃响应不会被解释为回滚命令；如果执行 future 因断线/进程退出而消失，已写事件仍可读，下一次状态查询将其收敛为 `unknown_commit`。取消 provider 请求 future 也不证明上游未接收请求，因此 journal 终态只陈述 Engine 可证明的事实。若 provider 已返回内容后才观察到取消，Engine 仍记录实际消耗的 token，但丢弃未持久化的生成内容并写入 `turn.cancelled`，避免把取消后的内容伪装为已提交消息。

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
2. Projection 后续：在已交付的 message/history 与 turn lifecycle projection 上增加通用审计视图；所有投影继续保持可重建，不成为第二真相。
3. Turn executor 后续：增加可配置停止条件、跨进程 provider operation reconciliation，以及不改变 journal 真相的长上下文压缩策略。
4. Compatibility adapters：让角色 chat、scene/group 和 Council 逐步消费通用内核；旧 API 在兼容期保持响应形状。
5. Recovery/export：纳入统一备份 manifest、完整性校验与恢复演练。

新的 Conversation 场景回合已经由 Engine 闭合；既有 WebUI/legacy scene 路径尚未迁移，因此不得宣称所有旧群聊入口已经闭合。
