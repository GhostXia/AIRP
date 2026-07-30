# 审计：桌面端 UI 接力计划 v1 vs 原开发计划文件

> 日期：2026-07-28
> 审计对象：`docs/2026-07-29-desktop-ui-canvas-relay-plan.md` v1（路线 A+：Tauri 壳 + 复用 webui 资产）
> 对照基准：`ui/README.md`、`docs/UI-PROTOCOL-DECISION.md`、`docs/WEBUI-PRODUCTION-PLAN.md`、`docs/WEBUI-PRODUCTION-ARCHITECTURE.md`、`docs/CURRENT-BASELINE.md` 及 `ui/src` 源码
> 审计性质：计划级独立审计（非 PR 审计），遵守 AGENTS.md 审计守则：不附和、可质疑、以源码证据为准
> 结论：**v1 路线 A+ 推翻**。它把桌面端降级为 webui 的壳，系统性丢弃了原计划为桌面端保留的性能架构与扩展性架构，且与三份原计划文件的明确定位直接冲突。计划书已改写为 v2。

---

## 1. 原计划文件对桌面端的定位（证据）

| 证据 | 出处 | 内容 |
|---|---|---|
| E1 | `ui/README.md:65` | "WebUI is currently the primary backend-incubation, contract-validation, and basic RP development surface. **This Tauri/Vue client remains the long-term product delivery surface** and should consume stable client-neutral contracts after they mature." |
| E2 | `docs/WEBUI-PRODUCTION-PLAN.md:28` | "Tauri/Vue 仍保留为**长期桌面客户端**，但不再是 WebUI 正式上线的前置 release gate。" |
| E3 | `docs/WEBUI-PRODUCTION-PLAN.md:178` | 非首发阻塞项（"这些能力不得消失"）："Tauri 安装包、sidecar 生命周期与 **100k 桌面虚拟列表验收**"。 |
| E4 | `docs/UI-PROTOCOL-DECISION.md` | 决策状态"已接受"：Blueprint/Widget Registry/RFC6902 store/Tauri+Vue shell/AgentBus/consent/sandbox **必须保留**；"UI 应该成为**强大、可扩展**的 AIRP 客户端"。 |
| E5 | `docs/UI-PROTOCOL-DECISION.md` 工程规则 6/7 | "UI 架构变更**必须包含打包运行时 smoke 和性能检查**"；"Widget/Blueprint 变更必须保持可观察、可迁移、可回退"。 |

原计划的三层分工清晰：**webui = 合同孵化器**（先把 client-neutral 合同跑熟），**桌面端 = 长期旗舰交付面**（合同成熟后消费），**协议/Widget 体系 = 桌面端性能与扩展性的载体**。v1 把这三层关系倒置了。

## 2. v1 路线 A+ 的缺陷清单

### A1（严重）颠覆产品定位
v1 让桌面端"= Tauri 壳 + webui/ 资产"，即长期交付面永久寄生在合同孵化器上。与 E1/E2 直接冲突。webui 的技术选型（无构建多页面、每屏独立 HTML）是为"合同验证 + 便携包"优化的，不是为旗舰桌面体验优化的。

### A2（严重）丢弃性能架构
桌面端现存且测试覆盖的性能机制，v1 全部旁路：

| 机制 | 源码证据 | v1 的命运 |
|---|---|---|
| id-keyed 聊天模型 `{messages, order}`，流式 patch 直写 `/messages/{assistant_id}/text` | `ui/README.md:64`、`App.vue:175` | 丢弃（webui 页面无此模型） |
| 虚拟列表窗口化数学（纯函数、可单测） | `ui/src/widgets/virtual-window.ts`（"performance contract: only render the viewport slice"） | 丢弃 |
| 窗口化状态切片合同 | `ui/src/state/store.ts:10`（"This store only holds the live windowed slice"） | 丢弃 |
| patch 前 `test` 预校验、失败不半应用；非法 patch fail-closed | `store.ts:31-34,62-65` | 丢弃 |
| path-first 角色导入（base64 不进 Vue state/props） | `ui/README.md:63` | 丢弃 |
| 100k 消息 perf spike 验收目标 | `ui/README.md:78`、E3 | 无法达成（webui 无虚拟列表） |

E3 明确"100k 桌面虚拟列表验收"**不得消失**——v1 使其在架构上不可能。

### A3（严重）丢弃扩展性架构

| 机制 | 源码证据 | 价值 |
|---|---|---|
| Blueprint 渲染合同（agent 运行时不可写前端代码） | `protocol/types.ts:99-130`、`BlueprintRenderer.vue` | 引擎/Agent 可声明式驱动 UI，安全边界 |
| Widget Registry + manifest（props/state JSON Schema、capabilities、intents、ESM 沙箱入口） | `types.ts:132-154`、`registry/` | 首方/第三方扩展面 |
| consent + iframe 沙箱纵深防御 | `registry/consent.ts`、`sandbox-bridge.ts`、`WidgetHost.vue` | 不可信 widget 隔离 |
| 版本化 Envelope 协议 + 传输无关 AgentBus 接缝 | `types.ts:13-33`、`bus-factory.ts` | 协议演进、多端复用 |
| Blueprint `theme.tokens` 注入点 | `types.ts:107-110` | **令牌级主题扩展——画布令牌的原生对接口** |
| Layout dock/grid/stack/tabs | `types.ts:112-121` | 可伸缩工作台布局，超越固定画布 |
| Agent 测试线束 | `agent-test.ts`、`App.vue:147-160` | GUI 自动化/Playwright 扩展 |

v1 的"Widget 收编为扩展面"是空头支票：webui 静态页没有 WidgetHost/BlueprintRenderer 宿主，要让 iframe 里的无构建页面承载 Vue widget 体系，本身就是一个未验证的新造桥项目（而现成桥 `sandbox-bridge.ts` 反而被弃用）。

### A4（一般）固定窗口决策 D2 缩窄桌面潜力
v1 把窗口钉死 1440×900"对齐样板基准画布"。但 STYLEGUIDE §3 明确"响应式策略由各派生实现自行决定"；且协议 Layout 体系（E7）支持 dock/grid 可伸缩工作台——这正是桌面端超越 web 固定画布的空间。默认值可取 1440×900，但不应作为硬约束。

### A5（一般）视觉一致性论证不成立
v1 主张"复用 webui 资产 → 视觉零漂移"。但：(a) webui 对齐样板是人工逐屏对齐的结果，不是资产复用的结果——同样的逐屏对齐纪律用于 Vue 派生同样成立；(b) 真正防漂移的机制是**令牌单一事实源 + 对拍门禁**，两条路线都可用；(c) WebView2 与 Chrome 渲染差异的风险两条路线同样承担，不构成区分项。

### A6（时机判断补正）
v1 未引用 E1 的时序：桌面端"consume stable client-neutral contracts **after they mature**"。当前 webui 已成熟（44 屏、SSE、cursor 分页、revision 合同、production 拓扑），正是原计划预设的桌面端重启时点。接力计划的正当性应建立在此上。

## 3. v1 中仍成立的部分（保留进 v2）

- 接力模型本身（自包含交接包、棒次划分、溯源表、审计门禁）——与路线无关，保留；
- 视觉对拍验收（令牌 0 偏差、取色 ±1、Playwright 像素级）——保留并加强；
- 四条数据安全语义不可降级、AGENTS.md 治理条款——保留；
- WebView2 渲染 spike——保留（任何路线都需要）；
- 不改 engine API 合同、不复制 tokens 事实源——保留。

## 4. 修正方向（v2 核心）

**路线 B+（旗舰协议路线）**：桌面端 = Vue 3 + 协议内核（保留全部性能/扩展性机制）+ 新建 Console 壳与屏视图（从样板逐屏派生，令牌经 Blueprint `theme.tokens` 注入）+ 消费 webui 已成熟的 client-neutral 合同。webui 继续承担 P1 产品线与合同验证；桌面端按计划接管长期旗舰交付面。性能（100k 虚拟列表）与扩展性（Widget/Blueprint/沙箱）恢复为一等目标并设 CI 闸门。

详见归档后的 `docs/archive/2026-07-29-desktop-ui-canvas-relay-plan.md`（v3.4；本审计针对 v1，其后 v2–v3.4 已在该归档文件内演进并**取代** v1/v2 作为草案正文，本审计结论仍作否决 v1 的历史证据保留）。
