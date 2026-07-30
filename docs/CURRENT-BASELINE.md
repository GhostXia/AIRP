# AIRP 当前开发基线

> 基线日期：2026-07-30  
> 代码基线：`main@4f3f792`（`test(281): add dry-run tests for update_relationship and advance_plot (#380)`）  
> 用途：冷启动开发、审计和产品判断的第一事实入口。  
> 真理顺序：当前源码、manifest、测试与可重复运行证据 > 本文 > 专题合同 > 路线图/研究材料 > 历史归档。

本文只记录当前代码树能够支持的结论。GitHub issues 是未完成工作的实时追踪面；PR、审计报告和历史测试数字只证明对应代码树，不自动证明当前 `HEAD`。

本次校准（2026-07-30 docs-pass）做了三件事：

1. 将代码锚点从中间工作树 `449a685` / 旧 `200fed9` 对齐到当前 `main@4f3f792`；  
2. 吸收 engine 独立审查 umbrella [#381](https://github.com/GhostXia/AIRP/issues/381) 的结构性事实（双轨会话、TurnCommit、锁/DNS、domain 边界），**不把已开 issue 写成已交付**；  
3. 收缩活文档面：完成的 Persona HTTP 计划与桌面画布接力草案迁入 `docs/archive/`，阅读路径见 [README.md](README.md)。

## 1. 产品与仓库边界

AIRP 是面向 Role Play 的 AI Agent 客户端，采用“无头 Engine + 可换 UI”结构。

| 路径 | 当前职责 | 产品状态 |
|---|---|---|
| `engine/` | `airp-core`：RP 数据、prompt 装配、LLM adapter、Agent loop、HTTP/SSE | 唯一业务内核 |
| `webui/` | 无构建、多页面、同源 WebUI（当前 44 屏） | **正式产品交付主面** |
| `airp-engine-console/` | WebUI 视觉与交互样板 | 设计基线，不是第二套运行时 |
| `protocol/` | `airp-state-protocol`：共享线协议类型 | Rust workspace 成员 |
| `ui/`、`ui/src-tauri/` | Vue + Tauri 桌面客户端 | 保留维护线，**近期发布暂停** |
| `deploy/windows-webui/` | Windows 便携 WebUI 包 | 当前优先 artifact |
| `deploy/linux-webui/` | Linux musl 便携包 | 手动构建 artifact |
| `deploy/production/` | 单实例自托管 HTTPS preview | P0 拓扑，不是正式发布 |
| `data/` | 运行时数据根规范与安全样例 | 不是共享素材库 |
| `tools/` | 依赖治理、SBOM、Agent 浏览器探索 | 工程工具，不进入 RP 角色平面 |

Rust workspace 只有 `engine`、`protocol`、`ui/src-tauri`。AIRP-Core/AIRPCLI、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 是作者的第一方前序项目，不是当前 runtime 依赖；吸收边界见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)。

## 2. 当前能力矩阵

“已实现”按层描述，不把底层模块、HTTP route、UI 页面或一次测试互相冒充。

| 能力域 | Engine / 数据 | HTTP / Agent | WebUI | 当前边界 |
|---|---|---|---|---|
| 角色、Persona、Preset、场景 | CRUD、导入、绑定、revision、装配 | 主要 CRUD/导入/预览 route；相关 Agent tools（Persona **无**对称 Agent tool） | 管理、选择、导入与诊断入口 | 高级生命周期、完整导出/恢复未闭合（#342/#346） |
| 会话与聊天（**产品主路径**） | durable JSONL、稳定 message ID、cursor、rollback、branch/swipe | OpenAI-compatible `/v1/chat/*` SSE、continue/regen/search、命名 session | 命名会话、流式聊天、编辑/删除/分支/Swipe、导出 | **产品 UI 只绑定本路径**；长会话虚拟化、跨资源 Turn 事务与完整崩溃恢复仍开放（#286/#122） |
| Conversation runtime（**Engine 合同，未绑产品 UI**） | versioned manifest、append-only event journal、message/turn/observability projection、scene round-robin、受控 policy 注入、长历史 checkpoint/summary 预算、legacy copy-only migration | `/v1/conversations*`、capabilities、policies、migration plan/execute/export/rollback；旧 chat/session/scene API 形状不变 | **尚未绑定**；客户端若接入只能经 capability discovery，不能注入 history/代码/调度语义 | 与 legacy Chat **双轨并存**；切流或冻结须战略决策（#381 E-P0-2）。自动 summary policy、内容型停止条件、全仓 migration registry、跨进程策略沙箱仍开放 |
| Worldbook / state / memory | v4 runtime、state history/schema、resident memory、revision | CRUD、图谱、事件、状态与记忆相关接口/工具 | 编辑、图谱、状态 HUD、记忆面板 | 大量 ST 字段仅为 advisory；完整 session 物化与记忆闭环未完成（#274） |
| Agent 与剧情 | 有界 loop、Director、Council、NPC、剧情弧、世界时钟、定时事件、遗忘曲线 | 约 30 个内置工具 + 可动态加载插件工具 | Agent run、剧情弧、群聊、世界事件 | 并发/失败路径有开放审计项（#284/#344/#381）；不是通用多 Agent 平台 |
| 创作工具 | 图片生成、角色模板、风格学习、对话示例、时间线、卡片 diff | 对应 HTTP | 屏 36–42 等已接入 | 功能存在 ≠ 真实 provider/工作流已验收 |
| Provider / 扩展 | 多 Provider 路由、OpenAI-compatible/Anthropic/Ollama、本地脚本/HTTP webhook 插件 | providers/routing/plugin-tools API；Agent registry 动态合并 | 设置与插件管理入口 | 插件非沙箱；DNS fail-open 与请求时不 pin 为已知 SSRF 残差（RR-014 / #381 E-P0-3 / #329 N3） |
| 部署 | production fail-closed 校验、原子配置更新、secret 脱敏 | loopback 默认；首方 gateway 同源代理 | Windows/Linux 便携包与 production preview | 非多租户；P1/P2/P3 发布门未闭合 |

### 2.1 结构性事实（2026-07-30 审查确认）

这些不是新功能承诺，而是避免误读代码树的硬事实：

1. **双轨会话**：正式 WebUI 走 `/v1/chat/*` + `ChatLog`/`ChatService`；Conversation 是并行 Engine 合同与 HTTP 面，**不能**因 route/测试存在就宣称产品已切换。  
2. **单资源原子写 ≠ 跨资源事务**：`finalize` 可对 message → state → volume 逐步 fail-closed，崩溃后跨资源一致性仍是 best-effort（RR-004 / #286）。  
3. **Domain 写路径未完全闭合**：shared service 是目标边界；Agent tools 等路径仍可能直接 `replace_file` / `fs` 写（#381 E-P1-3 / #160）。  
4. **锁模型分裂**：character/session/state/persona/conversation/decay/FTS/quota 等多套锁；async 路径上存在 std 锁 + 锁内磁盘 I/O；poison 策略不一致（#284/#220/#381）。  
5. **桌面线暂停**：`ui/` 保留；画布接力等草案在 archive，不进入当前执行队列。

## 3. 必须保持的不变式

1. **干净提示词**：RP 角色平面只含 RP 数据；工具/调度/审计留在结构化控制平面。`subagent_context_has_no_orchestrator_noise` 神圣不可弱化。  
2. **Engine 单一真相**：handler、UI、Agent tool 不复制持久化规则；写路径应收敛到 shared service。  
3. **有界 Agent**：step/token/墙钟/取消/可观察事件；UI consent 不替代 Engine 授权。  
4. **用户资产优先**：不兼容演进必须有 versioned migration、升级前备份、完整性验证、可读导出与回滚。  
5. **安全默认关闭**：production 监听前 fail-closed；密钥不进普通 settings/URL/前端存储/日志；Web/远端不得启用任意本地路径导入。  
6. **第三方独立实现**：只吸收公开理念/需求/行为/互操作经验；不复制第三方代码、prompt、测试、数据或视觉资产。  
7. **审计门禁**：本地全绿只允许开 PR；审计 bot 通过并修完阻塞意见后，仍由人工 review 决定合并。

## 4. 当前不能宣称

- 不能宣称已正式发布、适合公网多租户、通过完整 P1/P2/P3，或已获市场验证。  
- 不能用页面数、工具数或 Phase 合入数量替代黄金路径成功率、恢复能力与继续使用意愿。  
- 不能宣称完整 session 自包含、跨资源 Turn 事务、全仓统一 migration registry、自动定时备份/恢复、浏览器矩阵或长会话 soak 已交付。  
- 不能把 Conversation 的 copy migration / 可观测性能力外推为 legacy Chat 产品路径或其它资产已具备同等恢复力。  
- 不能宣称完整 MCP 生态、任意插件沙箱、跨设备同步、多语言 UI 或正式资产规格已交付。  
- 不能把桌面 Tauri、production preview、Windows/Linux 便携包的测试结果互相外推。  
- 不能把 Worldbook/Preset **advisory** 字段写成已执行语义；以 [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md) 为准。  
- 不能把「已开 GitHub issue / 已写审计 umbrella」写成「风险已关闭」。

## 5. 当前优先级

当前主线不是扩大功能面，而是把已合入能力收敛成可验证、可恢复的 **P1 有限试用**：

1. **Engine 一致性收敛**（[#381](https://github.com/GhostXia/AIRP/issues/381)）：  
   - 拍板 Chat vs Conversation（切流或冻结，E-P0-2）；  
   - Plugin DNS fail-closed + 请求时校验（E-P0-3，升权自 #329 N3）；  
   - Turn 级跨资源 commit/recovery（E-P0-1 → 执行面 #286，灾难恢复 #342）；  
   - 锁/async I/O/poison 与同 session 互斥（E-P0-4/5 → #284/#220/#160）。  
2. 用真实 provider 与真实浏览器验证：onboarding → 首聊 → 继续对话 → 刷新恢复 → 服务重启恢复。  
3. 校准 WebUI 运行时契约、视觉一致性、空/错/慢状态与 browser smoke（#311/#345 等）。  
4. 备份、恢复、migration 与回滚的最小闭环（#342）。  
5. 上述证据稳定前，默认不从 #312 启动无用户证据的新子系统扩张；遵守 #242 范围收敛。

### 5.1 实时工作入口（动手前用 `gh issue view` 复核状态）

| 主题 | Issue |
|---|---|
| Engine 审计 umbrella / 排序 | [#381](https://github.com/GhostXia/AIRP/issues/381) |
| Turn 两阶段 / 跨资源提交 | [#286](https://github.com/GhostXia/AIRP/issues/286) |
| per-session 并发串行化 | [#284](https://github.com/GhostXia/AIRP/issues/284) |
| 持久化/lock 遗留 | [#220](https://github.com/GhostXia/AIRP/issues/220) |
| 备份恢复导出 | [#342](https://github.com/GhostXia/AIRP/issues/342) |
| Plugin/engine 非阻塞遗留（含 DNS N3） | [#329](https://github.com/GhostXia/AIRP/issues/329) |
| Conversation migration 解耦 | [#371](https://github.com/GhostXia/AIRP/issues/371) |
| WebUI 能力展现 / 契约门禁 | [#311](https://github.com/GhostXia/AIRP/issues/311)、[#345](https://github.com/GhostXia/AIRP/issues/345) |
| 范围收敛 / 路线图索引 | [#242](https://github.com/GhostXia/AIRP/issues/242)、[#312](https://github.com/GhostXia/AIRP/issues/312) |

## 6. 验证快照

| 范围 | 命令 / 说明 | 结果（`main@4f3f792`，2026-07-30） |
|---|---|---|
| WebUI | `node --test webui/tests/*.test.mjs` | **67 passed** |
| Rust workspace | `cargo test --workspace --locked` | **本 docs-pass 未完成干净复跑**（维护者环境曾出现 `target` file lock 争用）。**不得**用 Batch 7 / `449a685` 的 1199 passed 数字冒充本 HEAD。合并代码前仍须按 DEV-GUIDE 本地全绿。 |
| 静态门禁 / production topology / 便携包 / 真实 provider·browser | 本 docs-pass 未跑 | 不得推断为通过 |
| 历史参考（**仅证明对应树**） | Conversation Batch 7 工作树基于 `449a685` 曾报 engine+protocol+tauri 等合计 1199 passed / 5 ignored | 不可外推到 `4f3f792` |

未在本次校准中运行的 production topology、Windows/Linux artifact、真实 provider/browser、网络故障、进程崩溃和真实 provider 长会话，不得由本表推断为通过。

## 7. 文档职责（校准后）

| 层级 | 文档 | 职责 |
|---|---|---|
| 事实入口 | [CURRENT-BASELINE.md](CURRENT-BASELINE.md)（本文） | 唯一人工维护的全仓能力基线 |
| 开发治理 | [DEV-GUIDE.md](DEV-GUIDE.md) | 地图、命令、不变式、交付流程 |
| 产品方向 | [PLAN.md](PLAN.md) | 稳定目标与阶段门；不复制 issue 队列 |
| 数据/运行时合同 | [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)、[CONVERSATION-CONTRACT.md](CONVERSATION-CONTRACT.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md) | 目标与已交付边界必须分开读 |
| 安全/发布 | [SECURITY.md](SECURITY.md)、[RISK-REGISTER.md](RISK-REGISTER.md)、[WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)、[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) | 威胁、风险、拓扑与发布门 |
| 接口/扩展草案 | [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)、[AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md)、[ASSET-SPEC.md](ASSET-SPEC.md) | 已决策边界；未实现部分不得当 runtime 事实 |
| 来源治理 | [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)、[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) | 第一方/第三方吸收与 provenance |
| 文档地图 | [README.md](README.md) | 分层与阅读路径 |
| 审计原始记录 | `docs/audits/` | 按 PR 归档；不压缩成当前能力清单 |
| 历史 | `docs/archive/` | 已完成设计、草案与月度摘要；**不能覆盖本文** |

参考材料（研究用，非当前能力）：[CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md)、[MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)、[TAVERN-PARITY.md](TAVERN-PARITY.md)、[HERMES-MEMORY.md](HERMES-MEMORY.md)、[LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)。

### 7.1 本 docs-pass 的文档整理动作

| 动作 | 路径 | 原因 |
|---|---|---|
| 归档 | `docs/archive/2026-07-persona-http-api-plan.md`（原 `docs/PERSONA-HTTP-API-PLAN.md`） | 实施计划已交付；接口事实以源码+基线为准 |
| 归档 | `docs/archive/2026-07-29-desktop-ui-canvas-relay-plan.md`（原未跟踪活路径草案） | 桌面发布暂停；草案不占活文档位 |
| 保留 | `docs/audits/2026-07-28-desktop-ui-relay-plan-audit.md` | 计划级审计原始记录 |
| 刷新 | 本文、README、PLAN、DEV-GUIDE、SECURITY、RISK-REGISTER 及合同头信息 | 对齐 `4f3f792` 与 #381 |

完整阅读路径与维护规则见 [README.md](README.md)。
