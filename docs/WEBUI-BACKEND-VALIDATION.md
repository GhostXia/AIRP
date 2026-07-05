# WebUI 后端可靠性验证路线

> 最后更新：2026-07-05
> 目的：把当前“先用 WebUI 验证后端，并让用户先体验完整 agent 能力”的方向整理成可执行开发路线。本文里的 WebUI 是临时后端验证面和早期可用入口，不是 AIRP 的长期产品 UI。

## 1. 开发判断

UI 设计已经在产品可用前消耗了过多时间。下一步不应该继续打磨桌面 UI 控件体验，而应该先证明 engine 作为无头 RP 后端是可靠、可观察、可复现的，并让用户先通过简陋 WebUI 体验后端的完整 agent 能力。

产品形态不变：

- **长期产品 UI**：Tauri/Vue 桌面端。
- **短期验证面与早期可用入口**：浏览器 WebUI / HTTP harness。
- **稳定核心**：`engine` 是独立 HTTP/SSE 服务，不嵌进 Tauri 壳。

WebUI 回答两个问题：

1. **后端能否稳定跑通最小 RP 闭环，让 UI 不再遮住后端不确定性？**
2. **用户能否先用简陋界面触达完整 agent 能力，为本地 UI 产品化争取时间？**

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
- `/v1/agent/run` 请求表单与 agent event log。
- history、rollback、regen、state、state history 视图。
- event log：request id、状态码、耗时、SSE 事件顺序、可见错误 body。
- 两个或更多并发 chat stream 的测试入口。

明确不做：

- 最终桌面布局、主题、动效、widget 体验打磨。
- 与 `ui/src/agent-test.ts` 竞争的第二套 agent 前端控制接口。
- 浏览器侧任意本地路径读取。
- 产品插件/runtime 决策。
- 让临时 WebUI 反向决定 Tauri UI 架构。

界面风格裁定：M1 WebUI 仿 Claude Code / Codex 的 agent console，而不是 Open WebUI 的平台型聊天产品。默认结构是左侧角色/session/run history，中间 chat transcript，右侧或下方 agent event log + diagnostics。Open WebUI 只能借鉴 session 侧栏、model selector、markdown 渲染、轻量 settings drawer；不借鉴 RBAC、RAG、插件市场、PWA、企业认证、多模型平台或独立后端/数据库架构。

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
- 发起一轮 `/v1/agent/run` 并显示 `plan/tool_call/tool_result/delta/done/error`
- 取 history 并做 regen/rollback
- 失败时不吞状态码和错误 body

验收标准：开发者能启动 engine，打开 WebUI，配置真实 provider，发一条消息并看到流式回复和持久化 history；还能发起 agent run 并看到 agent 事件流。

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

### 2026-07-05 · P0 `/v1/models` provider smoke hardening

范围：先修 WebUI provider smoke 的后端阻塞面，不触碰 Tauri UI 和长期协议重构。

本轮代码断言：

- `cargo test -p airp-core --test openai_compat models_proxy -- --nocapture`：3 passed。
- `cargo test -p airp-core --test openai_compat -- --nocapture`：9 passed。
- `cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise -- --nocapture`：1 passed。
- `cargo test -p airp-core`：lib 315 passed / 1 ignored，`openai_compat` 9 passed，`sse_wiremock` 5 passed。
- `cargo clippy -p airp-core -- -D warnings`：passed。

覆盖行为：

- `/v1/models` 上游 200 时继续透传 upstream JSON，WebUI 可直接读模型列表。
- `/v1/models` 上游 401 时返回 HTTP 502 + JSON error，包含 `error.code = "upstream_status"`、`upstream_status = 401`、`upstream_body`，避免 WebUI 只看到空 502。
- provider endpoint 无法映射到 `/models` 时返回 HTTP 502 + JSON error，包含 `error.code = "invalid_endpoint"` 和原始 endpoint detail。
- invalid endpoint detail 会脱敏 URL query/userinfo；`not-a-url?api_key=secret` 只返回 `not-a-url?redacted`，响应体不包含 `secret`。
- 请求已设置显式 timeout；真实慢 upstream 会返回 typed JSON error，而不是让浏览器无限等待。

真实 provider smoke：

- provider：HTTP API `127.0.0.1:8889`，默认模型 `gemini-3.1-pro-preview`。
- direct `GET http://127.0.0.1:8889/v1/models` 返回 200，模型列表包含 `gemini-3.1-pro-preview` 和 `gemini-3.5-pro-preview`。
- engine 启动命令：`target\debug\airp-core.exe daemon --port 8891`，环境变量 `AIRP_ENDPOINT=http://127.0.0.1:8889/v1/chat/completions`、`AIRP_MODEL=gemini-3.1-pro-preview`、`AIRP_DATA_DIR=target\p0-smoke-data`。
- engine `GET http://127.0.0.1:8891/version` 返回 200。
- engine `GET http://127.0.0.1:8891/v1/models` 返回 200，并透传同一组 provider 模型列表。
- `POST /v1/chat/completions` 返回 HTTP 200 SSE，但 SSE payload 是 `event: error`，body 指向 provider `Cloud Error 429`；断言是错误路径可见，不是生成成功。
- `POST /v1/agent/run` 返回 HTTP 200 SSE，事件序列包含 `plan -> tool_call -> tool_result -> plan -> done`，`done.stop_reason = "upstream_error"`；断言是 agent loop 和工具事件可见，生成阶段被 provider quota/error 截断。

WebUI browser smoke：

- 静态 WebUI：`http://127.0.0.1:9002/`；engine：`http://127.0.0.1:8892`。
- 连接后页面 `#models-display` 渲染 `gemini-3.1-pro-preview` 与 `gemini-3.5-pro-preview`；event log 记录 `200 GET /v1/models`。
- 临时把 engine endpoint 改为 `not-a-url` 后点击 models reload，页面渲染 `err:\ninvalid_endpoint\nprovider endpoint cannot be mapped to a /models URL\ndetail=not-a-url`；event log 记录 `502 GET /v1/models`。
- 恢复 endpoint 后点击 models reload，页面再次渲染两条 provider model id；event log 记录 `200 GET /v1/models`。

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
- agent run 的计划、工具调用、工具结果、文本增量、完成/错误事件可见。
- 浏览器导入不依赖可信本地 `card_path`。
- Tauri UI 可以按已知稳定的后端合同实现。

之后 WebUI 可以保留为 developer-only 诊断页，也可以删除。除非后续明确拍板，否则它不成为 AIRP 默认产品面。

### 2026-07-05 · P1 agent event log readability + diagnostics + destructive confirm

范围：WebUI P1 三项 harness 改动（agent event log 可读化、一键诊断、destructive 确认），纯 `webui/`，不碰 engine 契约。

本轮代码断言：

- `node --check webui/app.js`：通过。
- `cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise -- --nocapture`：1 passed（神圣不变式）。
- engine binary smoke（独立 data dir `target/p1-smoke-data`，port 8895，无 provider）：`/version` `/v1/settings` `/v1/characters` 均 200。

覆盖行为：

- `AgentEvent` serde tag 分类显示：PLAN/TOOL_CALL/TOOL_RESULT/DELTA/DONE 各类配色标签 + 一行摘要 + 折叠 raw JSON。`PlanAction` 按 snake_case 正确匹配（`call_tool`/`generate`/`finish`），三类 PLAN 摘要 ALL PASS。
- step counter 流式过程中每个 `plan` 事件实时刷新 `'step N · events · ms'`；DONE 事件后显示 `stop_reason · steps · ms`（stop_reason 经映射表人类可读化）。
- agent run 二次点击先 abort 前一个（AbortController），防 SSE 事件交错竞态；客户端 30s timeout；畸形 chunk 守护。
- `#agent-output` DOM 上限 500 行，超则删最早行。
- 一键诊断：依次跑 `/version` `/v1/settings` `/v1/models` `/v1/characters`，每端点 `AbortController` 5s timeout（engine 卡死时 fail-fast 而非永悬 `'诊断中…'`），输出可复制摘要。**v1 不含 chat/agent smoke**——避免消耗 provider quota，推迟到 P2/M2。
- regen/rollback 加 `window.confirm` 二次确认；rollback 输入校验失败路由进 event log 而非污染 chat transcript。

真路径验证：

- timeout 切断：造 hung server（连上永不响应）+ Node 复刻 `diagApi` 跑 `/v1/settings`，5007ms `AbortError` 抛出 + `finally` 清 timer + 返回 `timeout after 5000ms`，PASS。
- PLAN 摘要正确性：改后 `summarizeAgentEvent` 对真三 SSE JSON（`{action:{call_tool:{tool,params}}}` / `{action:"generate"}` / `{action:"finish"}`）摘要 ALL PASS。

审计 issue 处置（issue #43/#44/#45/#46）：

- 真 bug 已修：A（PlanAction 字段名错配）、C（诊断无 timeout）、D（agent run 二次点击竞态）、F（DOM 上限）、H（step counter 非实时）。
- stale（基于旧 commit）：B（stepCount dead，`73db44f` 已修）、E（max_steps 硬编码，`00d5650` 已改读输入框）。
- 真中已修：G（spec 偏离，README 明说 v1 只 4 端点）、K（验证证据回填，本段）、T（wire-shape test，见 `engine/src/agent/mod.rs` `agent_event_wire_shape`）。
- nit 已修：I（parseInt radix）、J（stop_reason 映射）、L（未知 type 尾空格 class）、M（max_steps cap 常量）、N（btnAgentClear textContent）、O（折叠 summary 带类型提示）。
- 设计取舍保留：R（rollback 用 `prompt` 取 index 是 harness 合理 UX，二次 `confirm` 已加）。
- pre-existing 不动：Q（三元两分支相同，非本 PR 引入）。
