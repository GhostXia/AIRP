# AIRP 文档地图

> 校准：2026-07-26，`main@200fed9`

文档按“事实、合同、治理、参考、历史”分层。新 session 不需要通读全部文档；先读事实入口，再按任务选择一个直接相关合同。

## 最短阅读路径

1. 根目录 [`AGENTS.md`](../AGENTS.md)：本机环境、审计、第三方独立实现和合并门禁；
2. [CURRENT-BASELINE.md](CURRENT-BASELINE.md)：当前能力、缺口、优先级和验证边界；
3. [DEV-GUIDE.md](DEV-GUIDE.md)：代码地图、命令、不变式和交付流程；
4. 任务直接相关的一个专题合同；
5. 开放 GitHub issue：实时范围与验收。

## 活文档

| 类别 | 文档 | 职责 |
|---|---|---|
| 当前事实 | [CURRENT-BASELINE.md](CURRENT-BASELINE.md) | 唯一人工维护的全仓能力基线 |
| 开发治理 | [DEV-GUIDE.md](DEV-GUIDE.md) | 开发/审计入口、验证和工作流 |
| 产品方向 | [PLAN.md](PLAN.md) | 稳定目标、阶段门和取舍；不复制 issue 队列 |
| 数据合同 | [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)、[CONVERSATION-CONTRACT.md](CONVERSATION-CONTRACT.md) | session、revision、history、通用 conversation、恢复 |
| RP 合同 | [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)、[ASSET-SPEC.md](ASSET-SPEC.md) | Worldbook runtime 与资产规格候选边界 |
| 接口/架构 | [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)、[AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md) | UI 协议决策与可配置编排草案 |
| 安全/发布 | [SECURITY.md](SECURITY.md)、[RISK-REGISTER.md](RISK-REGISTER.md)、[WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)、[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) | 威胁边界、风险、部署拓扑和发布门 |
| 来源治理 | [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)、[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) | 第一方来源裁定、第三方参考和 provenance |

## 参考材料

以下文档保留研究价值，但不代表当前能力、优先级或兼容承诺：

- [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md)：第一方前序项目能力筛选；
- [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)：前序 MCP-Server catalog 与候选落点；
- [TAVERN-PARITY.md](TAVERN-PARITY.md)：SillyTavern 公开行为与互操作性研究；
- [HERMES-MEMORY.md](HERMES-MEMORY.md)：长期记忆、skills、Soul 研究；
- [LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)：长篇 RP/authoring 研究；
- [PERSONA-HTTP-API-PLAN.md](PERSONA-HTTP-API-PLAN.md)：Persona HTTP 方案的历史实施依据；当前接口事实以源码和基线为准。

参考材料只在相关任务中读取。若其状态与当前源码、本页或基线冲突，以当前源码和基线为准。

## 历史与审计

- `docs/audits/`：PR 审计原始记录。按仓库规则保留并随 PR 归档；不得压缩成当前事实。
- `docs/archive/`：已完成设计、月度历史和审计索引。归档文档保持原始语境，不做滚动能力更新。
- `docs/sbom/`：生成的 SPDX、CycloneDX、inventory 和第三方声明快照。

已完成且没有独立合同价值的临时拆分计划、重复实施计划会删除，而不是继续留在活文档或 archive。历史 PR 和审计报告仍提供可追溯性。

## 维护规则

- 能力变化只更新 `CURRENT-BASELINE.md`；不要在多份研究文档复制同一“当前状态”。
- 稳定行为写进已有专题合同；不要为每个 PR 新建永久计划文档。
- “已实现”必须标明是 domain/data、HTTP、Agent、WebUI、desktop、artifact 还是 production evidence。
- 测试数字必须带命令和 commit，并按 test binary/runner 分桶；历史数字不外推到新 `HEAD`。
- GitHub issues 管理实时待办；文档只保存稳定依赖、阶段门和长期理由。
- 完成的计划先吸收仍有效合同，再删除或归档；审计记录不按此规则删除。
- 相对链接、状态标签、日期、commit 和“已交付/未交付”措辞是 docs-only 变更的最低校验项。
