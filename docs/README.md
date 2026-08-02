# AIRP 文档地图

> 校准：2026-07-30，`main@4f3f792`  
> 能力事实只以 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 为准；本页只负责分层与阅读路径。

文档按「事实、合同、治理、参考、历史」分层。新 session 不需要通读全部文档；先读事实入口，再按任务选一个直接相关合同，实时待办查 GitHub issues。

## 最短阅读路径

1. 根目录 [`AGENTS.md`](../AGENTS.md)：本机环境、审计守则、第三方独立实现、合并门禁；  
2. [CURRENT-BASELINE.md](CURRENT-BASELINE.md)：当前能力、缺口、优先级、验证边界；  
3. [DEV-GUIDE.md](DEV-GUIDE.md)：代码地图、命令、不变式、交付流程；  
4. 任务直接相关的**一个**专题合同；  
5. 开放 GitHub issue（尤其 engine 收敛 umbrella [#381](https://github.com/GhostXia/AIRP/issues/381)）。

## 活文档

| 类别 | 文档 | 职责 |
|---|---|---|
| 当前事实 | [CURRENT-BASELINE.md](CURRENT-BASELINE.md) | 唯一人工维护的全仓能力基线 |
| 开发治理 | [DEV-GUIDE.md](DEV-GUIDE.md) | 开发/审计入口、验证和工作流 |
| 产品方向 | [PLAN.md](PLAN.md) | 稳定目标、阶段门和取舍；不复制 issue 队列 |
| 数据合同 | [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)、[CONVERSATION-CONTRACT.md](CONVERSATION-CONTRACT.md)、[LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md) | session、history、Conversation、进程内锁序；**目标与已交付必须分读** |
| RP 合同 | [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)、[ASSET-SPEC.md](ASSET-SPEC.md) | Worldbook runtime 与资产规格策略（规格尚未正式发布） |
| 接口/架构 | [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)、[AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md) | UI 协议决策与可配置编排**草案** |
| 安全/发布 | [SECURITY.md](SECURITY.md)、[RISK-REGISTER.md](RISK-REGISTER.md)、[WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)、[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) | 威胁边界、风险、部署拓扑和发布门 |
| 来源治理 | [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)、[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) | 第一方来源裁定、第三方参考和 provenance |

根目录产品说明：[`README.md`](../README.md) / [`README.en.md`](../README.en.md)。引擎模块说明：[`engine/README.md`](../engine/README.md)。

## 参考材料（非当前能力）

只在相关任务中阅读。若与源码或基线冲突，以源码和基线为准：

- [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md) — 第一方前序项目能力筛选  
- [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md) — 前序 MCP-Server catalog  
- [TAVERN-PARITY.md](TAVERN-PARITY.md) — SillyTavern 公开行为与互操作研究  
- [HERMES-MEMORY.md](HERMES-MEMORY.md) — 长期记忆 / skills 研究  
- [LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md) — 长篇 RP/authoring 研究  

## 历史与审计

| 路径 | 规则 |
|---|---|
| `docs/audits/` | PR/计划级审计原始记录；随 PR 归档；**不得**压缩成当前事实清单 |
| `docs/archive/` | 已完成设计、已交付实施计划、暂停路线草案、月度历史索引 |
| `docs/sbom/` | 生成的 SPDX / CycloneDX / notices 快照 |

### 2026-07-30 归档迁入

| 原活路径 | 现位置 | 原因 |
|---|---|---|
| `docs/PERSONA-HTTP-API-PLAN.md` | [archive/2026-07-persona-http-api-plan.md](archive/2026-07-persona-http-api-plan.md) | A1/A2 已交付；非 route inventory |
| `docs/DESKTOP-UI-CANVAS-RELAY-PLAN.md` | [archive/2026-07-29-desktop-ui-canvas-relay-plan.md](archive/2026-07-29-desktop-ui-canvas-relay-plan.md) | 桌面发布暂停；草案待战略重启再读 |

历史索引（非基线）：[archive/PR-AUDITS-2026-07.md](archive/PR-AUDITS-2026-07.md)、[archive/PROJECT-HISTORY-2026-07.md](archive/PROJECT-HISTORY-2026-07.md)、[archive/WEBUI-HISTORY-2026-07.md](archive/WEBUI-HISTORY-2026-07.md)。

已完成且没有独立合同价值的临时拆分计划应删除或归档，而不是继续留在活文档。审计记录不按此删除。

## 维护规则

1. **能力变化只更新 `CURRENT-BASELINE.md`**；不要在多份研究/计划文档复制「当前状态」。  
2. 稳定行为写进已有专题合同；不要为每个 PR 新建永久计划文档。  
3. 「已实现」必须标明 domain/data、HTTP、Agent、WebUI、desktop、artifact 或 production evidence 中的哪一层。  
4. 测试数字必须带命令和 commit，并按 runner 分桶；历史数字不外推到新 `HEAD`。  
5. GitHub issues 管理实时待办；文档只保存稳定依赖、阶段门和长期理由。  
6. 完成的计划：吸收仍有效合同 → 删除或归档；审计记录保留。  
7. docs-only 变更最低校验：相对链接、状态标签、日期、commit、「已交付/未交付」措辞、与 #381 等开放风险是否一致。  
8. 桌面/暂停路线的新计划默认进 `docs/archive/` 或 issue，不占活文档位，除非用户显式恢复该产品线。
