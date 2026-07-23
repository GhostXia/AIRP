# 能力融入通则：四仓库"好能力"→融进 agent 客户端

> **战略筛选原则，不是全量复制授权**：所谓“好能力”必须通过当前 AIRP 用户需求、共享 domain service、最小授权和可验证验收四道筛选。源仓库存在不等于本仓应立即复制。当前优先级见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。
> 用户 2026-07-02：**四个源仓库里凡有"好能力"的，都应融入 agent 客户端**（不止 MCP-Server）。
> 原则：客户端 = engine（agent 内核 + 数据层 + 工具/工作流）+ UI（Tauri+Vue）两盒。四仓库的好能力**拆解重组进这两盒当原生能力**，而非当外部服务连、也非保留其"独立分发"的自我约束。
> 参见：[SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)（四源项目资产吸收/北极星降级）· [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)（MCP 详细 catalog）· [PLAN.md](PLAN.md) · [DEV-GUIDE.md](DEV-GUIDE.md)。
> 最后校准：2026-07-16；能力交付状态只看 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)

---

## 逐仓库：好能力 → 融入哪

### 1. AIRP-Core（`engine`）—— ✅ 已融（是内核本身）
客户端内核就是它：`adapter`(双provider流式) · `chat_pipeline`(三段) · `orchestrator`(装配) · `fsm`/`xml_unpacker`(流过滤) · agent loop(M_AGENT) · `volume_*`(封卷) · `png_parser`(正确卡解析) · `scene` · 数据层。**已在 engine，无需再融**——它就是融入的载体。

不继承：Core 的"独立、开源、乐高式 Agent 后端 / standalone 参考大脑"产品北极星。AIRP-Dev 中 Core 是 engine 主核，服从 AIRP RP 产品闭环。

### 2. AIRP-MCP-Server —— 融数据管理面（详见 [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)）
**38 工具 / 12 工作流提示词 / 19 资源 + 数据模型** → AIRP engine 的候选**内置 Agent 工具 + 工作流/技能 + 数据读 API + 数据层经验**。主载体 = M_AGENT-2；按当前需求独立重构，并修正来源项目已知解析问题。

不继承：纯 MCP 数据层、不调 LLM、不做推理、决策完全下放外部 Agent 的边界。AIRP-Dev 要把自有 RP 数据能力内化进 engine。

### 3. AIRP-Gateway —— 融能力，弃"纯桥"约束
Gateway 的好能力融进 engine 的**服务层 + MCP-client 子系统 + UI↔engine 传输**；它"纯协议桥·不含业务·库优先"的铁律是**它独立分发时的自我克制，对我们单一客户端不适用**——我们要它的能力，不要它的约束。

| 好能力 | 来源（AIRP-Gateway） | 融入哪 | 用途 |
|---|---|---|---|
| **MCP client + 传输**（stdio/streamable-HTTP + 连接池 + initialize 握手 + 协议版本协商） | `src/mcp/{client,pool,transport/*}.rs` | engine 的 **MCP-client 子系统** | 接第三方 MCP 工具生态（见 [PLAN.md](PLAN.md) §4.4） |
| **安全硬化**（SSRF 防护 / 请求·响应体积上限 / 错误脱敏 `is_client_safe` / 优雅关机 / 上游超时 / per-IP 限流 governor / 常数时间 bearer 鉴权 / stdio 命令白名单） | `src/{config,server/middleware}.rs` + ADR-008/009 | engine 的 **HTTP/服务层** | **web 就绪**——引擎对外暴露端口时的生产硬化（§0 web 未来） |
| **agentbus SSE Envelope 适配**（`POST /airp/dispatch` + `GET /airp/stream`，conn 关联，intent→dispatch→state patch） | `src/agentbus/*`（feature） | engine 的 **UI↔engine 传输**（web/SSE 路径） | Phase 0 的 `BusRelay` 已用简化版；web UI 走 SSEBus 时融此适配 |
| 声明式 RouteRule 路由 | `src/config.rs` `RouteRule` | 按需——engine 外部 API/第三方路由 | 中优先 |

> 注：Gateway 的 streaming(Stage 2) 是**桩**（未实现），不是能力，是缺口——engine 侧 SSE 流式我们自己做（Phase 0 已做 UI 侧）。

### 4. AIRP-State-Protocol —— ✅ 已融（是 UI + 协议两盒）
- **UI runtime**：widget 注册表 · BlueprintRenderer · WidgetHost · RFC6902 store · 虚拟滚动 · esm 沙箱 · consent 门 · Tauri 打包 → **已在 `ui/`**。
- **线协议**：Envelope/Blueprint/WidgetDef/Capability + Rust 绑定 → **已在 `protocol/`**。
- 结论：State-Protocol 的好能力**已经是客户端的 UI 盒 + 协议盒**，无需再融，直接用 + 演进。

不继承：通用 Agent UI 标准、协议优先、Widget 市场优先、MockBus/demo-first 的产品北极星。AIRP-Dev 先做 AIRP 专用客户端闭环。

---

## 融入 vs 保留 MCP 生态（澄清，防误解）
"把自有能力融进 engine" **≠ 放弃 MCP/外部接入**：
- **自有** RP 数据能力（MCP-Server 那套）→ 内化为 engine 原生工具（不走网络跳，快、简、单一真相）。
- **第三方** 能力 → engine 作 MCP client 经标准 MCP 协议接（Gateway 的 MCP-client 能力融进来正为此）。
- 两者不矛盾：内化自己的，开放接别人的。这正是“Agent 能力面 = 扩展面”（见 [PLAN.md](PLAN.md) §4.4）。

## 落地
- engine 已含 Core，直接演进。
- **M_AGENT-2** = MCP-Server 数据面内化主载体（[ABSORPTION](MCP-SERVER-ABSORPTION.md)）。
- Gateway 的 MCP-client、安全硬化与 SSE 经验分别随第三方工具接入、Web production 边界和客户端传输需求吸收，不必一次全搬。
- UI/协议直接用 `ui/`+`protocol/`。
- 原则：**看需求融能力，不搬约束、不建平行系统**（凡 engine/UI 已有等价的，不重造）。
