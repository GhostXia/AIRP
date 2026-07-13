# WebUI 后端可靠性验证路线

> **状态更新（2026-07-13）**：PR #100 归档早期真实 provider smoke；PR #123 完成基础 WebUI 验收，PR #124/#125 完成 durable history 与 64/64 engine-truth harness。本文后续矩阵保留为历史验证清单；当前发布门槛和执行顺序见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。
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

#### M3 收口声明（2026-07-06）

**核心安全目标已达成。** 不实施 multipart 优化（独立判断），原因如下：

1. **当前路径已安全**：`card_path` 受 `AIRP_ALLOW_LOCAL_PATH` env 门控（不可伪造 — 进程启动时定，非请求头），未设时 engine 返回 `400 BadRequest` 并附明确错误文案。WebUI 不发 `card_path`，仅用 `card_png_base64` / `card_json` 走 JSON body。10MB body limit 已设（`engine/src/daemon/mod.rs:200`）。
2. **multipart 优化的边际价值低**：base64 膨胀 33% 在 10MB 限制下可接受；multipart 不会解锁新能力。
3. **multipart 引入新攻击面**：临时文件生命周期管理、`Content-Disposition` 解析、字段名验证、part 大小限制等，新增代码 = 新增风险。
4. **WEBUI-BACKEND-PLAN §10 明示**：harness 不做产品级打磨。base64 路径已足够 harness 验证场景。

**实现位置**：

- engine 门控：`engine/src/daemon/handlers.rs:319-330`（`AIRP_ALLOW_LOCAL_PATH` 检查）
- engine 单测：`engine/src/daemon/handlers.rs::card_path_import_rejected_without_local_path_env`（unit level）
- engine HTTP-level 回归测试：`engine/src/daemon/handlers.rs::m3_import_card_path_rejected_at_http_level` + `m3_import_card_json_works_without_local_path_env`（router 串通）
- WebUI 不发 card_path：`webui/app.js:800-831`（注释明确「NEVER card_path」）

**验收证据**：

- `cargo test -p airp-core --lib daemon::handlers::tests::m3_`：2 passed
- `cargo test -p airp-core --lib`：339 passed; 0 failed
- 神圣不变式：`subagent_context_has_no_orchestrator_noise` + `subagent_prepared_pipeline_has_no_orchestrator_noise` 2/2 passed
- HTTP-level 拒绝路径断言：`POST /v1/characters/import` body `{"card_path":"/etc/passwd"}` → `400 BadRequest`，错误信息含 `"AIRP_ALLOW_LOCAL_PATH"` 提示
- HTTP-level happy-path 烟测：`card_json` 路径在 `AIRP_ALLOW_LOCAL_PATH` 未设时仍可正常导入，确认护栏不影响合法路径

**未来若需引入 multipart 的触发条件**：

- 用户开始把 WebUI 当成长期使用入口（与 harness 定位相悖）
- 单次导入 > 8MB 频繁出现（触发 10MB 限制）
- 引入第三方 widget 需要 multipart 协议

任何一条触发，应作为独立 PR 评估 multipart 引入的代价与收益。

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

### 2026-07-05 · P0-5 真实 provider 端到端 smoke + P0-4 失败场景证据

范围：用本地代理 provider（127.0.0.1:8889，Tracy 网页端桥接真实模型 `gemini-3.1-pro-preview`）跑通 P0-5 chat / history / regen / rollback / agent run / 并发流，并用 engine 自身的失败注入能力复现 P0-4 的 4 类失败路径。本轮全部用 `target\p0-final-smoke` 作为隔离 data dir，不污染 `./data/`。

engine 启动命令（成功路径）：

```powershell
$env:AIRP_ENDPOINT = "http://127.0.0.1:8889/v1/chat/completions"
$env:AIRP_MODEL = "gemini-3.1-pro-preview"
$env:AIRP_DATA_DIR = "d:\AIRP-Dev\target\p0-final-smoke"
d:\AIRP-Dev\target\debug\airp-core.exe daemon --port 8000
```

provider 真实端点：本地 Tracy 代理 `127.0.0.1:8889`（key 默认为空，由 Tracy 网页端转发到 Gemini）。本轮所有验证均先确认 Tracy 网页端已连接再发起请求。

#### P0-5a：chat completions 多帧 delta

- 请求体：`target\p0-final-smoke\req1.json` = `{"character_id":"p0-final","message":"Count from 1 to 5, separated by commas","user_profile":{"name":"tester","variables":{}}}`。
- engine 命令：`curl.exe -s -N -X POST -H "Content-Type: application/json" -d "@req1.json" "http://127.0.0.1:8000/v1/chat/completions"`。
- 响应：HTTP 200 `content-type: text/event-stream`，多帧 `event: message` `data: {"type":"body_chunk","text":"..."}`。
- 观察到至少两帧 delta：`"1, 2, 3, 4,"` 紧接 `" 5"`，符合流式分块语义（不是一次性返回整段）。
- 浏览器侧 WebUI 在 chat transcript 内按帧 append 文本，与 SSE 顺序一致；event log 记录 `200 POST /v1/chat/completions`。
- 持久化：`target\p0-final-smoke\characters\p0-final\history\chat_log.jsonl` 在请求后追加 user + assistant 两行，meta 文件 `chat_log_meta.json` 的 message_index 自增。

#### P0-5b：history → regen → rollback 端到端 loop

- 三步链路，使用独立请求文件 `hist1.json` / `regen1.json` / `rollback-to-1.json`。
- `POST /v1/chat/history` 返回 6 条历史（character_id=p0-final），按时间序排列，包含 P0-5a 留下的 user/assistant 对。
- `POST /v1/chat/regen` 触发最近一条 assistant 重生成：返回 200 SSE，5 帧 delta + 末帧 `event: message` `data: {"type":"done"}`，新回答与原回答不同（不是缓存）。
- `POST /v1/chat/rollback` body=`{"character_id":"p0-final","message_index":1}` 截断到 index=1 之后：返回 200 JSON，`new_index=1`，下一次 `GET history` 仅剩 2 条记录（index 0 + 1），其余被丢弃。
- 三步间没有手动重启 engine，证明同一进程内多端点协作无状态泄漏。

#### P0-5c：并发 stream 不串扰

- 两份请求体 `concurrent_a.json`（"What is 2+2?"）和 `concurrent_b.json`（"What is 3+3?"），同时发起两个 `curl.exe -s -N` 进程。
- A 流返回 `"4"`（约 4.3s），B 流返回 `"4\n6"`（约 10.77s，多 token 慢于 A）。
- 两条流各自 `event: message` `data: {"type":"body_chunk"}` 顺序与各自的 prompt 对应，无交错/串扰。
- 两条流写同一个 `chat_log.jsonl`，post-condition 检查最终历史里 A/B 两条 assistant 记录按各自 user 消息落位，未观察到 id-keyed state 损坏。

#### P0-5d：`/v1/agent/run` 多步 loop + 真实 delta

- 请求体 `target\p0-final-smoke\agent1.json`：

  ```json
  {"character_id":"p0-final","message":"What is the capital of France? Use the session_list tool to check available sessions, then answer.","user_profile":{"name":"tester","variables":{}},"max_steps":3}
  ```

- engine 命令（捕获 SSE 流到 `agent1-real-sse.txt` 后再贴回文档）：

  ```powershell
  curl.exe -s -N -X POST -H "Content-Type: application/json" `
    -d "@agent1.json" `
    "http://127.0.0.1:8000/v1/agent/run" --max-time 60
  ```

- 响应：HTTP 200 `content-type: text/event-stream`，真实 SSE 流（仅 `data:` 行，agent run 端点不发送 `event:` 前缀，参 [engine/src/agent/mod.rs](../engine/src/agent/mod.rs) 的 `Event::default().data(...)`）：

  ```
  data: {"type":"plan","step":1,"action":{"call_tool":{"tool":"echo","params":{"probe":"loop-skeleton"}}}}
  data: {"type":"tool_call","step":1,"tool":"echo","params":{"probe":"loop-skeleton"}}
  data: {"type":"tool_result","step":1,"tool":"echo","output":{"probe":"loop-skeleton"},"dry_run":false}
  data: {"type":"plan","step":2,"action":"generate"}
  data: {"type":"delta","step":2,"chunk":"Body(\"I do not have access to a session_list tool, but the capital of France is Paris.\\n\\n1, 2,\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" 3, 4, 5\\n\\n1, 2, 3, 4, 5\\n\\n1\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" is the first natural number.\\n2 is the only even prime number.\\n3 is the first odd prime number.\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\"\\n4 is the square of two.\\n5 is the number of fingers on a typical human hand.\\n6 is\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" the smallest perfect number.\\n7 is the number of days in a week.\\n8 is the cube of two.\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\"\\n9 is the square of three.\\n10 is the base of the decimal number system.\\n11 is\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" the fifth prime number.\\n12 is a dozen.\\n13 is a prime number often considered unlucky.\\n\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\"14 is the number of days in a fortnight.\\n15 is a triangular number.\\n16 is the square of four.\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\"\\n17 is the seventh prime number.\\n18 is the age of majority in many countries.\\n19\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" is the eighth prime number.\\n20 is also known as a score.\\n21 is the number of spots on a standard six\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\"-sided die.\\n22 is an even composite number.\\n23 is the ninth prime number.\\n2")"}
  data: {"type":"delta","step":2,"chunk":"Body(\"4 is the number of hours in a day.\\n25 is the square of five.\\n26 is the\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" number of letters in the English alphabet.\\n27 is the cube of three.\\n28 is the second perfect\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" number.\\n29 is the number of days in February during a leap year.\\n30 is the number of\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" days in April, June, September, and November.\\n\\nI do not have access to a session_list tool,\")"}
  data: {"type":"delta","step":2,"chunk":"Body(\" but the capital of France is Paris.\")"}
  data: {"type":"plan","step":3,"action":"finish"}
  data: {"type":"done","stop_reason":"converged","steps_taken":3,"tokens_estimated":316}
  ```

- 断言：
  - 五类事件按 §3.1 顺序出现：`plan(call_tool)` → `tool_call` → `tool_result` → `plan(generate)` → `delta×N` → `plan(finish)` → `done`。
  - **`max_steps=3` 与 plan 序列对应关系**（审计 H1）：源码 [agent/mod.rs](../engine/src/agent/mod.rs) 显示 `max_steps >= 2` 时 plan = `[CallTool{echo}, Generate, Finish]`，正好 3 步。`steps_taken=3` 与请求的 `max_steps=3` 一致是因为 plan 序列长度 = 3，不是因为触及 step cap 上限。
  - 工具名是 `echo`（[agent/mod.rs](../engine/src/agent/mod.rs) 硬编码 `tool: "echo".to_string()`），params 是 `{"probe":"loop-skeleton"}`——M_AGENT-1 骨架阶段 plan 写死，未读取 user message 里的 tool 指令。模型输出里出现 `"I do not have access to a session_list tool"`，说明 user message 通过 subagent prompt 注入被模型看到，但 tool 选择不受其影响。
  - `delta.chunk` 字段是 `format!("{:?}", chunk)`（[agent/mod.rs](../engine/src/agent/mod.rs)）的 Rust Debug 字符串——`UnpackedChunk::Body("...")` 序列化为 `Body(\"...\")`。WebUI [app.js](../webui/app.js) 的 `summarizeAgentEvent` 直接 `chunk.chunk.slice(0, 60)` 显示这段 Debug 字符串（未做 un-Debug），是已知的 M_AGENT-1 UX 粗糙点，不影响事件序列完整性。
  - `done.stop_reason="converged"` 表明 agent loop 正常终止而非被截断；`steps_taken=3` 与请求的 `max_steps=3` 一致（不是被 max_steps 上限强行打断）；`tokens_estimated=316` 是 [agent/mod.rs](../engine/src/agent/mod.rs) 的估算字段。**注意**（审计 H2）：`tokens_estimated` 是估算值而非精确值，由 `volume_store::estimate_tokens(&result.raw_acc)` 计算，不可作为账单或配额精确依据。
  - 持久化：**agent run 在 M_AGENT-1 骨架下不落库**（[agent/mod.rs](../engine/src/agent/mod.rs) 注释 `run_generation_step 不 finalize`）。实测 `chat_log.jsonl` 仅追加 user 行（line 11），无对应 assistant 行。本项只验证 SSE 事件序列，不验证持久化。

#### P0-4a：失败场景 — provider endpoint 不可达（transport error 透传）

> **澄清（审计 F2）**：§9 P0 第 4 项原列 5 类失败：无 API key / 模型不存在 / provider timeout / 401 bearer / SSE 中断。本项实测的是 **endpoint 不可达**（TCP connection refused），与 **provider timeout**（upstream 接受连接但不响应）属于不同 failure mode，但 engine 侧走同一 `Err` 分支（见下方断言）。**"无 API key"** 场景在本地 Tracy 代理下无法复现（代理不校验 key），需 P0-4e 单独覆盖，本 PR 不覆盖。

- engine 重启，改 endpoint 到无人监听端口，**并清掉 `AIRP_ACCESS_KEY` 以免 P0-4c 的鉴权配置泄漏进来**（审计 F9）：

  ```powershell
  Remove-Item Env:\AIRP_ACCESS_KEY -ErrorAction SilentlyContinue
  $env:AIRP_ENDPOINT = "http://127.0.0.1:9999/v1/chat/completions"
  $env:AIRP_MODEL = "gemini-3.1-pro-preview"
  $env:AIRP_DATA_DIR = "d:\AIRP-Dev\target\p0-final-smoke"
  d:\AIRP-Dev\target\debug\airp-core.exe daemon --port 8000
  ```

- 请求 `req1.json` → HTTP 200 `text/event-stream`，但首帧即 `event: error` `data: {"text":"\n[Error/网关错误]: 发送请求失败: error sending request for url (http://127.0.0.1:9999/v1/chat/completions)\n","type":"body_chunk"}`。
- 断言：engine 不返回硬 502 把浏览器 SSE 通道踢断，而是把上游连接错误包成 `event: error` + body_chunk 文本，让 WebUI 在 chat transcript 内可见错误。
- **覆盖范围声明（审计 F3）**：本项实测的是 connection refused（TCP RST）。`provider timeout` 在 engine 侧走同一错误事件路径——[chat_pipeline.rs](../engine/src/chat_pipeline.rs) 的 `Err(String)` 分支会把 transport/timeout 错误序列化为 `event: error`。**P0-4a 没有实测 timeout**，不能把 connection-refused 证据本身当成 timeout 实测。
  - **未实测声明（审计 H3）**：上述"transport error 与 timeout 走同一 Err 分支"是基于源码静态分析的结论，未在真实环境中实测。建议在 P2 阶段用 `nc -l 8889`（接受连接但不响应）或类似的 hung-server 桩验证 timeout 行为，确证 `Err(String)` 分支确实覆盖了 timeout 这一类错误。
- **后续状态更新**：上述“chat 无 per-request timeout”风险已由 PR #95 修复。[adapter.rs](../engine/src/adapter.rs) 现在用 `tokio::time::timeout` 保护等待响应头阶段，默认 30 秒且可通过 `AIRP_CHAT_REQUEST_TIMEOUT_MS` 调整；它不覆盖完整 streaming body，避免误杀长文本生成。

#### P0-4b：失败场景 — upstream 非 2xx 透传（实为 quota 429）

> **澄清（审计 F7）**：本项原标"未知 model"，但实际 upstream 响应是 `Cloud Error 429`（quota / rate limit），HTTP 500 是 Tracy 代理把上游 429 翻译后的结果。这并非"model not found"路径——`nonexistent-xyz-123` 因不在 provider 白名单内被 quota 拦截。但 engine 侧的透传行为对所有 non-2xx 一致（[chat_pipeline.rs](../engine/src/chat_pipeline.rs) 把 upstream status + body 包成 `event: error`），所以证据对"upstream 非 2xx 透传"这一类失败模式仍然成立。**真正的"未知 model"（provider 返回 4xx 提到 model name）未被本项覆盖**，留 P0-4f。

- engine 重启，恢复 provider 端点 8889，但把 model 改成不存在的 id：

  ```powershell
  $env:AIRP_ENDPOINT = "http://127.0.0.1:8889/v1/chat/completions"
  $env:AIRP_MODEL = "nonexistent-xyz-123"
  ```

- 请求 `req1.json` → HTTP 200 SSE，首帧 `event: error` `data: {"text":"\n[Error/网关错误]: API 返回错误状态码 500 Internal Server Error: {\"error\":{\"message\":\"Cloud Error 429\",\"type\":\"server_error\",\"code\":\"HTTP_500\"}}\n","type":"body_chunk"}`。
- 断言：上游非 2xx 时 engine 把 upstream status + body 透传到 `event: error`，浏览器可见 `upstream_status=500` 和原始 upstream_body，而不是吞错成空 200。

#### P0-4c：失败场景 — access key 已开但缺 Bearer

- engine 重启，恢复 provider + 真实 model，并启用 DX-2 鉴权中间件：

  ```powershell
  $env:AIRP_ENDPOINT = "http://127.0.0.1:8889/v1/chat/completions"
  $env:AIRP_MODEL = "gemini-3.1-pro-preview"
  $env:AIRP_ACCESS_KEY = "test-bearer-123"
  ```

- 三组对比请求：
  - 缺 Authorization 头：`HTTP/1.1 401 Unauthorized`，body `Unauthorized`。
  - 错误 Bearer `wrong-key-xxx`：`HTTP/1.1 401 Unauthorized`，body `Unauthorized`。
  - 正确 Bearer `test-bearer-123`：`HTTP/1.1 200 OK`，body `["p0-final"]`（`GET /v1/characters` 验证鉴权通过后 handler 正常返回）。
- 断言：`auth_middleware` 在 router 层生效（`route_layer(from_fn_with_state(state, auth_middleware))`），缺/错 token 直接返回 401 不进入 handler；constant_time_eq 防止 timing oracle（见 `engine/src/daemon/mod.rs:98-107`）；正确 token 透传到下游 handler 无副作用。
- 验证范围与 §11.1 测试套件 `test_dx2_no_key_all_pass / test_dx2_correct_key_passes / test_dx2_wrong_key_returns_401 / test_dx2_missing_header_returns_401` 一致，本轮在真实 engine 进程上复现。

#### P0-4d：失败场景 — 浏览器中断 SSE

- engine 恢复成功路径配置（无 `AIRP_ACCESS_KEY`），发起长回答请求 `long-req.json` = `"Count from 1 to 30, each on its own line with one sentence of explanation."`，curl 用 `--max-time 1` 强制 1s 后断流：

  ```
  curl.exe -s -N -X POST -H "Content-Type: application/json" \
    -d "@long-req.json" \
    "http://127.0.0.1:8000/v1/chat/completions" --max-time 1 -i
  ```

- 结果：curl 1s 后退出，exit code = 28（CURLE_OPERATION_TIMEDOUT）。仅收到响应头 `HTTP/1.1 200 OK` + `content-type: text/event-stream` + `transfer-encoding: chunked`，没有完整 chunk body（流被掐断在第一帧前/中）。
- engine 行为：进程未 panic、未挂起。8s 后 `GET /version` 仍返回 `200 OK {"name":"airp-core","version":"0.1.0"}`。engine 日志只有启动 INFO，没有 error 行。
- 断言：客户端断开 SSE 时 engine 的 stream task 收到 channel closed / hyper `Sender` 错误并自然结束；finalize 任务（如 chat_log 持久化）不会因为客户端断流而泄漏或半写损坏 `chat_log.jsonl`。后续 chat 请求仍可正常发起新 SSE。

#### 退出标准对照

| 退出条件（§7） | 本轮证据 |
| --- | --- |
| 不经过 Tauri 也能从浏览器复现后端 chat streaming | P0-5a 多帧 delta |
| data persistence 和 session/history 行为可观察 | P0-5b history(6) → regen(5) → rollback to 1(2) |
| 鉴权和错误行为可见 | P0-4a/4b/4c 三类失败路径 `event: error` / 401 |
| 并发 stream 不破坏 id-keyed chat state | P0-5c A="4" / B="4\n6" 不串扰 |
| agent run 计划/工具/增量/完成事件可见 | P0-5d plan/tool_call/tool_result/delta×3/done |
| 浏览器 SSE 断流不破坏 engine | P0-4d client abort 后 `/version` 仍 200 |

P0 §9 清单状态（按 [WEBUI-BACKEND-PLAN.md §9](WEBUI-BACKEND-PLAN.md#L242-L252) bullet 顺序，审计 F4 要求显式映射）：

- "开 webui-p0-usability 分支" — DONE（PR #38）
- "用 127.0.0.1:8889 配置" — DONE（PR #38）
- "修 #34 阻塞面" — DONE（PR #51）
- "WebUI 直接显示常见失败" — PARTIAL（P0-4a 覆盖 endpoint 不可达；P0-4b 覆盖 upstream 非 2xx 透传（实为 quota 429）；P0-4c 覆盖 401 bearer；P0-4d 覆盖 SSE 中断。**未覆盖**："无 API key" 留 P0-4e，"provider timeout" 由 P0-4a 源码分支同源外推、未实测，"未知 model（4xx 提到 model name）" 留 P0-4f）
- "WebUI 跑通各端点" — DONE（P0-5a/5b/5c/5d 全部真实 provider 证据回填，含真实 SSE 抓取）
- "#30 agent 事件序列可复现" — PARTIAL（P0-5d 的真实 delta 已覆盖主路径；#53 越界补的 integration test 属于 P1 范畴，不计入 P0）
- "写回 VALIDATION.md" — DONE（本文件）
