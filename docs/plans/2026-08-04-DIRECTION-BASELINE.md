# 方向共识基线决策文档

> ⚠️ **Superseded for current status**：本文保留 2026-08-04 方向决策的可追溯记录；当前版本、发布门和优先级以 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md)（`main@affa315`）为准。

> **已审定（2026-08-07 用户批准转正入库）**；并入 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md) 的增量校准留待 v0.0.5 docs-pass。
> 日期：2026-08-04
> 基线：`main@1b14a7c`（v0.0.3 已于 2026-08-03 发布）；本文 §0/§7 中引用的 PR 状态以 2026-08-07 为准：#424–#426/#455–#460/#462–#465 均已合并，v0.0.4 已发布。
> 来源：本轮用户与专家团确立的方向共识。本文只做忠实整合，不引入共识之外的新决策。
> 与 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md) 的关系：本文是其方向层补充；冲突处以 CURRENT-BASELINE 为准。

---

## 1. 宪法层：四原则

四原则凌驾所有工作层级，是**强制验收门禁**：任何层级的工作违反四原则，即为阻塞级问题。

| 原则 | 内容 |
|---|---|
| **解耦** | 模块边界清晰、可替换；domain 模型不被第三方依赖侵入。 |
| **兼容性** | 资产可导入导出；行为不欺骗用户。导入第三方资产时明确告知哪些字段未被执行——宁"不欺骗"，不追"全等价"。 |
| **扩展性** | 以 capability manifest 与声明式接缝保障架构可演进；作为架构边界坚守，但**不承诺**产品级插件生态时间表。 |
| **迁移性** | 数据必须支持完整备份、恢复、迁移与回滚。 |

---

## 2. 工作层：0-3 层排序

### 层 0 · 还债加固（先于一切）

| 项 | 对应 issue / PR |
|---|---|
| BUG-1 多用户会话数据根不对称 | BUG-1（PR #462 审计定性阻塞级），修复载体 [#463](https://github.com/GhostXia/AIRP/pull/463) |
| BUG-2 会话恢复 / 断线重连 | BUG-2（阻塞级），同属 [#463](https://github.com/GhostXia/AIRP/pull/463) |
| last-write-wins 静默数据丢失 | [#432](https://github.com/GhostXia/AIRP/issues/432)（CAS） |
| SSE 协议合同固化（RR-007） | [#464](https://github.com/GhostXia/AIRP/pull/464) |
| Session Coordinator O2–O4 | [#394](https://github.com/GhostXia/AIRP/issues/394) |
| async 中同步 IO 阻塞 | [#433](https://github.com/GhostXia/AIRP/issues/433) |
| 真实 provider / browser / Compose 验收 | [#130](https://github.com/GhostXia/AIRP/issues/130) |

栈式合并顺序：**#462 → #463 → #464 → #465**，严格顺序，每层合并后下一层 base 重定 main。

### 层 1 · 补齐自身核心闭环缺口

- 以 ST 做**缺口校准**，但不追 ST 功能等价；遵守 [TAVERN-PARITY.md](../TAVERN-PARITY.md) 已决策：不照搬机械管线、世界书 advisory 哲学。
- 重点：[#345](https://github.com/GhostXia/AIRP/issues/345) / [#346](https://github.com/GhostXia/AIRP/issues/346) / [#311](https://github.com/GhostXia/AIRP/issues/311)——资产 import/export/history/rollback、关键页面行为门禁、能力展现层。

### 层 2 · 第三方理念吸收排队

- 模式：理念吸收、独立实现、ACKNOWLEDGEMENTS 归档（[#424](https://github.com/GhostXia/AIRP/pull/424) / [#426](https://github.com/GhostXia/AIRP/pull/426) / [#455](https://github.com/GhostXia/AIRP/pull/455) / [#457](https://github.com/GhostXia/AIRP/pull/457) / [#460](https://github.com/GhostXia/AIRP/pull/460) 模式）。
- 吸收项进 [#381](https://github.com/GhostXia/AIRP/issues/381) 候选池；实现入版本需评审，不因"别人有"而进范围。

### 层 3 · 冻结区按需解冻

- [#242](https://github.com/GhostXia/AIRP/issues/242) 冻结的七个区。
- 解冻触发器**仅限**：维护者真实使用痛点，或真实用户反馈。
- agent 审计产出的 backlog **不构成**触发器。

---

## 3. 开发者模式扩展机制（合宪设计）

### 3.1 双轨信任模型（沿 [#163](https://github.com/GhostXia/AIRP/issues/163) 设计）

| 轨道 | 信任语义 |
|---|---|
| 普通模式 | deny-by-default + capability manifest 最小权限 |
| 开发者模式 | 显式开启、常驻可见提示、视同 OS 用户权限；可授予最大化 / Admin 权限 |

### 3.2 合宪四条件

1. **激活权威在引擎**：开发者模式状态由引擎定义并持久化，不可伪造。
2. **常驻可见性**：开发者模式激活期间常驻可见提示。
3. **权限通道走引擎声明的接缝**：扩展点声明权永远归引擎——hook 阶段、类型化 payload、嫁接接口均由引擎定义；**禁止未声明渗透**（monkey-patch / 网络拦截 / 反射改内部状态）。
4. **结构级改动仅开发者模式可做**，且必须版本化 migration + 改动前备份 + 可回滚——迁移性宪法对所有人（包括开发者）生效。

### 3.3 数据嫁接边界

- 状态嫁接数据放**插件命名空间侧表**，生命周期跟随宿主资产，不碰 domain 结构体。

### 3.4 声明式接缝优于未声明渗透的实用理由

- 引擎升级后依然有效、可审计。

---

## 4. 平台化方向

**主张**：尽可能完善后端能力作为平台杠杆——引擎能力 + 声明式扩展点 + 开发者模式，降低第三方插件 / 组件作者的开发与实现难度，促成插件与组件多样化。

### 落地三要素

1. **后端管线稳定可信**：层 0 是 hook 价值的前提。
2. **接缝质量决定作者难度**：稳定、类型可预期、文档化的扩展点清单；payload 变更走版本化，不静默改。
3. **作者体验**：示例插件 + harness 自测 + 扩展点文档。

### 时序纪律

- 接缝可随层 0/1 还债顺路长出（如 [#464](https://github.com/GhostXia/AIRP/pull/464) 固化 SSE 合同时预留插件可见接缝）。
- 对外招募作者与生态分发仍按层 3 纪律，等真实需求触发。
- 明确**不做**市场宣传导向的规划；最小分发（规范便携包 release）视为工程质量的一部分，而非宣传。

---

## 5. 战略备忘处置

### 5.1 PR #458（Chimera Blueprint）与 #459（战略再审计）

- 均作为**研究归档**处理，不进入路线。
- [#458](https://github.com/GhostXia/AIRP/pull/458) 若合并，须加"战略备忘（未经采纳）"状态标注。
- [#459](https://github.com/GhostXia/AIRP/pull/459) 须后于 #458 合并（相对链接依赖）。
- 论证留痕价值：#459 关于"分发与反馈循环是真缺口"的结论，作为未来市场检验设计（E1 冒烟 / E2 冷启动 / E3 卡片兼容）的输入保留；但当前路线**不以市场占有为目标**。

### 5.2 市场可行性研究结论摘要

坚持"低门槛 + 生态兼容 + 扩展性"原始主张，**有条件地**能占据小众市场；但条件目前均未满足：

- 低门槛须经 #130 级真实验证；
- 兼容须做到不欺骗；
- 至少一条分发渠道。

最强候选维度是**低门槛**（零依赖便携包），最弱是**扩展性**（当前为纯承诺）。

详见 `.tmp/market-feasibility-original-philosophy.md`（该文件为工作产物，正式归档时可另行处置）。

---

## 6. 开放决策点（留待用户后续裁定）

| 决策点 | 说明 |
|---|---|
| [#453](https://github.com/GhostXia/AIRP/issues/453) 桌面接力路线 | v4 / v5 / 融合三选一；v0.0.4 团队的硬前置 |
| [#130](https://github.com/GhostXia/AIRP/issues/130) 定位 | 主线门禁 vs future track；#242 与 CURRENT-BASELINE 存在状态漂移 |
| [#344](https://github.com/GhostXia/AIRP/issues/344) Director/Council | 闭合 vs 文档降级 |
| [#387](https://github.com/GhostXia/AIRP/issues/387) Benchmark | 是否立项 |
| [#388](https://github.com/GhostXia/AIRP/issues/388) PulsarAI | 短期两项是否吸收 |
| [#163](https://github.com/GhostXia/AIRP/issues/163) / [#461](https://github.com/GhostXia/AIRP/issues/461) 扩展生态 | 正式立项时机；当前按层 3 纪律冻结（接缝设计除外） |

---

## 7. 附：本批 docs PR 处置速查表

来源：`.tmp/v005-consolidated-analysis.md` 各 PR 裁决。

| PR | 主题 | 处置 |
|---|---|---|
| [#424](https://github.com/GhostXia/AIRP/pull/424) | 记忆/分卷系统对比审计 | 门禁后可合 |
| [#426](https://github.com/GhostXia/AIRP/pull/426) | AiChat 远程只读研究 | 门禁后可合 |
| [#455](https://github.com/GhostXia/AIRP/pull/455) | LoopX 控制面研究 | 需修正（漏 ACKNOWLEDGEMENTS 文件或更正 body） |
| [#456](https://github.com/GhostXia/AIRP/pull/456) | 用户自定义流程设计（C 方案） | 小修后可合 |
| [#457](https://github.com/GhostXia/AIRP/pull/457) | Marinara-Engine 研究 | 需归口（与 #458 定 Marinara 条目归属） |
| [#458](https://github.com/GhostXia/AIRP/pull/458) | Chimera Blueprint | 研究归档处置；合并须加状态标注（见 §5.1） |
| [#459](https://github.com/GhostXia/AIRP/pull/459) | 对 #458 的战略再审计 | 研究归档处置；须后于 #458 合并 |
| [#460](https://github.com/GhostXia/AIRP/pull/460) | talemate 深度审计归档 | 门禁后可合（先与 #458 归口 Talemate 条目） |
| [#462](https://github.com/GhostXia/AIRP/pull/462) | 深度审计基线报告 | 走完整门禁（含 .rs 改动，不适用 docs-only 豁免） |
| [#463](https://github.com/GhostXia/AIRP/pull/463) / [#464](https://github.com/GhostXia/AIRP/pull/464) / [#465](https://github.com/GhostXia/AIRP/pull/465) | 栈式代码修复 | 走完整门禁，按栈序 462→463→464→465 合并 |

注：docs-only 豁免仅适用纯文档 PR；全部 PR 合并仍受 CURRENT-BASELINE 审计门禁与 #242 范围收敛约束。#465 触及 `ui/`，与 v0.0.4 桌面线存在文件冲突面（待核实具体冲突范围）。

---

*本文档于 2026-08-07 经用户审定转正；其中 §6 开放决策点仍待逐项裁定，§0/§7 中已合并 PR 的处置描述为历史快照。*
