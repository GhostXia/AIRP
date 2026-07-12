# AIRP 文档审计与权威层级

> 最后更新：2026-07-12

2026-07-12 已在 PR #118/#119/#121 合并后重新核对源码、开放 issues、仓库 Markdown 与验证结果。[CURRENT-BASELINE.md](CURRENT-BASELINE.md) 是实时事实入口；[PROJECT-AUDIT-2026-07-10.md](PROJECT-AUDIT-2026-07-10.md) 保留其 dated audit 价值。

## 权威顺序

1. **源码、manifests、测试、运行时工具目录和可重复运行证据**：判断已交付能力的最高事实来源；
2. **[CURRENT-BASELINE.md](CURRENT-BASELINE.md)、当前入口 README、本文与 [DEV-GUIDE.md](DEV-GUIDE.md)**：当前能力和实施入口；
3. **[PROJECT-AUDIT-2026-07-10.md](PROJECT-AUDIT-2026-07-10.md)**：带明确基线的历史状态、风险和排序；
4. **[PLAN.md](PLAN.md)**：长期产品原则与目标架构，不用于证明某功能已完成；
5. **专题设计文档**：只约束其明确标注的范围；
6. **`docs/audits/`、`docs/issues/`、`docs/superpowers/plans/`**：历史证据与实施记录。除非明确更新为当前状态，否则不得当作当前待办或完成清单。

## 当前实现快照

- 默认 Agent registry 有 19 个工具；`GET /v1/agent/tools` 是实际运行时目录，WebUI 的 allow/confirm 控件消费该目录。
- 角色卡读取在 data、HTTP、Agent adapter 间复用同一 JSON object 合同。
- 模型可见的大文本使用 UTF-8 安全上限，`AIRP_MAX_READ_BYTES` 默认 32 KiB。
- `apply_lorebook`、`merge_lorebooks`、`seal_volume`、`export_context_bundle` 已注册；替换和封卷继续受 destructive confirm 保护。
- context bundle 固定写入 data root，稳定材料在易变 state 之前，并明确供 fresh isolated subagent 使用。产物措辞测试不替代独立的 no-orchestrator-noise 不变式。
- WebUI 已接通 Persona/Preset/session lifecycle；剩余门槛是零密钥 mock-provider 全链路 browser acceptance。

## 文档类型与本轮全量审计处置

| 类型 | 文件范围 | 使用规则 / 处置 |
|---|---|---|
| 当前入口 | 根、`engine/`、`protocol/`、`ui/`、`webui/`、`data/` README，`AGENTS.md` | 已检查；受本轮影响的工具数、验证与数据路径已同步。`AGENTS.md` 保持为操作政策。 |
| 当前架构/运维 | `PLAN.md`、`DEV-GUIDE.md`、`SECURITY.md`、`RISK-REGISTER.md`、`SOURCE-PROJECT-DECISIONS.md`、`UI-PROTOCOL-DECISION.md`、`ASSET-SPEC.md` | 已检查；同步受影响的 Agent/data 边界，无关协议和资产合同保持不变。 |
| 能力候选 | `PARTS.md`、`MCP-SERVER-ABSORPTION.md`、`TAVERN-PARITY.md`、`HERMES-*`、`LEARN-*`、extension/widget 文档 | 已检查；仍是候选资产/路线，不等于本仓 capability inventory；受影响条目已同步。 |
| WebUI 计划/验证 | `WEBUI-BACKEND-*`、`WEBUI-ANALYSIS-AND-OPTIMIZATION.md`、`WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md`、smoke evidence | 已检查；dated plans 保留历史状态，当前行为以 `webui/README.md` 为准。 |
| dated project/PR audits | `PROJECT-AUDIT-2026-07-10.md`、`AUDIT-AND-ROADMAP-2026-07.md`、`docs/audits/*.md` | 保留为证据。项目审计增加 post-baseline 注记；合并后的 PR audit 不改写。 |
| issue/实施记录 | `docs/issues/*.md`、`docs/superpowers/plans/*.md` | 保留历史 line reference、旧数字和当时判断，不把它们伪装成实时状态。 |

## 维护规则

1. 声称“已实现”必须能指向当前源码入口和验证；
2. 能力表必须区分 domain/data、HTTP、Agent tool、UI 四层；
3. 测试数字只在同日实际运行后更新，并写明命令与边界；
4. 历史 audit/plan 不删除，但在文件顶部标注其历史状态或由本文明确分类；
5. 新 PR 若改变当前能力，应至少同步 README 或本文/DEV-GUIDE 中的对应入口；
6. 不把 issue 的建议措辞升级为架构不变式；不把 helper、schema 或 UI mock 当作用户价值闭环。

## 当前近期计划入口（2026-07-12）

[CURRENT-BASELINE.md](CURRENT-BASELINE.md) 是新 session 的唯一实时入口。[WEBUI-MVP-PLAN.md](WEBUI-MVP-PLAN.md) 继续定义验收合同：PR A 实现范围已经完成，当前只执行 browser acceptance 与其阻塞修复。通过后必须基于真实证据和开放 issue 重新排序。
