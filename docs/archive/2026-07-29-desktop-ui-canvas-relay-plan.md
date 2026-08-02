> **已归档（2026-07-30）**：桌面发布暂停期间不占活文档位。审计见 `docs/audits/2026-07-28-desktop-ui-relay-plan-audit.md`。重启桌面产品线前须重新校准基线。

# 桌面端 UI 画布接力开发计划书 v3.7

> 状态：草案 v3.7（待用户评审）
> 日期：2026-08-02
> 修订轨迹：v1（路线 A+：Tauri 壳复用 webui 资产）经计划级审计推翻（`docs/audits/2026-07-28-desktop-ui-relay-plan-audit.md`）→ v2 恢复旗舰协议路线 B+ → v3 增强三性（自定义性/扩展性/兼容性）→ v3.1 第三方 Widget 导入提升为一等交付 → v3.2 AI 驱动 Widget 生成三级路径 → v3.3 酒馆无缝衔接层 → v3.4 两手策略（第三方兼容模式声明式开放）→ v3.5 带数据库/后端的第三方插件治理（§4.6.5）→ v3.6 HTML/富渲染治理（§4.7）→ **v3.7 吸收第三方架构评审（`airp-frontend-architecture-report`，2026-08-02）：安全模型框为「爆炸半径遏制」、新增引擎 Hook API 与插件数据共存模型（§4.8）、沙箱 E2E spike 提前 R0 等战术项；立场翻转项不直接改，列入附录 D 待拍板**。
> 上位文件：`airp-engine-console/STYLEGUIDE.md`、`docs/CURRENT-BASELINE.md`、`docs/UI-PROTOCOL-DECISION.md`、`docs/WEBUI-PRODUCTION-PLAN.md`、`AGENTS.md`

---

## 0. 对「Widget 是不是最佳组件模型」的裁决

**结论：Widget 是对的，予以保留并升为核心载体；但裸 Widget 不够，必须配齐围绕它的三层体系。**

依据：
- 现有 WidgetDef manifest（props/state JSON Schema、capabilities、intents、ESM 沙箱入口）已是受控扩展的成熟形态，与 VS Code / Obsidian 等验证过的模型同构；
- 安全红线（UI-PROTOCOL-DECISION 工程规则 4：不运行 agent 生成的前端代码）排除了"用户脚本/Lua/运行时拖拽生成代码"类方案——**沙箱 ESM Widget 是自定义性与安全边界的唯一交集**；
- 但审计发现架构缺口：`ui/src` 中**不存在扩展点（slot）注册表**——widget 只能整体替换区域，不能挂载进主界面的命名位置。这是"裸 widget"与"可自定义工作台"的分水岭。

因此 v3 的答案 = **Widget（组件单元）+ Slot（挂载点）+ Layout Profile（布局自定义）+ 命令注册表（行为自定义）+ Theme Pack（外观自定义）**，五者合称**自定义体系**。

**v3.1 补充裁决（Widget 使命）**：Widget 的定位不是"换肤单元"——换肤只是首方存量 widget 的视觉收口（R5 的一小部分）。Widget 的真正使命是**开放式前端功能载体**：任何第三方前端功能（新面板、新工具、新可视化、新交互）都通过「新建第三方 Widget → 本地导入 → 挂载到 Slot / 注册 intent」获得与首方**同权**的原生支持。导入通道、SDK 与生命周期管理因此是一等交付物，不是市场时代的附属品。

## 1. 定位（沿用 v2，不再变动）

| 角色 | 归属 |
|---|---|
| 桌面端（Tauri/Vue）= 长期旗舰产品交付面 | `ui/` |
| WebUI = 合同孵化器 + 当前 P1 产品线 | `webui/` |
| 画布/样板 = 唯一视觉事实源 | `airp-engine-console/` |

一等工作量：视觉对齐样板、性能（100k 虚拟列表）、**自定义性/扩展性/兼容性（v3 提升）**。

## 2. 目标架构（v3）

```text
Tauri 壳（默认 1440×900 / min 1024×720 / 可缩放工作台）
└── Vue 3 应用（ui/src）
    ├── theme/            令牌映射层：tokens.css → CSS vars + Blueprint theme.tokens 同步器
    │                     【v3+】Theme Pack 加载器（用户主题包导入/导出/切换）
    ├── console/          Console 壳（框架A/B、顶栏、status-pill 标准件）
    ├── console/screens/  44 屏 Vue 视图（从样板逐屏派生）
    ├── console/slots/    【v3 新】扩展点注册表：屏内命名挂载点（见 §3.2）
    ├── commands/         【v3 新】命令注册表：intent → 命令面板/快捷键（见 §3.4）
    ├── layout/           【v3 新】布局 Profile 服务：用户布局持久化/切换（见 §3.3）
    ├── protocol/         版本化 Envelope、guard、AgentBus 接缝、TauriBus
    │                     【v3+】三层版本协商（§5）
    ├── state/            RFC6902 store（test 预校验/fail-closed/窗口化切片）
    ├── registry/         Widget Registry、consent、沙箱桥
    │                     【v3+】hostApi 版本协商 + settingsSchema 自动设置 UI
    ├── components/       WidgetHost、BlueprintRenderer
    ├── widgets/          首方 RP widget（聊天/角色/记忆/情绪/物品/任务/地图）
    └── agent-test.ts     GUI 自动化线束
Rust 侧：BusRelay → engine HTTP/SSE；sidecar 生命周期；桌面能力 command
```

## 3. 自定义性体系（最高优先，四乘一层模型）

### 3.1 主题层：Theme Pack

- 基础：令牌单一事实源仍是 `airp-engine-console/assets/tokens.css`；桌面端经映射层消费，禁复制硬编码。
- 自定义：**Theme Pack = JSON 令牌覆盖包**（`{ name, extends: "airp-light", tokens: {...} }`），经 Blueprint `theme.tokens` 注入点生效——协议原生支持，无需新机制。
- 用户操作：导入/导出/切换/重置；校验器拒绝未知令牌名与非法色值（防注入）。
- 深色主题：画布当前只有 Light 变量集；深色 Theme Pack 需先回写画布（STYLEGUIDE §6 增屏流程），不私造。

### 3.2 组件层：Slot 扩展点注册表（v3 核心新增）

Console 屏在派生时声明**命名挂载点**，widget 经 manifest 请求挂载：

```ts
// 屏视图声明（派生样板屏时标注）
<SlotOutlet name="chat.message-actions" :context="{ messageId }" />
<SlotOutlet name="character-card.sections" :context="{ characterId }" />
<SlotOutlet name="topbar.status-area" />

// Widget manifest 请求挂载
{ "type": "core.emotion", "version": "1.2.0",
  "slots": ["chat.side-panel", "character-card.sections"],
  "hostApi": ">=1 <2" }
```

- 首批 slot（随屏派生自然沉淀，不提前设计全集）：`topbar.status-area`、`chat.message-actions`、`chat.side-panel`、`character-card.sections`、`settings.panels`、`diagnostics.panels`；
- 多 widget 挂同一 slot：按 manifest `order` 排序，用户可在设置中禁用单个挂载；
- **Slot context 强类型 + schema 校验**（v3.7）：每个 slot 的 context 形状有 JSON Schema，宿主喂出与 widget 接收双向校验；第三方 manifest 声明的 context 需求同样校验——类型松动不得拖垮主界面；
- 工程规则 2 不变：slot 只在首方工作流具体化后添加，禁止空扩展点。R5 交付**公开文档化的 slot 目录**（名称/context schema/示例），作为第三方扩展的稳定合同面。

### 3.3 布局层：Layout Profile

- 协议 Layout 本支持 dock/grid/stack/tabs；v3 增加**用户布局持久化**：面板显隐/宽高/位置/标签分组 → 存为命名 Layout Profile（如「写作」「调试」「沉浸 RP」）。
- 存储边界：Profile 存 **engine data root**（版本化 schema + migration），不进 localStorage——与资产安全治理一致，跨设备恢复后置但格式现在就兼容。
- Blueprint `profile` 字段（已存在于 types.ts）作为 Profile 标识：`console.default` / `rp.immersive` 等，引擎可按场景推送，用户可覆盖并另存。

### 3.4 行为层：命令注册表 + 快捷键

- intent 已是协议原生概念（`IntentBody`）；v3 把 intent 注册进**命令注册表**：`{ id, title, intent, params?, when? }`。
- 两个消费面：**命令面板**（Ctrl+K，模糊搜索）与**用户快捷键绑定**（keybindings.json，存 engine data root，冲突检测）。
- 首方 intent 全集自动入册（chat.send/regen/rollback/swipe…）；widget 声明的 `intents` 自动入册——第三方功能与首方同权可达。

### 3.5 自定义数据总表

| 数据 | 存储 | 版本化 | 迁移 |
|---|---|---|---|
| Theme Pack | engine data root `/themes/` | schema v1 | migration registry（§5.3） |
| Layout Profile | engine data root `/layouts/` | schema v1 | 同上 |
| Keybindings | engine data root `/keybindings.json` | schema v1 | 同上 |
| Widget 启用/禁用态 | engine data root `/widgets.json` | schema v1 | 同上 |

## 4. 扩展性工程（v3.1：第三方 Widget 导入 = 一等交付）

### 4.1 第三方 Widget 导入通道（本计划交付，不等市场）

用户/开发者路径：**本地新建 widget 包 → 导入 → 原生运行**。包形态与生命周期：

- **包形态**：目录或 zip，含 `widget.manifest.json`（WidgetDef + hostApi + slots + settingsSchema + capabilities + entry）与 ESM 入口源码；单文件 ESM 也允许（manifest 内嵌注释头后置，v1 先目录/zip）。
- **导入**：桌面端「设置 → Widget 管理」导入入口（文件对话框，path-first 纪律）；导入时**静态校验**：manifest schema、hostApi 兼容、capability 白名单、entry 来源（仅本地文件/受信来源；远程 URL 加载后置）。
- **授权**：首装弹 consent 面板——逐项列出声明的 capabilities 及其含义，用户批准才启用；consent 记录持久化（engine data root `/widgets.json`），升级版本变化时重新授权。
- **生命周期**：安装 / 启用 / 禁用 / 升级（同 type 替换，hostApi 重校验）/ 卸载（连 consent 与 slot 挂载一起清除）；每步留 audit 记录。
- **provenance**：导入包计算 SHA-256 并与 manifest 的 author/homepage/license 一并入库，widget 管理面板可见——市场签名体系到来前的诚实溯源。

### 4.2 Widget SDK 与 Host API 表面（开发者合同）

第三方要"建得出来"，宿主必须给出稳定合同：

- **WidgetContext（沙箱代理，已有雏形于 `sandbox-bridge.ts`）**：`state` 推送（宿主→widget）、`emit(intent, params)`（widget→宿主）、`settings` 读取、`theme.tokens` 读取（widget 自动随 Theme Pack 换肤）、`capabilities` 查询。
- **SDK 包**：`ui/sdk/`（或 `tools/widget-sdk/`）——TS 类型定义 + manifest 脚手架 + 示例 widget（hello-panel、chat-toolbar-button 两个）+ 本地调试线束（复用 agent-test harness：宿主内直接挂载开发中 widget，热重载）。
- **文档**：`docs/WIDGET-DEVELOPMENT.md`——manifest 字段全表、Host API 参考、capability 清单与 engine 侧强制映射、slot 目录、打包/导入/调试流程、安全边界（widget 不持 RP 真相源，engine owns truth）。

### 4.3 沙箱与运行时验收

- 沙箱模型已有（`sandbox="allow-scripts"` 无 `allow-same-origin`，opaque origin，postMessage 桥）；**遗留缺口**：真实端到端沙箱加载（远程 ESM in iframe）从未运行时验证（见 `sandbox-bridge.ts:29` 注释）——列为 R5 硬验收，不许再以"协议层测试过"放行。
- 沙箱网络约束：widget 默认无出站网络（CSP `connect-src 'none'`）；需要网络的 widget 必须声明 capability 并经 engine 侧代理调用（`call:tool`），不直连。

### 4.4 宿主治理（不变项）

1. **hostApi 版本协商**：manifest `hostApi: ">=1 <2"`，不兼容拒绝加载 + 明示迁移路径，禁静默尝试。
2. **settingsSchema → 自动设置 UI**：宿主零硬编码。
3. **capability 红线**：新 capability 必须有 engine 侧强制方案；consent + 沙箱是纵深防御，不是唯一边界。
4. **屏即组合**：Console 屏 = 首方 Vue 视图 + Slot + 可选 Blueprint 区域；引擎驱动面仍走 Blueprint 合同。
5. **第三方市场后置**：签名/远程目录/审核流不进本计划；但导入通道 + SDK + 沙箱验收后，市场只是"远程包源 + 签名替换 provenance"，零重构接入。

### 4.5 AI 驱动 Widget 生成（v3.2 新增：无代码用户的一等路径）

**用户画像**：大量 RP 用户无代码基础，惯用「自然语言 + AI 生成」获得前端功能（ST 生态证据：柏宝箱类性能插件、玉子手机类悬浮手机界面、角色卡内嵌前端代码）。桌面端必须把这变成**原生、安全、可治理**的路径，而不是让用户去塞不可信代码。

**用户旅程**：自然语言描述（"我要一个悬浮手机，能收角色短信、看时间、显示心情"）→ AI 生成 → 实时预览 → 一键安装（consent）→ 挂 Slot / 命令面板可用。

**三级生成路径**（按产出物风险分级，默认走最左）：

| 级 | AI 产出物 | 宿主执行方式 | 覆盖场景 | 安全定性 |
|---|---|---|---|---|
| **L1 声明式 Widget**（默认） | **数据，不是代码**：受信组件树 spec（JSON：布局/文本/表单/列表/表格/图表原语 + props 绑定 + intent 触发 + theme.tokens 引用） | 新 `entry.kind: "declarative"`，由宿主**受信渲染器**渲染，零新代码执行 | 面板/状态栏/表单/表格/简单交互 ≈ 80% 需求 | 天然合规工程规则 4（等同 Blueprint 渲染） |
| **L2 受信模板实例化** | 参数槽：样式/字段/数据绑定/提示词片段 | 模板本体是经审查的首方/社区 widget（手机/背包/任务板/状态栏等常见 RP 形态），AI 只填槽 | 常见形态的快速个性化 | 天然合规（模板已审查） |
| **L3 ESM 代码生成**（闸门管道） | 完整 ESM 源码 + manifest | **生成闸门**：静态扫描（禁 API 黑名单/CSP/hostApi 校验）→ 沙箱试运行自检 → 用户预览确认 → consent → 安装；provenance 标 `ai-generated`；capability 默认最严（无网络/无敏感读） | 玉子手机级复杂交互 | **需决策点 D4** |

**L1 的组件词汇表 = 样板组件清单**：`airp-engine-console/assets/components.css` 的 24+ 组组件（topbar/status-pill/card/field/table/bubble/banner…）就是声明式渲染器的原语库——AI 生成的 spec 天然对齐样板视觉，无代码用户产出物自动"长得像 AIRP"。

**ST 生态兼容边界（铁律，AGENTS.md 第三方规则）**：
- 柏宝箱/玉子手机等第三方项目：**只吸收公开行为与功能理念，AI 按 AIRP domain model 独立实现等价 widget**；禁复制/翻译/改写其源码、HTML/CSS、视觉资产（玉子手机未见许可证，更须按独立实现处理）；研究记录入 `docs/ACKNOWLEDGEMENTS.md`。
- 柏宝箱的教训反向佐证路线正确：它 90% 是"替宿主补性能"（长聊天渲染/懒加载/请求合并）——AIRP 桌面端性能合同（虚拟列表/patch 流式/窗口化）原生覆盖，此类需求不需要以插件形式存在。
- **角色卡内嵌前端代码**：不执行卡内原始 JS/HTML；提供「卡片意图导入」——AI 读取卡面声明的功能意图 → 生成 L1 spec 或匹配 L2 模板 → 用户在导入向导确认。卡的原始代码永远不进渲染进程。

**决策点 D4（治理裁决，需项目 owner 批准）**：工程规则 4「不运行 agent 生成的前端代码；只能渲染 Blueprint，或加载**已安装、已审查**的 widget 模块」——L1/L2 天然合规；L3 的「AI 生成 + 确定性静态扫描 + 沙箱 + consent + 最严 capability」是否构成"已审查"，需 owner 显式裁决。**D4 未批准前只交付 L1/L2**；L3 管道设计预留。

### 4.6 酒馆衔接：两手策略（v3.4 重写）

**v3.4 用户裁决**：不做逐个插件/前端项目的官方适配（每个都由我们驱动 AI 适配，工程量不可控）。改为**两手并行**：

- **手一（默认、安全）**：原生生成支持——L1 声明式 / L2 模板 / SDK 导入（§4.1–4.5），安全路径不变；
- **手二（可选、用户自担风险）**：**第三方兼容模式（Third-party Compat Mode）**——声明式开放，用户明示风险后，可直接安装运行酒馆风格的第三方前端扩展与卡内嵌前端，AIRP 提供兼容运行时，不为单个扩展的功能/安全背书。

#### 4.6.0 决策声明 D6：是否开放第三方兼容模式（首要裁决）

这是v3.4 的**前置治理声明**，需 owner 明确批准并同步修订两份上位文件：

1. **工程规则 4 修订**：从「不运行未审查前端代码」改为**分域适用**——默认域不运行未审查代码；**兼容域**（用户 opt-in + 风险自担 + 隔离运行时）允许运行第三方前端代码。规则原文的"已审查"边界不变，兼容域是新增的显式例外。
2. **AGENTS.md 第三方规则修订**：兼容模式下用户自行安装的第三方扩展代码是**用户资产在用户侧运行**，不是 AIRP 复制/移植第三方代码进仓库——仓库代码侧仍全程独立实现（shim/API 表面全部自研），两条规则不冲突，但需在 AGENTS.md 补一句澄清。
3. **D6 未批准**：整个手二不交付，手一不受影响。

#### 4.6.1 兼容模式的遏制设计（用户担风险 ≠ 无护栏）

| 闸门 | 设计 |
|---|---|
| 全局开关 | 默认关；开启需阅读并确认风险声明（数据泄露/密钥风险/稳定性/无官方支持）；开关与确认记录入审计日志；可随时整体关闭（立即停全部第三方扩展） |
| 隔离运行时 | 第三方扩展跑在**独立兼容域**（opaque-origin iframe 集群或独立 webview），与主域（设置/密钥/engine 凭证）硬隔离；扩展崩溃不拖垮主域 |
| 密钥硬边界 | provider key、engine bearer、AIRP data root 凭证**永不进入兼容域**；扩展需要模型/数据能力 → 走 capability 代理（同 widget `call:tool` 模型），不直连 |
| ST-API Shim | 兼容的核心工程是**一个 shim 层**：自研实现酒馆扩展常用 API 子集（DOM 挂载点、事件订阅、设置存取、消息钩子），让酒馆风格扩展"能跑"；shim 永不完备，扩展不兼容时**诚实报错**而非假装支持 |
| 视觉标记 | 兼容域 UI 有常驻标识（边框/横幅「第三方兼容模式」），用户随时知道自己处于风险区 |
| 已知可用清单 | 不做官方适配目录；只维护**社区反馈驱动的兼容性列表**（用户报告"能用/不能用"），纯信息，不承诺 |

#### 4.6.2 迁移向导（保留，资产迁移无风险）

- 只读扫描酒馆数据目录（角色/世界书/preset/persona/regex/QuickReplies/themes/settings + extensions 清单），**原目录永不修改**（复制语义）；
- 资产批量导入走 engine 已有 ST 格式合同；迁移报告（已导入/已转换/跳过原因/需人工）+ 可回滚；
- 扫描到的扩展清单 → 提供两个动作：**手一**：AI 生成 L1/L2 等价物；**手二**（D6 批准后）：标记"可尝试兼容模式直接运行"，由用户选择；
- 接入 33 向导屏「从酒馆迁移」步骤（回写画布）。

#### 4.6.3 交互习惯兼容包（保留）

Layout Profile「酒馆模式」+ 独立实现的「酒馆主题」Theme Pack + 「酒馆习惯」快捷键预设；一键应用、随时切回。兼容包是起点不是锁定。

#### 4.6.4 自定义 CSS 受控出口（保留，决策点 D5）

Theme Pack `customCss` 白名单（仅样式声明，禁 `url()`/`expression()`/`@import`，作用域限定）；酒馆 CSS 经 AI 转换器映射 + 不可转部分逐条列明由用户取舍；原始 CSS 不直接注入主域。**D6 批准后，兼容域内的第三方扩展自带 CSS 不受此白名单限制（风险自担域）**，但仍限定在兼容域作用域，不外溢主域。

#### 4.6.5 带自有数据库/后端的第三方插件（v3.5 补全）

形态识别：玉子手机类（插件自带 SQLite 表）、柏宝库类（依赖独立后端服务/数据库 + Early Bridge）。比纯前端多一个**数据维度**——两手策略同样适用，但数据治理是新增重点。

**手一（原生路径）**：
- **插件数据存储 API**：engine 提供命名空间化存储（KV/表），widget 经 capability（`read:state`/`write:state` 或新增 `plugin-data` capability）访问；engine owns truth（工程规则 3 不变），widget/插件不持 RP 真相源；
- **迁移向导扩展**：识别插件自有数据库文件（SQLite 等）→ 一次性导入转换为 AIRP 原生存储——校验 + provenance 记录 + **原文件不动**；导入后原插件数据与 AIRP 脱钩。

**手二（兼容模式，D6 批准后）**：
- **插件自带后端 = 用户自管 sidecar 进程**（如柏宝库类服务）：用户自行安装、运行、升级，自担风险；AIRP 不内嵌、不分发、不背书；
- 兼容域经 **loopback 桥**与 sidecar 通信，桥接走显式 capability 授权（用户可见、可撤销）；sidecar **永不获得** engine bearer / provider key / AIRP data root 凭证；
- **插件本地数据留在插件侧**：诚实声明**不纳入 AIRP 备份/迁移/完整性合同**（`/widgets.json` 标 `external: true`，Widget 管理面板可见），不假装全包；
- **RP 真相单向流**：第三方数据库内容只可经显式导入（校验后）进入 engine；engine 数据**永不自动回写**第三方库——防第三方损坏/污染用户 RP 资产。

#### 4.6.6 优先级声明

对酒馆迁移用户，"无感+无缝"优先于"理念纯净"：衔接是 onboarding 的一部分。两手分工——**手一给安全与原生体验，手二给覆盖广度与零适配成本**；官方精力投手一与 shim 层，不接单个扩展的适配请求。兼容产物（Theme Pack/L1 spec/宏/Profile）落地即为 AIRP 原生格式，与酒馆无耦合。

### 4.7 HTML/富渲染治理（v3.6 新增）

酒馆生态里"HTML 渲染"有两种性质，治理路径不同：**数据侧富文本**（消息气泡/角色卡描述/世界书条目/正则脚本输出里的 HTML）与**代码侧独立渲染模块**（第三方 Markdown 变体、排版引擎、图表/3D 渲染器、自定义楼层渲染）。

#### 4.7.1 数据侧：受控富文本渲染管线（主域唯一入口）

- **默认管线**：Markdown → 白名单 HTML。白名单含标签（`p/em/strong/code/pre/blockquote/ul/ol/li/h1-h6/hr/br/table 系/img/a`）与属性（`href/src/alt/title/class(限令牌前缀)`），**禁** `script`/事件处理器（`on*`）/`javascript:` URL/`iframe`/`form`/`style` 内联样式（对齐生产 CSP：无 `unsafe-inline`/`unsafe-eval`）；
- **远程资源**：`img-src` 限本地与显式允许域（防追踪像素与内容泄露）；外部链接 `rel="noopener"` + 点击确认跳转；
- **铁律**：用户内容（消息/卡/世界书/正则输出）**永不 `v-html` 直渲**，一律过 sanitize 管线；管线是 R2 聊天屏的硬验收项（XSS 夹具库负向测试）；
- **正则脚本输出含 HTML**：与消息文本同管线，不因"来自 regex"获得特权。

#### 4.7.2 数据侧超出白名单的富 HTML（前端卡/复杂排版卡）

- **手一（默认）**：AI 结构化提取——把卡内 HTML 的**内容与结构意图**转换为 L1 声明式 spec 或受控 HTML 子集，原始 HTML 不进主域 DOM；转换损失逐条列明由用户取舍；
- **手二（D6 批准后）**：原始 HTML 在**兼容域沙箱 iframe** 内渲染（opaque origin，无 `allow-same-origin`），风险自担；主域只接收经 capability 代理的结构化数据（如表单提交值），不接收 DOM。

#### 4.7.3 代码侧：独立渲染模块（第三方渲染器/排版/图表引擎）

- **渲染器即 widget**：第三方渲染模块以沙箱 ESM widget 形态加载（§4.1 导入通道同规），输入是经 sanitize 的数据，输出限定在其 iframe 文档内；宿主通过 postMessage 桥喂数据，渲染器不碰主域 DOM；
- **L1 渲染原语扩展**：常用渲染需求（代码高亮/表格/图表/公式/折叠块）沉淀为 L1 声明式渲染原语（首方实现、随宿主审查更新），AI 生成 spec 直接引用——无代码用户的"自定义渲染"多数停在这一层，不需要真渲染器；
- **渲染性能合同**：渲染 widget 与普通 widget 同规（懒加载、挂载计数入 50-widget spike）；消息列表内的行内渲染（代码块/表格）必须走虚拟列表窗口化，禁止全量 DOM。

#### 4.7.4 与 CSP/生产拓扑的一致性

桌面端渲染治理与 webui 生产 CSP（`default-src 'self'`、无 unsafe-inline/eval、`img-src 'self' data: blob:`）对齐：**同一套白名单、同一套禁项**，两端不各自为政；兼容域是唯一例外域且已被 D6 闸门覆盖。安全负向测试（注入夹具）双端共享。

### 4.8 引擎 Hook API 与插件数据共存模型（v3.7 新增）

源自第三方架构评审（2026-08-02）：评审指出核心诉求含「允许插件改变部分后端」，而原文档前端扩展厚、后端扩展薄。本节把后端扩展升为一等交付，但守住两条不让渡：**密钥硬边界**、**引擎对插件数据读时校验**。

#### 4.8.1 安全模型框架：从「准入审批」到「爆炸半径遏制」

- **准入零人工审批**：安装/运行不卡审；危害靠自动化遏制——opaque-origin 隔离、capability **引擎侧强制**（非 UI-only）、数据命名空间化 + 事务化、崩溃隔离默认（非兼容模式专属）。坏插件能崩自己、污染自己数据域，但绝不能碰主程序、用户密钥或其他插件数据。
- **「沙箱即审查」不成立**（本计划与评审的分歧点）：沙箱管隔离不管意图——经合法 capability 的数据外泄、RP 内容破坏仍可能发生。因此保留**一键轻量 consent**（安装时可见、可撤的 capability 清单）与密钥硬边界为不可让渡底线；D4/D6 是否在此框架下放宽，列入附录 D 由 owner 拍板，不由评审单方改写。

#### 4.8.2 引擎 Hook / 扩展 API（一等交付物，契约设计前置到 R5 开工）

- **事件钩子 v1 清单**（R5 交接包定稿）：`onMessageReceived` / `onMessageSending`（可改写）/ `onStateChange` / `onSessionLoad` / `onRender` 等；
- **作用域数据变更 API**：插件读写自己的命名空间存储 + 经事务的限定 RP 域操作（如追加消息元数据）；**不直接改写会话真相**（工程规则 3 的一般真相源不让渡）；
- **版本化协商**：`hookApi` 与 hostApi 同规双轨 semver；不兼容 → 优雅禁用并明示（降级矩阵同 §5.3）；
- 全部 Hook 调用经 engine 侧强制点，UI 侧检查只是辅助。

#### 4.8.3 插件数据共存模型（两级，替代 v3.5 单一级 external）

| 级 | 语义 | 备份/迁移合同 | 写入规则 |
|---|---|---|---|
| **P1 插件私有域**（默认） | 插件命名空间 KV/表，标 `external: true` | 不纳入（v3.5 不变，诚实排除） | 插件自由读写 |
| **P2 受管命名空间**（opt-in capability） | 进备份/迁移/完整性合同 | **纳入** | 作用域限定 + **事务化写入**；engine 读时一律 schema 校验（不盲信结构） |

配套机制：**安装/升级前自动打还原点**（复用迁移向导 rollback 能力，一键回滚）——后端可写插件 = 存档损坏风险，缓解前置。评审建议的「插件完全成为后端公民」收窄为 P2 受管域：既交付「改后端」的用户价值，又不让渡一般真相源。工程规则 3 是否按 P2 修订，列入附录 D 待拍板（D7）。

#### 4.8.4 治理：透明溯源 + 社区策展

provenance（SHA-256 + author + license）已有；加**社区可用清单**（兼容性反馈驱动，纯信息不承诺）；签名/远程目录仍后置，但「信任信号」部分（哈希/来源/社区标注）随 R5 提前交付——无门槛不等于无信号。

## 5. 兼容性工程（v3 新增，三层）

### 5.1 协议层
- Envelope `v` 版本号 + `hello.accept` 能力协商（协议原生已有）；**feature detection 优先于版本判断**（按 accept 列表降级，不解析版本号猜能力）。
- Blueprint/patch schema 变更必须带版本与迁移说明（工程规则 7：可观察、可迁移、可回退）。

### 5.2 Widget 层
- `WidgetDef.version`（widget 自身）× `hostApi`（宿主合同）双轨；同 type 多版本拒绝并存，启动期报冲突。
- props/state 按 manifest JSON Schema 校验失败 → widget 降级为错误占位卡（不拖垮宿主）。

### 5.3 引擎合同层
- 桌面端消费 webui 成熟的 HTTP/SSE/cursor/revision 合同；启动时经 `/version` + 能力端点探测，建**降级矩阵**：旧 engine 缺能力 → UI 诚实禁用并说明（继承样板 26–31 状态变体的诚实语义），禁静默失败。
- 合同缺口回写 engine/webui issue，不在桌面端私造。
- 自定义数据（§3.5）迁移走版本化 migration registry：启动 dry-run + 失败回滚，与 engine 数据治理同规。

### 5.4 兼容性验收矩阵（R6 门禁）

| 组合 | 期望 |
|---|---|
| 新宿主 + 旧 widget（hostApi 兼容） | 正常加载 |
| 新宿主 + 旧 widget（hostApi 不兼容） | 拒绝 + 明示迁移路径 |
| 新宿主 + 旧 engine（缺能力） | 降级矩阵生效，对应 UI 禁用并说明 |
| 旧 schema 自定义数据 + 新宿主 | migration 成功；失败则回滚到备份并告警 |
| 导入包缺 manifest / schema 不合 / capability 超白名单 | 拒绝导入 + 逐项明示原因 |
| 第三方 widget 版本升级（capabilities 变化） | 重新 consent 后才启用 |
| 兼容域扩展尝试读取主域密钥/凭证（D6 批准后） | 硬隔离拦截 + 审计日志 + 用户告警 |
| 兼容域扩展崩溃/死循环（D6 批准后） | 主域不受影响；兼容域单独终止并提示 |
| 插件 sidecar 请求 engine bearer/provider key/data root 凭证（D6 批准后） | 拒绝 + 告警；loopback 桥只放授权 capability |
| 插件自有数据库（SQLite 等）导入 | 校验 + provenance 记录 + 原文件不动；导入后脱钩 |
| 第三方库数据回写 engine（非显式导入路径） | 拒绝；RP 真相单向流 |
| 消息/卡/世界书/正则输出含 XSS 夹具（script/on*/javascript:/iframe） | sanitize 管线剥除，渲染输出零违规（CSP violation 监听为空） |
| 富 HTML 卡直进主域 DOM | 阻断；只许 AI 结构化提取（手一）或兼容域渲染（手二） |
| 渲染 widget 试图挂载主域 DOM | postMessage 桥无此通道；widget 只能渲染自己 iframe 文档 |
| 任意 widget/插件崩溃或死循环（默认域即生效，非兼容模式专属） | 主域不受影响；故障单元单独终止并提示错误占位卡（崩溃隔离是默认纪律） |
| P2 受管命名空间写入 | 事务化写入 + engine 读时 schema 校验（不盲信结构）；非事务/校验失败拒绝并告警 |
| 插件安装 / 升级 | 自动打还原点（复用迁移向导 rollback）；失败一键回滚，防存档损坏/锁死 |

## 6. 性能合同（沿用 v2，不降级）

| 合同 | 机制 | 验收 |
|---|---|---|
| 流式不整树重渲 | id-keyed `{messages, order}` + patch 直写 | R2 渲染计数断言 |
| 长列表 O(视口) | `virtual-window.ts` | **100k 消息 spike**（R6 CI 门禁） |
| 有界前端状态 | store 只持窗口化切片 | soak 内存封顶 |
| patch 原子性 | test 预校验、fail-closed | 既有单测绿 |
| 大对象不进响应式 | path-first 导入 | R1/R4 审查项 |
| 蓝图更新低开销 | shallowRef + patch 时 clone | 既有单测绿 |

新增一条：**slot/widget 数量增长不劣化首屏**——挂载点懒解析，R6 加 50-widget 挂载 spike（防扩展性吃掉性能）。

## 7. 接力棒次（R0–R6，v3 调整）

交接包规范不变（基线 commit / 样板参照 / 允许改动路径 / 任务清单 / 验收命令 / 对拍标准 / 回写项 / 禁区），落 `docs/progress/2026-MM-DD-desktop-relay-R<N>.md`。

| 棒次 | 主题 | 内容 | v3 变化 |
|---|---|---|---|
| **R0** | 基线校准 + 令牌层 + 壳骨架 + 沙箱证伪 | 基线/README 校准；令牌映射层 + `theme.tokens` 同步器；壳骨架（框架A/B）；窗口默认尺寸；WebView2 对拍 spike（3 屏）；**真实沙箱 E2E spike**（iframe 加载 hello-panel，先证伪隔离边界——评审最高权重项：门开得越大，围栏越要先证伪） | +Theme Pack 加载器骨架；+协议三层协商骨架；+沙箱 E2E 硬门禁（补 sandbox-bridge.ts:29 遗留缺口，未过不进 R1） |
| **R1** | 管理面一批（01/03/04/05/06/08/12） | 角色/设置/Preset 屏派生；engine 合同接通 | +首批 slot 声明（settings.panels、character-card.sections）；+降级矩阵 v1 |
| **R2** | 聊天主链路（02/07/27/28/31 + 26/29/30） | ChatWidget 对齐样板；SSE patch 流式；Agent Run；状态变体屏；虚拟列表接入 | +chat.* slot；+命令注册表（首方 intent 入册 + Ctrl+K 面板）；+**受控富文本渲染管线**（Markdown→白名单 sanitize + XSS 夹具负向测试，v3.6 硬验收） |
| **R3** | 管理面二批（09/10/11/13–17/20–25/33） | 世界书/Persona/诊断/模态/表格/向导；安全语义逐屏核对 | +diagnostics.panels slot；+33 向导新增「从酒馆迁移」步骤（回写画布） |
| **R4** | 桌面能力 + 酒馆迁移向导 | 文件对话框（path-first）、托盘、全局快捷键、窗口状态持久化、updater 评估；**ST Migration Wizard**：只读扫描酒馆数据目录/批量导入/迁移报告/回滚 | +快捷键系统对接命令注册表（含「酒馆习惯」预设） |
| **R5** | 自定义体系 + 第三方 Widget 导入收口 | **第三方导入通道**：包校验/consent 授权/安装-启用-禁用-升级-卸载全生命周期/provenance 哈希入库/Widget 管理面板；**Widget SDK**：类型 + 脚手架 + 2 示例 widget + 调试线束 + `docs/WIDGET-DEVELOPMENT.md`；**真实沙箱 E2E 验收**（补 sandbox-bridge 遗留缺口）；**AI 生成 L1**：声明式渲染器（原语=样板组件清单）+ spec schema 校验 + 会话式生成向导（描述→预览→安装）；**AI 生成 L2**：2 个受信模板（手机/状态栏）+ 参数槽；L3 闸门管道按 D4 裁决排期；**酒馆衔接（v3.4 两手）**：手一＝「酒馆模式」Layout Profile + 「酒馆主题」Theme Pack + customCss 转换器（按 D5）；手二＝**ST-API Shim 兼容域 spike**（隔离运行时 + 密钥硬边界 + 视觉标记 + 已知可用清单 + **插件数据存储边界：sidecar loopback 桥 / 外部数据 `external` 标注 / 单向导入**（v3.5），按 D6 裁决，未批不建）；Slot 注册表完整验收；Layout Profile 服务；Theme Pack 导入导出；hostApi 协商 + settingsSchema 自动设置 UI；首方 RP widget 令牌化换肤；**引擎 Hook API 契约 v1**（§4.8.2：事件钩子清单 + 作用域数据 API + hookApi 版本协商，契约设计前置到 R5 开工）；**插件数据共存模型（P1/P2 两级）+ 安装/升级前还原点机制**（§4.8.3，后端可写插件 = 存档损坏风险，缓解前置） | v3.4：两手策略入棒；导入通道为主，换肤为末；v3.7：Hook API + 数据共存 + 还原点入棒 |
| **R6** | 性能与兼容性闸门 + 打包发布候选 | 100k spike + 50-widget 挂载 spike + soak；兼容性验收矩阵（§5.4）全绿；NSIS 安装包 smoke；updater 决策落档；基线重写 | +兼容性矩阵 +50-widget spike |

屏清单事实源：`airp-engine-console/assets/screens.js`（33 屏 + 3 planned）。webui 34–44 屏复用样板令牌/布局语言并抽验 ≥3 屏。预留屏（18/19/24）只接信息架构。

## 8. 验证体系（v3 增补）

| 层 | 工具 | 通过标准 |
|---|---|---|
| 类型/单测 | `npm run typecheck && npm test -- --run` | 98+ 全绿，新逻辑必带测试 |
| Rust 壳 | `cargo test --workspace --locked` | 9+ 全绿 |
| 视觉对拍 | Playwright 截屏 vs 样板屏 | 令牌 0 偏差；取色 ±1 |
| 协议回归 | guard/store/registry/e2e-smoke | 全绿，禁降级修改 |
| **自定义体系** | Theme Pack 校验、slot 挂载、Layout Profile 持久化、命令注册表、keybindings 冲突 | 专项单测 + R5 验收 |
| **第三方 Widget** | 导入校验、consent 流、生命周期操作、沙箱 E2E 加载、SDK 示例 widget 冒烟 | 专项单测 + R5 验收（沙箱 E2E 为硬门禁） |
| **兼容性** | §5.4 矩阵 | R6 全绿 |
| 性能 | 100k spike、50-widget spike、soak、渲染计数 | §6 上限 |
| 打包 | NSIS smoke | 安装→启动→导入→流式一轮 |

## 9. 治理（沿用）

R0 校准基线/README；每棒 1 PR 走审计阻塞门禁；审计遗留合并后提 issue；样板溯源表加「桌面端 commit」列；冲突以样板为准回写 issue；手工冲突 PR 评论点明。

## 10. 里程碑与风险

| 里程碑 | 标志 |
|---|---|
| M1 壳与令牌层 | R0+R1 合并，3 屏对拍过 |
| M2 聊天旗舰链路 | R2 合并，patch 流式 + 命令面板可用 |
| M3 功能面齐平 | R3 合并 |
| M4 桌面体验完整 | R4 合并，快捷键可自定义 |
| M5 自定义体系就位 | R5 合并：第三方 widget 可「新建→导入→原生运行」全流程走通 + slot + Layout Profile + Theme Pack 全验收 |
| M6 发布候选 | R6 合并：性能 + 兼容性矩阵 + 安装包全绿 |

| 风险 | 等级 | 缓解 |
|---|---|---|
| 44 屏 Vue 派生工作量 | 中 | 子棒切分；管理面允许「结构对齐先行、精修后置」两级验收 |
| slot/扩展点过度设计 | 中 | 工程规则 2：只随首方工作流沉淀，首批仅 6 个命名 slot |
| 自定义数据 migration 复杂度 | 中 | schema v1 从简；迁移走 engine 同规 registry + dry-run |
| WebView2 渲染差异 | 中 | R0 spike 先验 |
| 100k/50-widget spike 暴露瓶颈 | 中 | 前置到 R2/R5 初步接入时先跑一轮，R6 收口 |
| 扩展性拖垮性能 | 低 | 懒解析 + 50-widget spike 门禁 |
| updater 基建 | 低 | R4 只评估预留，不阻塞 M6 |

## 11. 立即行动项（评审通过后）

1. 拍板 **D6（是否开放第三方兼容模式——v3.4 前置治理声明，涉工程规则 4 与 AGENTS.md 分域修订）**、D1（路线 B+）、D2（窗口方案）、**D3（自定义体系四层模型 + 首批 6 个 slot）**、**D4（L3 AI 代码生成闸门是否构成"已审查"；未批只交付 L1/L2）**、**D5（Theme Pack customCss 白名单放开程度）**、**D7（工程规则 3 是否按 P2 受管域修订——插件成为受管命名空间公民，让渡部分真相源归属）**；
2. **附录 D 立场翻转项待拍板**：评审建议「沙箱即审查消解 D4 / D6 默认放行 / 市场提前」——均不直接改写文档立场，由 owner 在 D4/D6/市场时机上拍板；
3. 起草 R0 交接包（含沙箱 E2E spike 硬门禁）；
4. **设计引擎 Hook API 契约 v1**（§4.8.2）+ **插件数据还原点机制**（§4.8.3）——两项为「允许插件改后端」的前置交付，先于 R5 开工定稿；
5. 随 R0 校准 CURRENT-BASELINE / ui README / 溯源表加列。

---

## 附录 A：原计划性能/扩展机制源码索引（保留清单）

| 机制 | 文件 |
|---|---|
| id-keyed 聊天 + patch 流式 | `ui/README.md:64`、`ui/src/App.vue:175`、`ui/src/widgets/ChatWidget.vue` |
| 虚拟列表窗口化 | `ui/src/widgets/virtual-window.ts` |
| RFC6902 store | `ui/src/state/store.ts` |
| 协议类型（Envelope/Blueprint/WidgetDef/Capability/theme.tokens/Layout/IntentBody） | `ui/src/protocol/types.ts` |
| 运行时 guard | `ui/src/protocol/guard.ts` |
| AgentBus 接缝 + TauriBus | `ui/src/protocol/bus.ts`、`bus-factory.ts`、`tauri-bus.ts` |
| Registry/consent/沙箱桥 | `ui/src/registry/` |
| WidgetHost/BlueprintRenderer | `ui/src/components/` |
| Agent 测试线束 | `ui/src/agent-test.ts` |
| Rust BusRelay | `ui/src-tauri/src/bus.rs`（931 行） |

## 附录 B：速查

- 令牌事实源：`airp-engine-console/assets/tokens.css`（primary `#C4663B`、bg `#FAFAF7`、radius 6/10/14/9999、Inter + JetBrains Mono）
- 屏注册表：`airp-engine-console/assets/screens.js`（33 屏 + planned 3）
- 现有测试基线：ui 98 项 / shell 9 项 / webui 50 项 / engine lib 1056 项

## 附录 C：v3 设计决策记录

| 决策 | 备选 | 取舍理由 |
|---|---|---|
| Widget 为组件单元 | 用户脚本/Lua/运行时生成代码 | 安全红线（不运行未审查前端代码）；沙箱 ESM 是唯一交集 |
| **Widget 使命 = 开放功能载体，非换肤单元** | widget 仅首方换肤收编 | 用户裁决（v3.1）：第三方前端功能经「新建→导入」获得原生同权支持；换肤只是首方存量收口 |
| **第三方导入通道本计划交付** | 等市场一起做 | 导入/授权/生命周期/SDK 是市场的地基且独立有用户价值（开发者自用、小圈子分享）；市场只加"远程包源+签名" |
| 沙箱 widget 默认无出站网络 | 允许直连 | 网络需求走 `call:tool` 经 engine 代理，capability 可强制；直连绕过 consent 模型 |
| Slot 命名挂载点 | 整区域替换 / 自由注入 | 可枚举、可禁用、可审计；自由注入不可治理 |
| Layout Profile 存 engine data root | localStorage | 资产安全治理一致；版本化+迁移+跨设备兼容 |
| settingsSchema 自动生成设置 UI | 每 widget 自绘设置页 | 宿主零硬编码；第三方与首方同权 |
| feature detection 优先 | 版本号比较 | 版本号猜能力脆弱；accept 列表是协议原生 |
| Theme Pack 走 theme.tokens 注入 | CSS 变量直改 | 协议原生通道，引擎/蓝图/用户三方同源；第三方 widget 读 tokens 自动随主题换肤 |
| 第三方市场后置 | 本计划内做 | 签名/审核/目录是独立大工程；导入通道+SDK+沙箱验收后零重构接入 |
| AI 生成默认走 L1 声明式（产数据不产代码） | AI 直接生成 ESM 代码运行 | 工程规则 4 红线；L1 spec 经样板组件原语渲染，无代码用户产出物自动对齐样板视觉，约覆盖 80% 需求 |
| L2 受信模板填槽 | 每需求都生成新 widget | 常见 RP 形态（手机/背包/状态栏）模板化后 AI 只填参数，安全且更快 |
| L3 代码生成走闸门管道 + D4 裁决 | 直接放行 | "AI 生成+静态扫描+沙箱+consent"是否算"已审查"是治理问题，owner 未批不交付 |
| ST 资产只独立实现不搬运（仓库侧） | 转换/包装 ST 插件代码进仓库 | AGENTS.md 第三方铁律；玉子手机无可见许可证；柏宝箱类性能插件被桌面端性能合同原生覆盖，无需移植 |
| **酒馆衔接 = 两手策略（v3.4 用户裁决）** | 逐个插件官方适配（v3.3 映射目录） | 逐个适配工程量不可控；手一（原生生成，默认安全）+ 手二（兼容模式，用户自担风险）并行，官方精力投手一与 shim 层 |
| **第三方兼容模式 = 声明式开放（D6）** | 全面禁止第三方代码 / 无声明直接放行 | 工程规则 4 分域修订：默认域不运行未审查代码，兼容域（opt-in+风险自担+隔离运行时）是显式例外；用户安装的扩展代码是用户资产在用户侧运行，不违反仓库独立实现规则 |
| 用户担风险 ≠ 无护栏 | 兼容模式裸奔 | 全局开关默认关 + 风险明示 + 隔离运行时 + 密钥硬边界 + 视觉标记 + 审计日志；兼容域崩溃不拖垮主域 |
| 迁移向导只读扫描 + 复制语义 | 原地修改/剪贴酒馆目录 | 用户资产底线：酒馆数据是用户的退路，原目录永不被修改 |
| 兼容产物全部落地为 AIRP 原生格式 | 长期保留 ST 格式依赖 | 兼容是桥不是锁：Theme Pack/L1 spec/宏落地后与酒馆无耦合，兼容包可随时卸载 |
| customCss 受控白名单出口（D5） | 全禁 / 全放 | 全禁断送重度美化用户；全放破 CSP 安全边界——白名单 + 作用域限定是中间解 |
| 用户内容永不 v-html 直渲，统一过白名单 sanitize（v3.6） | 富文本直渲保"原汁原味" | XSS 是 RP 客户端最高频攻击面（消息/卡/正则全是注入源）；白名单管线与生产 CSP 对齐是唯一可持续解 |
| 富 HTML 卡走 AI 结构化提取（手一）/ 兼容域渲染（手二） | 主域直接渲染原始卡 HTML | 原始 HTML 进主域 DOM = 绕过 sanitize 的唯一洞；两手分别给安全转换与风险自担出口 |
| 独立渲染模块 = 渲染器即 widget + L1 渲染原语扩展 | 渲染器作主域插件直跑 | 渲染器输出限定其 iframe 文档；常见需求沉淀为受审原语后，多数"自定义渲染"不需要真代码 |
| 插件自带后端 = 用户自管 sidecar（v3.5） | AIRP 内嵌/分发第三方后端 | 内嵌把不可控的运维、安全与更新责任揽进仓库；sidecar 保持"用户资产在用户侧"，AIRP 只提供 loopback 桥 + capability 授权 |
| 插件数据默认不入 engine 真相，单向导入（v3.5） | 双向同步 | 工程规则 3：engine owns truth；第三方库只经显式校验导入单向进入，engine 永不自动回写——防污染用户 RP 资产 |
| 插件本地数据诚实排除出备份合同（v3.5） | 假装全包 | 用户资产底线 = 不承诺做不到的；`external: true` 标注让排除可见，而不是事后才发现丢数据 |

## 附录 D：第三方架构评审（2026-08-02）立场翻转项 — 待拍板（不由评审单方改写）

评审基于「可魔改引擎」目标，建议将下列原文档立场翻转。本计划**未直接采纳**，分别挂回对应决策点由 owner 拍板；采纳与否均不改变 §4.8 已落地的「爆炸半径遏制」框架与不可让渡底线（密钥硬边界 + 一键轻量 consent）。

| 评审建议翻转项 | 对应原立场 | 本计划处置 | 拍板点 |
|---|---|---|---|
| 「沙箱即审查」——准入零门槛，D4 闸门消解 | 工程规则 4：不运行未审查前端代码；L3 受 D4 闸门 | **不采纳**：沙箱管隔离不管意图，合法 capability 仍可外泄/破坏 RP 内容。保留一键轻量 consent + 密钥硬边界为底线的双轨 | D4（是否放宽 L3 闸门） |
| D6 兼容模式默认放行（非 opt-in 风险区） | D6：兼容模式默认关、opt-in | **列入待拍板，未采纳默认放行**：默认开 = 静默放大攻击面；但「可魔改引擎」目标确实要求低摩擦。建议折中：默认关但首装引导一键开 + 全局视觉标记 | D6 |
| 插件完全成为后端公民（双向、进真相源） | 工程规则 3（engine owns truth）+ v3.5（external 排除） | **收窄为 P2 受管命名空间**（§4.8.3），不一般让渡真相源 | D7 |
| 市场提前（签名/远程目录不再是独立大工程） | 市场后置决策 | **部分采纳信任信号提前**：provenance + 社区可用清单随 R5 交付；签名/远程目录仍后置（独立大工程，且不阻塞一手体验） | 市场时机（待拍板） |
| Slot 主动设计完备公开目录 | 工程规则 2：Slot 仅随首方工作流沉淀 | **已部分采纳**（§3.4 / line 86）：R5 交付「公开文档化的 slot 目录」作为稳定合同面，但新增仍受首方工作流约束，不鼓励空扩展点 | 已落地 |

### 评审已采纳 / 部分采纳项一览（与 §4.8 对应）

- ✅ 沙箱/隔离边界最先证伪 → R0 真实沙箱 E2E spike 硬门禁（§7 R0 行）
- ✅ L1 声明式渲染器优先、零 JS 执行性能红利、首屏 localStorage 读穿、Slot 强类型 context → 已写入 §3/§4/§6
- ✅ 引擎 Hook/扩展 API 升为一等交付 → §4.8.2 + R5 契约设计前置
- ✅ 插件数据共存与还原点模型 → §4.8.3（P1/P2 两级 + 安装前还原点）
- ✅ 治理从「审批闸」转「透明溯源 + 社区策展」 → §4.8.4
- ✅ 后端可写插件 = 存档损坏风险，缓解前置 → §5.4 还原点行 + §10 风险表可补

> 评审报告原文：`D:\.WorkBuddy\2026-08-02-09-29-37\airp-frontend-architecture-report.md`（性质：架构评审，未改动仓库文件）。
