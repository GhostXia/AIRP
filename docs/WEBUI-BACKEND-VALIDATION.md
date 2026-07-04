# WebUI 后端可靠性验证路线

> 最后更新：2026-07-04
> 目的：把当前“先用 WebUI 验证后端”的方向整理成可执行开发路线。本文里的 WebUI 是临时后端验证面，不是 AIRP 的长期产品 UI。

## 1. 开发判断

UI 设计已经在产品可用前消耗了过多时间。下一步不应该继续打磨桌面 UI 控件体验，而应该先证明 engine 作为无头 RP 后端是可靠、可观察、可复现的。

产品形态不变：

- **长期产品 UI**：Tauri/Vue 桌面端。
- **短期验证面**：浏览器 WebUI / HTTP harness。
- **稳定核心**：`engine` 是独立 HTTP/SSE 服务，不嵌进 Tauri 壳。

WebUI 只回答一个问题：**后端能否稳定跑通最小 RP 闭环，让 UI 不再遮住后端不确定性？**

## 2. 当前起点

当前仓库已经有足够的后端面可以先验证：

- workspace 是 `engine + protocol + ui/src-tauri`。
- `engine` 已暴露 `/v1/*` HTTP/SSE 端点，包括 chat、agent run、characters、sessions、state、settings、scenes、presets、models、history、rollback、regen。
- `ui/src-tauri/src/bus.rs` 已把 `chat.send` 路由到 `POST /v1/chat/completions` 并消费 SSE。
- 角色导入在 Tauri/本地 sidecar 路径下已是 path-first。
- chat 状态已是 id-keyed `{messages, order}`，`chat_lock` 已移除。
- Tauri 打包链路已携带 engine sidecar，但真实配置下的 GUI runtime smoke 还没有收口。

所以当前瓶颈不是“从零发明整个应用”，而是**让后端行为可见、可重复、不可被 UI 表象误读**。

## 3. 范围

临时 WebUI 只做后端控制和观察：

- Engine URL、access key、当前 settings（API key 脱敏）。
- health/version 检查。
- 角色列表与 fixture/upload 形式的导入路径。
- session 列表、创建、选择。
- chat 请求表单与 SSE 流式 transcript。
- `/v1/agent/run` 请求表单。
- history、rollback、regen、state、state history 视图。
- event log：request id、状态码、耗时、SSE 事件顺序、可见错误 body。
- 两个或更多并发 chat stream 的测试入口。

明确不做：

- 最终桌面布局、主题、动效、widget 体验打磨。
- 与 `ui/src/agent-test.ts` 竞争的第二套 agent 前端控制接口。
- 浏览器侧任意本地路径读取。
- 产品插件/runtime 决策。
- 让临时 WebUI 反向决定 Tauri UI 架构。

## 4. 验证里程碑

### M0. 后端端点矩阵

从当前源码整理一张小矩阵：

- endpoint path
- method
- 是否需要鉴权
- request shape
- response shape
- 是否 streaming
- data 目录副作用
- 已知风险或缺口

这张矩阵必须来自 router/source 检查，不从旧设计 prose 里抄。

### M1. 先做 HTTP Harness，不做产品 UI

最小浏览器验证面先能完成：

- 调 `/version`
- 读写 `/v1/settings`
- 列角色
- 创建/列 session
- 发一条 `/v1/chat/completions` 并按顺序渲染 token stream
- 取 history 并做 regen/rollback
- 失败时不吞状态码和错误 body

验收标准：开发者能启动 engine，打开 WebUI，配置真实 provider，发一条消息，并看到流式回复和持久化 history。

### M2. 可靠性检查

用 WebUI 覆盖不舒服但真实的情况：

- API key 缺失或错误
- model 错误
- provider timeout 或 SSE 断流
- access key 已开启但缺 bearer token
- 同一角色或不同角色的两个并发 chat stream
- 成功 chat 后刷新页面
- streamed response 后 regen/rollback
- chat 前后的 state/history 端点

验收标准：每个失败路径都有可见解释；成功路径留下可预测的文件/state。

### M3. 数据安全边界

浏览器/WebUI 导入不能复用可信本地 Tauri 的 `card_path` 语义。只允许：

- bundled/test fixture id，
- multipart 或 streaming upload，
- 受控复制到 engine 管理的临时 import 目录。

验收标准：浏览器调用方不能要求 engine 读取任意本地绝对路径。

### M4. 把已验证行为回灌到 Tauri

M1-M3 可复现后，再把同一批行为接回桌面 UI：

- settings 可见性和错误展示
- streaming chat 状态
- history/regen/rollback 交互
- character/session state refresh
- 与后端 error body 对齐的失败提示

验收标准：桌面 UI 开发变成对已验证后端行为的产品化，而不是继续探索后端不确定性。

## 5. 验证证据

每次验证都要记录：

- engine 启动命令
- WebUI URL
- engine URL，以及 access key 是否开启
- provider/model 名称，API key 脱敏
- request path 和 payload 摘要
- status code 和耗时
- streaming 调用的 SSE event sequence
- 触碰的数据目录
- 失败路径的截图或保存日志

不要只写“已验证”，要写清楚具体断言。

## 6. 立即开发顺序

1. 暂停不阻塞后端验证的桌面 UI 产品打磨。
2. 补后端端点矩阵。
3. 做最小 WebUI/HTTP harness。
4. 验证 settings、characters、sessions、chat streaming、history、regen、rollback、state。
5. 跑并发和错误路径。
6. 把结果带具体证据写回文档。
7. 再恢复 Tauri UI 产品化，把已证明稳定的后端路径接进去。

## 7. 退出条件

临时 WebUI 完成使命的标准：

- 不经过 Tauri 也能从浏览器复现后端 chat streaming。
- data persistence 和 session/history 行为可观察。
- 鉴权和错误行为可见。
- 并发 stream 不破坏 id-keyed chat state。
- 浏览器导入不依赖可信本地 `card_path`。
- Tauri UI 可以按已知稳定的后端合同实现。

之后 WebUI 可以保留为 developer-only 诊断页，也可以删除。除非后续明确拍板，否则它不成为 AIRP 默认产品面。
