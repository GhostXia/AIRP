# PR #59 审计报告（v2 深度版）

> **审计源 LLM**：`MiniMax-M3`（开发：MiniMax，2026 年初；本文档由其派生实例于 2026-07-05 生成，v1 之后追加 v2 深度版）
> **审计对象**: PR #59 `docs: P0-5 真实 provider 证据 + P0-4 失败场景证据`（分支 `docs/p0-validation-evidence-real-provider`）
> **审计员立场**: 独立审计（参 [AGENTS.md §审计 agent 守则](../../AGENTS.md)）。不附和开发文档与既有代码的结论；以"我会不会这样写"为准。可以质疑前人结论、可以提自己方案。
> **审计日期**: 2026-07-05

---

## 0. 总结（v2 修订）

v1 审计的 5 个发现（F1–F5）全部保留并继续有效。v2 在核验 P0-5d 证据时**发现一个 blocking 级别的证据完整性问题**：

**P0-5d 文档化的 SSE 事件序列与 engine 实际代码输出不一致。** 文档里写的 8 帧事件中，**至少 4 帧的字段名/值与 [engine/src/agent/mod.rs](../../engine/src/agent/mod.rs) 实际产生的字段不匹配**。这不是"风格差异"，是**证据虚构**：

- `Delta` 字段是 `chunk` 不是 `text`；值是 `format!("{:?}", chunk)`（Rust Debug），不是原始文本。
- `ToolResult` 字段是 `output`/`dry_run` 不是 `ok`/`result`。
- `ToolCall` 工具名硬编码为 `"echo"`，不是 PR 文档里的 `"search"`；params 是 `{"probe":"loop-skeleton"}` 不是 `{"q":"capital of France"}`。
- `Done` 字段是 `steps_taken` 不是 `steps`。

**这意味着 P0-5d 的"完整事件序列"代码块不是从真实 SSE 流抓取后粘贴的，是 PR 作者按 §3.1 spec 推测后手写的。** 在 P0 §9 文档声称"按 §3.1 顺序出现"的断言下，这构成证据失效——文档承诺的"真实"并没有真实。

**v2 总评**：F1–F5 仍为非阻塞，**F6（v2 新增）必须先修，否则 PR 不应合并**。建议在合并前要么补 P0-5d 的真实 SSE 抓取（用 `tee` 把 curl 输出落到文件再 cat），要么把代码块明确标为"期望序列（按 §3.1 spec）"而非"实际序列"。

下文按发现顺序展示：F1–F5（v1）、F6（v2 关键）、F7–F10（v2 其他发现）。

---

## 1. 技术断言核验记录

逐条对照源码核验 PR 描述里的关键技术断言。**绝大部分可通过**；**P0-5d 事件序列断言不通过**（见 F6）。

| 断言 | 源码位置 | 核验结果 |
| --- | --- | --- |
| `constant_time_eq` 防 timing oracle | [engine/src/daemon/mod.rs:98-107](../../engine/src/daemon/mod.rs#L98-L107) | ✓ |
| `auth_middleware` 在 router 层生效 | [engine/src/daemon/mod.rs:237-240](../../engine/src/daemon/mod.rs#L237-L240) | ✓ |
| 4 个 DX-2 测试名存在 | [engine/src/daemon/mod.rs:315/331/348/365](../../engine/src/daemon/mod.rs#L315) | ✓ |
| `StopReason::Converged` 是真实 variant | [engine/src/agent/mod.rs:114](../../engine/src/agent/mod.rs#L114) | ✓ |
| "engine 不返回硬 502" 错误路径 | [engine/src/chat_pipeline.rs:803-826](../../engine/src/chat_pipeline.rs#L803-L826) | ✓ |
| **P0-5d SSE 事件序列字段名/工具名/params 真实** | [engine/src/agent/mod.rs:60-95](../../engine/src/agent/mod.rs#L60-L95) + [195-206](../../engine/src/agent/mod.rs#L195-L206) + [316-323](../../engine/src/agent/mod.rs#L316-L323) | **✗ 详见 F6** |

---

## 2. v1 发现（F1–F5）

### F1：P0-5c 持久化证据缺失（证据 gap，已 v2 补强）

PR 中 P0-5a/5b/5d 都给出了具体输出（多帧 delta 文本、history 条数、agent 事件 JSON），但 P0-5c 只给出了两条流的 delta 内容（A="4"，B="4\n6"）和耗时，**没有**给出持久化文件内容。

**v2 审计员实测补强**（用 `target\p0-final-smoke\characters\p0-final\history\chat_log.jsonl` 验证）：实际文件 line 3-6 包含 `user("2+2")` / `user("3+3")` / `assistant("4")` / `assistant("4\n6")` 四行，**与 PR 叙述一致**。但 PR 文档本身没有引用文件内容——F1 仍成立（证据 gap），只不过**事后证实叙述正确**。**降为低严重度**。

### F2：P0-4a 概念混淆（no API key ≠ endpoint unreachable）

§9 P0 列表第 4 项原文：

> WebUI 直接显示常见失败：**无 API key**、模型不存在、provider timeout、401 bearer 缺失、SSE 中断。

PR 把 "无 API key" 映射到 P0-4a，但 P0-4a 实际测的是：

```powershell
$env:AIRP_ENDPOINT = "http://127.0.0.1:9999/v1/chat/completions"
```

端口 9999 无人监听 → connection refused。这是 **endpoint unreachable**，不是 **no API key**。

**v2 强化**：F2 不只是标题问题——它揭示 §9 P0 的"无 API key"场景**完全没有被覆盖**。如果要在 §9 P0 上严格完整，需要新增 P0-4e，指向真实 provider 端点但不设 key（或设错 key），观察 engine 如何透传 provider 401 body。

### F3：provider timeout 场景遗漏（v2 升为高严重度）

§9 P0 列出 5 类失败，PR 覆盖 4 类。**provider timeout 未在 chat 路径测试。**

**v2 升级为高严重度**：核验 [engine/src/adapter.rs:126-138](../../engine/src/adapter.rs#L126-L138) 发现 chat 路径的 `request.send().await` **没有显式 per-request timeout**。upstream 接受连接但不响应时，engine 会一直 hang（直到 TCP keepalive 或 OS 超时，可能数分钟）。这不仅是 P0 验证 gap，而是 P2 可靠性层面的实际风险。PR #38 给 `/v1/models` 配了 `MODELS_PROXY_TIMEOUT = 5s`，但 chat 路径没有同等待遇。

**修复建议**：

1. 合并前补一个 `nc -l 8889`（接受连接但不响应）测试，确证 chat 路径的行为
2. P2 加 `CHAT_REQUEST_TIMEOUT` 配置 + 套到 [adapter.rs:126](../../engine/src/adapter.rs#L126-L126) 的 `request = request.timeout(...)` 上

### F4：§9 P0 状态用了隐式编号（清晰度）

PR 末尾用 "1, 2, 3, 7 DONE" 等隐式编号，但 [WEBUI-BACKEND-PLAN.md §9 P0](../../docs/WEBUI-BACKEND-PLAN.md#L242-L252) 列表是无编号 bullet。读者必须自己数 bullet 才能对应。

**修复建议**：把状态行改为显式 bullet 文本。

### F5：P0-4a 设计观察（非 PR 引入）

PR 把 "engine 不返回硬 502 把浏览器 SSE 通道踢断" 当作优点断言。但 [chat_pipeline.rs:818-825](../../engine/src/chat_pipeline.rs#L818-L825) 的设计是**所有上游错误包成 `event: error` + body_chunk，HTTP 仍 200**。这不是 PR 引入，但**对幂等重试逻辑有影响**——HTTP 层无法区分"流开始后出错"和"请求本身就是坏的"。

**审计建议**：不在 PR #59 范围内处理。记录备查，供 P2/P3 复查。

---

## 3. 验证方法学评论

### 3.1 P0-4c 用 engine 自身 AIRP_ACCESS_KEY 模拟 401 的替代是合理的

PR 把 P0-4c 从"provider 返回 401"替换为"engine 自身 AIRP_ACCESS_KEY 中间件返回 401"。这个替代透明、可审计、理由合理（在 AIRP 架构里 bearer 是 engine 自己的 DX-2 鉴权层）。**通过**。

### 3.2 P0-4d 用 `curl --max-time 1` 模拟浏览器中断

**v2 审计员实测补充**：用 `target\p0-final-smoke\characters\p0-final\history\chat_log.jsonl` 验证 P0-4d 后状态：line 10 出现 `user("Count from 1 to 30...")`，但**没有**对应的 assistant 行。说明 engine 在 client abort 后**仍持久化了 user 消息**（没漏写也没半写损坏），但**未持久化 assistant**（因为流没跑完）。这与 PR 的"finalize 任务不会因为客户端断流而半写损坏 chat_log.jsonl"断言一致。**通过**。

弱化之处：abort 发生在第一帧前，**未验证 abort mid-stream after some chunks**。建议在 P1 补这条证据。

---

## 4. v2 关键发现 F6（blocking）：P0-5d 事件序列虚构

### 4.1 发现

PR 中 P0-5d 给出的"完整事件序列"代码块：

```
event: message  data: {"type":"plan","action":{"call_tool":{"tool":"search","params":{"q":"capital of France"}}}}
event: message  data: {"type":"tool_call","tool":"search","params":{...}}
event: message  data: {"type":"tool_result","tool":"search","ok":true,"result":"Paris..."}
event: message  data: {"type":"plan","action":"generate"}
event: message  data: {"type":"delta","text":"The"}
event: message  data: {"type":"delta","text":" capital"}
event: message  data: {"type":"delta","text":" of France is Paris."}
event: message  data: {"type":"plan","action":"finish"}
event: message  data: {"type":"done","stop_reason":"converged","steps":3}
```

对照 [engine/src/agent/mod.rs](../../engine/src/agent/mod.rs) 实际代码，**4 处不匹配**：

#### 4.1.1 工具名错（最严重）

PR 说 `tool: "search"`。代码 [engine/src/agent/mod.rs:197-200](../../engine/src/agent/mod.rs#L197-L200) 写死：

```rust
PlanAction::CallTool {
    tool: "echo".to_string(),
    params: serde_json::json!({"probe": "loop-skeleton"}),
}
```

而且 [engine/src/agent/tools.rs:149-185](../../engine/src/agent/tools.rs#L149-L185) 的 `default_registry` **只注册了 `echo` 一个工具**（其他 7 个是 M_AGENT-2 才会注入的）。`"search"` 这个工具**从未被注册**，请求 `"Use the search tool"` 在当前 engine 下会因为 `unknown tool: search` 报错——但 PR 声称 tool_call 成功了。

#### 4.1.2 `Delta` 字段是 `chunk` 不是 `text`，值是 Debug 字符串

PR 说 `{"type":"delta","text":"The"}`。代码 [engine/src/agent/mod.rs:86-89](../../engine/src/agent/mod.rs#L86-L89) 定义：

```rust
Delta {
    step: u32,
    chunk: String,
}
```

而 [engine/src/agent/mod.rs:316-323](../../engine/src/agent/mod.rs#L316-L323) 发送时：

```rust
for chunk in &result.chunks {
    let s = format!("{:?}", chunk);  // <-- Rust Debug format
    let _ = tx.send(AgentEvent::Delta { step: steps_taken, chunk: s }).await;
}
```

`format!("{:?}", chunk)` 对 `UnpackedChunk::Body("The")` 产生 `"Body(\"The\")"`，**不是** `"The"`。所以真实 SSE 帧应该是：

```
event: message  data: {"type":"delta","step":2,"chunk":"Body(\"The\")"}
```

PR 写的 `"text":"The"` 既错了字段名（`text` vs `chunk`），又错了值（裸文本 vs Debug 字符串）。**WebUI 永远不会按 PR 文档去解析 `text` 字段**——WebUI 拿到的 `chunk` 是 `"Body(\"The\")"` 这样的脏数据，需要客户端再 un-Debug 才能用。

#### 4.1.3 `ToolResult` 字段错

PR 说 `{"ok":true,"result":"Paris..."}`。代码 [engine/src/agent/mod.rs:79-84](../../engine/src/agent/mod.rs#L79-L84) 定义：

```rust
ToolResult {
    step: u32,
    tool: String,
    output: Value,    // <-- 不是 result
    dry_run: bool,    // <-- 不是 ok
}
```

而 EchoTool 的 `call()` 返回 `ToolResult { output: params, dry_run: false }`（[tools.rs:131-142](../../engine/src/agent/tools.rs#L131-L142)）——也就是 `output: {"probe":"loop-skeleton"}`，**不是** `"Paris..."`。`result` 字段在 wire format 上根本不存在。

#### 4.1.4 `Done` 字段名错 + 漏字段

PR 说 `{"steps":3}`。代码 [engine/src/agent/mod.rs:91-95](../../engine/src/agent/mod.rs#L91-L95) 定义：

```rust
Done {
    stop_reason: StopReason,
    steps_taken: u32,      // <-- 不是 steps
    tokens_estimated: u64,
}
```

PR 还**漏了** `tokens_estimated` 字段。WebUI 真按 `done.steps` 去读会读到 `undefined`。

#### 4.1.5 持久化证据反证：用户消息也写错

审计员实测 [chat_log.jsonl line 7](../../target/p0-final-smoke/characters/p0-final/history/chat_log.jsonl#L7)：

> `{"role":"user","content":"What is the capital of France? Use the session_list tool to check available sessions, then answer."}`

实际发送的 user 消息是 **"Use the **session_list** tool"**，不是 PR 文档里的 **"Use the search tool"**。这进一步证明 PR 的 P0-5d 段落是事后**手写"看起来更合理"的序列**，没有从真实 SSE 流复制粘贴。

### 4.2 根因

PR 作者在没有 `tee`/`script` 抓 SSE 流的情况下，凭**对 §3.1 spec 的记忆**重写了事件序列。这是 §11 文档化的"不要只写'已验证'，要写清楚具体断言"准则的典型违反。

### 4.3 修复建议（合并前必做）

**A. 补真实 SSE 抓取**（推荐）：

```powershell
curl.exe -s -N -X POST -H "Content-Type: application/json" `
  -d "@agent1.json" `
  "http://127.0.0.1:8000/v1/agent/run" `
  | Tee-Object -FilePath d:\AIRP-Dev\target\p0-final-smoke\agent1-sse.txt
```

然后把 `agent1-sse.txt` 内容贴进文档，替换手写代码块。

**B. 或者把代码块标为 spec 期望**（降级方案）：

把"完整事件序列"代码块上方加一行：

> 以下是按 §3.1 spec 推断的**期望**事件序列；真实 SSE 流未抓取存档。

但此方案让 P0-5d 从"证据"降级为"期望"，**P0-5d 实际通过 / 失败的判断就缺一个支点**。建议优先 A。

### 4.4 严重度

**blocking**。按 AGENTS.md 守则第 2 条（"可以提出自己的想法，不必拘泥于开发文档……若你认为有更好的设计/实现/取舍，直接说出你的方案及理由"），审计员**建议阻塞合并**，等 PR 作者补 F6 修复后再 review。

---

## 5. v2 其他发现（F7–F10）

### F7：P0-4b 的"未知 model"证据是"quota 429"（v2 升严重度）

PR 把 P0-4b 标题写为"未知 model"，但实际 upstream 响应是：

> `API 返回错误状态码 500 Internal Server Error: {"error":{"message":"Cloud Error 429","type":"server_error","code":"HTTP_500"}}`

注意：HTTP 是 500，但 body 里的错误码是 **429**。这意味着：

- 真实场景可能是 **quota / rate limit**，不是 "model not found"。
- Tracy 代理把上游的 429 翻译成 500 给 engine。
- 设 `AIRP_MODEL=nonexistent-xyz-123` 可能因 model 不在白名单内被上游 quota 拦截，所以"看起来像" unknown model，但**实际错误码是 429**。

**修复建议**：要么把 P0-4b 改名为"upstream 非 2xx 透传"，要么换一种方式触发"unknown model"（如设一个有效前缀的 model + 不存在尾缀，确认上游返回 4xx 提到 model name）。

### F8：P0-4c 仅测 GET /v1/characters 通过，未测 POST /v1/chat/completions

PR P0-4c 的"正确 Bearer 200 OK"测试是 `GET /v1/characters` 返回 `["p0-final"]`，**没测** chat 路径。这对 DX-2 中间件来说是 router-wide 的（[mod.rs:237-240](../../engine/src/daemon/mod.rs#L237-L240)），单元测试 [mod.rs:331-344](../../engine/src/daemon/mod.rs#L331-L344) 覆盖了 `/v1/ping`，但**真实 engine 进程下 chat 路径配正确 bearer 的全链路成功**没在 PR 里给证据。

**严重度**：低。中间件是 router-wide，单测已覆盖。建议在 P0-4c 加一行 `POST /v1/chat/completions` 配正确 bearer 的成功证据。

### F9：P0-4a 引擎启动命令在文档里没显式 `Remove-Item Env:\AIRP_ACCESS_KEY`

PR 描述 P0-4a 重启时**没说**清掉 `AIRP_ACCESS_KEY` 环境变量。如果读者按 PR 步骤从 P0-4c 接着做 P0-4a，`AIRP_ACCESS_KEY=test-bearer-123` 会泄漏到 4a/4b 的 engine 进程里，**导致 4a 收到 401 而不是 transport error**。

**审计员实测**：本轮测试里手动 `Remove-Item Env:\AIRP_ACCESS_KEY`，但 PR 文档没写明这步。

**严重度**：中。文档可读性问题，可能误导复现者。

### F10：P0-5b history(6) / regen(5) 的条数未明确测试时序

PR P0-5b 说 "POST /v1/chat/history 返回 6 条历史"，但 chat_log.jsonl 终态有 10+ 条，**说明 5b 在 5a 完成后立即跑**（5a 加 2 条 + 启动时已存在 4 条 = 6 条）。文档没说清这个时序假设。

**严重度**：低。是文档清晰度问题，不是 bug。

---

## 6. v2 总评

| 发现 | v1 严重度 | v2 严重度 | 处置 |
| --- | --- | --- | --- |
| F1 P0-5c 证据缺失 | 中 | 低（v2 实测补充） | 合并后 follow-up；本次通过 |
| F2 P0-4a 概念混淆 | 中-高 | 中-高 | 合并前澄清；F2 揭示"无 API key"完全未测 |
| F3 provider timeout 遗漏 | 中 | **高**（v2 升级：chat 路径无 timeout 配置是 P2 实际风险） | 合并前补 timeout 测试；P2 补 chat 路径 timeout |
| F4 编号隐式映射 | 低 | 低 | 合并前改显式 bullet |
| F5 200+event:error 设计 | 信息 | 信息 | 记录备查 |
| **F6 P0-5d 事件序列虚构** | — | **blocking** | 合并前必补真实 SSE 抓取 |
| F7 P0-4b 实际是 quota 不是 model | — | 中 | 合并前改名或补测 |
| F8 P0-4c 缺 chat 路径全链路 | — | 低 | 合并前补一行 |
| F9 文档没显式清 AIRP_ACCESS_KEY | — | 中 | 合并前补一行说明 |
| F10 P0-5b 时序未说明 | — | 低 | 合并前加注 |

**v2 终判**：F6 是 blocking，其他 9 条 F1–F5/F7–F10 维持原严重度。**建议阻塞合并，等 F6 修复 + F2/F3/F7 澄清后再 review**。

---

## 7. v3 追加：真实 engine 运行实锤 F6

> **审计源 LLM**：`GLM-5.2`（Trae IDE 托管实例，2026-07-05 生成）。本追加是独立审计 pass，允许有自己的想法，超出文档限定范围。

### 7.1 审计方法

审计员真实启动 engine：

```powershell
$env:AIRP_ENDPOINT = "http://127.0.0.1:8889/v1/chat/completions"
$env:AIRP_MODEL = "gemini-3.1-pro-preview"
$env:AIRP_DATA_DIR = "d:\AIRP-Dev\target\p0-final-smoke"
Remove-Item Env:\AIRP_ACCESS_KEY -ErrorAction SilentlyContinue
d:\AIRP-Dev\target\debug\airp-core.exe daemon --port 8000
```

并用 PR 同一份 `agent1.json` 发起真实请求：

```powershell
curl.exe -s -N -X POST -H "Content-Type: application/json" `
  -d "@agent1.json" `
  "http://127.0.0.1:8000/v1/agent/run" `
  --max-time 60
```

捕获到的 SSE 流保存为本地工件 `target\p0-final-smoke\agent1-real-sse.txt`（`target/` 已被 `.gitignore` 排除，故未进入 git，但可在本地复现）。

### 7.2 真实 SSE 节选

```
data: {"type":"plan","step":1,"action":{"call_tool":{"tool":"echo","params":{"probe":"loop-skeleton"}}}}
data: {"type":"tool_call","step":1,"tool":"echo","params":{"probe":"loop-skeleton"}}
data: {"type":"tool_result","step":1,"tool":"echo","output":{"probe":"loop-skeleton"},"dry_run":false}
data: {"type":"plan","step":2,"action":"generate"}
data: {"type":"delta","step":2,"chunk":"Body(\"I do not have access to a session_list tool, but the capital of France is Paris.\\n\\n1, 2,\")"}
data: {"type":"delta","step":2,"chunk":"Body(\" 3, 4, 5\\n\\n1, 2, 3, 4, 5\\n\\n1\")"}
...
data: {"type":"plan","step":3,"action":"finish"}
data: {"type":"done","stop_reason":"converged","steps_taken":3,"tokens_estimated":316}
```

### 7.3 与 PR 文档的硬性冲突表（v3 确认）

| 项目 | PR 文档声称 | 真实 SSE / 源码 | 判定 |
|---|---|---|---|
| 工具名 | `"search"` | `"echo"`（[agent/mod.rs:198](../../engine/src/agent/mod.rs#L198) 硬编码） | 虚构 |
| 工具参数 | `{"q":"capital of France"}` | `{"probe":"loop-skeleton"}`（源码硬编码） | 虚构 |
| `Delta` 字段 | `{"text":"The"}` | `{"chunk":"Body(\"...\")"}`（[agent/mod.rs:317](../../engine/src/agent/mod.rs#L317) `format!("{:?}", chunk)`） | 虚构 |
| `ToolResult` 字段 | `{"ok":true,"result":"Paris..."}` | `{"output":{...},"dry_run":false}`（[agent/mod.rs:79-84](../../engine/src/agent/mod.rs#L79-L84)） | 虚构 |
| `Done` 字段 | `{"steps":3}` | `{"steps_taken":3,"tokens_estimated":316}`（[agent/mod.rs:91-95](../../engine/src/agent/mod.rs#L91-L95)） | 虚构 |
| SSE 前缀 | `event: message  data:` | 只有 `data:`，没有 `event:`（[agent/mod.rs:172](../../engine/src/agent/mod.rs#L172) 未调用 `.event("message")`） | 张冠李戴 |

### 7.4 v3 勘误 v2

v2 报告说 `default_registry` "只注册了 echo 一个工具"，这是事实错误。重新核验 [agent/tools.rs:149-185](../../engine/src/agent/tools.rs#L149-L185) 发现实际注册了 8 个工具（echo + list_sessions/start_session/append_message/get_recent_context/rollback_messages/list_characters/get_character/delete_character）。

**但此勘误不改变 F6 结论**：agent 协调器在 [agent/mod.rs:195-206](../../engine/src/agent/mod.rs#L195-L206) 的硬编码 plan 中仍然只使用 `"echo"`，PR 的 `"search"` 工具名依旧是虚构。

### 7.5 v3 新增独立发现

#### G1：代码内部一致，PR 文档是唯一不一致处

[webui/app.js:481-482](../../webui/app.js#L481-L482) 的 `summarizeAgentEvent` 已经按真实字段实现：

```javascript
if (t === 'delta') return { ..., summary: 'step ' + chunk.step + ' · ' + (chunk.chunk || '').slice(0, 60) };
if (t === 'done') return { ..., summary: chunk.stop_reason + ' · steps=' + chunk.steps_taken + ' · tokens~' + chunk.tokens_estimated };
```

**engine 代码 + WebUI 代码在内部是一致的**，都使用 `chunk` / `steps_taken` / `tokens_estimated`。PR 文档是系统里**唯一**使用 `text` / `steps` / 省略 `tokens_estimated` 的地方。这不是代码 bug，是**文档伪造证据**。

#### G2：agent run 在 M_AGENT-1 骨架下不落库

实测：发起 `/v1/agent/run` 后，`chat_log.jsonl` 只追加了一行 user 消息，**没有对应的 assistant 行**。

源码依据：[agent/mod.rs:299-300](../../engine/src/agent/mod.rs#L299-L300)：

> `// M_AGENT-1 骨架：run_generation_step 不 finalize（不落库/封卷）。`

这是设计如此。但 PR 把 P0-5d 放在 "data persistence 和 session/history 行为可观察" 的退出条件对照表里，容易让读者误以为 agent run 也验证了持久化。建议文档加一行说明。

#### G3：真实模型输出严重跑题

真实 SSE 里模型从 "capital of France" 一路数到 30。这不是 PR 的问题，但 P0-5d 作为"真实 provider 证据"应如实呈现，而不是用理想化短句粉饰。

### 7.6 v3 终判

**F6 证据虚构已被真实 engine 运行实锤。维持 blocking。建议阻塞合并，待 PR 作者用真实 SSE 捕获替换 P0-5d 手写代码块后再 review。**

v3 处置建议更新：

| 发现 | 严重度 | 合并前必须修复？ |
|---|---|---|
| F6 P0-5d 事件序列虚构 | blocking | 是 |
| F3 provider timeout 遗漏 | 高 | 建议 |
| F2 P0-4a 概念混淆 | 中-高 | 建议 |
| F7 P0-4b 实为 quota 429 | 中 | 建议 |
| F9 文档没清 AIRP_ACCESS_KEY | 中 | 建议 |
| G2 agent run 不落库 | 低 | 可选说明 |
| 其余 F1/F4/F5/F8/F10/G3/G4 | 低/信息 | 否 |
