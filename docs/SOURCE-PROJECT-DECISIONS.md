# 源项目资产吸收决策

> 状态：已接受
> 日期：2026-07-03

## 总原则

四个 AIRP 源项目都按同一原则处理：

> 吸收资产，不继承产品北极星。

AIRP-Dev 的北极星只有一个：专精 RP 的 AI Agent 客户端，形态是无头 engine + 可换 UI。四个源项目的代码、文档、戒律、路线图、模块边界都是高价值素材，但不自动成为 AIRP-Dev 的路线约束。

采纳时按四个问题审查：

1. 它服务 AIRP 当前产品闭环吗？
2. 它能让代码更开放、更透明、更易修正、更易迭代吗？
3. 它是否会引入平行真相源、额外网络跳或过早标准化？
4. 它是否把源项目为独立分发而设的自我克制带进了 AIRP 主线？

## 1. AIRP-Core / AIRPCLI

审查对象：`D:\AIRPCLI\README.md`、`AGENT_BACKEND_PLAN.md`、`AGENT_CLIENT_ASSESSMENT.md`。

### 吸收资产

- 双 provider 流式 adapter：OpenAI 兼容 SSE + Anthropic Messages SSE。
- `chat_pipeline` 三段式：prepare -> stream -> finalize。
- orchestrator 装配：card/preset/lorebook/state/memory/history。
- 干净提示词纪律：角色平面与控制平面物理隔离。
- bounded agent loop 骨架：step/token/cost/wall-clock/cancel 闸。
- `subagent_context_has_no_orchestrator_noise` 不变式。
- daemon HTTP/SSE API、settings 热重载、模型列表、历史/回滚/regen。
- 数据层基础：chat JSONL、volume、scene、gating、state、newtype ID。
- 正确 PNG 角色卡解析。
- FSM/XML 流式过滤与拆包。

### 不继承的北极星

- 不继承“独立、开源、乐高式 Agent 后端”作为产品目标。
- 不继承“Core 必须 standalone、兄弟仓都只是可选”的产品叙事。
- 不继承“Core 是生态参考大脑”的外部分发定位。
- 不继承把 State-Protocol 输出留给 Gateway 适配的边界；AIRP-Dev 的 UI 可直接消费 engine。

### AIRP-Dev 定位

Core 是 AIRP engine 的主核，不是并列外部项目。它的好能力应原生进入 `engine/`，由 AIRP 产品闭环约束。

## 2. AIRP-MCP-Server

审查对象：`D:\airp-mcp-server\SKILL.md`、`README.md`、`docs\ROADMAP.md`、`docs\skills-vs-mcp.md`。

### 吸收资产

- 38 个工具的能力目录，作为 engine 内置工具规格。
- 19 个 `airp://` 资源形态，作为 engine 内部资源/API 参考。
- 12 个工作流提示词，转成 engine 内置技能/工作流指南。
- RP 数据模型：characters/sessions/presets/scenes/plugins/state/memory/gating。
- 路径沙箱：`safe_resolve_for_write`、`validate_id_segment`。
- 插件零 schema 数据面：KV、JSONL、blob。
- `png_path` 大文件传输纪律。
- `export_context_bundle` 与隔离 subagent 写 RP 的实践。
- `validate_card` / `validate_preset` 的未知内容审查思路。
- skill 与 MCP 的静态/动态分工判断。

### 不继承的北极星

- 不继承“纯 MCP 数据层，不调 LLM，不做推理”的产品定位。
- 不继承“决策完全下放外部 Agent”的边界。
- 不继承“standalone MCP server 任取子集使用”的分发叙事。
- 不继承“通用优先于特供”到牺牲 AIRP RP 产品闭环的程度。

### AIRP-Dev 定位

MCP-Server 是 engine 数据层、内置工具和工作流技能的规格来源。自有 RP 数据能力应内化进 engine，不作为外部 MCP 后端多跳调用。MCP 生态仍保留给第三方工具接入。

## 3. AIRP-Gateway

审查对象：`D:\AIRP-Gateway\README.md`、`docs\DESIGN.md`、`docs\AGENTBUS-ADAPTER.md`、`docs\ROADMAP.md`。

### 吸收资产

- MCP client 能力：stdio、streamable HTTP、initialize、版本协商、连接池。
- 前端服务层硬化：bearer 鉴权、CORS、限流、请求/响应体积上限。
- 上游安全：SSRF 防护、stdio 命令白名单、args 校验、错误脱敏。
- 健壮性：超时、优雅关机、EOF drain、构建回滚、故障注入测试思路。
- RouteRule/Bridge 的声明式分发思想，在需要外部路由时参考。
- AgentBus SSE 适配中的连接关联、dispatch/stream 分离、scope 过滤。
- 自给自足 e2e 与 mock upstream 的验证策略。

### 不继承的北极星

- 不继承“纯协议桥，不懂业务”的产品定位。
- 不继承“任意前端 -> 任意 MCP 服务”的通用桥目标。
- 不继承“库优先、无独立 exe”的约束作为 AIRP 主线。
- 不继承“业务逻辑全部归上游 MCP 服务”的边界。
- 不继承 Gateway 的 `chat.send -> MCP 数据工具` 方向；AIRP 的 `chat.send` 必须进入 engine 推理。

### AIRP-Dev 定位

Gateway 是传输、安全、MCP-client、SSE 适配和验证策略的资产来源。AIRP-Dev 不需要一条独立 Gateway 网络跳作为默认路径；UI 默认直连 AIRP engine。

## 4. AIRP-State-Protocol

审查对象：`D:\AIRP-State-Protocol\README.md`、`docs\PLAN.md`、`docs\extension-points.md`、`docs\SECURITY.md`，细节见 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)。

### 吸收资产

- Blueprint 声明式 UI。
- Widget Registry / WidgetHost / manifest。
- RFC6902 state patch store。
- Envelope/Body 线协议类型。
- TypeScript/Rust 绑定思路。
- guard、错误隔离、虚拟滚动、consent、sandbox。
- Tauri + Vue 桌面壳和打包经验。
- “不运行 agent 生成前端代码”的安全边界。

### 不继承的北极星

- 不继承“通用 Agent UI 标准”作为当前目标。
- 不继承“协议是核心资产”的产品定位。
- 不继承“乐高，不是套件”的默认路径设计。
- 不继承第三方 widget 市场优先的路线。
- 不继承 MockBus/demo-first、真实运行时验证后置的工程取舍。

### AIRP-Dev 定位

State-Protocol 是 AIRP UI 与线协议资产库。Blueprint/Widget 必须吸收，但服从 AIRP RP 产品闭环。

## 落地规则

1. 文档引用源项目时，默认称为“资产来源”或“零件来源”，不要称为 AIRP-Dev 的产品目标。
2. 若某源项目戒律与 AIRP 工作流冲突，以 AIRP 工作流为准。
3. 若某资产需要引入平行服务、平行数据真相或额外网络跳，先尝试内化为 engine/UI 原生能力。
4. 若某资产是为了源项目独立分发而存在的约束，移植前必须重新审查。
5. 任何吸收都要保持代码取向：更开放、更透明、更易修正、更易迭代。
