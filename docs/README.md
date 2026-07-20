# AIRP 文档地图

> 最后校准：2026-07-20，`main@7895f8c`
>
> 当前事实入口：[CURRENT-BASELINE.md](CURRENT-BASELINE.md)

源码、manifest、测试与可重复运行证据高于文档；GitHub issues 是未完成工作的实时追踪面。本文定义文档角色，防止计划、研究和历史材料冒充当前能力。

## 最短阅读路径

1. [CURRENT-BASELINE.md](CURRENT-BASELINE.md)：当前能力、缺口、顺序与最近证据；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：工程不变式、仓库边界、验证与交付流程；
3. [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)：当前 release gates；
4. 与任务直接相关的专题合同；
5. [PLAN.md](PLAN.md)：长期产品原则，不用于证明交付。

## 活文档与合同

| 范围 | 文档 | 角色 |
|---|---|---|
| 当前事实 | [CURRENT-BASELINE.md](CURRENT-BASELINE.md) | 唯一人工维护的全项目实时基线 |
| 开发交接 | [DEV-GUIDE.md](DEV-GUIDE.md)、根 `AGENTS.md` | 工程不变式、本机约束、审计与交付规则 |
| 产品与发布 | [PLAN.md](PLAN.md)、[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) | 长期方向与近期 P1–P3 门禁 |
| 生产架构 | [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)、[SECURITY.md](SECURITY.md)、[RISK-REGISTER.md](RISK-REGISTER.md) | 已接受 P0 边界、安全规则与开放风险 |
| 会话与历史 | [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md) | 目标存档/revision 合同与已实现 durable history |
| Persona | [PERSONA-HTTP-API-PLAN.md](PERSONA-HTTP-API-PLAN.md)、[Persona WebUI closure spec](archive/2026-07-15-persona-webui-closure-design.md) | 已实现 HTTP/pipeline/effective/WebUI 绑定闭环；高级生命周期仍开放 |
| Worldbook | [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)、[Worldbook closure spec](archive/2026-07-15-worldbook-management-design.md) | 当前 canonical schema、normalizer、runtime 语义与已交付主面板闭环 |
| 资产策略 | [ASSET-SPEC.md](ASSET-SPEC.md) | 候选版本化资产规格；尚非已发布标准 |
| Agent/扩展 | [AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md)、[UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) | 待实现编排规范与已接受 UI/Widget 边界 |
| 来源吸收 | [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)、[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) | 第一方来源规则、第三方研究与 provenance |

## 候选目录与研究资料

以下文档用于选型或需求审计，不是当前 capability inventory：

- [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md)：四个第一方前序项目的能力筛选；
- [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)：源 MCP-Server catalog 与 AIRP 落点；
- [PARTS.md](PARTS.md)：源项目零件索引；
- [TAVERN-PARITY.md](TAVERN-PARITY.md)：SillyTavern 功能/互操作性对标；
- [HERMES-MEMORY.md](HERMES-MEMORY.md)：长期记忆、skills、Soul 候选方向；
- [LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)：长篇 RP/authoring 研究参考。

这些文档中的状态图例通常描述“来源有无”或“候选价值”，不证明 AIRP 的 domain、HTTP、Agent tool 或 UI 已交付。

## 目录入口

- [../README.md](../README.md)：项目入口与常用命令；
- [../engine/README.md](../engine/README.md)：engine 能力、API 与模块；
- [../ui/README.md](../ui/README.md)：暂停中的桌面客户端资产；
- [../webui/README.md](../webui/README.md)：WebUI 开发与验证；
- [../data/README.md](../data/README.md)：数据根和入仓边界；
- [../deploy/production/README.md](../deploy/production/README.md)：P0 production preview 操作。

## 历史归档

- [archive/PROJECT-HISTORY-2026-07.md](archive/PROJECT-HISTORY-2026-07.md)：项目审计与实施历史；
- [archive/WEBUI-HISTORY-2026-07.md](archive/WEBUI-HISTORY-2026-07.md)：已完成 WebUI 计划和验证；
- [archive/PR-AUDITS-2026-07.md](archive/PR-AUDITS-2026-07.md)：逐 PR 审计索引；
- [archive/2026-07-15-persona-webui-closure-design.md](archive/2026-07-15-persona-webui-closure-design.md)：Persona WebUI 闭环设计；
- [archive/2026-07-15-worldbook-management-design.md](archive/2026-07-15-worldbook-management-design.md)：Worldbook 管理设计；
- [archive/2026-07-16-unified-revision-design.md](archive/2026-07-16-unified-revision-design.md)：统一 revision 设计；
- [archive/2026-07-17-onboarding-wizard-design.md](archive/2026-07-17-onboarding-wizard-design.md)：Onboarding 向导设计；
- [archive/2026-07-19-chat-experience-upgrade-design.md](archive/2026-07-19-chat-experience-upgrade-design.md)：聊天体验升级设计。

归档不提供当前任务排序。被压缩的原文可按归档页记录的 commit 使用 `git show` 恢复。

## 维护规则

1. “已实现”必须指向当前源码入口和测试，并区分 domain/data、HTTP、Agent tool、WebUI、desktop 与 production evidence。
2. 测试数字只属于明确 commit/命令快照；不能在后续变更中沿用。
3. 普通 PR review 留在 GitHub；未修审计意见在 PR 合并后写 issue，不复制到多份 Markdown。
4. 新专题文档只有在现有活文档无法承载且会持续维护时才创建。
5. 完成旧计划后，先吸收仍有效合同，再压缩进 archive 并删除散落原文。
6. 新增研究或第三方依赖时同步 `ACKNOWLEDGEMENTS.md`；研究状态不得升级为交付声明。
7. 每次基线校准至少检查：tracked Markdown、相对链接、日期/commit、开放 issue、敏感信息、本地路径、重复入口和历史状态残留。
