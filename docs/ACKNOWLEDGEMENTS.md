# 项目沿革、设计参考与致谢

> 状态：**待持续更新的活文档**
>
> 全表最后仓库校准：2026-07-30，`main@4f3f792`；2026-08-12 的 #564 UI 调研小节在 `main@7a90d88` 单独复核。本轮未重新查询其余上游版本，具体版本与许可证的实际核验日期仍以各表/小节为准。`tools/dep-governance/` 提供 Cargo + npm 依赖发现与 SPDX/CycloneDX SBOM 生成器，当前 SBOM 快照存于 `docs/sbom/`；该工具是手动离线运行，不替代引入新依赖时的逐项许可证/provenance 核验。

本文区分 AIRP 作者自己的前序项目、第三方设计参考，以及未来可能发生的第三方资产复用。新增研究对象、实际采用外部资产或上游许可证变化时，必须同步更新本文。

## 1. 第一方项目沿革

以下仓库均为 AIRP 作者自己的前序项目，不属于第三方致谢或外部依赖：

| 前序项目 | 在 AIRP 中的关系 |
|---|---|
| AIRP-Core / AIRPCLI | AIRP engine 主核、provider adapter、chat pipeline、orchestrator、Agent loop 与数据层的主要前序来源 |
| AIRP-MCP-Server | RP 数据域、工具目录、工作流、路径沙箱和插件数据面的前序来源 |
| AIRP-Gateway | 传输、安全硬化、路由与 MCP client 设计的前序来源 |
| AIRP-State-Protocol | Blueprint、Widget、Envelope、state patch、guard、虚拟滚动及 consent/sandbox 的前序来源 |

这些资产在当前仓库中按 AIRP 产品需求汇聚和重构，不继承各前序仓库原有的独立产品目标。详细决策见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)。

## 2. 第三方设计参考

下表记录已经在文档或 GitHub issue 中形成明确研究结论的第三方项目。

| 项目 | AIRP 研究或吸收的经验 | 当前关系 | 审查基线（固定版本/日期） | 许可证核验 | AIRP 记录 |
|---|---|---|---|---|---|
| [SillyTavern](https://github.com/SillyTavern/SillyTavern) | RP 功能清单、角色卡/世界书兼容面、Preset 与 Persona 交互、扩展生态、多来源世界书绑定、用户数据隔离，以及用户数据目录内集中保存且默认不回显 API secrets 的公开行为。SillyTavern 在 RP 客户端领域 GitHub 活跃度（stars/forks/release 频率）长期处于头部（2026-07-19 复核），其用户基数可作为"RP 用户痛点样本真实存在"的正当性背书；但用户基数只能背书痛点真实性，不能背书 ST 具体功能实现的正确性（ST 亦有历史包袱与妥协产物），更不能决定 AIRP 是否应采纳——每个功能仍需 AIRP 从自身定位、阶段、成本和差异化方向（Agent 驱动的 RP 增强）独立判断。 | 功能、公开行为与互操作性参考；AIRP 采用自己的版本化 `data/secrets.json` schema、稳定 ID、session 自包含快照和 provenance 独立实现，不复用其代码或 schema | `380e31e8c58d196969b6a0da74f431ba999c7e0a` / 2026-07-12 checkout，secrets 行为 2026-07-19 复核，用户基数背书范围 2026-07-20 复核 | AGPL-3.0 | [TAVERN-PARITY.md](TAVERN-PARITY.md)、[SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[#168](https://github.com/GhostXia/AIRP/issues/168) |
| [Hermes Agent](https://github.com/NousResearch/hermes-agent) | 有界长期记忆、frozen snapshot、skills、用户建模、headless Agent 形态、credential redirect 边界 | 重要架构理念参考 | `3b2ef789df` | MIT | [HERMES-MEMORY.md](HERMES-MEMORY.md)、[#117](https://github.com/GhostXia/AIRP/issues/117) |
| [NeuroBook](https://github.com/notnotype/neuro-book) | 结构化 prompt 装配、长篇记忆、角色知识视角、Agent change inbox 与 authoring workflow | 研究参考；未作为当前 capability 事实 | `138e16d216` | AGPL-3.0 | [LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)、[#117](https://github.com/GhostXia/AIRP/issues/117) |
| [pi-forge](https://github.com/MacroSony/pi-forge) | Preset 导入报告、prompt assembly trace、一次性 payload inspector、history integrity | 理念参考；AIRP 已按自身模型独立实现 Preset 报告/原始 sidecar/原子版本切换，以及真实 pipeline 驱动的脱敏 HTTP/WebUI trace；完整 revision/provenance 仍由 issue 跟踪 | `161f434ba5` | MIT | [#115](https://github.com/GhostXia/AIRP/issues/115)、PR #172/#174/#176/#177 |
| [llmlint](https://github.com/notnotype/llmlint) | 声明式风格规则、候选诊断、确认式修复、误报与分层评测 | 规划参考，尚待 issue 实施 | `9aabfc2839` | PolyForm Noncommercial 1.0.0 | [#116](https://github.com/GhostXia/AIRP/issues/116) |
| [caveman PR #554](https://github.com/JuliusBrussee/caveman/pull/554) | CJK 输出邻近结构化 tool call 时发生截断的用户实遇信号；将“已确认现象、根因假设、未验证缓解方案”分层记录，并以真实复现样本作为兼容性扩展门槛 | 仅作审计与兼容性决策方法参考；未确认是 AIRP 缺陷，不复用上游代码、规则文本或 prompt | `5b80d5ae15` / 2026-06-23 | MIT | [#149](https://github.com/GhostXia/AIRP/issues/149)、[AIRP 提交者的复现与换行候选说明](https://github.com/JuliusBrussee/caveman/pull/554#issuecomment-4785334058) |
| [PulsarAI](https://github.com/NullStarrySky/Pulsarai) | 会话拓扑容器树、条目式记忆（更新函数绑定消息版本 + 状态版本链 + 路径前缀重放）、明确拒绝模型生成 JSON Patch、压缩式记忆的摘要前沿与 Merkle DAG 来源哈希、静态容器作为压缩粒度、失败消息 `type: "error"` 排除在 activePath 之外、`ReplayMode` 三分类（pure / conversation-state / external-once）、capabilities 单一真相、每条更新函数的执行预算与连续失败策略 | 理念参考；AIRP 按自身 domain model、Rust workspace、不变式（`subagent_context_has_no_orchestrator_noise`、干净提示词、有界 Agent）独立实现，不复用其代码、IMD 格式、CodeAct/Sandbox 协议、Plugin 文件树约定或视觉资产 | `9644848c11f968e988416731669390fe6d1f9e42` / 2026-07-31 | 仓库未声明 LICENSE 文件；按 GitHub 默认规则保留所有权利。AIRP 仅研究公开设计文档（`design.md`、`memory-design.md`、`AGENTS.md`），不复制代码、prompt、测试或资产，不依赖其许可证授权 | [#388](https://github.com/GhostXia/AIRP/issues/388)（PulsarAI 研究备注，挂 [#381](https://github.com/GhostXia/AIRP/issues/381) umbrella） |
| [LoopX](https://github.com/huangruiteng/loopx) | 面向长程 Agent 的「轻量状态内核 + 本地控制面」分层：7 层持久控制面（registry / goal state / adapter pre-tick / run log / run history / status-attention / compute quota）+ 4 责任模型（Agent / Provider / Capability / Kernel 各自 owns 与 must-not-own）；状态外置分类法（goal / todo / frontier / authority / evidence / quota / monitor / recovery / lineage）；Authority 四分法（能看见 / 能提议 / 能执行 effect / 能提交 terminal 分离）；Claim / Lease / Gate 区分；Evidence + Effect receipt + readback 链路（proposal → authority → effect → readback → receipt → state commit）；Canonical state vs Projection 单一真相原则（projection 不可反向成为第二套真相）；"Merged Is Not Runtime-Active" 发布纪律；Capability / Provider / Extension 正交分离（capability 按调用者结果命名，extension 不变成第五种 runtime 责任）；Public / Private boundary 三句话规约（public repo 回答"how"，project repo 回答"what"，runtime root 回答"recent ticks"）；Lifetime Goal Invariant（目标可存活数年但每 turn 仍须穿过 authority/boundary/quota/validation/writeback）；Agent-native Kanban 投影；CLI packet 过程协议 | 理念参考；AIRP 按自身 domain model、Rust workspace、RP 产品闭环与已有审计守则独立实现，不复用其 Python 代码、CLI 命令、prompt、测试、fixtures、HTML/CSS 或视觉资产。LoopX 面向工程/研究/recurring 监控类长程任务，AIRP 面向 RP 客户端，领域与 runtime 拓扑不同 | `f4bf2684a903e5e204fb46bc1a7d281643a41b0f` / 2026-07-27（main 分支 README + `docs/architecture.md` + `docs/development/control-plane-course/00-concept-primer.md` + `docs/public-private-boundary.md` + `AGENTS.md` + `docs/product/vision.md`） | MIT | [LEARN-LOOPX.md](LEARN-LOOPX.md) |
| [talemate](https://github.com/vegu-ai/talemate) | 双层状态分离（WorldState 实体快照 + GameState 游戏变量）、Reinforcement 周期性 Q&A 真相校准（带 `interval`/`due` 倒计时）、Episode/Chapter 章节管理、InsertionMode 四态注入策略（sequential / conversation-context / all-context / never）、3 种长期记忆检索策略（recent-context / AI-query / AI-Q&A）、角色进展提议（Suggestion 玩家审批）、条件上下文钉（AnnotatedContextPin + ConditionGroup）、Summarizer 双触发（时间推进 + token 阈值）、ISO 8601 duration 时间表达。同时作为反面教材：ActiveAgent 调用栈共享 `state: dict` 违反 AIRP 神圣不变式 #6（`subagent_context_has_no_orchestrator_noise`）；覆盖式持久化与 AIRP revision/lock_order 合同冲突；单消息线性版本栈弱于 AIRP 分支对话树（`message_parents` + `active_leaf` + swipe）；per-agent client 选择弱于 AIRP `RouteContext` 5 级路由（character > scene_role > task_kind > default > first_default）；重 Python 单体依赖（chromadb + torch + sentence_transformers 同进程）是 AIRP Rust + WebUI 拆分架构的反例。AIRP 不复用其 pydantic 模型、Jinja2 模板、ChromaDB 记忆实现、节点图脚本引擎或任何代码 | 纯理念参考；AIRP 按自身 Rust + WebUI 架构、revision/lock_order 合同、双平面 Agent 隔离独立实现，不复用代码、prompt、模板、数据或视觉资产 | `v0.38.0` / 2026-08-04 调研（pyproject.toml `requires-python = ">=3.11,<3.14"`，依赖 `chromadb>=1.0.12` + `torch>=2.7.1` + `sentence_transformers>=2.7.0` + `jinja2>=3.0` + `RestrictedPython>7.1`） | AGPL-3.0（比 SillyTavern 同等严格，网络效应条款适用） | [2026-08-04-talemate-research-audit.md](audits/2026-08-04-talemate-research-audit.md) |
| [Marinara-Engine](https://github.com/Pasta-Devs/Marinara-Engine) | SillyTavern 活跃 fork（AGPL-3.0）上 Pasta-Devs 的新增设计：(1) Personal Extensions 的用户可见 SHA-256 哈希审批 + "Requested access" 与 exact-hash 两次权限展示 + 草稿/审批职责分离（Professor Mari 能起草但不能批准自己的草稿）；(2) Server Extensions "按设计禁用而非裸跑"——macOS Seatbelt / Linux Bubblewrap 可用时沙箱执行，Windows/Android 无 OS 沙箱则 ⛔ Disabled，明确不回退到 unsandboxed；(3) Agent 目录分发模型（独立 catalog 仓库 + Engine major version 兼容 lane + stable/staging 双轨 + 询问后更新 + 离线韧性 + custom repos `agents.json` at root + 手动同步无后台轮询）；(4) `ADMIN_SECRET` 强动作门槛（backups / bulk import / update apply / sidecar install·download·delete / haptics / custom tool mutation 额外需要）；(5) Card Evolution Auditor 作为角色卡修订回归审计 agent 概念；(6) 机器可读 design tokens + 命名规则模式（"The X Rule"：`The Blush Is Earned Rule`、`The No Tiny Mystery Rule`、`The Reading Surface Rule`）。明确不吸收：Full page access 逃生口（违背 AIRP [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) §4 "不运行 agent 生成的前端代码"）、31 agent 计数作为目标（违背审计守则"不能用工具数替代成功率"）、HACS / Game Mode / "Velvet Game Console" 品牌方向（偏离 AIRP "RP 特化 Agent 客户端"定位）、staging channel 自动选择（与 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) §5.0 E-P0-2/B 显式冻结立场冲突）、`DESIGN.json` 自定义格式（AIRP 应使用 W3C DTCG tokens）。 | 公开文档参考；AIRP 按自身 domain model、Rust workspace、UI-PROTOCOL-DECISION 不变式与审计守则独立实现，不复用其代码、prompt、`DESIGN.json` 格式、Personal Extensions API、Agent catalog 协议、Seatbelt/Bubblewrap 配置或视觉资产 | `v2.4.0` / commit `c82291d` / 2026-08-04，仅公开文档（README、DESIGN.md、docs/CONFIGURATION.md、docs/extending/personal-extensions.md），未读 `packages/` 源码 | AGPL-3.0；AIRP 仅研究公开文档与设计哲学，不读源码、不复制代码/prompt/测试/数据/视觉资产，规避 AGPL 衍生风险 | 本次审计报告（chat session 2026-08-04）；候选 issue 待挂 [#381](https://github.com/GhostXia/AIRP/issues/381) umbrella：(a) 用户可见哈希审批流作为 widget/plugin 统一前置门禁、(b) 沙箱按设计禁用作为 Windows/Android plugin 路线、(c) `ADMIN_SECRET` 作为 [#447](https://github.com/GhostXia/AIRP/issues/447) W-02 缓解的结构化替代、(d) Card Evolution Auditor agent 概念 |
| [AiChat](https://github.com/dghiffjd7/AiChat) | 本地优先 AI 聊天 App（Tauri v2，手机+Windows 桌面，灵感来自 SillyTavern）：(1) 插件/扩展系统的**权限三级模式**（`safe`/`power`/`legacy`）+ 细粒度权限清单 + 事件钩子体系 + `ui.inject`/`registerSidebar`/`registerChatCard`/`openModal` 声明式注入点 + SillyTavern 兼容层；(2) **iframe-host** 独立 WebView 沙箱承载第三方 UI；(3) **Agent Center** 翻面卡片统一管 Agent + 运行记录/审计 + 写操作权限确认（「女仆」NL Agent 样板）；(4) **per-preset profile + 一次性迁移 + lazy migrate + fallback resolver** 的向后兼容迁移范式；(5) **原子写 KV** 存储防断电丢数据；(6) **vendored 离线依赖**（不依赖运行期 CDN）；(7) 「本次请求」血缘图（elkjs 布局）对应的 prompt 透明度 | 理念参考；AIRP 按自身 domain model、AIRP-State-Protocol 的 Blueprint/Widget/RFC6902/consent/sandbox 与 Tauri/Vue 构建链独立实现 Widget manifest 权限 schema、Widget 挂载面、hook 类型、Agent/Widget 中心化与审计、profile 版本化迁移与本地持久化原子写，不复用其代码、manifest 字段名、api 形状、prompt、正则或视觉资产 | `9b9f4fbb04020b28238d9abb71e5a34135c6c5c4`（v0.7.0-preview.2）/ 2026-08-02，远程只读研究，未克隆 | AGPL-3.0（与 AIRP Apache-2.0 不兼容，仅可作理念参考，禁止任何代码采用） | [LEARN-AICHAT.md](LEARN-AICHAT.md) |
| [RisuAI](https://github.com/kwaroran/Risuai) | 视觉小说美学、富文本编辑器、分支对话树、移动端优先设计、轻量化定位、内置 lorebook 系统、自定义背景与立绘的角色卡 | 战略分析参考；未作为当前 capability 事实；AIRP 按自身 domain model 独立实现等价能力，不复用其代码、UI 资产、视觉风格或 prompt | 2026-08-04 战略分析（未固定版本/commit；仓库 `main` 分支 `pushed_at: 2026-07-30T07:06:18Z`） | GPL-3.0（LICENSE 文件 SHA `19d89695339d193efad53476925969721820a563`，2026-08-04 核验；注意是 GPL-3.0，非 AGPL-3.0） | [CHIMERA-BLUEPRINT.md](CHIMERA-BLUEPRINT.md) |
| [Agnai / Agnaistic](https://github.com/agnaistic/agnai) | 多用户协作 RP（共享服务器/协作角色扮演/小型团队）、端到端加密与本地优先设计、多人多 bot 同时群聊、Memory 与 Lore book 持久世界观、多种角色定义格式支持（W++/SBF/Boostyle/纯文本）、基于 PygmalionAI Galatea-UI | 战略分析参考；未作为当前 capability 事实；AIRP 按自身 domain model 独立实现等价能力，不复用其代码、UI 资产或协议 | 2026-08-04 战略分析（未固定版本/commit；仓库 `dev` 分支 `pushed_at: 2026-06-15T05:01:37Z`） | AGPL-3.0（GitHub 仓库元信息 `spdx_id: AGPL-3.0`，2026-08-06 复核更正；原 PR 标注「pending」为核验遗漏） | [CHIMERA-BLUEPRINT.md](CHIMERA-BLUEPRINT.md) |
| [AI Dungeon](https://play.aidungeon.com/) | Story Cards + Memory Banks 按需调出上下文相关记忆、UGC 场景发现/分享/fork、多人联机冒险、分级订阅模式、定制微调模型提升 RP 体验 | 战略与商业模型参考；未作为当前 capability 事实；AIRP 仅研究其公开产品行为与商业模型，不复制代码、prompt、场景数据或视觉资产 | 2026-08-04 战略分析（基于公开产品页面与文档） | proprietary（Latitude 公司闭源商业产品，无公开源代码仓库） | [CHIMERA-BLUEPRINT.md](CHIMERA-BLUEPRINT.md) |
| [Friends & Fables](https://www.friendsandfables.com/) | AI Game Master（Franz）叙述/裁决规则/实时响应世界变化、D&D 5e 规则引擎、战术回合制战斗、AI GM + 虚拟桌面集成、UGC 世界浏览、动态故事线与高级记忆 | 战略与商业模型参考；未作为当前 capability 事实；AIRP 仅研究其公开产品行为，不复制代码、规则文本、prompt 或视觉资产 | 2026-08-04 战略分析（基于公开产品页面与文档） | proprietary（闭源商业产品，无公开源代码仓库） | [CHIMERA-BLUEPRINT.md](CHIMERA-BLUEPRINT.md) |
| [KoboldAI / KoboldCpp](https://github.com/KoboldAI/KoboldAI-Client) | 本地模型极致优化与采样参数精细控制（mirostat/rep penalty/tail-free 等）、可编脚本动作（Lua）介入生成流程、记忆管理、世界信息、社区 RP 微调模型生态（Erebus/Nerys 等） | 战略分析参考；未作为当前 capability 事实；AIRP 按自身 domain model 独立实现等价能力，不复用其代码、Lua 脚本环境或模型权重 | 2026-08-04 战略分析（未固定版本/commit；仓库 `main` 分支 `pushed_at: 2025-01-16T17:01:49Z`） | AGPL-3.0（GitHub 仓库元信息 `spdx_id: AGPL-3.0`，2026-08-06 复核更正；原 PR 标注「pending」为核验遗漏） | [CHIMERA-BLUEPRINT.md](CHIMERA-BLUEPRINT.md) |

### 2026-08-12 #564 UI 调研补充

以下记录只补充 #564 的桌面信息架构与交互输入。AIRP 不复用其源码、schema、prompt、测试、CSS、图标、截图或视觉资产；版本/许可证基线仅说明研究证据边界，不授权复制实现。

| 项目 | 本轮公开行为观察 | 固定版本/日期 | 许可证核验与 AIRP 边界 |
|---|---|---|---|
| [NeuroBook](https://github.com/notnotype/neuro-book) | 稳定领域导航、中央写作工作区、上下文 Agent 面板，以及 World/Plot/Trace 分离，支持 AIRP 的“动态 Surface + Context Inspector”方向 | `844abc29ec8fc67c8ae9764e03da083a676dea32` / 2026-08-12 | AGPL-3.0；只参考公开界面行为，AIRP 独立设计布局合同与视觉 |
| [Talemate](https://github.com/vegu-ai/talemate) | 故事居中，场景、世界摘要及 Agent/模型状态在周边可观察；作为 RP 可观测性参考，不默认暴露内部编排 | `c12a82930e913816fdac21aedada1962ac45c3d7`，release `0.38.0` / 2026-08-12 复核 | AGPL-3.0；延续上表的纯理念参考边界 |
| [RisuAI](https://github.com/kwaroran/Risuai) | 轻量/高级用户渐进披露、受控调色板和非阻塞 UI 事件支持 #564 的简单 Story 默认面与按需高级面板 | `72ce721878d65b09baf4339638dfd221d1788261`，release `v2026.6.215` / 2026-08-12 | GPL-3.0；不复用代码、主题、UI 资产或事件 API 形状 |
| [Open WebUI](https://github.com/open-webui/open-webui) | 统一设置、操作归并、键盘/ARIA、持久错误与简单空状态作为交互卫生参考；其通用单聊天画布不作为 AIRP 产品骨架 | `01f4282f1ffe0d6212f58d3afbeae21fffd0c4be`，release `v0.11.0` / 2026-08-12 | GitHub API 未声明 SPDX；按保留所有权利处理，仅研究公开产品行为 |
| [LobeHub](https://github.com/lobehub/lobehub) | Workspace/Project/Agent/长期任务分离、结构化 composer context 与能力驱动模型控制作为工作区和输入参考 | `ca27228d55bb604f8bcf455a18f5e87d3ba9f9a5`，release `v2.2.13` / 2026-08-12 | GitHub API 未声明 SPDX；按保留所有权利处理，仅研究公开产品行为 |
| [SillyTavern](https://github.com/SillyTavern/SillyTavern) | Swipe picker、生成取消、Persona 生命周期和扩展信任提示作为成熟 RP 边缘场景清单，不采纳其累积式主界面密度 | `8172dcd0ee672d3cd9a5e5f7af134f91a45cd2b8`，release `1.18.0` / 2026-08-12 | AGPL-3.0；延续上表的公开行为与互操作性参考边界 |
| [VCPToolBox](https://github.com/lioensky/VCPToolBox) | 独立记忆查看器与 panel pin/pop-out 只作为 M2 后 power-user 候选；默认多浮窗与密集标签墙明确排除 | `351dadc74836ebf78d25fa942619cd34d9c82987` / 2026-08-12 | GitHub API 未声明 SPDX；按保留所有权利处理，不复制任何代码或视觉资产 |

列入本表仅表示 AIRP 曾研究其公开设计、产品行为或互操作格式，不表示原项目维护者认可、参与或支持 AIRP，也不自动表示 AIRP 复用了其代码、规则、数据、测试或视觉资产。

## 3. 第三方研究与普通依赖规则

用于学习、对标或吸收理念的第三方项目只允许研究公开行为、协议、格式和需求洞察；AIRP 必须按自己的 domain model、命名、控制流、安全边界和测试独立实现，不复制、翻译、改写或移植其源码、规则文本、prompt、测试、数据、HTML/CSS、图标或视觉资产。许可证表面允许也不改变这条默认规则。

普通第三方依赖库和基础设施可以模块化接入并深度参与功能，但必须满足：

1. 依赖解决的是明确的工程问题，而不是替 AIRP 决定产品边界；
2. 接入点有清晰接口、默认配置和移除/替换路径，核心数据真相不归依赖所有；
3. 锁定版本，记录上游、许可证、用途、运行时/构建时关系和分发义务；
4. 通过 AIRP 自己的安全、失败、升级和回归测试，不把上游默认值当作项目合同；
5. 若形成运行时或发布依赖，进入 provenance、notices 与 SBOM，而不是继续列作“理念参考”。

因此，模块化深度干预不违背“便于维护和未来移植”；不可替换、边界不透明、把第三方内部模型扩散到全项目才违背该准则。

### 已单独核验的普通依赖与基础设施

| 组件 | 固定版本/核验日期 | 计划用途 | 许可证与 provenance | 当前状态 |
|---|---|---|---|---|
| [Caddy](https://github.com/caddyserver/caddy) / [Docker Official Image](https://hub.docker.com/_/caddy) | `2.11.4` / 2026-07-13 | WebUI 首方 OCI/Compose bundle 的 HTTPS、Basic perimeter auth、静态文件、安全 headers 与 reverse proxy | 上游 Caddy `v2.11.4` 为 Apache-2.0；官方 multi-platform image 固定为 `sha256:af5fdcd76f2db5e4e974ee92f96ee8c0fc3edb55bd4ba5032547cbf3f65e486d` | 已进入 `deploy/production/Dockerfile.gateway`；仍须在 P3 生成完整基础镜像/传递组件 notices 与 SBOM 后才能正式发布 |
| [Debian](https://www.debian.org/) Docker Official Image | `bookworm-slim` / 2026-07-13 | `airp-core` runtime base 与 CA trust store | 官方 multi-platform image 固定为 `sha256:60eac759739651111db372c07be67863818726f754804b8707c90979bda511df`；各 Debian package 许可证须由最终 SBOM/notices 枚举 | 已进入 `deploy/production/Dockerfile.engine` runtime stage；正式发布 provenance 仍开放 |
| [Rust](https://www.rust-lang.org/) Docker Official Image | `1.96.0-bookworm` / 2026-07-13 | 仅用于可重复构建 `airp-core` 的 builder stage，不进入 runtime image | 官方 multi-platform image 固定为 `sha256:5e2214abe154fe26e39f64488952e5c991eeed1d6d6da7cc8381ae83927f0cfc`；Rust toolchain 为 Apache-2.0 OR MIT | 已进入 `deploy/production/Dockerfile.engine` builder stage；不随 runtime 分发 |
| [Playwright Core](https://github.com/microsoft/playwright) | `1.61.1` / 2026-07-13 | 仅作为 CI dev dependency 驱动 runner 预装的 system Chrome，验证 production WebUI CSP、文本注入安全与 SSE 取消 | npm lockfile 固定 tarball integrity；上游许可证 Apache-2.0；未下载或分发 Playwright browser bundle | 已进入 `ui/package-lock.json` 与 production topology smoke；不进入 AIRP runtime images，不复用上游测试或实现代码 |
| [Vite](https://github.com/vitejs/vite) / [Vue plugin](https://github.com/vitejs/vite-plugin-vue) / [Vitest](https://github.com/vitest-dev/vitest) | `8.1.4` / `6.0.8` / `4.1.10`，2026-07-16 核验 | `ui/` 的 Vue 构建、开发服务器与测试工具链 | 三个上游均为 MIT；manifest 使用不跨主版本的有界兼容范围，npm lockfile 固定实际版本、来源与 tarball integrity | 仅为开发/测试依赖，不进入 production WebUI gateway 或 engine runtime image；升级由 #137 / PR #191 审计 |

AIRP 只配置并分发普通上游组件，不复制、翻译或改写其源码/文档。上表分别记录 P0 artifact 的精确镜像和开发/测试依赖的锁定版本；它不把 preview artifact 写成正式发布能力。正式 tag 前仍必须补构建 provenance、机器可读 notices 与完整 SBOM。

## 4. 维护待办

- [ ] 每次新增外部研究 issue 或学习文档时，将项目补入本表。
- [ ] 每次准备复用第三方资产时，在合入前完成许可证与 provenance 审查。
- [ ] 发布前复核所有上游许可证是否变化，并更新“最后核验”日期。
- [ ] 若第三方组件随二进制分发，建立并维护机器可读的 third-party notices/SBOM。
- [ ] 定期检查“规划参考”是否已经落地；落地后补充实现位置和验证证据。

## 5. 当前非致谢对象

审计模型、IDE 托管服务、代码审查机器人及贡献者 trailer 属于工具或历史来源记录，不因参与审计就成为 AIRP 的设计上游。此类来源应保留在对应 audit/commit/issue 中，不加入本页项目致谢，除非未来确实吸收了其公开项目设计。
