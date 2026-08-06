# UI 协议与 Widget 决策

> 状态：已接受
> 决策日期：2026-07-03；边界复核：2026-07-30，`main@4f3f792`；2026-08-06，`main@e28ea02`（C-P0~C-P4 落地后复核，见文末修订记录）
> 上位决策：[SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)

## 结论

AIRP-State-Protocol 是有价值的 UI 协议与 Widget 资产来源，但不是 AIRP 的产品北极星。

AIRP 首先是一个带无头引擎的 RP 特化 AI Agent 客户端。UI 协议服务于这个产品闭环，不能把项目重新拉回"通用 Agent UI 标准"或"协议优先平台"。

同时，UI 协议与 Widget 代码必须服务全项目代码取向：更开放、更透明、在未来更易修正、且更易迭代更新。这里的"开放"不是提前做公共标准，而是接口清楚、扩展点可控；"透明"是状态、错误、权限和运行时验收可观察；"易修正/易迭代"是边界低耦合、协议版本化、能小步迁移。

## 必须保留

- **Blueprint**：UI 渲染来自引擎的声明式 Blueprint。Agent 不得在运行时写 Vue、JavaScript 或任意前端代码。
  - ⚠️ **superseded（2026-08-05）**：「UI 渲染来自引擎的声明式 Blueprint」的表述已被文末「修订记录 → 2026-08-05：Blueprint 定位校准（C-P4 扩展合同收口）」章节取代：Blueprint 在 AIRP 中的定位是声明式 slot 组合 / 仪表盘合同层（当前由 catalog 的 slot 计划承担），不是运行时 Vue 渲染器；运行时渲染由 webui 原生 JS 承担。后半句不变式（Agent 不得在运行时写前端代码）仍然有效。
- **Widget 系统**：保留 Widget Registry、WidgetHost、首方 widget、manifest 元数据和受控第三方 widget 加载。
- **状态 patch**：保留 RFC6902 风格 patch，用于细粒度 UI 状态更新。
- **Envelope 类型**：在有助于当前 Tauri 和未来 web 客户端共用时，保留传输无关消息形状。
- **运行时 guard**：状态进入渲染器前必须做结构校验。
- **性能纪律**：保留虚拟列表、稳定 ID、有界前端状态、patch 优先更新。
- **consent 与 sandbox 思路**：保留 UI 侧授权和 iframe 沙箱，但它们只是纵深防御，不是唯一安全边界。

## 必须改写

- Blueprint 是 **AIRP 内部 UI 合同**，暂时不是公共标准化目标。
- Widget 扩展必须 **产品驱动**。首方 RP 工作流优先：聊天、角色卡、记忆、情绪/state、物品、任务、地图、设置、诊断。
- 不可信 widget 接触敏感数据或触发特权动作前，必须有 engine 侧 capability 强制。仅靠 UI 检查不够。
- 默认链路必须真实且可验收：`UI -> Tauri bridge -> engine -> state patch -> Blueprint/widget render`。MockBus 只保留给测试和演示。
  - ⚠️ **superseded（2026-08-06）**：「Tauri bridge 链路」表述已被 C-P0 落地事实取代：默认链路现为 `webui（浏览器或 Tauri 壳同源承载）-> REST/SSE -> engine -> catalog/slot 计划 -> widget render`，壳仅承担 bearer 注入与 token 续期通道（见 CURRENT-BASELINE §2.3）。「默认链路必须真实可验收、MockBus 只留给测试演示」的要求本身不变。
- 运行时验证是功能的一部分，尤其是打包 `.exe` 启动、engine 集成、GUI smoke 和 perf spike。

## 必须降级

- "通用 Agent UI 标准"是未来可能性，不是当前目标。
- "协议是核心资产"降级。核心资产是 AIRP 的 RP agent 引擎与产品闭环。
- "乐高，不是套件"降级。AIRP 先需要一条可靠默认集成链路，再谈可选扩展点。
- Gateway 作为默认 UI 后端降级。Gateway 的传输/安全思路可以吸收，但 AIRP UI 默认连接 AIRP engine。
- 第三方 widget 市场后置，等首方 widget 面和 capability 强制稳定后再谈。
  - 2026-08-06 复核：首方 widget 面与 capability 强制已首批落地（C-P1~C-P4），受控第三方安装面（digest-pinned + 沙箱 + consent）已存在；但「市场」（分发/签名/吊销）仍后置，本条结论不变。

## 工程规则

1. 不能因为旧 State-Protocol 项目有某个抽象，就接受一个 UI 功能。它必须服务 AIRP 工作流。
2. 不要在首方工作流具体化前新增扩展点。
   - ⚠️ **superseded（2026-08-06）**：C-P1~C-P4 已在首批首方 widget（时钟/状态胶囊）接线同批落地了受控扩展点（slot 封闭集、digest-pinned 安装、capability 授权、Widget SDK）；本条的历史结论（扩展点必须产品驱动、随首方工作流一起验收）以新事实延续，不再作为「冻结扩展点」依据。
3. 不要让 widget 持有 RP 数据真相源。引擎拥有真相；widget 只渲染并发出 intent。
4. 不运行 agent 生成的前端代码。只能渲染 Blueprint，或加载已安装、已审查的 widget 模块。
5. 任何面向 widget 的新 capability 都必须有 engine 侧强制方案。
6. UI 架构变更必须包含打包运行时 smoke 和性能检查。
7. Widget/Blueprint 变更必须保持可观察、可迁移、可回退：schema 变动有版本，状态 patch 可审计，错误能落到明确边界。
8. SSE 生成流事件合同以 [`protocol/sse-events.json`](../protocol/sse-events.json) 为机器可读唯一事实源（additive-only；发射端形状由 engine 测试锁定，消费端一致性由 webui/deploy 合同测试守护）。
9. widget intent 执行面合同以 [`protocol/widget-intents.json`](../protocol/widget-intents.json) 为机器可读唯一事实源（additive-only；C-P2 拒绝默认，capability 字段为 C-P3 逐调用强制预留）。扩展目录（manifests + slot 计划）由 engine `GET /v1/extensions/catalog` 权威下发，webui 静态 slots.json 仅作降级。

## 当前采纳表

| 资产 | 决策 |
|---|---|
| Blueprint schema/concept | 保留并改造为 AIRP 内部渲染合同 ⚠️ superseded（2026-08-05）：「内部渲染合同」的定位已被文末「修订记录 → 2026-08-05：Blueprint 定位校准」取代——Blueprint 是声明式 slot 组合 / 仪表盘合同层，非运行时渲染技术；保留结论本身不变 |
| Widget Registry / WidgetHost | 保留；首方 RP widget 优先 |
| RFC6902 store | 保留；`test` 已做 patch 前预校验，失败不半应用 |
| Tauri + Vue shell | 保留；当前桌面客户端 ⚠️ superseded（2026-08-06）：C-P0（PR #480）后 Tauri 壳转为同源承载 engine webui + bearer 注入通道，Vue 主面归档；见 CURRENT-BASELINE §2.3 |
| AgentBus 抽象 | 保留为 UI 侧接缝，但默认实现指向 AIRP engine |
| MockBus | 仅测试/演示 |
| Capability declarations | 保留；敏感用途前必须补 engine 侧强制 ✅ 已落地（2026-08-06）：C-P3（PR #486）实现 engine 权威逐调用强制（`/v1/widget-intents` + capability 封闭集 + `GET /v1/grants`）；MCP/plugin 授权主体尚未接入统一面 |
| Consent/sandbox | 保留为 UI 纵深防御 |
| 通用协议/市场野心 | 仅未来可能性，非当前范围 |

## 实践方向

正确姿态是：

> 积极吸收 Blueprint 与 Widget 架构，但让 AIRP 的产品闭环掌握方向。

UI 应该成为强大、可扩展的 AIRP 客户端，而不是一个刚好能跑 RP 的通用协议 demo。

## 修订记录

### 2026-08-05：Blueprint 定位校准（C-P4 扩展合同收口）

**背景**：原决策第 17 行将 Blueprint 描述为"UI 渲染来自引擎的声明式 Blueprint"，未界定其与 widget 系统、slot 计划的边界。C-P1～C-P3 落地后，实际架构清晰：

- **slot 计划**（`GET /v1/extensions/catalog` 权威下发，降级 `webui/assets/widgets/slots.json`）= 声明式 slot 组合 / 仪表盘合同层：哪些 widget 实例挂在哪些 slot 上。
- **widget manifest**（catalog 下发的 manifests 列表）= widget 元数据合同层：type / version / capabilities / entry.source / sandbox。
- **Blueprint**（原 State-Protocol 资产中的 Vue 声明式渲染层）= **未来声明式组合层**，当前未在 webui 主线落地。

**校准结论**：

1. Blueprint 在 AIRP 中的定位是 **声明式 slot 组合 / 仪表盘合同层**，不是运行时 Vue 渲染器。当前由 catalog 的 slot 计划承担该职责；未来若引入声明式仪表盘编辑器，其输出即为 slot 计划 JSON。
2. 核心不变式完整满足：
   - Agent 不得在运行时写 Vue / JavaScript / 任意前端代码（原不变式）；
   - widget 不得持 RP 数据真相源，引擎拥有真相（原不变式）；
   - 第三方 widget 接触敏感数据或触发特权动作前，必须有 engine 侧 capability 强制（C-P3 已落地）；
   - 运行时验证是功能的一部分（原不变式）。
3. "声明式 Blueprint"在 AIRP 语境中 = catalog 下发的 slot 计划 + widget manifest 合同。它不是 agent 生成的 Vue 组件树，而是 engine 权威下发的、可审计、可回退的 JSON 合同。
4. 原 State-Protocol 的 Vue BlueprintRenderer 壳留在 `.worktrees/strategic-reaudit`（PR #458 研究归档），不进入主线。AIRP webui 用原生 JS + slots.js + widget-host.js 承担渲染职责。

**与原决策的差异**：

| 原决策表述 | 校准后定位 | 理由 |
|---|---|---|
| "UI 渲染来自引擎的声明式 Blueprint" | slot 计划 + widget manifest 是声明式合同层；运行时渲染由 webui 原生 JS 承担 | C-P1～C-P3 实际架构 |
| Blueprint 是 Vue 渲染器 | Blueprint 是合同层概念，非具体渲染技术 | 解耦合同与渲染实现 |
| Agent 生成 Blueprint | Agent 不得生成前端代码；catalog 由 engine 权威下发 | 原不变式保留，语义更明确 |

**不变项**：原决策"必须保留"清单全部保留（Blueprint 概念、Widget 系统、状态 patch、Envelope 类型、运行时 guard、性能纪律、consent/sandbox）。本次校准仅明确 Blueprint 的实现定位，不削弱任何不变式。

### 2026-08-06：v0.0.4 边界复核（C-P0~C-P4 落地，审计遗留 #485 D1）

**背景**：原 header 的边界复核锚点停在 `main@4f3f792`（v0.0.3 engine gate 前）。PR #480~#487/#491/#492 落地后，widget-extension 安装/catalog/授权面与 desktop-session token rotation 已是 runtime 事实，本决策文档的若干历史结论与新事实不一致。按「不删历史结论、标注 superseded 并指向新事实」处理：

1. **header 锚点**：补记 2026-08-06 复核（`main@e28ea02`）。
2. **「必须改写」默认链路**：`UI -> Tauri bridge -> engine -> ...` 已被 C-P0 事实取代（webui 同源承载 + REST/SSE + bearer 注入通道），就地标注 superseded。
3. **工程规则 2（首方具体化前不新增扩展点）**：C-P1~C-P4 的扩展点与首批首方 widget 同批落地并带契约测试，历史结论以「产品驱动、随首方验收」语义延续，就地标注 superseded。
4. **采纳表**：`Tauri + Vue shell` 行标注 C-P0 后新定位；`Capability declarations` 行标注 engine 侧强制已落地（C-P3）。
5. **工程规则 8/9（SSE 事件合同与 widget intent 合同）**：已在 PR #464/#487 固化为机器可读事实源，与当前事实一致，无需改动。
6. **未变项**：「必须保留」清单与「必须降级」清单（含第三方 widget 市场后置）结论不变；consent/sandbox 仍定位为纵深防御而非唯一安全边界。

**当前事实入口**：桌面壳与扩展合同的交付边界与已知限制（含 #485 剩余 W4/W5/W6/T1、壳续期循环未经 GUI 真机确认）以 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) §2.3 为准；安全边界见 [SECURITY.md](SECURITY.md)；新风险见 [RISK-REGISTER.md](RISK-REGISTER.md) RR-015~RR-017。

