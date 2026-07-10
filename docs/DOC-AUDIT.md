# AIRP 文档审计与权威层级

> 最后更新：2026-07-10

2026-07-10 已完成一次基于源码、近期 PR、全部 open issues、仓库 Markdown 与本地验证的全项目独立审计。完整结果见 [PROJECT-AUDIT-2026-07-10.md](PROJECT-AUDIT-2026-07-10.md)。本文件不再保存已经过期的“待拍板问题”，只定义文档如何使用。

## 权威顺序

1. **源码、manifests、测试和可重复运行证据**：判断已交付能力的最高事实来源；
2. **[PROJECT-AUDIT-2026-07-10.md](PROJECT-AUDIT-2026-07-10.md)**：当前状态、风险、issue 顺序和近期成功判据；
3. **[DEV-GUIDE.md](DEV-GUIDE.md)**：当前实施 Agent 的执行入口；
4. **[PLAN.md](PLAN.md)**：长期产品原则与目标架构，不用于证明某功能已完成；
5. **专题设计文档**：只约束其明确标注的范围；
6. **`docs/audits/`、`docs/issues/`、`docs/superpowers/plans/`**：历史证据与实施记录。除非明确更新为当前状态，否则不得当作当前待办或完成清单。

## 文档类型

| 类型 | 文件 | 使用规则 |
|---|---|---|
| 当前入口 | 根 `README.md` | 只写今天可证明的状态和入口 |
| 当前审计 | `PROJECT-AUDIT-2026-07-10.md` | 新审计出现前的状态快照 |
| 长期原则 | `PLAN.md`、`SOURCE-PROJECT-DECISIONS.md`、`UI-PROTOCOL-DECISION.md` | 解释为什么，不自动代表已实现 |
| 实施手册 | `DEV-GUIDE.md` | 开头的当前接手入口覆盖旧 Phase 记录 |
| 能力候选 | `PARTS.md`、`MCP-SERVER-ABSORPTION.md`、`TAVERN-PARITY.md` | 候选资产/需求清单，不等于本仓 capability inventory |
| 研究资料 | `HERMES-*`、`LEARN-*`、`ASSET-SPEC.md` | 路线研究或 draft，必须标注落地状态 |
| 历史记录 | `docs/audits/`、`docs/issues/`、`docs/superpowers/plans/` | 保留来源与当时判断，不回写成当前事实 |

## 本轮发现的文档问题

- 多份文档停留在 PR #13，遗漏 PR #77–#100；
- `engine/README.md` 混入源 AIRP-Core 的 standalone/乐高定位和不存在的 persona/plugin/MCP 工具；
- “底层已有能力”“HTTP 已暴露”“Agent 已注册工具”三种状态被混写；
- 世界书被写成 0%，但实际已有基础实现；Agent tools 被写成仅 echo，但实际 registry 有 11 个；
- 研究路线和源仓库数字（38 工具、12 prompts、19 resources）被误当成本仓交付目标；
- decompose implementation plan 已实施，却仍保留未勾选任务和执行交接语气。

## 维护规则

1. 声称“已实现”必须能指向当前源码入口和验证；
2. 能力表必须区分 domain/data、HTTP、Agent tool、UI 四层；
3. 测试数字只在同日实际运行后更新，并写明命令与边界；
4. 历史 audit/plan 不删除，但在文件顶部标注其历史状态；
5. 新 PR 若改变当前能力，应至少同步 README 或当前审计/DEV-GUIDE 中的对应入口；
6. 不把 issue 里的建议措辞升级为架构不变式；不把 helper、schema 或 UI mock 当作用户价值闭环。
