# UI 协议与 Widget 决策

> 状态：已接受
> 日期：2026-07-03
> 上位决策：[SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)

## 结论

AIRP-State-Protocol 是有价值的 UI 协议与 Widget 资产来源，但不是 AIRP 的产品北极星。

AIRP 首先是一个带无头引擎的 RP 特化 AI Agent 客户端。UI 协议服务于这个产品闭环，不能把项目重新拉回"通用 Agent UI 标准"或"协议优先平台"。

同时，UI 协议与 Widget 代码必须服务全项目代码取向：更开放、更透明、在未来更易修正、且更易迭代更新。这里的"开放"不是提前做公共标准，而是接口清楚、扩展点可控；"透明"是状态、错误、权限和运行时验收可观察；"易修正/易迭代"是边界低耦合、协议版本化、能小步迁移。

## 必须保留

- **Blueprint**：UI 渲染来自引擎的声明式 Blueprint。Agent 不得在运行时写 Vue、JavaScript 或任意前端代码。
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
- 运行时验证是功能的一部分，尤其是打包 `.exe` 启动、engine 集成、GUI smoke 和 perf spike。

## 必须降级

- "通用 Agent UI 标准"是未来可能性，不是当前目标。
- "协议是核心资产"降级。核心资产是 AIRP 的 RP agent 引擎与产品闭环。
- "乐高，不是套件"降级。AIRP 先需要一条可靠默认集成链路，再谈可选扩展点。
- Gateway 作为默认 UI 后端降级。Gateway 的传输/安全思路可以吸收，但 AIRP UI 默认连接 AIRP engine。
- 第三方 widget 市场后置，等首方 widget 面和 capability 强制稳定后再谈。

## 工程规则

1. 不能因为旧 State-Protocol 项目有某个抽象，就接受一个 UI 功能。它必须服务 AIRP 工作流。
2. 不要在首方工作流具体化前新增扩展点。
3. 不要让 widget 持有 RP 数据真相源。引擎拥有真相；widget 只渲染并发出 intent。
4. 不运行 agent 生成的前端代码。只能渲染 Blueprint，或加载已安装、已审查的 widget 模块。
5. 任何面向 widget 的新 capability 都必须有 engine 侧强制方案。
6. UI 架构变更必须包含打包运行时 smoke 和性能检查。
7. Widget/Blueprint 变更必须保持可观察、可迁移、可回退：schema 变动有版本，状态 patch 可审计，错误能落到明确边界。

## 当前采纳表

| 资产 | 决策 |
|---|---|
| Blueprint schema/concept | 保留并改造为 AIRP 内部渲染合同 |
| Widget Registry / WidgetHost | 保留；首方 RP widget 优先 |
| RFC6902 store | 保留；`test` 已做 patch 前预校验，失败不半应用 |
| Tauri + Vue shell | 保留；当前桌面客户端 |
| AgentBus 抽象 | 保留为 UI 侧接缝，但默认实现指向 AIRP engine |
| MockBus | 仅测试/演示 |
| Capability declarations | 保留；敏感用途前必须补 engine 侧强制 |
| Consent/sandbox | 保留为 UI 纵深防御 |
| 通用协议/市场野心 | 仅未来可能性，非当前范围 |

## 实践方向

正确姿态是：

> 积极吸收 Blueprint 与 Widget 架构，但让 AIRP 的产品闭环掌握方向。

UI 应该成为强大、可扩展的 AIRP 客户端，而不是一个刚好能跑 RP 的通用协议 demo。
