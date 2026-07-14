# AIRP 文档地图

> 最后校准：2026-07-14
>
> 当前事实入口：[CURRENT-BASELINE.md](CURRENT-BASELINE.md)

本文定义文档权威层级和最短阅读路径。源码、manifest、测试与可重复运行证据始终高于文档；
GitHub issues 是未完成工作的唯一实时追踪面。

## 新开发 session：只读这 5 份

1. [CURRENT-BASELINE.md](CURRENT-BASELINE.md)：当前能力、缺口、风险和下一步；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：工程不变式、工具链和交付流程；
3. [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)：当前 WebUI 上线门禁；
4. [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)：P0 拓扑与威胁边界；
5. [PLAN.md](PLAN.md)：长期产品原则，不用于证明功能已交付。

## 活文档

| 范围 | 文档 | 职责 |
|---|---|---|
| 当前事实 | `CURRENT-BASELINE.md` | 唯一实时基线；每次能力或优先级变化时更新。 |
| 开发流程 | `DEV-GUIDE.md`、根 `AGENTS.md` | 工具链、不变式、审计与交付规则。 |
| 产品/发布 | `PLAN.md`、`WEBUI-PRODUCTION-PLAN.md` | 长期方向与当前 release gates。 |
| 架构/安全 | `WEBUI-PRODUCTION-ARCHITECTURE.md`、`SECURITY.md`、`RISK-REGISTER.md` | 已接受边界和仍开放风险。 |
| 数据合同 | `LONG-HISTORY-CONTRACT.md`、`WORLDBOOK-SEMANTICS.md`、`ASSET-SPEC.md` | 已实现合同与明确标注的候选规格。 |
| 扩展方向 | `AGENT-ORCHESTRATION.md`、`UI-PROTOCOL-DECISION.md` | 待实现规范；不能写成现有 runtime。 |
| 来源边界 | `SOURCE-PROJECT-DECISIONS.md` | 第一方源仓吸收原则。 |
| 候选能力 | `CAPABILITY-ABSORPTION.md`、`MCP-SERVER-ABSORPTION.md`、`PARTS.md` | 候选目录，不是当前 capability inventory。 |
| 外部研究 | `ACKNOWLEDGEMENTS.md`、`TAVERN-PARITY.md`、`HERMES-MEMORY.md`、`LEARN-NEUROBOOK.md` | 第三方研究、许可证和独立实现边界。 |

根目录及 `engine/`、`ui/`、`webui/`、`data/`、`deploy/production/` 的 README 是各自入口，
但不得覆盖当前基线的全项目结论。

## 历史归档

历史材料已压缩为三份索引：

- [archive/PROJECT-HISTORY-2026-07.md](archive/PROJECT-HISTORY-2026-07.md)
- [archive/WEBUI-HISTORY-2026-07.md](archive/WEBUI-HISTORY-2026-07.md)
- [archive/PR-AUDITS-2026-07.md](archive/PR-AUDITS-2026-07.md)

归档不提供当前任务排序。被合并原文可从 `main@6736755` 用 `git show` 精确恢复。

## 维护规则

1. “已实现”必须能指向当前源码入口和验证；issue 或设计稿不算交付。
2. 能力描述要区分 domain/data、HTTP、Agent tool、WebUI/desktop UI 四层。
3. 测试数字只记录同日真实运行结果，并注明命令和边界。
4. 普通 PR review 留在 GitHub；不要为每个 PR 永久新增一份 Markdown。
5. 未修审计意见在 PR 合并后写 GitHub issue，不在多份文档中复制待办。
6. 新专题文档只有在无法归入现有活文档、且会被持续维护时才新增。
7. 旧计划完成后，将仍有效合同吸收到活文档，再压缩进归档并删除散落原件。
