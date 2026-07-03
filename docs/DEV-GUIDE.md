# AIRP 开发文档（交接给实现 Agent）

> **读者**：冷启动、无对话上下文的实现 Agent。本文自包含——照此即可动手，无需追溯任何对话。
> **配套设计文档（背景/依据，动手前通读）**：[PLAN.md](PLAN.md)（总设计+待决项）· [PARTS.md](PARTS.md)（四仓零件清单+file:line）· [TAVERN-PARITY.md](TAVERN-PARITY.md)（酒馆功能对标+扩展接口+解耦重组）· [HERMES-MEMORY.md](HERMES-MEMORY.md)（自进化记忆）。
> **真理顺序**：源码 > 本文 > 设计文档 > 对话。冲突时先改文档再继续。
> 最后更新：2026-07-01

---

## 0. 一句话与铁律（先读，任何时候不许破）

**我们做的是"专精 Role Play 的 AI Agent 框架"**：一个无头 Agent 引擎（Claude Code/Codex 级能力：bounded loop + 工具 + MCP + 记忆 + 技能 + 子agent + 扩展钩子）+ 一个可换 UI（当前 Tauri 桌面优先，未来暴露端口接 web），专精 RP。

**六条不变式（红线，实现中永不许破）：**
1. **干净提示词（灵魂）**：喂模型的**角色平面**只装 RP 数据（卡/世界书/预设/state/记忆/历史），**零 agent 脚手架**；工具定义/调用/结果走**控制平面**=模型 API 原生结构化字段（OpenAI `tools`/`tool_calls`/`tool` role；Anthropic `tools`/`tool_use`/`tool_result`），**永不拼进角色平面自然语言**。**不用 in-prompt ReAct**（把工具说明写进 prompt 文本 = 自我污染）。**CI 守 `subagent_context_has_no_orchestrator_noise`——违反即红，这个测试神圣不可删。**
2. **有界 agent**：loop 必有 step/token/成本/墙钟上限，任一触顶即停；可取消；每步流式可观测（不黑箱）。
3. **工具受控**：allowlist + capability 门；破坏性工具默认 dry-run 需确认；幂等键去重；同角色/资源并发写串行化。
4. **数据单一真相**：RP 数据引擎内一处存、一个真相（不要 Core 一份 + MCP-Server 一份并存——那是原仓为独立分发的设计，对我们是负担）。
5. **性能有界**（防重蹈酒馆覆辙）：见 §7 性能契约 7 条硬约束。**酒馆崩因是无界 DOM+单线程阻塞+内存泄漏，非算力；Tauri WebView2=Chromium，不会多吃硬件。**
6. **数据传输纪律（大数据不走窄管）**：大文件/大 blob（角色卡 PNG、世界书、预设、插件 blob、任何可能 >~64KB 的内容）**一律 path-first 或流式**——传**文件路径**让引擎读盘（引擎有磁盘访问），或分块/流式。**严禁大 blob 落进/驻留这三处**：① **模型上下文**（烧 token，社区实测"蓝屏级卡死"——MCP `png_path` 设计的由来，SKILL.md:118）；② **reactive store / Blueprint / 渲染树**（13MiB 串进状态树=内存/性能爆）；③ 日志/历史逐字节复制。
   - **精化（裁定 2026-07-02）**：被禁的是"**落进/驻留** store/模型/日志"，**不是"经过传输通道"**。一个**转瞬即转发给引擎、不 setState/不渲染/不落日志**的请求负载不算违反——即 base64 fallback 经 `intent`→bus→HTTP 立即转发引擎、结果只存 id，**合规**。**不必**为此另开 intent 之外的通道（Tauri 壳里 UI→引擎唯一通道就是 bus.dispatch(intent)，path 只是几十字节小串，照走 intent）。
   - base64 仅在**无真实路径**（未来 web/拖拽内存文件）时作 fallback；web 期优先用 multipart/二进制上传避 base64 的 33% 膨胀（延后）。**判据：数据要落进模型/store/日志之前，先问"它多大？大就传引用、且别驻留"。**

**扩展开放模型**：受控开放——丰富结构化钩子（工具/事件/宏/命令/技能）对第三方开放，但过 capability 门 + 沙箱；**拒执行 agent/第三方生成的任意代码**（UI 只渲染声明式 Blueprint，esm 第三方 widget 走 opaque-origin iframe 沙箱 + 用户同意）。

---

## 1. 现状（起点）

工作区 `D:\AIRP-Dev` 已被 PR #1（目录对齐）收成"两盒"结构——workspace 只剩 engine+protocol+ui（gateway/mcp-server 已从工作区移除，退回原始独立仓当零件库）：

```
D:\AIRP-Dev/
├── Cargo.toml    workspace（members：engine, protocol, ui/src-tauri）
├── engine/       ← 引擎（crate airp-core，源自 AIRPCLI）。自带 LLM adapter + agent loop 骨架 + orchestrator + 数据层 + 正确 png_parser + 完整 /v1/* HTTP+SSE
├── protocol/     ← 线协议契约（crate airp-state-protocol）：Envelope/Blueprint/widget/capability + Rust 绑定（TS 侧手镜像在 ui/src/protocol/types.ts）
├── ui/           ← Tauri+Vue UI（crate airp-ui）。widget/Blueprint/RFC6902 patch/虚拟滚动/沙箱/consent/打包 全代码完成；src-tauri/src/bus.rs = UI↔引擎桥
├── data/         ← 运行时 RP 数据（角色卡/预设/会话/世界书…；含个人数据，见 §0 泄露清理待办）
└── docs/         ← 设计文档：本 DEV-GUIDE + PLAN + PARTS + TAVERN-PARITY + HERMES-MEMORY

零件库（原始独立仓，随用随取，不在 workspace）：
  D:\AIRP-Gateway     ← MCP client/传输/安全硬化（第三方 MCP 生态用，要时挖）
  D:\airp-mcp-server  ← ★核心·未来刚需★ 完整 RP 数据管理面（38工具/12工作流提示词/19资源+数据模型）= engine 数据层+agent工具的规格。**能力要融进 engine**（非外部连），见 [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)。仅局部解析有 bug（见 §5/§6），修不是丢。
```

**关键事实（决定起点）：**
- **四块从没端到端一起跑过**（各自 mock 自测）——**Phase 0（本 PR #2）正是首次让 UI↔引擎真跑通**：UI `BusRelay` 已从 mock 改为 HTTP 直连引擎 `/v1/chat/completions`，流式回填 `w-chat`。
- **`engine` 已是完整 RP 后端**（80% 后端功能已实现且带测试）：`/v1/chat/completions`(单回合 SSE)、`/v1/agent/run`(多步 loop M_AGENT-1)、characters/sessions/scenes/state/history/rollback/regen/settings；adapter 双 provider(OpenAI+Anthropic)；orchestrator 装配；fsm+xml_unpacker 流过滤；封卷；**png_parser 正确解析酒馆卡**。
- **`D:\airp-mcp-server` = 未来刚需·能力要融进 engine**（用户 2026-07-02）：它的 38 工具/12 工作流提示词/19 资源 + 数据模型是 engine 数据层 + agent 内置工具的**完整规格**（engine 现只重实现了子集）。**融入=M_AGENT-2 把数据操作包成 agent 工具**，详见 [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)。**注意区分**：它的**角色卡/世界书解析确有 bug**（zTXt-only、Vec 结构错）——移植时改用 engine 的 png_parser、修解析；但这是局部要修的点，**不是丢掉整个 MCP-Server 的理由**。

**起点决策（已落地）：引擎 = 以 `engine` 为核演进，并把 `D:\airp-mcp-server` 的完整能力面（38工具/12提示词/19资源+数据模型）逐步融入 engine 成原生 agent 工具+工作流+数据层**（主载体 = M_AGENT-2，见 [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)）；`D:\AIRP-Gateway` 留作 engine 作 MCP client 接**第三方** MCP 的参考；`ui/` 直接用。

---

## 2. 目标架构（2 盒）

```
┌─ UI（Tauri 桌面优先；未来 web）──────────────────────────────┐
│  Vue：Blueprint 渲染 · widget 注册表 · RFC6902 patch store    │
│  · 虚拟滚动 · esm 沙箱 · consent 门                           │
└──────── State-Protocol Envelope（Tauri IPC 现 / SSE / 将来 WS）┘
                         ↕  传输无关线协议（web = SSE 路径）
┌─ AIRP 引擎（无头独立网络服务 HTTP/SSE/WS）────────────────────┐
│  ■ Agent 内核（通用框架·脊柱）                                 │
│    bounded loop（纯净 subagent 编排器）· adapter（多provider流式）│
│    原语面：Tool(内置+MCP client) · Memory(三层,§6) · Skill      │
│    (agentskills.io兼容) · Event Hook · Prompt-Interceptor       │
│    · Macro · Subagent  ← 此面 = agent 能力 = 第三方扩展面        │
│  ■ RP 特化层                                                   │
│    干净提示词纪律(两平面) · RP数据层(单一真相) · 酒馆导入        │
│    (解耦重组,§5) · orchestrator 装配                            │
│  ■ HTTP/服务层：鉴权 · 限流 · capability 强制 · 线协议端点      │
└───────────────────────────────────────────────────────────────┘
                         ↕  （可选，要时才接）
         第三方 MCP server / 工具生态 · 外部记忆 provider
```

**内核原语面 = 扩展面（合一）**：Tool/Memory/Skill/Hook/Macro/Subagent 既给 agent 用、也给第三方接。这是"充分暴露接口、无缝支持第三方"的落地——不是另造一套扩展系统，而是把 agent 自己的能力面开放出去。第三方工具优先走 **MCP server**（跨语言、进程隔离，比酒馆同进程 JS 更无缝更安全）。

**RP 特化叠在内核上、不侵内核**：理论上换掉 RP 数据层+导入+装配就能做别的 agent 应用。但**当前只交付 RP，不过度泛化**。

---

## 3. 引擎详细设计

### 3.1 Agent 内核

- **loop = 纯净 subagent 编排器**（这是干净提示词的落地形态）：
  - 真正的 RP 书写交给**引擎原生派生的隔离纯净 subagent**——只装 RP 数据，无工具说明/规划指令/编程噪声。
  - 单回合 = 派一个 subagent；多步 = 按需派多个 + 中间夹工具（取世界书 → 掷骰 → 派 A 写 → 派 B 写 → 落 state）。
  - 协调器自身多步状态活在**它自己的上下文**，每个书写 subagent 是**全新纯净上下文**。两层物理隔离。
  - **必须引擎原生派生**（不能用第三方 runtime 的 subagent，那仍裹它的脚手架）。
  - 复用 Core 现成资产当库：`adapter::call_streaming_api_auto`、`chat_pipeline::prepare_pipeline` / `build_sse_stream` / `run_finalize`。**一行 SSE/provider/拆包都不重写**（Core 已成熟）。
- **控制平面 = 结构化 tool-calling**：OpenAI/Anthropic 原生工具字段。`<action>` XML 作不支持结构化工具的模型的回退。
- **原语面（新建/补全，MVP 后立骨架）**：
  - **Tool**：`Tool` trait + `ToolRegistry`（Core `agent/tools.rs` 有骨架，现仅 mock echo）。元数据 readonly/mutate/destructive/append。内置工具（数据读写/掷骰/…）+ MCP upstream 工具（`McpClient`，可从 `parts/gateway` 挖）。
  - **Memory**：三层，见 §6。
  - **Skill**：md+YAML front matter，渐进披露；**兼容 agentskills.io 开放标准**（白捡生态）；从经验自建、反馈更新。
  - **Event Hook**：引擎发全生命周期事件（消息收发/编辑/swipe、生成起止、流式 token、世界书命中、工具调用、state 变更），第三方订阅。对标酒馆 `eventSource`。
  - **Prompt-Interceptor**：生成前把装配好的**角色平面**交已授权扩展过目/改——但**守铁律1**：拦截器改 RP 数据层，不能偷塞脚手架，且过 capability 门。
  - **Macro**：`{{char}}/{{user}}/{{roll}}/{{random}}/{{time}}` + 第三方注册自定义宏，装配层展开。
  - **Subagent**：引擎原生派生隔离 subagent；支持"脚本经 RPC 调工具、压成零上下文成本一轮"（Hermes 式）。

### 3.2 RP 特化层

- **数据层（单一真相）**：`data/` 目录布局（沿用 Core，见 §3.4）。角色卡/会话/世界书/state/记忆/预设/场景/插件。以 Core 数据层为基，吸收 mcp-server 的沙箱（`safe_resolve_for_write`+`validate_id_segment`）+ 插件零schema + 域模型优点。
- **orchestrator 装配**：默认序 `card → preset → checkpoint gating → known context → 卷 → lorebook`（Core `orchestrator/`）。**这是引擎的干净装配默认序，非回放导入预设的排版**（预设是建议素材，§5）。多角色场景每 NPC 独立纯净上下文。
- **载荷按可变性排序**（缓存友好，与 §6 记忆 frozen snapshot 同原理）：稳定块（persona/preset/lorebook）在前、易变块（live state/per-turn）在后，稳定前缀跨轮字节一致。缓存翻译（`[[CACHE_BREAK]]`→Anthropic `cache_control`）在 adapter 层。

### 3.3 HTTP/服务层

- 无头网络服务：Core daemon 已有（axum + `/v1/*` + 鉴权 `AIRP_ACCESS_KEY` + 限流 10req/s）。**web 就绪的关键：引擎是独立 service，不嵌 Tauri**（Tauri 把引擎当 sidecar 打包）。
- **UI↔引擎线协议**：倾向**复用 State-Protocol Envelope**（`protocol` 已有 Rust/TS 绑定，UI 已配套）。引擎实现 AgentBus 面：上行 `POST /airp/dispatch`(Envelope) + 下行 `GET /airp/stream`(SSE)，或 Tauri IPC。**注意：intent `chat.send` 要路由到引擎的推理 loop（生成），不是路由到某个 MCP 数据工具**——这是原 agentbus demo 的缺口，必须重定。
- 安全默认坑（Core deploy A2-3）：默认无鉴权 + CORS `*` 有本地 CSRF/DNS-rebind 风险。桌面本地默认可接受，但要文档化；对外必须设 `AIRP_ACCESS_KEY`。

### 3.4 数据目录布局（沿用 Core）

```
data/
├── settings.json
├── characters/{id}/
│   ├── card/ (card.json + card.png)   greetings/   world/lorebook.json
│   ├── state/ (live.json + history.jsonl + schema.json)
│   ├── gating/checkpoints.json        analysis/    memory/(current.md/index.md/volumes/vol_*.md)
│   └── sessions/{sid}/ (meta.json + chat.jsonl + memory/)
├── presets/{id}/ (preset.json + preset.md + regex/*.json + analysis/)
├── scenes/{id}/ (scene.json + memory/ + world/lorebook.json)
└── plugins/{name}/ (任意文件树，零 schema)
```

---

## 4. UI 详细设计（Tauri 桌面优先）

- **技术**：Tauri 2 + Vue（`ui/`）。只渲染引擎下发的 **Blueprint**（声明式 JSON），**不执行 agent 生成的代码**。
- **已实现（代码完成、运行时未验证）**：Widget Registry（vue/module/esm 三类）、BlueprintRenderer、WidgetHost（错误隔离）、RFC6902 state store（`test` op 非事务，注意）、首方 widget（chat/emotion/memory/inventory/quest/map/card + clock）、虚拟滚动（`virtual-window.ts` `computeWindow`）、esm 沙箱（opaque-origin iframe）、consent 门（授权绑 `{type,version,source}` + localStorage 持久化）、边界 guard、Tauri `.exe` 打包（已验证产出 exe+NSIS）。
- **必做（MVP 第一步）**：把 mock `BusRelay`（`ui/src-tauri/src/bus.rs`）换成**连真引擎**——加 `SSEBus`（非 Tauri 环境）或让 `BusRelay` 内部 HTTP 调引擎（Tauri 壳内 IPC→Rust核→引擎）。`bus-factory.ts` 已按 `__TAURI_INTERNALS__` 选 bus，接线点清楚。
- **半永久 Blueprint / RP=UI Profile**：首次进 RP → agent 推导 Blueprint → 存储+UUID；同一 RP 以后直接读。RP 类型定画像（恋爱→聊天、经营→数据面板、桌游→卡牌、跑团→属性栏）。
- **必须跑 Perf Spike**（见 §7）——代码有虚拟滚动但从没真跑过 10 万条验证。

---

## 5. 酒馆格式导入（硬需求 + 解耦重组）

**原则**：用户手动导入文件，必须兼容酒馆现有格式。**但不照搬酒馆机械管线——每个功能拆成"底层能力"再用 agent 原语重组**（详见 [TAVERN-PARITY.md](TAVERN-PARITY.md) §4）。

| 类型 | 状态 | 做法 |
|---|---|---|
| **角色卡（V2/V3）** | ✅ 白捡 | 用 Core `png_parser.rs`（已正确）+ `types.rs` `TavernCardV2`（spec/data 封装 + system_prompt/alternate_greetings/character_book + v1 归一化）。确保导入路径用它，别用 mcp-server 坏版 |
| **世界书** | 🆕 **最大新建件** | 四仓皆无完整实现。酒馆是 `{entries:{"0":{...}}}` uid-keyed object。**重组**：解析全字段进数据层 + 把 position/depth/selective/constant/probability/递归 当**给 agent 的建议元数据 + 检索 Tool**（agent 生成中按需调 `lorebook_lookup`），**非硬编注入管线**。MVP 可先"解析+关键词触发（aho-corasick）"，插入语义增量补 |
| **预设** | 🔧 | 预设是**建议素材非机械回放**：agent 理解意图按当前模型适配（`analyze_preset`/`tune_preset` 思路）。采样参数=adapter 建议值。正则脚本→**消息格式化 Hook**。用 Core/mcp-server 的 `preset_regex.rs`（正确骨架），杀掉 `preset.rs` 里瞎起名的 `RegexScript` 冲突版，补 trimStrings/minDepth/maxDepth 等字段 |

**已知代码修点**（挖对应零件时一并修，详见 PARTS.md §M）：mcp-server 角色卡 zTXt-only（用 core 替换）、世界书 Vec 结构、预设两套 RegexScript 冲突、state 写入不 clamp（模型可写越界值，`persist_live_state` 落盘前按 schema clamp）、list 排序漂移、import_preset 绕沙箱、constant_time_eq 长度侧信道、错误码全归 INTERNAL_ERROR、并发写无文件锁、RFC6902 test 非事务。

---

## 6. 自进化记忆（"越用越懂你"—— 核心差异化，详见 [HERMES-MEMORY.md](HERMES-MEMORY.md)）

对标 Hermes Agent。酒馆每轮重灌静态卡+世界书、无跨会话学习；我们让 RP **复利**：玩得越久，积累情节记忆+用户模型+书写技能+角色深度。

**三层记忆：**
1. **常驻有界记忆（🆕）**：每角色/存档一份有界 md（RP-MEMORY=情节/关系/世界事实 + USER=用户文风偏好），always-injected 当**稳定前缀**（**frozen snapshot**：本轮改动落盘、下轮才进 prompt→合缓存纪律），超 80% 容量 agent 自动整理合并。从对话**自动抽取**更新（"越用越懂你"的魔法）。
2. **归档卷（✅ 已有）**：Core 封卷 volumes——长会话压缩归档。封卷永不自动（阈值信号→loop 拍板）。
3. **历史检索（🆕）**：SQLite FTS5 全文 + LLM 摘要的 `session_search`。**非向量 RAG，轻量**——回忆任意历史片段。RAG 暂缓。

**Soul 动态人格演化（已定加入·第二档优先级）**：用 **base+drift overlay 双层**（复用 Core User Persona M_UP 模式）——原角色卡=作者写死的不可变 base（`persona.lock` 式契约），soul-drift=学习 overlay 叠加注入、**不改原卡**、可读可审可回滚。演化角色性格深度/说话习惯/关系态度 + agent 书写风格贴合用户。守铁律1（drift 是 RP 数据进角色平面正当，抽取逻辑走控制平面）。

**RP 技能自建**：怎么写角色/场景套路/用户文风，从经验自建、反馈更新，接进 Skill 注册表（agentskills.io 兼容）。

**守铁律1**：记忆进角色平面是 RP 数据（正当）；抽取/整理/演化的控制逻辑走控制平面，不脏化角色 prompt。

---

## 7. 性能契约（硬约束，UI 层不许破，`背景整理 §6`）

**7 条：** ①聊天/长列表强制虚拟滚动（只渲染视口 DOM）②全量历史真相在引擎、UI 窗口分页拉取 ③状态更新 patch 优先、禁全量重灌 ④稳定 ID 做 key ⑤重计算留 Rust（diff/正则/装配/持久化离渲染线程，JS 重活走 Web Worker）⑥流式增量追加渲染、禁每 token 重解析整段 markdown ⑦内存卫生（离屏 widget 销毁、listener 清理、消息窗口封顶）。

**Perf Spike 验证门（开发前尽早做）**：Tauri 壳灌 10 万条假消息，验收 60fps + 内存封顶（不随历史线性涨）+ 流式追加不卡。过了才锁 Tauri+Vue。

---

## 8. 开发路线（分阶段 + 验收标准）

> 原则：每阶段自身可跑、可测、可验收。**MVP 优先证明端到端，再谈扩展。**
>
> **📍 当前状态（2026-07-02）**：Phase 0 ✅ 已完成合并（PR #2）。**下一步 = Phase 1，从 Task 1.1（角色卡导入 UI）开始**，见下。

### Phase 0 · 引擎+UI 直连，跑通一次干净对话（MVP 地基）—— ✅ 已完成（PR #2 合并入 main）
- 已落地：`ui/src-tauri/src/bus.rs` 的 `BusRelay` 从 mock 改为 HTTP 直连引擎 `/v1/chat/completions`，消费 SSE、按 `w-chat` scope 流式回填；角色列表 `characters.list`。
- 已验证：真实酒馆卡端到端对话通；`cargo test -p airp-ui`(5) + `vitest`(92) + `vue-tsc`(0) + **`subagent_context_has_no_orchestrator_noise` ✅**（神圣不变式）全绿。
- **遗留到 Phase 1**：角色卡导入 UI（Phase 0 用预置 `data/characters/` 卡验证）；`.exe` 打包后真跑 GUI；Perf Spike 10 万条（§7）；reasoning/action 渲染；chat_lock → id-keyed 重构（Task 1.2）。

### Phase 1 · 酒馆导入完整 + 基础会话
> 按下列顺序推进，每个 Task 自身可验收。**从 Task 1.1 开始。**

**Task 1.1（👉 从这里开始）· 角色卡导入 UI** —— 补完 Phase 0 只用预置卡的缺口
- UI 加"导入卡"：Tauri 文件对话框拿路径 → `characters.import` intent（只带路径）→ 引擎读盘 + png_parser 解析落盘 → 刷新 `characters.list`。
- **⚠️ 必须 path-first，禁止把整卡 base64 塞进 store（对齐 MCP `png_path` 设计，SKILL.md:118）**：
  - **做法**：`@tauri-apps/plugin-dialog` open 拿**绝对路径** → 传**路径**（几十字节）给引擎 → **引擎读盘 + png_parser 解析**。
  - **禁止**：UI 里 `FileReader`→base64 整卡进 reactive store/渲染树。base64 仅无路径（未来 web）时 fallback，转瞬转发引擎、不 setState。
  - 路径小串照走 `intent` 通道即可（不必另开通道）；store 只放结果（角色 id 列表）。
- **实施裁定（审计 2026-07-02，回该实施 agent 的 Q1-Q3）**：
  - **Q1 = 是，须改引擎端点**（我此前"引擎已内化 png_path"是想当然，已核实 `handlers.rs:22` `ImportCharacterRequest` 只有 `card_json`/`card_png_base64`、无 path）。给它加 `card_path: Option<PathBuf>`（主路径），handler `fs::read(path)` 后复用现有 `import_card_to_disk` 解析；保留 base64/json 作 fallback。
  - **Q2 = 合规**（见不变式6 精化）：fallback base64 经 intent→bus→HTTP 立即转发引擎、不 setState = 不违纪律；**不必**另开 intent 之外通道。
  - **Q3 = 引擎从 `card.name` 派生 id + UI 可选覆盖**（采该 agent 的 (b) 变体）：`character_id` 从必填改**可选**；不传时引擎解析卡后 slugify `data.name` 当默认 id 并返回，传了则用 UI 的。**注意**：① slugify 须 sanitize 成合法 id_segment（`validate_id_segment` 实测：点号非行首 OK 如 `v1.2`，只拒空/行首点/`..`/`/ \ : \0`——比该 agent 以为的宽松，主要处理 `/ \ :`、空格、行首点）；② **重名碰撞**须处理（已存在则加后缀 `-2` 等，勿覆盖）。
  - 范围认可该 agent 的"五"：改引擎端点(加path+id可选) + bus 加 `characters.import` 分支 + CharactersWidget 导入按钮 + 确认 `card/raw.json` 已写（**已核实 `handlers.rs:273` 确在写，最小 sidecar 满足**）。不做 agent 分析/完整 canonical 骨架/base64 UI。
- **实施裁定 2（审计 2026-07-02，回 Q4-Q6 · UI 拿路径方式）**（引擎+bus 已改完且与此解耦，仅动 CharactersWidget + 依赖）：
  - **Q4 = 批准选项 A（`@tauri-apps/plugin-dialog` + `tauri-plugin-dialog`）为主入口**。它是 Tauri 2 拿"用户主动选文件路径"的官方一等正路，风险低；选项 B（HTML file input）技术上拿不到绝对路径，**排除**；选项 C（拖放 `onDragDropEvent`，零新依赖）**批为可选辅助入口、后补**，不作主入口（点按钮选文件才是预期 UX）。不强制 D。当前工作区已按该路线落地依赖与权限：`package.json` / `Cargo.toml` / main.rs `.plugin(init())` / `capabilities/default.json`，继续沿此路线实现 UI 即可。（AIRP-Core 打包出 .exe 且可运行已验证，此路可测。）
  - **Q5 = 只授 `dialog:allow-open`（最小权限，守不变式3）**。`allow-save` 等真做导出 round-trip 时再加，现在不预授。
  - **Q6 = 桌面期 path 直喂引擎合规**（引擎是本地 sidecar 进程、同机同盘，`fs::read(abs_path)` 有效）。web 期/远端引擎时 path-first 失效 → 优先走 multipart/streaming upload；base64 只作无二进制上传能力时的最后 fallback（引擎保留 `card_json`/`card_png_base64` 入参是兜底，不是推荐主路）。
  - **🔒 审计追加护栏（该 agent 没提，但必须记）**：`card_path` = **引擎侧读任意绝对路径**。当前"本地可信 UI + 用户经对话框亲选"模型下可接受；但**严禁暴露给不可信调用方**（未来 web 客户端 / 能发 import intent 的第三方 widget）——否则是任意文件读漏洞。**规则**：`card_path`（服务端任意路径读）只给**可信本地 UI**；不可信/远端调用方一律只能 multipart/streaming upload，base64 最后兜底，**不得走服务端任意路径读**。引擎转多客户端/web、或放开第三方 widget 发 import intent 时，`card_path` 必须 capability 门控或对不可信方禁用。
- **验收**：从 UI 选真实酒馆 PNG 卡导入成功、出现在列表、可对话；`character_book` 一并入库；**传给引擎的是路径非 base64**（大卡不卡顿、store 无大字符串）；`card/raw.json` 有原始留存（sidecar）。
- **🔒 未来必做待办（PR #3 已落地 path-first，本项未做、审计豁免当前阶段）**：引擎 `/v1/characters/import` 的 `card_path` 端点目前**无调用方可信度校验**——任意能发 HTTP 请求者均可令引擎读任意绝对路径。当前单本地 UI + 同机 sidecar 模型下审计豁免；但**任一以下条件触发时必须先补门控再放行**：①引擎转多客户端/web（引擎不再只听本机）；②放开第三方 widget 发 `characters.import` intent；③`card_path` 暴露给非可信调用方。门控方式：capability/白名单限制 `card_path` 仅可信本地 UI 来源，或对不可信方强制走 base64/multipart 上传分支而禁用 path 分支。**未来审计 agent 应主动核查此待办是否已补，未补而条件已变 = 漏洞。**

**Task 1.2 · chat 消息改 id-keyed 寻址（去掉 Phase 0 的 chat_lock）** —— 先于会话操作与多角色
- **背景**：Phase 0 的 `BusRelay` 用 `Arc<tokio::sync::Mutex<()>>`（chat_lock）串行化所有 chat 流，因为流式回填靠 `replace /messages/-/text`（"最后一个元素"寻址），`-` 在 apply 时才解析——并发流会互相覆盖。锁治标：① 全局串行挡住多角色 N 个 NPC 并发流式（§3.6），② user_echo 锁外同步发、顺序仍可能小错乱。
- **做法**：chat scope 消息模型数组 → **id-keyed map + order 数组**（`{messages:{"a1":{...}}, order:["u1","a1"]}`），每流 patch 自己那条 `replace /messages/{id}/text`，**删 chat_lock**。改动面：`ui/src-tauri/src/bus.rs`（patch 构造 + 去锁）、`ui/src/state/store.ts`、`ui/src/widgets/ChatWidget.vue`（渲染改 `order.map(id => messages[id])`）。
- **验收**：两条 chat.send 并发不串扰、顺序对、无锁。

**Task 1.3 · 世界书引擎（最大新建 · 关键路径）** —— 见 §5 + [PARTS.md](PARTS.md) F
- 解析酒馆 world info（`{entries:{"0":{...}}}` uid-keyed object，全字段：keys/secondary_keys/position/depth/order/probability/selective/constant/递归…）入数据层；关键词触发用引擎已有 aho-corasick 扫描。
- **解耦重组（守 §5 原则）**：position/depth/selective 等**机械插入语义降为"给 agent 的建议元数据 + 检索 Tool**（agent 生成中按需调 `lorebook_lookup`），非硬编注入管线。MVP 先"能解析 + 关键词触发注入"，插入语义增量补。
- **验收**：真实酒馆世界书文件导入解析无丢字段；对话中相关条目按关键词命中注入角色平面；守干净提示词不变式。

**Task 1.4 · 会话操作** —— 接引擎已有端点
- swipe（多候选，engine 有 regen）、编辑、regen、继续、删除/隐藏；reasoning 块显示（engine SSE 的 `think_chunk` Phase 0 已解析未渲染，这里渲染 + 折叠）。接 `POST /v1/chat/{history,rollback,regen}`。
- **验收**：swipe/编辑/regen 可用；一次多轮 RP 顺畅；reasoning 可折叠显示。

**Task 1.5 · 预设导入** —— 见 §5 + §3.3
- 正确解析酒馆预设（结构块 + 正则脚本），当**建议素材**交 agent；正则→消息格式化 Hook。杀掉重复的 `RegexScript` 冲突版，补 trimStrings/minDepth/maxDepth 等字段。
- **验收**：真实酒馆预设文件导入；正则后处理生效；预设作 agent 适配素材（非机械回放）。

**Phase 1 总验收**：从 UI 导入真实酒馆卡/世界书/预设并生效；swipe/编辑/regen 可用；一次多轮 RP 顺畅；干净提示词不变式全程绿。

### Phase 2 · 自进化记忆 + Soul + 扩展接口地基
- 三层记忆（常驻有界 + 用户模型自动抽取 + session_search FTS5）；Soul base+drift 演化（第二档）。
- 扩展接口骨架：**事件总线 + 函数工具注册（走 MCP）+ 宏系统**（尽早立，第三方生态越晚开接口越难改）；agentskills.io 兼容技能。
- **验收**：跨会话记忆生效（关掉重开记得你）；用户偏好自动积累；一个第三方 MCP 工具能接入并被 agent 调用；一个第三方技能能装载。

### Phase 3+ · 酒馆功能补齐 + 扩展态 + web
- Author's Note/Character's Note/Instruct Mode/Connection Profiles/群聊调度；slash 命令+脚本+Quick Replies；消息格式化管线。
- 扩展态（走扩展接口，不进内核）：TTS/STT/图像生成/翻译/Web搜索/立绘/Data Bank-RAG。
- **web UI**：引擎暴露端口，web 前端复用同一线协议（SSE 路径）。

---

## 9. 测试要求（不可省）

- **干净提示词 CI 不变式**：`subagent_context_has_no_orchestrator_noise`——断言送进 adapter 的角色平面 prompt 无脚手架标记。**神圣，不许删/改弱。**
- **格式导入 fixture**：用**真实酒馆导出文件**（PNG 卡/世界书 JSON/预设 JSON）做测试样本，不是自造的。
- **Perf Spike**：10 万条假消息 60fps + 内存封顶。
- **数据传输纪律门（§0 不变式6）——现为 review 门，未来落 workflow**：
  - **现在（AIRP 无 CI）= 强制 PR review 检查项**：任何导入/大数据改动，review 必须核对——传给引擎/模型/前端的是**路径/引用**还是**大 blob**？intent/Envelope/store 里有没有塞 base64 或大字符串？有=打回。
  - **未来 = 自动化门禁（workflow）**：立 CI/lint 检查——(a) 静态扫：`emit(intent…)` / `dispatch` / setState 的 payload 不得含 base64 大字段或 >阈值字符串；(b) 测试：导入大文件后断言 state store 无大字符串、intent 体积有上限；(c) 引擎侧断言 import 接口收路径而非内容。**这条纪律优先级足够高，应尽早从"review 门"升级为"自动门"。**
- 沿用 Core 现有：FSM proptest（chunk 边界独立）、wiremock mock 上游 SSE 集成测试。

---

## 10. 构建环境（本机 —— 已验证可本地 check+test）

- **默认工具链 `stable-x86_64-pc-windows-gnu`**（本机无 MSVC linker，用 GNU）。**三个 env 必须都指 D 盘**，漏 `CARGO_HOME` 会让 cargo 落到 `C:\Users\<user>\.cargo` 用错工具链、build script 报 SxS `os error 14001`：
  ```powershell
  $env:RUSTUP_HOME = "D:\.rustup"
  $env:CARGO_HOME  = "D:\.cargo"          # ← 关键，别漏（漏了就 SxS 14001）
  $env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
  cd D:\AIRP-Dev
  cargo check -p airp-ui                                               # 已验证 exit 0
  cargo test  -p airp-ui                                               # 已验证 5 passed
  cargo test  -p airp-core subagent_context_has_no_orchestrator_noise  # 神圣不变式，已验证 ok
  ```
  用**默认 target dir**（`D:\AIRP-Dev\target`）即可，本轮无 os error 5、无需重定向 `CARGO_TARGET_DIR`。Linux CI 用 `CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu`。
- **本机工具链自检（2026-07-03）**：`D:\.cargo`、`D:\.rustup`、`D:\msys64`、`D:\nodejs`、`D:\npm-global` 均存在；当前 shell 的 `cargo/rustc/rustup` 指向 `D:\.cargo\bin`，`node/npm` 指向 `D:\nodejs`。**不要把 Rust/Node/npm 全局依赖、缓存或构建工具塞回 C 盘**；若命令试图写 `C:\Users\<user>\.cargo`、`.rustup` 或 npm 全局/cache，先停下来改 env/prefix。
- **本地 check + test 都能跑**（上面实测全绿）——不必只靠 CI。审计 bot 已下线（2026-07-03），PR review 由开发者自审 + 人工承接，不阻塞在"等审计 bot"；自审按 `AGENTS.md` 的 Audit Agent Charter 三原则（独立 / 可提己见 / 可质疑历史并查证）。
- 引擎启动：`cargo run -p airp-core -- daemon --port 8000`。配置三层合并 default→`data/settings.json`→env→request，env 有 `AIRP_ENDPOINT`/`AIRP_API_KEY`/`AIRP_MODEL`/`AIRP_ACCESS_KEY`。UI 侧 `BusRelay` 默认连 `http://127.0.0.1:8000`，`AIRP_ENGINE_URL` 可覆盖。

---

## 11. 工程约定（沿用 Core，保持）

- `pub(crate)` 内部模块（fsm/xml_unpacker/volume_store/volume_manager 等），不对外暴露。
- **热路径无 `Arc<Mutex>`**：`MutableConfig` 用 `std::sync::RwLock`；设置热重载走 `POST /v1/settings`。
- **JSONL chat log**：`OpenOptions::append` 唯一写路径，O(1) 追加。
- **newtype ID**：反序列化即校验（`CharacterId`/`PresetId`/`SessionId`/`SceneId`），下游免重复校验。
- `estimate_tokens` 是 ±30% 启发式（非真 tiktoken）；预算阈值留安全边际，或接真 tokenizer。
- 三处对齐（改线协议时）：`protocol/schema`（真相）→ Rust 绑定 → TS 绑定 → 更新 spec + examples。

### 11.1 工作流与提交纪律（硬约束，所有实施 agent 必守）

- **一 Task 一分支 → PR，禁止把代码 WIP 摊在 `main` 工作树上**：
  - 动手前 `git checkout -b <phase-x-task-name>`（如 `phase-1-card-import`）。
  - **代码改动**（`engine/` `ui/` `protocol/` 等）**一律走分支 → 本地测试绿 → PR → 合并**，**绝不直接 commit/推 `main`**，更不许把改了一半的代码留在 `main` 的工作树里（会跟别的 agent 踩脚、污染共享树）。
    - **审计环节现状（2026-07-03 更新）**：原"审计 bot 复核"已下线（bot 不存在）。PR 现由**开发者自审 + 人工 review** 承接——本地测试全绿（含神圣不变式）即可开 PR，由人决定合并，不阻塞在"等审计 bot"。未来若重新引入审计 agent，以 `AGENTS.md` 的「Audit Agent Charter」为其入职守则（独立审计 / 可提自己的想法 / 可质疑历史决策并查证）。开发 agent 自审时也应按该 Charter 三原则自我要求，而非机械对照本文档放行。
  - **仅文档**（`docs/`、`*.md`）改动可直接 commit `main`（低风险、无 CI），但保持独立 commit、别夹带代码。
  - AIRP 仓**无 CI**，本地测试 = 唯一门：PR 前必跑 `cargo test -p airp-core`（动引擎时）+ `subagent_context_has_no_orchestrator_noise`（神圣不变式）+ 相关 `cargo test -p airp-ui` / `vitest` / `vue-tsc`。
- **提交卫生**：
  - **只 `git add <明确文件>`，禁止 `git add -A` / `git add .`**——会把垃圾/临时文件（如 `nul`、`_check.bat`、编辑器/构建产物）扫进提交。
  - 提交前 `git status` 核对暂存清单；发现游离垃圾文件先清（Windows 保留名 `nul` 用 `del \\.\nul`）或加 `.gitignore`。
  - 引擎运行时产物（`config.json`/`quota.json`/`data/*/history/` 等）已在 `.gitignore`，别再入库。
- **个人/敏感数据不入库**：角色卡/预设/真实聊天记录/API key 不进提交（历史泄露教训见记忆库；`data/` 下个人数据待清理）。

---

## 12. 动手前必须拿到用户拍板的开放决策（不要擅自定，见 PLAN §4）

1. **引擎内数据层熔合设计**：以 Core 为基吸收 mcp-server 数据域，具体怎么熔（工程取舍，Phase 1 前定）。
2. **UI↔引擎线协议**：复用 State-Protocol Envelope（推荐）vs 简化自定义。
3. **纯净度代价**：干净提示词把靠 in-prompt-ReAct 的纯文本模型挡在 loop 工具外——接受（纯净优先）还是留"污染模式"开关。
4. **capability 引擎侧强制时机**：MVP 先做还是随扩展面一起。
5. **世界书插入引擎完整度**：MVP 先关键词触发、增量补完整语义，还是一步到位。

> Phase 0（引擎+UI 直连跑通对话）方向已定、可直接动手；上述决策影响 Phase 1+，动到时先问用户。
