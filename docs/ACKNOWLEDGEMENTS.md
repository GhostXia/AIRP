# 项目沿革、设计参考与致谢

> 状态：**待持续更新的活文档**
>
> 最后仓库校准：2026-07-30，`main@4f3f792`；本轮未重新查询全部上游版本，具体版本与许可证的实际核验日期仍以各表为准。`tools/dep-governance/` 提供 Cargo + npm 依赖发现与 SPDX/CycloneDX SBOM 生成器，当前 SBOM 快照存于 `docs/sbom/`；该工具是手动离线运行，不替代引入新依赖时的逐项许可证/provenance 核验。

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
| [talemate](https://github.com/vegu-ai/talemate) | 双层状态分离（WorldState 实体快照 + GameState 游戏变量）、Reinforcement 周期性 Q&A 真相校准（带 `interval`/`due` 倒计时）、Episode/Chapter 章节管理、InsertionMode 四态注入策略（sequential / conversation-context / all-context / never）、3 种长期记忆检索策略（recent-context / AI-query / AI-Q&A）、角色进展提议（Suggestion 玩家审批）、条件上下文钉（AnnotatedContextPin + ConditionGroup）、Summarizer 双触发（时间推进 + token 阈值）、ISO 8601 duration 时间表达。同时作为反面教材：ActiveAgent 调用栈共享 `state: dict` 违反 AIRP 神圣不变式 #6（`subagent_context_has_no_orchestrator_noise`）；覆盖式持久化与 AIRP revision/lock_order 合同冲突；单消息线性版本栈弱于 AIRP 分支对话树（`message_parents` + `active_leaf` + swipe）；per-agent client 选择弱于 AIRP `RouteContext` 5 级路由（character > scene_role > task_kind > default > first_default）；重 Python 单体依赖（chromadb + torch + sentence_transformers 同进程）是 AIRP Rust + WebUI 拆分架构的反例。AIRP 不复用其 pydantic 模型、Jinja2 模板、ChromaDB 记忆实现、节点图脚本引擎或任何代码 | 纯理念参考；AIRP 按自身 Rust + WebUI 架构、revision/lock_order 合同、双平面 Agent 隔离独立实现，不复用代码、prompt、模板、数据或视觉资产 | `v0.38.0` / 2026-08-04 调研（pyproject.toml `requires-python = ">=3.11,<3.14"`，依赖 `chromadb>=1.0.12` + `torch>=2.7.1` + `sentence_transformers>=2.7.0` + `jinja2>=3.0` + `RestrictedPython>7.1`） | AGPL-3.0（比 SillyTavern 同等严格，网络效应条款适用） | [2026-08-04-talemate-research-audit.md](audits/2026-08-04-talemate-research-audit.md) |
| [Marinara-Engine](https://github.com/Pasta-Devs/Marinara-Engine) | SillyTavern 活跃 fork（AGPL-3.0）上 Pasta-Devs 的新增设计：(1) Personal Extensions 的用户可见 SHA-256 哈希审批 + "Requested access" 与 exact-hash 两次权限展示 + 草稿/审批职责分离（Professor Mari 能起草但不能批准自己的草稿）；(2) Server Extensions "按设计禁用而非裸跑"——macOS Seatbelt / Linux Bubblewrap 可用时沙箱执行，Windows/Android 无 OS 沙箱则 ⛔ Disabled，明确不回退到 unsandboxed；(3) Agent 目录分发模型（独立 catalog 仓库 + Engine major version 兼容 lane + stable/staging 双轨 + 询问后更新 + 离线韧性 + custom repos `agents.json` at root + 手动同步无后台轮询）；(4) `ADMIN_SECRET` 强动作门槛（backups / bulk import / update apply / sidecar install·download·delete / haptics / custom tool mutation 额外需要）；(5) Card Evolution Auditor 作为角色卡修订回归审计 agent 概念；(6) 机器可读 design tokens + 命名规则模式（"The X Rule"：`The Blush Is Earned Rule`、`The No Tiny Mystery Rule`、`The Reading Surface Rule`）。明确不吸收：Full page access 逃生口（违背 AIRP [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) §4 "不运行 agent 生成的前端代码"）、31 agent 计数作为目标（违背审计守则"不能用工具数替代成功率"）、HACS / Game Mode / "Velvet Game Console" 品牌方向（偏离 AIRP "RP 特化 Agent 客户端"定位）、staging channel 自动选择（与 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) §5.0 E-P0-2/B 显式冻结立场冲突）、`DESIGN.json` 自定义格式（AIRP 应使用 W3C DTCG tokens）。 | 公开文档参考；AIRP 按自身 domain model、Rust workspace、UI-PROTOCOL-DECISION 不变式与审计守则独立实现，不复用其代码、prompt、`DESIGN.json` 格式、Personal Extensions API、Agent catalog 协议、Seatbelt/Bubblewrap 配置或视觉资产 | `v2.4.0` / commit `c82291d` / 2026-08-04，仅公开文档（README、DESIGN.md、docs/CONFIGURATION.md、docs/extending/personal-extensions.md），未读 `packages/` 源码 | AGPL-3.0；AIRP 仅研究公开文档与设计哲学，不读源码、不复制代码/prompt/测试/数据/视觉资产，规避 AGPL 衍生风险 | 本次审计报告（chat session 2026-08-04）；候选 issue 待挂 [#381](https://github.com/GhostXia/AIRP/issues/381) umbrella：(a) 用户可见哈希审批流作为 widget/plugin 统一前置门禁、(b) 沙箱按设计禁用作为 Windows/Android plugin 路线、(c) `ADMIN_SECRET` 作为 [#447](https://github.com/GhostXia/AIRP/issues/447) W-02 缓解的结构化替代、(d) Card Evolution Auditor agent 概念 |
| [AiChat](https://github.com/dghiffjd7/AiChat) | 本地优先 AI 聊天 App（Tauri v2，手机+Windows 桌面，灵感来自 SillyTavern）：(1) 插件/扩展系统的**权限三级模式**（`safe`/`power`/`legacy`）+ 细粒度权限清单 + 事件钩子体系 + `ui.inject`/`registerSidebar`/`registerChatCard`/`openModal` 声明式注入点 + SillyTavern 兼容层；(2) **iframe-host** 独立 WebView 沙箱承载第三方 UI；(3) **Agent Center** 翻面卡片统一管 Agent + 运行记录/审计 + 写操作权限确认（「女仆」NL Agent 样板）；(4) **per-preset profile + 一次性迁移 + lazy migrate + fallback resolver** 的向后兼容迁移范式；(5) **原子写 KV** 存储防断电丢数据；(6) **vendored 离线依赖**（不依赖运行期 CDN）；(7) 「本次请求」血缘图（elkjs 布局）对应的 prompt 透明度 | 理念参考；AIRP 按自身 domain model、AIRP-State-Protocol 的 Blueprint/Widget/RFC6902/consent/sandbox 与 Tauri/Vue 构建链独立实现 Widget manifest 权限 schema、Widget 挂载面、hook 类型、Agent/Widget 中心化与审计、profile 版本化迁移与本地持久化原子写，不复用其代码、manifest 字段名、api 形状、prompt、正则或视觉资产 | `9b9f4fbb04020b28238d9abb71e5a34135c6c5c4`（v0.7.0-preview.2）/ 2026-08-02，远程只读研究，未克隆 | AGPL-3.0（与 AIRP Apache-2.0 不兼容，仅可作理念参考，禁止任何代码采用） | [LEARN-AICHAT.md](LEARN-AICHAT.md) |

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
