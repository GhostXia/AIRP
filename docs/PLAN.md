# AIRP 客户端 —— 设计计划

> 状态：长期产品原则与目标架构；当前事实见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)，近期执行入口见 [DEV-GUIDE.md](DEV-GUIDE.md)。
> 最后更新：2026-07-14
> **权威 = 我们这个客户端的实际需求。** 四个原仓库的文档与代码都**仅供参考**——是作者已想清的宝贵先例/解法，但不是必须遵守的法律。它们的理念、戒律、模块边界、ADR、路线图**均为参考**，与我们实际需求冲突时以需求为准。本 PLAN 的每个决策先问"我们的客户端需要什么"，再问"哪个仓库有可借鉴的现成解法"，**绝不问"文档规定了什么"**。

## 当前执行方向（2026-07-14：P0 已完成，推进 WebUI P1）

PR #118、#119、#121、#123 已完成基础 WebUI RP 闭环；PR #124/#125 已完成 durable history 与窗口化，PR #127 已建立多 Persona 存储地基；PR #132/#133/#135/#136 已完成 P0 production mode、首方部署 artifact 与真实 HTTPS topology gate。当前目标仍是：**让 WebUI 能够作为单实例、自托管、单用户产品正式上线使用**；当前执行点已经从 P0 转入 #114/#115/#126 的 P1 RP 正式使用面。详细 release gates 与阶段顺序见 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)。

1. **生产地基已完成**：同源 HTTPS 入口、私有 engine、强制生产 access key、remote import policy、首方部署产物与 production smoke 已由 P0 交付；
2. **闭合正式 RP 使用面**：按 #114/#115/#126 完成 Persona/Preset/Worldbook 管理、绑定、迁移报告、有效配置与 PromptAssemblyTrace 摘要；
3. **数据可靠性**：补版本化 migration、备份/恢复、可恢复删除、readiness、脱敏日志和运维 runbook；当前 Caddy access log 只算 P0 的局部实现，不代表 P2 可观测性合同已完成；
4. **发布门禁**：覆盖浏览器矩阵、安全负向、旧数据升级、备份恢复、长会话 soak、artifact 与回滚演练；P0 已有的 system-Chrome smoke 不等于完整 P3；
5. **后移非首发能力**：#117 ChangeInbox、#87 Agent-first 工作台、#116 Style Review 不再抢占 WebUI 上线主链；
6. **纵向交付纪律**：每项能力贯通 engine shared service → HTTP/SSE → WebUI → production tests，验收以持久化、恢复和真实部署证据为准。
7. **桌面计划暂停**：保留 `ui/`、客户端无关合同与既有 issue，但近期不投入 Tauri UI 功能、打包或性能验收；只有用户重新排定优先级后才恢复。

Agent 调度采用**可插拔编排策略**，不把某个“强模型规划 + 固定数量低成本 executor”配方写死为产品唯一方案。系统提供有界执行、capability、trace、validator 和升级闸门；用户可以编排自己的角色、模型/provider、任务图、并发、预算、验收、仲裁与人工节点。完整原则见 [AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md)。

这不改变“两盒”“干净提示词”“数据单一真相”或客户端无关合同等既定原则。WebUI 当前是正式产品交付主面；Tauri/Vue 代码作为后续桌面客户端资产保留，但开发计划暂时搁置。已完成的基础可用验收已压缩进 [WebUI 历史归档](archive/WEBUI-HISTORY-2026-07.md)。

## 0. 背景与定位

- **目标**：开发**专为 AIRP 设计的全新 AI Agent 客户端**做 Role Play，替代 SillyTavern（"酒馆"）。酒馆功能全，但架构老、跟不上 AI Agent 时代——**重写 > 移植整个酒馆**。
- **本质定性（用户 2026-07-01 点破）——我们做的就是一个"专精 RP 的 AI Agent 框架"（Hermes 级），不是"带 agent 功能的 RP 客户端"。** 主次顺序：
  - **内核 = 通用 Agent 框架**：bounded loop + 工具 + MCP client + 记忆/技能（自进化）+ 子agent + 扩展钩子 + 无头服务。（≈ Hermes/Claude Code 那一类）
  - **RP = 领域特化**：叠在框架上——RP 数据层（卡/世界书/state/场景）+ **干净提示词纪律**（区别通用 agent 的灵魂，§1）+ 酒馆格式导入 + RP 味 UI。
- **🎯 当前首要目标（用户 2026-07-13 定，覆盖此前“先能运行”的阶段目标）**：**WebUI 能够正式上线使用。** “能运行”与基础 RP 闭环已经建立；新的判据是可安全部署、可持续日用、可升级、可备份恢复并有正式发布证据。唯一上线合同见 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)。
- **最终形态（用户 2026-07-01）**：**像 Codex / Claude Code 那样的完整 AI Agent 客户端**——完整 Agent 运行时（工具/多步 loop/规划/子 agent/MCP/扩展钩子），专精 RP。**agent loop 是脊柱，非可选加项。**
- **⚠️ 酒馆功能必须解耦二次重组，不可照搬（用户 2026-07-01）**：酒馆是"固定 prompt 装配管线 + 外挂插件"架构；我们是"agent 自主决策 + 能力以工具/钩子暴露"架构，**根子不同**。照搬酒馆的机械管线塞不进 agent 框架。**原则：把每个酒馆功能拆成"底层用户能力"，再用 agent 框架原语（工具 / 记忆 / 技能 / 事件钩子 / prompt 装配规则 / 宏）重新表达。** 重组映射见 [TAVERN-PARITY.md](TAVERN-PARITY.md) 第四部分。
- **范围诚实**：框架形的内核，但**RP 特化交付**——不追 Hermes 的全宽度（20+ 消息平台 / RL 训练 / 全部终端后端）。那是天花板参考，不是我们的目标。框架架构要干净到"将来能泛化"，但当前只交付 RP 客户端所需。
- **代码取向（用户 2026-07-03）**：代码必须**更开放、更透明、在未来更易修正、且更易迭代更新**。这不是泛化优先，而是工程可持续：接口和扩展点清晰开放；状态、决策、错误和验收结果可观察；模块边界低耦合、可替换；协议/数据结构版本化，允许小步迁移。
- **UI 无关 + Web 正式交付（2026-07-13 更新）**：WebUI 是当前正式产品交付主面，Tauri/Vue 资产保留但开发计划暂停。故**引擎必须是无头、独立的网络服务**（HTTP/SSE/WS，传输无关线协议），不嵌进任何单一 UI；所有客户端消费同一 engine 合同。
- **四个原仓库 = 参考素材（理念 + 代码都仅供参考）**：作者按需求拆过四个项目、写清了各自的解法。它们是极有价值的先行思考，但**一切以我们客户端的实际需求为准**——不被它们的模块划分/戒律/命名/实现束缚。需要功能时去对应仓库挖可借鉴的代码/思路搬来改。用户对四仓库有完整版权，无侵权顾虑。酒馆当功能清单参考。
- **源项目统一定位已拍板（2026-07-03，见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)）**：AIRP-Core、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 都按同一原则处理：**吸收资产，不继承产品北极星**。Core 是 engine 主核但不继承其 standalone 乐高后端叙事；MCP-Server 是数据/工具/工作流规格来源但不继承纯 MCP 数据层边界；Gateway 是传输/安全/MCP-client 资产来源但不继承纯协议桥目标；State-Protocol 是 UI/协议资产来源但不继承通用 Agent UI 标准目标。
- **State-Protocol 定位已拍板（2026-07-03，见 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)）**：原 AIRP-State-Protocol 的"通用 Agent UI 标准 / 乐高化显示层"理念不作为 AIRP 主线；但 **Blueprint、Widget Registry/Host、RFC6902 patch、Envelope、guard、虚拟滚动、consent/sandbox** 是必须吸收的成熟资产。结论：**吸收 Blueprint/Widget 架构，降级通用协议优先定位**。
- **历史快照（2026-07-04）**：PR #1–#13 完成了 workspace 收敛、UI→engine 直连、角色卡导入、id-keyed chat 和 sidecar 打包链路。此后 PR #77–#100 已继续实现 HTTP CRUD、Agent tools、decompose/analysis、审计修复和真实 WebUI SSE 证据；不要再用本段判断当前完成度，见 [项目历史归档](archive/PROJECT-HISTORY-2026-07.md)。

## 1. 产品命根子：干净提示词（干净提示词 / pristine prompt）

**这是我们客户端立身之本**（你最早就点明的差异化），恰好也是四仓库共同的灵魂——采纳它是因为它契合我们需求，不是因为文档写了。参考出处 `AIRPCLI/AGENT_BACKEND_PLAN.md:44-53,187-201`。

> **关键调和（回应 §0"像 Claude Code"看似的矛盾）**：我们要 Claude Code / Codex **级的能力**（loop/工具/MCP/记忆/子agent），但**提示词纯净度上恰恰相反**。Claude Code 这类通用 agent 框架**自带关不掉的系统提示词/脚手架**——那是它们的产品本体，对 RP 就是**上下文污染源**。**这正是"必须自建框架、不能把 RP 跑在别人框架之上"的根本理由**：只有我们**原生拥有 loop**，才能保证进模型的每个 token 由我们全权决定。**"尽可能保证提示词纯净度"是本框架的一等不变式（§2.1 不变式①，本地/未来 CI 门禁、不可破）**——我们是 Claude-Code-级能力 + 纯净度优先的 RP 特化框架。

- **为什么**：角色上下文里**每个 token 都影响角色保真度与文风**。一句外来的 "You are a helpful assistant / 请一步步思考 / 安全前导词" 就能把角色拉出戏（社区所谓"死人化"）。
- **为什么第三方 Agent 客户端不行**：Claude Code / Cursor / Codex 必然在 RP 内容外裹自己的脚手架，关不掉 → 提示词污染。**即便用 subagent 隔离上下文也不能根除——隔离 ≠ 纯净**。
- **结论**：loop 必须由我们自己（引擎）原生拥有，**进模型上下文的每个 token 由引擎全权决定**，不经任何第三方 runtime 的手。
- **反噬自检**：Core 自己的 agent 脚手架若塞进角色 system prompt，Core 就成了新污染源 → 由此推出下面的执行机制。

### 执行机制：两个物理隔离平面

| 平面 | 内容 | 走哪条道 |
|---|---|---|
| **角色平面** | 喂给模型的真实 RP 上下文 | 只由 orchestrator 装配 RP 数据（card/lorebook/preset/state/memory/history），**零 agent 脚手架** |
| **控制平面** | 工具定义 / 工具调用 / 工具结果 | 走模型 API 的**原生结构化字段**（OpenAI `tools`+`tool_calls`+`tool` role；Anthropic `tools`+`tool_use`+`tool_result` block），**永不拼进角色平面的自然语言** |

- **不用 in-prompt ReAct**：把 ReAct 指令写进 prompt 文本 = 控制平面灌进角色平面 = 自我污染。结构化工具调用是守此律的唯一干净路径。
- **本地/PR CI 强制**：`tests::subagent_context_has_no_orchestrator_noise`——断言送进 adapter 的角色平面 prompt 字符串里不含任何 agent 脚手架标记（工具名/规划指令/observe 包装），违反即红。这个测试必须保留、优先保护；自动 PR gate、审计 bot 阻塞复核与人工 review 共同承接门禁。
- **已知代价（诚实声明）**：此律把靠 in-context ReAct 脚手架的纯文本模型挡在门外。为"纯净后端"接受此代价。

### loop = 纯净 subagent 的编排器（`AGENT_BACKEND_PLAN §4.0` 最深表述）

这是干净提示词的落地形态，也直接回答"RP 要不要 subagent"：

- **真正的 RP 书写交给 Core 原生派生的隔离纯净 subagent**——只装 RP 数据（卡/世界书/state），无工具说明、编程噪声、规划指令。文笔不被主上下文压扁（"死人化"）。
- **单回合 = 派一个 subagent；loop = 按需派多个 + 中间夹工具**（先取世界书/掷骰/查 state → 派 A 写 → 派 B 写 → 落 state）。多角色场景：NPC A 一个纯净 subagent（只看 A 的卡）、NPC B 另一个。
- **两层物理隔离**：协调器自己的多步状态（调了哪些工具、轮到谁）活在**它自己的上下文**，每个书写 subagent 都是**全新纯净上下文**、看不到协调器噪声。比"单一 ReAct 上下文累积工具调用"更纯。
- **为何必须 Core 原生派生**（"即便 subagent 也不能根除污染"的答案）：第三方 subagent（Claude Code 的 Task）仍跑在它 runtime 里、裹它的 system prompt/脚手架 → 从一开始就不纯。**只有 Core 亲手装配，subagent 上下文才 100% 纯净**。这是编排器必须在 Core 内、不能外包的根本原因。

## 2. 目标架构：RP 特化 Agent 框架 = 无头引擎 + 可换 UI（2 盒）

> 定稿方向（用户 2026-07-01 定性）：我们做的是**专精 RP 的 AI Agent 框架**（§0）。架构随之从"四仓四层"收敛为**两盒**——一个无头 Agent 引擎（内核 + RP 特化层），一个可换的 UI（Tauri 先 / web 后，都是引擎的客户端）。四仓库降为引擎的**零件来源**（见 [PARTS.md](PARTS.md)），不再是四个必接的盒子。
> 旧"四层图"（State-Protocol `背景整理 §3.3`）是三仓时代、且最底"推理层"曾是空框——已被 Core 填上、被本次定性取代，仅作历史参考。旧图里"Gateway=未来核心/最值钱是 State Protocol"是 `背景整理 §3` 明标的 ChatGPT 非定论意见，不采纳。

```
┌─ UI（可换，当前正式产品=WebUI；Tauri=后续桌面客户端）─────────┐
│  Vue WebView：Blueprint 渲染 · widget 注册表 · RFC6902 patch store  │
│  · 虚拟滚动 · 沙箱 · consent 门                                     │
└───────────────── State-Protocol Envelope（SSE / Tauri IPC / 将来 WS）┘
                                  ↕  （传输无关线协议；web=SSEBus 路径）
┌─ AIRP 引擎（无头独立网络服务，HTTP/SSE/WS）─────────────────────────┐
│                                                                     │
│  ■ Agent 内核（通用框架 —— 脊柱）                                    │
│    bounded loop（纯净 subagent 编排器，§1）· adapter（多 provider 流式）│
│    原语面：Tool（内置+MCP client）· Memory（三层自进化,§3.4）        │
│    · Skill（agentskills.io 兼容）· Event Hook · Prompt-Interceptor   │
│    · Macro · Subagent —— 此面既是 agent 能力，也是第三方扩展面（§3.8）│
│                                                                     │
│  ■ RP 特化层（叠在内核上）                                           │
│    干净提示词纪律（两平面隔离，§1，产品灵魂）                        │
│    · RP 数据层（卡/世界书/会话/记忆/state/场景，单一真相）           │
│    · 酒馆格式导入（解耦重组为原语，TAVERN-PARITY §4）                │
│    · orchestrator 装配（RP 数据→角色平面）                          │
│                                                                     │
│  ■ HTTP/服务层：鉴权 · 限流 · capability 强制 · 线协议端点           │
└─────────────────────────────────────────────────────────────────────┘
                                  ↕  （可选，要时才接）
          第三方 MCP server / 工具生态 · 外部记忆 provider
```

- **引擎是无头独立 service**（承 §0 web 就绪）：Core daemon 已是此形态（`/v1/*` HTTP+SSE），白捡为引擎雏形。UI 经线协议连引擎，Tauri 和 web 都是客户端。
- **内核原语面 = 扩展面**（§3.8 合一）：Tool/Memory/Skill/Hook/Macro/Subagent 既给 agent 用、也给第三方接。MCP client 让"第三方工具=标准 MCP server"（跨语言、进程隔离）。
- **RP 特化叠在内核上、不侵内核**：换掉 RP 数据层+导入+装配，这个框架就能做别的 agent 应用（框架形内核的意义）。但当前只交付 RP，不过度泛化（§0 范围诚实）。
- **原四仓的"协议桥(Gateway)/数据底座(MCP-Server)独立盒子"降级**：它们的价值零件（Gateway 的 MCP client/传输/安全硬化、MCP-Server 的数据域/沙箱）**吸收进引擎**；"独立一跳"的形态对单客户端是负担（PARTS §L），不再是必接盒子。第三方 MCP 生态仍可经内核的 MCP client 接入（可选）。
- **仍未决（见 §4）**：数据单一真相的具体落盘归属、UI↔引擎线协议选型（复用 State-Protocol Envelope vs 简化）、capability 引擎侧强制的实现。

### 2.1 引擎不变式（我们的，从四仓戒律提炼重组 —— 非照搬）

> 四仓各有为**独立分发**定的戒律，是宝贵参考非法律。为我们的**单一 RP agent 框架**重组成下面这套引擎不变式（采纳契合的、丢掉为独立性自我设限的）。

- **① 干净提示词（灵魂，不可破）**：两平面物理隔离（§1）。角色平面只装 RP 数据、零脚手架；控制平面走结构化工具调用。本地/未来 CI 守 `subagent_context_has_no_orchestrator_noise`。
- **② 有界 agent**：步数/token/成本/墙钟上限任一触顶即停；可取消；每步可观测流式（不黑箱）。（承 Core 6 戒律 1-3）
- **③ 工具受控**：allowlist + capability 门；破坏性工具默认 dry-run 需确认；幂等键去重；同角色/资源并发写串行化。（承 Core 戒律 4-5 + 安全模型）
- **④ 数据单一真相**：RP 数据（卡/世界书/会话/记忆/state/场景）引擎内**一处存、一个真相**——**丢弃**原 Core"自带数据"与 MCP-Server"另一份数据"并存的乐高设计（那是为各自独立分发，对单产品是负担）。具体落盘归属见 §4-1。
- **⑤ 建议非强制**：数据/预设/世界书是给 agent 的**建议素材**，最终决定权归 agent（承 MCP-Server 哲学 + §3.3 预设适配）。**但"引擎不拼 prompt/不调 LLM"这类为独立数据底座定的自我克制，我们不采纳**——我们的引擎既存数据又调 LLM 出干净 prompt，一块干完。
- **⑥ 扩展受控开放**：内核原语面（Tool/Hook/Skill/Macro/事件）对第三方开放，但过 capability 门 + 沙箱（承 State-Protocol 安全立场：拒执行 agent 生成的代码，第三方扩展声明能力+用户授权）。详见 §3.8 / [TAVERN-PARITY.md](TAVERN-PARITY.md) 第二部分。

### 2.2 UI 层：半永久 Blueprint / RP = UI Profile（`背景整理 §2.3-2.4`）

- UI **只渲染声明式 Blueprint、不执行 agent 生成的代码**（安全立仓之本：否决"Agent 每轮写 Vue"——token 浪费+不稳+前端执行任意 LLM 代码风险）。
- 首次进 RP → agent 推导 Blueprint（widget 列表 JSON）→ 存储 + UUID；同一 RP 以后直接读、不再生成。RP 类型决定 UI 画像：恋爱→聊天、经营→数据面板、桌游→卡牌、跑团→属性栏。
- 首批候选 widget：`chat / memory / emotion / inventory / quest / map / card`（`背景整理 §7-2` 先做哪几个待定）。widget 注册表开放（`namespace.name`，`core.*` 保留），capability 由引擎强制。
- 方向约束：Blueprint/Widget 是 **AIRP 内部 UI 合同与扩展面**，不是当前阶段的公共协议标准化工程。默认路径必须先跑通并验收 `UI → Tauri bridge → engine → state patch → Blueprint/widget render`；MockBus 只留给测试/演示。

### 2.3 WebUI 正式产品面（2026-07-13 更新）

- **定位**：WebUI 是当前正式产品交付主面，目标为单实例、自托管、单用户正式上线；后端可靠性验证仍是职责之一，但不再是它的产品上限。
- **目的**：让用户在受支持的生产部署中完成 provider 配置、角色与 RP 资产管理、会话和长期流式 RP，并可安全升级与恢复。
- **约束**：WebUI 不走 `card_path` 任意路径读；远端/浏览器导入只用安全上传/JSON/base64 合同。WebUI 不建立平行 domain model，所有权威数据与安全策略仍在 engine shared services。
- **完成条件与执行路线**：[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) 是唯一正式上线合同；历史基础验收见 [WebUI 历史归档](archive/WEBUI-HISTORY-2026-07.md)。

### 2.4 Agent UI Test Harness（临时受控测试接口）

- **目标**：给开发 agent 一个可程序化控制前端 UI 的能力，让 agent 能自己启动 UI、选择角色、发消息、观察 DOM/状态/日志、截图、断言结果，避免每次 GUI 验收都靠人工目测。
- **当前形态（已收口）**：一个可删除的运行时模块 `ui/src/agent-test.ts`，显式开启后暴露 `window.__AIRP_AGENT_TEST__`，由 Codex browser control 或 Playwright 调用。它是当前唯一默认测试面；dev-only widget、Tauri dev command、WebUI 前端控制面只能作为替换方案提出，不得与它并行新增，除非先说明为什么一个入口不足并移除/降级旧入口。这里限制的是 agent 驱动 GUI 的控制入口，不限制 §2.3 的后端可靠性 WebUI。
- **用户关闭方式**：删除 `ui/src/agent-test.ts` 后重新手动构建；`App.vue` 只在文件存在时加载该模块，相关单测不阻断无模块构建。普通用户文档只暴露这一条关闭路径。
- **安全边界**：默认关闭，只在 dev/test build 或显式 env flag 开启；能力白名单；不得暴露任意文件读写、任意命令执行、未授权 shell/plugin 权限；不得成为第三方扩展默认能力。
- **验收能力**：至少能执行 `load fixture → select/import character → send chat.send → wait streamed reply → read state/DOM → screenshot/log`，并能在失败时输出可复现证据。

## 2.5 性能契约（产品级硬约束 —— 防止重蹈酒馆覆辙，`背景整理 §6`）

这是被我一度漏掉、但产品级的硬约束。**酒馆崩溃根因 = 无界 DOM + 单线程阻塞 + 内存泄漏，不是算力不足**：4090+64G 照样崩；给更多 CPU/GPU 是崩得更快不是更慢（10 万 DOM 是内存/布局树问题，填充率救不了）。**关键心智纠错：Tauri 的 WebView2 就是 Chromium，与浏览器同引擎，不会"多吃"硬件——"本地客户端能吃 CPU/GPU 所以不崩"是错误心智模型，指望它必重蹈覆辙**。性能是"有界 vs 无界"，不是"算力多寡"；装刹车（虚拟化/上界）不花硬件。

**7 条硬约束（不可违反，实现方谁都不许破，`背景整理 §6.2`）**：
1. 聊天/长列表**强制虚拟滚动**，永远只渲染视口内 DOM（这是"2000 条崩"与"10 万条丝滑"的分界）。
2. 全量历史真相在引擎，UI **窗口分页**拉取，不前端常驻全量。
3. 状态更新 **patch 优先**，禁每轮全量重灌 state。
4. **稳定 ID 做 key**，细粒度响应式只更新变化节点。
5. **重计算留在 Rust sidecar**（状态 diff / 正则 / prompt 拼装 / 持久化离开渲染线程；JS 重活走 Web Worker）。
6. **流式增量追加渲染**，禁每 token 重解析整段 markdown。
7. **内存卫生**：离屏 widget 销毁、listener/interval 清理、消息窗口封顶。

**开发前先做 Perf Spike 验证门（`背景整理 §6.4`）**：Tauri 壳 + 虚拟滚动灌 10 万条假消息，验收滚动稳定 60fps + 进程内存封顶（不随历史线性涨）+ 流式追加无卡顿。过了才锁定 Tauri+Vue；不过才有理由评估 Flutter 等方案。

## 2.6 现状真相 + 复用地图（亲读六份承重文档后校准）

> 本节保留 2026-07-01 的源项目吸收判断。当前 AIRP-Dev 已完成 UI `BusRelay` 直连、id-keyed chat、动态 Agent loop、统一 Chat/State/Lorebook services 与 decompose/analysis；仍待闭合的是 Windows GUI artifact 实跑、MCP/skills/plugin 生态与跨 provider 扩展。当前能力以 [当前基线](CURRENT-BASELINE.md) 和 [文档地图](README.md) 为准。

### 什么能用 / 什么是桩 / 什么从没联调

| 块 | 代码成熟度 | 关键桩 / 缺口 | 联调状态 |
|---|---|---|---|
| **Core/engine（推理后端）** | 较高——daemon、双 provider 流式 RP、统一 Chat/State/Lorebook services、结构化 tool-call loop、19 个受 capability/allowlist/confirm 控制的内建工具、运行时工具目录、场景/卷与 decompose/analysis 均有测试 | MCP client、完整 worldbook 高级语义、跨设备稳定身份仍未建 | 本地 Agent loop、统一服务边界和 context bundle 可用；开放式 MCP/插件生态仍后置 |
| **MCP-Server（资产来源）** | 中——框架全，stdio 真 MCP、HTTP 已补；其 truncate、lorebook merge/apply、seal volume、context bundle 资产已按本仓边界吸收 | 源仓的解析与产品边界不能直接视为本仓事实 | 不作为 AIRP-Dev 运行时依赖；只吸收经过复核的资产 |
| **Gateway（协议桥）** | 高——已硬化、测试全绿的纯桥 | **streaming(Stage 2)是返回 Unimplemented 的桩**（唯一明确功能缺口）；嵌入 Core(Stage 5)未做 | e2e 全用自带 mock；**从没接真 MCP-Server / Core** |
| **UI（显示层）** | 高——widget/registry/RFC6902 patch/沙箱/**虚拟滚动(computeWindow已实现)**/边界guard/`.exe`打包已有；AIRP-State-Protocol 原项目曾验证打包 exe 可启动并简单交互；Phase 0 已接 engine SSE；Task 1.2 已把 chat 改为 id-keyed 并去掉 `chat_lock` | **perf spike(10万条)代码在但没跑过**；原项目 exe 验证不覆盖当前 AIRP-Dev 与 engine 集成后的完整 GUI 验收；真实 API key/settings 下的打包启动闭环未验收 | UI↔engine 聊天链路已接；当前 GUI 运行时验收与性能验收待补 |

### 复用地图（从哪挖什么 —— 参考，最终按我们需求裁）

- **后端主体挖 Core**：`AGENT_CLIENT_ASSESSMENT §附` 给了精确索引——`adapter.rs`(双 provider 流式)、`chat_pipeline.rs`(prepare→stream→finalize)、`orchestrator/`(装配)、`fsm`/`xml_unpacker`(流过滤)、`png_parser.rs`(角色卡正确解析)。这些当库复用，别重写。
- **UI 壳挖 State-Protocol**：整套 Tauri+Vue + widget 生态 + 打包仍是主要资产；BusRelay 与 id-keyed chat 已落地。主要剩余是当前 AIRP-Dev Windows artifact 的真实 runtime smoke、sidecar 生命周期和 Perf Spike。
- **协议桥挖 Gateway**：纯桥/传输/路由/安全硬化可参考；但要补 streaming、且要第一次真接后端。
- **数据格式解析挖 MCP-Server + Core**：MCP 有数据域框架，Core 有正确的 png_parser；酒馆兼容要按 §3 补齐。

## 3. 功能支柱（需求 → 参考解法 → 状态）

### 3.1 角色卡（导入，兼容酒馆）
- **需求**：用户手动导入文件，必须兼容酒馆 Character Card V2/V3。
- **规格策略（用户 2026-07-02，覆盖卡/世界书/预设三类）**：建**我们自己的开源版本化"AIRP 资产规格"= engine canonical 数据模型的正式化**，但**超集 V3 不重造 + 剔除≠销毁（存储层 passthrough sidecar 全保留、只在活动/装配层剔无用参数）**。详见 [ASSET-SPEC.md](ASSET-SPEC.md)。字段随导入 Task 增量固化，不前置写死。
- **文档解法**：`AIRPCLI/README.md:361` 定规范——PNG 覆盖 `tEXt`/`zTXt`/`iTXt`（含 zlib）、`ccv3`(V3) 优先 `chara`(V2) 回退、v1 平铺卡归一化 v2。归属 MCP-Server（数据层）。
- **状态**：✅ 正确实现已存在于 `engine/src/png_parser.rs`(262行) + `AIRPCLI/src/types.rs:37-62`(`TavernCardV2`/spec+data 封装/`normalize_v1_to_v2`)，且在 Core 里是列入 built-in 工具的（`AGENT_BACKEND_PLAN §4.1`）。mcp-server 那份是自己 zTXt-only 坏解析器（`character_store.rs:217`），从没接对。**工作量=接线**：把 Core 已验证的 png_parser 接到"最终定为数据真相的那一层"（Core 自带 vs 归 MCP-Server，取决于 §4-1 拍板），杀掉 mcp-server 的坏实现。

### 3.2 世界书（导入+注入）
- **需求**：手动导入，兼容酒馆 world info；按需注入而非全量灌。
- **文档解法**：`airp-mcp-server/SKILL.md` 列了目标字段（keys/secondary_keys/comment/content/constant/selective/insertion_order/enabled/position/case_sensitive/priority；position: before_char/after_char/an_top/an_bottom/at_depth），但自认"取子集、可扩展"。注入用 aho-corasick 单 DFA 扫描（`AIRPCLI` 实测 11.37× 加速），按需取触发条。
- **状态**：⚠️ PR #141 交付 v2 `constant` 运行时合同；worldbook v3 已交付 shared `WorldbookNormalizer`（统一 PNG 导入、PUT API、Agent tool 三入口）+ advisory metadata（`secondary_keys`/`case_sensitive`/`extensions`，`#[serde(default)]` 向后兼容，trigger 不消费）+ import diagnostics（`WorldbookImportReport`）。SillyTavern 别名（key/keysecondary、disable、order/insertion_order、caseSensitive）归一化已覆盖，ST-only 字段原样保留在 `extensions` 不丢弃。position/depth/selective/secondary/probability/matchWholeWords/递归的**运行时触发与注入语义**仍未实现，当前仅作 advisory 保留；后续按“给 agent 的建议元数据 + 检索 Tool”方向推进，非硬编注入器。

### 3.3 预设（导入，兼容酒馆）—— 建议素材，非机械套用
- **需求**：手动导入，兼容酒馆预设。
- **核心哲学（`airp-mcp-server/SKILL.md:15-16` 全局约束）**：预设（连同卡/世界书）是**给 Agent 的参考建议，不具强制性，最终决定权归 Agent**。预设**不是要机械回放的 prompt 结构指令**——是 Agent 要理解意图后按当前 LLM 自行决断怎么用的素材。
- **这是相对酒馆的核心差异化**：
  - 酒馆机械注入预设的精确 prompt 结构 → 预设**跟模型强绑定**，换模型就崩、要找"特供预设"。
  - AIRP 把预设当**意图**交给 Agent，Agent 按当前模型重新实现 → **为弱模型硬凑的特供预设，能被 Agent 理解意图后干净适配到强模型**（强模型不需要那些粗暴压制脚手架）。`SKILL.md:62`："特供预设……常是跨模型的压制脚手架在当前模型上过度压制"；`:64`："若预设本就适配当前模型，可不动"。
  - 再次落到 §1 干净提示词：不搬酒馆的跨模型压制脚手架进 prompt，prompt 才干净。
- **文档给的工具**：`analyze_preset`（读懂每个 prompt 块用途/提取正则/总结文风）、`tune_preset`（按当前模型热调预设源头，best-effort）。采样参数（temperature 等）同为建议值，Agent/adapter 判断是否契合当前模型，不契合可不用——**不存在"采样参数必须落到某处机械生效"的问题**。
- **数据层仍需做的**：正确**解析+存储**酒馆预设文件（结构块 + 正则脚本），把素材原样交给 Agent。正则脚本兼容 `find_regex/replace_string/affects/placement/disabled/markdown_only/prompt_only/run_on_edit`，用于"八股后处理"。
- **状态**：⚠️ 解析层有正确骨架 `preset_regex.rs`（placement=AI-Output 是故意的部分 scope，非 bug），但 `preset.rs:50-56` 另有一套瞎起名的 `RegexScript` 冲突。需杀重复 + 补字段。**处理层（Agent 适配）靠 analyze_preset/tune_preset 提示词 + Core loop，非机械管线**。

### 3.4 会话与记忆（含"越用越懂你"自进化记忆 —— 核心差异化）
- **需求**：短期消息历史（编辑/regenerate/swipe）；长期超窗记忆；**随使用时长复利式变强**（对标 Hermes Agent，用户 2026-07-01 强调重要）。
- **已有基础**：消息 JSONL append-only；长期=**封卷（volume seal）**归档 vol_N（md）+ index，封卷永不自动（阈值信号→loop 拍板）；gating/checkpoints/timeline 进度锚点；Core User Persona（base+drift）。
- **借 Hermes 补的自进化记忆（详见 [HERMES-MEMORY.md](HERMES-MEMORY.md)）—— 这是相对酒馆的核心差异化**（酒馆每轮重灌静态卡+世界书、无跨会话学习）：
  - **常驻有界记忆（🆕）**：每角色/存档一份有界 md（RP-MEMORY=情节/关系/世界事实 + USER=用户文风偏好），always-injected 当稳定前缀（frozen snapshot：本轮落盘下轮生效，天然合 §3.5 缓存纪律），超 80% 自动整理合并。
  - **用户模型自动抽取（🆕）**：从对话自动抽取用户偏好/文风更新 USER 层——"越用越懂你"的魔法（我们现有 drift 偏手动）。
  - **历史检索 session_search（🆕）**：SQLite FTS5 全文 + LLM 摘要，回忆任意历史片段。**Hermes 证明这是非向量的轻量长程记忆，正合"RAG 暂缓、先简单检索"**。
  - **RP 技能自建**：怎么写角色/场景套路/用户文风，从经验自建、反馈更新——接进 agent loop 工具/技能注册表（与 §3.8 扩展面共底座）。**兼容 agentskills.io 开放标准**（Hermes 也用）→ 白捡第三方技能生态，不自造标准。
  - **subagent + RPC 零上下文成本工具调用**（Hermes 印证 Core"loop=纯净 subagent 编排器"）：多步工具压成脚本一次调、不堆主上下文——合干净提示词，设计时纳入。
  - **角色成长模型（统一 · base + 多维 drift）—— 角色随剧情成长、非一成不变（用户 2026-07-02）**：不建新系统，**复用已有的 User Persona M_UP 双层模式（base=persona.json 可 lock 的不可变契约 / drift=累积变化，Agent 自推断 base-vs-drift 冲突，如"不会打篮球(base) vs 学会了(drift)"）套到角色上**。角色 = 作者写死的卡（base）+ **成长 drift overlay**，drift 多维度，把 NeuroBook 净新点 + 我们已有零件统一：
    - **知识维（净新·来自 NeuroBook 点2）**：该角色"已知/被告知/观察/推断/误解"的——随剧情增长。注入角色平面的是 **base + 该角色视角的知识子集**，非全知世界书（治"角色知道太多"，超出现有关键词懒加载）。
    - **人格/文风维（= Soul 演化，已定·第二档）**：性格深度/说话习惯/关系态度随剧情+反馈演化 + agent 书写风格贴合用户。
    - **关系/状态维（已有）**：好感度/HP/location（`state/live.json` + `history.jsonl`）。
    - **剧情进度维（已有）**：gating/checkpoints/timeline。
  - 统一原则：base 不可变（`persona.lock` 式），drift 叠加注入、不改原卡、可读/可审/可回滚；Agent 自推断冲突（不判定语义、守戒律）；全程守干净提示词。详见 [HERMES-MEMORY.md](HERMES-MEMORY.md) §四 + [LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md) 点2。
- **状态**：封卷/persona/state/gating 框架在（成长零件已散在各处，需**统一成角色 base+drift 模型**）；常驻有界记忆+用户自动抽取+session_search+角色知识维需新建。**优先级：MVP 后紧接做**（核心卖点，不宜太后）。

### 3.5 Prompt 装配管线（承 §1）
- **需求**：card+preset+gating+memory+lorebook+history+用户输入的拼装、token 预算、**零脚手架**。
- **文档解法**：Core `orchestrator/` 拥有，**默认**装配顺序 `card → preset → checkpoint gating → known context → 卷 → lorebook`（`AIRPCLI/README.md:210`）——这是 Core 的干净装配默认序，**不是回放导入预设的排版**（预设是建议素材，见 §3.3）。`chat_pipeline` 三段式 prepare→stream→finalize。多角色场景每个 NPC 独立纯净上下文。token 估算是 ±30% 启发式（非真 tiktoken）。
- **载荷排序（prompt-caching 承重决策）**：装配输出必须按可变性排——稳定块（persona/preset/lorebook）在前、易变块（live state/per-turn）在后，保证稳定前缀跨轮字节一致 → 命中缓存。缓存翻译（`[[CACHE_BREAK]]`→`cache_control`）在引擎 adapter 层做。**与 §3.4 Hermes frozen-snapshot 同原理**（记忆/装配当稳定前缀）。
- **状态**：核心资产，最该保护。AIRP-Dev 的 `export_context_bundle` 已按稳定 card/preset/extensions/lorebook 在前、易变 live state 在后的顺序输出，并受 UTF-8 安全的上下文大小上限保护。

### 3.6 LLM 连接层
- **需求**：多 provider、流式、参数预设。
- **文档解法**：Core `adapter.rs` `BackendEngine`——**Direct**(OpenAI 兼容 SSE) + **AnthropicMessages**(原生 `/v1/messages` SSE) 双格式都在（**没有砍多 provider**）；**ClaudeCodeSdk 是 stub**，定位为"可选生成 engine，**绝不当 loop owner**"（第三方 runtime 脚手架关不掉会污染）。对外统一 OpenAI 兼容 + 结构化工具调用（协议标准护城河）。
- **状态**：Direct/Anthropic 双引擎可用；ClaudeCodeSdk 未实现（低优先）。

### 3.7 UI / Widget
- **需求**：聊天界面（流式/swipe/编辑）、角色管理、连接设置、可扩展面板（状态条/好感度/物品栏）。
- **解法**：Tauri+Vue，只渲染**引擎**下发的 Blueprint（不执行 agent 生成的代码）。widget 三类（Vue 首方 / Module / esm 动态 import）。面板=widget 实例，state 走 RFC6902 patch。capability 消费门 + 沙箱（esm+sandbox → opaque-origin iframe）。**交付=签名二进制，绝不运行时 clone 编译（RCE 风险）**。引擎作 sidecar 随包默认自带、零配置；可一键换远程引擎 URL（承 §0 web 就绪：同一线协议）。
- **性能是硬需求**：本支柱必须守 §2.5 的 7 条硬约束 + Perf Spike 验证门——UI 是最容易重蹈酒馆覆辙的一层。
- **状态**：UI runtime（Registry/BlueprintRenderer/WidgetHost/store+patch/虚拟滚动/沙箱/consent/打包）主体在；`BusRelay` 已直连 engine 聊天 SSE；chat 已改成 id-keyed 消息模型并移除 `chat_lock`；PR #13 已打通 engine sidecar 打包；Agent UI Test Harness 已有 `ui/src/agent-test.ts` 最小入口。仍待补 GUI 运行时验收、Perf Spike、harness 接 Codex/Playwright 的截图/日志证据、reasoning/action 渲染与后续会话操作。

### 3.8 Agent 能力 + 扩展生态（合一 —— 产品脊柱 + 硬需求）
- **需求（用户 2026-07-01 强调）**：必须充分暴露接口，无门槛、无缝兼容第三方扩展。对标酒馆——它的扩展性是护城河。详见 [TAVERN-PARITY.md](TAVERN-PARITY.md) 第二部分。
- **关键洞见——Agent 能力面 = 扩展面（同一套接口）**：既然最终形态是 Claude Code 式完整 Agent（§0），Agent 的能力面（函数工具 / MCP 工具 / 事件 / prompt 拦截 / 宏 / slash 命令 / 子 agent）**就是**第三方扩展面。二者不是两套东西，是同一套接口的两种用途。工具注册表 + MCP client + 事件总线是共同底座。**agent loop 是脊柱，MVP 就立骨架**，非后期。
- **酒馆扩展面（标杆）**：manifest + 生命周期钩子、`getContext()`、事件总线（`eventSource`）、prompt 拦截器（`generate_interceptor`）、slash 命令/STscript、函数工具注册、宏系统、消息格式化管线钩子、生成 API（generateRaw/QuietPrompt）、状态持久化（写进卡/会话 metadata）。
- **我们的现状**：UI 侧 State-Protocol 已有开放 widget 系统（manifest/esm/capability/沙箱），**比酒馆更安全**但只覆盖 UI widget；**引擎侧扩展钩子基本空白**（事件/拦截/工具/宏/命令无对等物）——这是最大缺口。
- **结构性优势**：agent loop + MCP client 让"第三方工具 = 标准 MCP server"（跨语言、进程隔离），比酒馆同进程 JS **既更无缝又更安全**，可能是差异化卖点。
- **关键张力（§4 新增决策）**：酒馆"零门槛"=无限制 JS+DOM 全访问（有安全风险）；我们 State-Protocol 刻意反向（声明式+沙箱+能力强制）。选"能力受控开放"（暴露丰富结构化钩子但过 capability 门）还是"酒馆式无限制"？
- **状态**：🆕 引擎侧接口面需新建。**建议尽早立骨架**（事件总线+函数工具注册+宏系统）——第三方生态越晚开、接口越难改。非纯后期功能。

## 4. 待决策事项（真正的开放题——文档未定或需你拍板的）

> **架构已定（§2 重写后收敛）**，故以下不再是开放题：**2 盒（引擎+UI）**、**单一数据真相在引擎内**（不再多盒/乐高并存）、**推理路由缺口消解**（UI→引擎，引擎内部自己取数据+调 LLM，不存在"Gateway 桥到 Core"问题）、**原 Gateway/MCP 独立盒降级为引擎的零件来源**。以及此前已定：角色卡解析归属（用 Core 正确 png_parser）、多 provider（OpenAI+Anthropic 双引擎）、长期记忆（volume+FTS 检索，RAG 缓）、数据迁移（手动导入+酒馆兼容）、Core/引擎 UI 无关。
>
> **引擎雏形 = Core**：Core 单体已是完整 RP 后端（正确数据层 + 完整 `/v1/*` HTTP+SSE API + agent loop 骨架 + adapter），无头 service 形态白捡。引擎 = 以 Core 为核，吸收 MCP-Server 数据域优点（酒馆格式解析/沙箱/插件零schema）+ 补扩展面。

**仍需你拍板的真开放题：**

1. **引擎内数据层的存储设计**（原"数据归属"收敛后剩的）：单一真相已定在引擎内；剩的是怎么把 **Core 自带数据层**（png_parser 正确、chat_store/volume/scene）与 **MCP-Server 数据域**（角色/世界书/state/预设的域模型 + 沙箱 + 插件零schema）**熔成一套**——以 Core 为基吸收 MCP 优点，还是反之。多为工程取舍，可动手时定。
2. **UI↔引擎线协议落地细节**：方向已定为吸收 State-Protocol 的 Blueprint/Widget/RFC6902 patch/Envelope 资产，且默认链路直连 AIRP engine；剩余是具体接口边界、版本策略、错误语义和 engine 侧 capability 强制的实现细节。原 `agentbus` 自重写 Envelope 的重复问题随之消解（引擎直接用 state-protocol 类型）。
3. **近期收口顺序**：基础 WebUI、长历史和多 Persona 存储地基已由 PR #123/#124/#125/#127 完成，production P0 已由 PR #132/#133/#135/#136 完成；当前顺序为 #137 工具链安全 → #114/#115/#126 RP 使用面 → 数据恢复/运维 → release candidate。#117/#87/#116 后移；桌面 #98/#29/#122 保留追踪但开发计划暂停。
4. **纯净度代价是否接受**（Core §10-1）：干净提示词把靠 in-prompt-ReAct 的纯文本模型挡在 loop 工具外。接受（纯净优先），还是留"污染模式"开关兼容那类模型？
5. **capability 扩展范围**：Agent 工具已有 engine-side `call:tool` + allowlist + destructive confirm 强制；未来 widget intent、MCP、hook 与 plugin storage 必须复用同一权威模型，不能退回 UI 单边限制。
6. **世界书插入引擎完整度**：关键词触发、`constant`、shared normalizer 与 import diagnostics 已交付；position/depth/selective/secondary/probability/matchWholeWords/递归暂作 v3 advisory metadata 保留，**不进入运行时注入管线**。后续是否将其中部分提升为运行时语义，取决于“给 agent 的建议元数据 + 检索 Tool”方向能否覆盖实际 RP 需求（见 §3.2/TAVERN-PARITY §4），而非硬编注入器。
7. **扩展开放模型**（硬需求）：受控开放（丰富结构化钩子过 capability 门+沙箱，推荐）vs 酒馆式无限制 JS（零门槛但违安全立仓之本）？详见 [TAVERN-PARITY.md](TAVERN-PARITY.md) 第二部分。已倾向受控开放（见 §2.1 不变式⑥）。
8. **Soul 动态人格演化的实现细节**（已定加入·第二档）：base+drift overlay 的 drift 抽取粒度/回滚 UI 等，做时再细化。

数据层开放题已进一步收敛：一个 `session_id` 定义一个独立开局/存档槽位；第三方世界书进入独立素材库，被采用后连同角色/场景/Persona 世界书一起物化为 session 内可编辑快照，运行时以 session manifest 为唯一加载清单，并通过 revision 保留每轮可复现性。当前仅 history/memory 已按命名 session 隔离，state、第三方库、世界书物化与 revision 仍待分阶段实现。完整合同见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)（#168）。

## 5. 修订记录

- 2026-07-15：收敛 #168 session 数据模型：`session_id` = 独立开局/存档槽位；确定第三方世界书素材库、session 全量可编辑世界书物化、manifest/provenance/revision、复制分叉与显式提升边界，记录于 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)。该方向吸收 SillyTavern 的公开多来源绑定经验，但采用 AIRP 稳定 ID 与自包含存档独立实现。
- 2026-07-14：审计 bot 已恢复并重新作为 PR 合并前阻塞门禁；本地测试全绿只代表可以开 PR，必须等待审计完成、修复全部阻塞意见并取得通过状态，再由人工 review 决定是否合并。同步 `AGENTS.md`、`README.md`、`DEV-GUIDE.md`、`PARTS.md` 与本计划的流程表述。
- 2026-07-14：同步 worldbook v3：shared `WorldbookNormalizer` 统一 PNG 导入/PUT API/Agent tool 三入口 SillyTavern→AIRP 归一化；v3 schema 新增 `secondary_keys`/`case_sensitive`/`extensions` advisory 字段（`#[serde(default)]`，trigger 不消费）；`WorldbookImportReport` 提供 converted/aliases_normalized/advisory_preserved/invalid/needs_review 诊断。#126 shared normalizer + import diagnostics 已交付，position/depth/selective 等运行时语义仍开放。
- 2026-07-14：同步 PR #141：worldbook v2 `constant` 运行时合同、SillyTavern 字段归一化、WebUI 常驻编辑和 merge-safe 激活已交付；#126 继续承载 shared normalizer/import diagnostics 与其余高级语义，审计遗留字符串布尔兼容进入 #142。
- 2026-07-13：用户将当前开发目标定为“WebUI 能够正式上线使用”；新增 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)，首发收敛为单实例、自托管、单用户，同源 HTTPS + 私有 engine，并把生产部署、RP 使用面、数据恢复和 release gates 设为主链。
- 2026-07-13：同步 PR #124/#125：durable message ID、cursor history、rollback-by-ID、完整 JSONL 保留、50 条 WebUI window、增量 DOM 和滚动保持已交付；主线前移到 #114/#115 的 RP Profile、migration report 与 PromptAssemblyTrace，#37/#122 只保留 branch/swipe/edit 和产品 UI 性能门。
- 2026-07-12：用户校准当前开发策略：继续围绕 WebUI 增补，优先借此提升 engine shared service、HTTP/SSE、数据合同和可观测能力，降低未来 Tauri/Vue 接入时的后端返工；桌面 artifact/sidecar 验收保留为阶段性 release gate。
- 2026-07-12：新增 [AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md)，明确内建分级执行与升级闸门只是参考 profile；用户可编排特有方案，系统只强制安全、有界、纯净、可观测与确定性验收边界。
- 2026-07-11：近期执行基准改为“最快形成基本可用 WebUI”。该计划已完成并并入 [WebUI 历史归档](archive/WEBUI-HISTORY-2026-07.md)：单默认 Persona、Preset 选择/导入、session lifecycle、provider redirect 安全与三轮 RP smoke 均已进入后续基线。
- 2026-07-14：开发基线重校准并收敛文档。先处理 #137 开发工具链安全，再推进 #114/#115/#126；逐 PR 审计和完成计划不再各占一份永久文档。
- 2026-07-04：用户澄清 WebUI 定位：它是临时后端可靠性验证面，用来验证 engine/API/SSE/数据层，不替代 Tauri/Vue 桌面 UI；桌面 UI 继续作为长期产品面慢慢推进。Agent UI Test Harness 已收口为 `ui/src/agent-test.ts` 一文件 dev/test 入口，默认关闭、能力白名单；普通用户删除这一文件即可在 fork 构建中移除 agent 控制面。补入反冗余要求：不要并行新增第二套测试面或把内部测试文件暴露成用户操作步骤。
- 2026-07-03：同步 GitHub 合并历史后的当前状态：PR #1 收敛两盒 workspace，PR #2 完成 UI↔engine 直连，PR #3/#4 完成并加固 path-first 角色卡导入；将仍写着 mock BusRelay、四仓入 workspace、CI 强制等旧状态的段落改成当时事实。后续基线由 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 接管。
- 2026-07-03：新增 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)，拍板 AIRP-State-Protocol 的定位：不继承"通用 Agent UI 标准优先 / 乐高优先"作为产品北极星，但必须吸收 Blueprint、Widget、state patch、guard、虚拟滚动、consent/sandbox 等成熟 UI 资产。
- 2026-07-03：补入代码取向：更开放、更透明、未来更易修正、更易迭代更新；并解释为接口/扩展点清晰、状态与决策可观察、低耦合可替换、协议和数据结构版本化。
- 2026-07-03：新增 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)，逐项审查 AIRP-Core、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol，并统一为"吸收资产，不继承产品北极星"。
- 2026-07-01：**重写 §2 架构章为定稿方向**——"RP 特化 Agent 框架 = 无头引擎（Agent 内核原语面 Tool/Memory/Skill/Hook/Macro/Subagent + RP 特化层 + HTTP/服务层）+ 可换 UI（Tauri 先/web 后）"两盒图，取代旧四层图。§2.1 引擎不变式（从四仓戒律重组：干净prompt/有界/工具受控/数据单一真相/建议非强制/扩展受控开放）。§2.2 UI 层（Blueprint/widget）。据此收敛 §4：数据归属/拓扑/seam 等随架构消解，剩 8 条真开放题（引擎数据层设计/线协议选型/MVP/纯净度代价/capability强制/世界书完整度/扩展模型/Soul细节）。同步纠正 §3.5/3.7 中 Gateway/MCP-Server 旧措辞为"引擎"。
- 2026-07-01：初稿，基于四仓库架构排查 + 产品目标澄清。
- 2026-07-01：比对酒馆源码，发现三类格式当前实现均不兼容真实酒馆文件。
- 2026-07-01：翻查四原仓库文档确认角色卡正确实现已存在、世界书需新建、合并无丢失。
- 2026-07-01：**通读四仓库全部设计文档，以文档为准重写本 PLAN**——确立干净提示词为产品命根子、两平面执行机制、四层目标架构、各模块戒律；将 Core 去存储化/角色卡归属/多 provider/长期记忆/数据迁移从"开放题"降为"文档已定"，剩 5 条真正开放决策见 §4。
- 2026-07-01：**修正对"导入资产如何处理"的理解**（§3.3/3.5）——预设/卡/世界书是给 Agent 的**参考建议素材**（`SKILL.md:15-16` 全局约束），Agent 理解意图后按当前 LLM 自行适配，非机械回放。据此消解此前误列的"采样参数落脚点""预设顺序 vs 装配顺序"两个伪缺口；确立"特供预设跨模型适配"为相对酒馆的核心差异化。
- 2026-07-01：**深度审计——亲读三份承重文档全文（SKILL.md / 架构背景 / AGENT_BACKEND_PLAN），纠正靠子代理摘要导致的 5 处错误**：①删除"Core 去存储化是既定路线"错误论断——Core 文档明定保留进程内自带数据操作、standalone 自足，MCP-Server 是可选增强，数据能力有意并存；整合产品的数据真相归属改列 §4-1 地基未决项。②补回整段漏掉的**性能契约 §2.5**（防重蹈酒馆覆辙 7 硬约束 + Perf Spike 门）。③§1 补"loop = 纯净 subagent 编排器"最深表述。④降级"Gateway=未来核心"为 ChatGPT 非定论意见；补 Core 保持 UI 无关。⑤补回半永久 Blueprint/RP=UI Profile + 记录四仓不做物理 monorepo 吞并的原立场（用户 2026-07-01 新指令已更新之，且四原仓仍独立于 GitHub、乐高独立未破）。四仓本地已确认与 GitHub 完全同步，本地即权威。
- 2026-07-01：**用户强调重申（我一度过度旋转成"文档即圣经/戒律不可破"后）**——(1)项目=专为 AIRP 设计的 AI Agent 客户端；(2)四仓理念仅供参考；(3)四仓代码仅供参考；**一切以我们客户端的实际需求为准**。据此改框架：权威 = 我们的需求，四仓文档/代码/戒律全降为"参考"，纠正 §0/§1/§2 措辞。记忆库同步（[[feedback-integration-approach]]）。
- 2026-07-01：**再亲读三份承重文档（Gateway DESIGN / State-Protocol PLAN / AGENT_CLIENT_ASSESSMENT），新增 §2.6 现状真相+复用地图**——核心发现：四块从没端到端一起跑过（全用 mock 自测），整合是全新工作；Core daemon=后端 80% 已带测试、UI 最成熟只差换真 bus+跑 perf spike、Gateway streaming 是桩且从没接真后端、MCP 酒馆兼容基本假。复用地图给出从哪挖什么（带 file:line 参考）。
- 2026-07-01：**研究 Hermes Agent（Nous Research）自进化记忆，新建 [HERMES-MEMORY.md](HERMES-MEMORY.md)**——五支柱（有界常驻 md 记忆 MEMORY/USER + frozen snapshot 稳定前缀 + 80% 自动整理 / skills md+YAML / soul 动态人格 / crons / 自进化闭环）+ SQLite FTS5 `session_search` 非向量长程检索。发现：①几乎就是 Claude Code 自己的记忆+技能模式；②主要靠**扩展我们已有件**即可（封卷/persona/生态 skills）；③是**相对酒馆的核心差异化**（酒馆无跨会话学习）；④frozen-snapshot 稳定前缀坐实我们 §3.5 缓存纪律；⑤FTS5+摘要正合"RAG 暂缓"。更新 §3.4 加"越用越懂你"自进化记忆为核心卖点。另：确认 Tauri 桌面优先、web 后加端口（§0）。
- 2026-07-01：**最终形态澄清（用户）**——(1)目标=像 Codex/Claude Code 的完整 AI Agent 客户端，agent loop 是脊柱非可选，Agent 能力面=第三方扩展面（合一）；(2)未来可能适配 WebUI→引擎须无头独立网络服务、不嵌 Tauri，Tauri+web 都当客户端走同一协议。更新 §0/§3.8，记忆库新增 [[project-final-form-vision]]。这坐实"引擎+UI 两盒"拆法、且引擎须为独立 service。
- 2026-07-01：**实读酒馆仓库+文档（docs.sillytavern.app），新建 [TAVERN-PARITY.md](TAVERN-PARITY.md)**——(1)酒馆功能全集对标我们缺口（最大新建件=世界书插入引擎；应加：swipe/branch/Author's Note/Character's Note/Instruct Mode/Connection Profiles/宏/群聊调度等）；(2)落实硬需求"充分暴露接口无缝支持第三方"——研读酒馆完整扩展 API（getContext/事件总线/prompt拦截器/slash命令+STscript/函数工具注册/宏/消息格式化管线/生成API），对比发现我们 UI 侧 widget 系统更安全但引擎侧钩子空白；关键张力=受控开放(过capability门,推荐) vs 酒馆式无限制JS。更新 §3.8 + §4-9。
- 2026-07-01：**按用户要求亲读完全部 19 份文档，无死角**（补齐 Gateway ROADMAP/CUSTOMIZATION/AGENTBUS-ADAPTER/MCP-SERVER-REQUIREMENTS、mcp-server ROADMAP/deployment-tavern-agent/skills-vs-mcp/prompt-caching/configuration、State-Protocol protocol.md/extension-points/widget-authoring/GATEWAY-NEED/SECURITY、AIRPCLI README/AGENTS/deploy）。新挖两条硬事实并重写 §4-1/2/4：①**Core 单体已是完整 RP 后端**（自带正确数据层含正确 png_parser + 全 HTTP API，可 standalone，UI 可直连）→ 最简 MVP 后端可以只用 Core；②**推理路由缺口**——所有文档把 `chat.send` 映射到 MCP 数据工具（只存取不生成），但我们需要映射到 Core 推理，且 Gateway 纯 MCP 桥接不到非 MCP 的 Core（`/mcp/v1` 已剥离），无现成方案。据此推荐 MVP=UI→Core 直连。另记若干细节：mcp-server §0 澄清"组装 prompt 是 MCP 本职、在界内"（红线是不调LLM/不强制，非不组装）；mcp-server E 系列已知代码问题（挖它 storage 时的修点）；prompt-caching 的 `[[CACHE_BREAK]]`→`cache_control` 翻译在边缘；deployment-tavern-agent 是"保留酒馆当前端"的旁路部署，非我们的替代路径。
