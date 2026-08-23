# #564 PR 5：Engine UI Surface 投影与可恢复事件流

> 状态：PR 5 实现记录。运行代码候选：`8d4efbdb5a358097d838f354c3a1a0a3b1a43628`。
>
> 本 PR 交付只读 Engine Surface snapshot/SSE、确定性 session 投影、有界 replay 与
> 脱敏 Activity 失败回执；不交付 `HttpEngineBus`、`/desktop/`、Widget intent 执行器
> 或正式桌面入口。

## 1. 纵切边界

首个 Surface 为 `session:<session_id>`，固定投影四个首方 Widget：

- `core.chat`：活动分支最近 50 条消息及稳定 message ID；
- `core.memory`：当前 session resident memory；
- `core.character-state`：Engine `StateService` 的只读结果；
- `core.activity`：实时 Coordinator 状态与最近 32 条脱敏失败回执。

Blueprint 由 Engine 确定性构建，布局为 `split → tabs/stack → widget`。动态领域值只进入
Widget props，不成为另一份角色、会话、记忆或状态真相。投影读取使用现有 domain service
与只读 session/history resolver；缺失资源返回 404，不创建目录、不迁移旧布局。

## 2. API 与恢复合同

- `GET /v1/ui/surfaces/session/:session_id?character_id=...&user_id=...` 返回当前 snapshot
  与 opaque cursor；
- `GET /v1/ui/surfaces/session/:session_id/events?...` 返回 `text/event-stream`，事件名封闭为
  `snapshot` 与 `patch`；
- 客户端只把成功应用的 SSE `id` 原样放入 `Last-Event-ID`，不得解析或合成 cursor；
- 同 boot、同有效数据根、同角色/session 且事件仍在 ring 中时连续 replay；外来、过期、
  未来或前一 boot cursor 均回退到完整 snapshot；
- registry 最多保留 128 个最近发布的 Surface，ring 同时受 256 条事件和 1 MiB
  序列化字节限制；淘汰 scope 下次请求重建 snapshot，慢消费者不会建立无界 channel；
- 小范围 props 变化发相邻 revision patch，超过 64 KiB patch 合同上限时回退到合法 snapshot；
- 机器传输合同为 `protocol/surface-sse-events.json`，payload schema 继续引用
  `protocol/surface-protocol-v2.json`。

投影所需磁盘读取在 blocking pool 执行；Surface registry 锁只覆盖内存 publish/replay，
不跨磁盘 I/O 或异步等待。

## 3. 隔离与认证

Surface route 仍经过 daemon bearer/desktop-session 鉴权；daemon 未配置 access key 时额外
fail closed 为 `503 surface_auth_unavailable`。查询中的 `CharacterId`、`SessionId` 与
`UserId` 都经现有类型校验。

内部 `SurfaceScope` 包含有效数据根、角色与 session，避免两个用户拥有同名角色/session 时
共享 snapshot、revision、cursor 或 replay ring。对外 `surface_id` 不包含用户路径。只读
Surface 查询不会调用会创建用户目录的 `resolve_effective_root`；缺失用户根保持不存在。
当前 access key/desktop token 仍是 daemon 级权限，`user_id` 只选择现有有效根，不构成
租户绑定的身份认证；因此本 PR 证明投影不串根，不把 AIRP 宣称为多租户服务。

## 4. Activity 与 prompt 边界

实时 Activity 复用 `SessionCoordinatorStatus`；非终态 `TurnCommit` 仍通过 Coordinator 投影为
`recovering`。普通 chat/Agent 的上游与 finalization 关键失败另写入 session memory 下的
`ui-activity.json`，以便 reload、daemon restart 或 snapshot resync 后继续可见。

回执 schema 是封闭的，只允许：schema version、AIRP 生成的 activity ID、时间、枚举 source、
固定 `failed/error` 分类、稳定错误码与 generation ID。禁止任意 `Value`、message、prompt、
tool params/output/chunk、provider endpoint、API key 或 RP 正文。文件最大 64 KiB、最多 32 条，
原子替换；畸形、超限或未知 schema 不覆盖原文件，Surface 将 Activity 标记为 unavailable，
但不破坏 Chat/Memory/State 投影。

依赖方向保持单向：控制面证据 → 脱敏 Activity → Surface Widget。`ui_activity` 与
`ui_surface` 不进入 `prepare_pipeline`、Orchestrator 或 `ChatCompletionRequest`，因此 Activity
不会进入角色 prompt。

## 5. 验证与未交付项

已覆盖：

- 确定性 builder、相邻 revision、unchanged 去重、patch 超限 snapshot fallback；
- event 数/字节双上限、过期/外来/前一 boot cursor resync；
- 同角色/session 跨有效用户根不别名，缺失用户查询不创建目录；
- bearer、desktop-session token、auth-disabled fail closed；
- 真实 snapshot/patch SSE wire 与机器合同 parity；
- Activity bounded、malformed fail closed、registry 重建后错误仍可见且无内容字段；
- strict Clippy、格式与受影响 Rust 测试。

明确留给后续 PR：

- PR 6：`HttpEngineBus`、浏览器 `/desktop/`、Tauri 双入口与 reconnect/resync 客户端；
- PR 7：Engine-authoritative Widget registry/grant 与 Vue host 安全等价迁移；
- PR 8/9：Chat、Memory、Character State 的真实 intent 写闭环；
- Conversation observability、工具级进度和成功历史的更丰富 Activity 投影。当前 PR 只保证
  live 状态、recovery 与关键失败，不宣称完整任务中心。
