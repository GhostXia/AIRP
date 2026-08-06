# 思维窃取：AiChat（第三方设计参考学习笔记）

> 研究方式：**远程只读**（GitHub REST API + raw 文件 + README + `docs/plugin-system.md` + `docs/agent-center-agent-card-migration-plan.md` + `src/iframe-host.html` + `src/lib` 目录清单）。**未克隆、未下载代码到本地。**
> 合规红线：AiChat 为 **AGPL-3.0**，与 AIRP 的 Apache-2.0 不兼容。本笔记只吸收**理念 / 公开行为 / 互操作格式**，**绝不复制、翻译或改写其源码、manifest、prompt、正则、HTML/CSS、图标或视觉资产**。AIRP 必须按自身 domain model、协议（AIRP-State-Protocol 的 Blueprint/Widget/RFC6902/consent/sandbox）、命名与控制流独立实现。
> 审查基线：`v0.7.0-preview.2`，commit `9b9f4fbb04020b28238d9abb71e5a34135c6c5c4`，日期 2026-08-02。

---

## 0. 项目定位与产品形态（公开行为）

- 本地优先的 AI 聊天 App，灵感来自 SillyTavern，源于 Discord「类脑」相关讨论。
- 形态：手机 + Windows 桌面，**Tauri v2**。所有数据本地存储，不经第三方服务器。
- 功能面：私聊 / 群聊 / 创意写作 / 世界书（角色设定）/ 角色卡导入（SillyTavern 格式 PNG·JSON）/ 角色库 / 记忆系统 / 预设（兼容酒馆）/ 动态（类朋友圈）/ 图片生成 / 贴图表情包 / 「女仆」助手（App 内 Agent）。
- 接入的 AI 服务：Gemini、DeepSeek、OpenAI、Claude、任意 OpenAI 协议服务；**每个聊天室单独指定 API 与模型**。
- 近期（0.7.0-preview）重点：女仆助手（NL 操作 + 权限确认 + 长期记忆）、AI 回复整轮验收后落库、回复逐条揭示动画、桌面双栏布局、「本次请求」浏览器（请求概览 + 完整 Prompt + 血缘图 + 响应诊断）、Agent Center、原子写 KV 存储防断电丢数据。

→ 与 AIRP 对齐：AIRP 桌面端定位（Tauri/Vue）、「酒馆迁移一级需求」、Agent 驱动的 RP 增强、Widget 作为第三方前端能力核心载体——方向高度一致，是本笔记重点对标对象。

---

## 1. 技术栈洞察（vendored 离线优先）

`src/lib` 实际 vendored 的依赖（global build，非 npm 运行时）：

| 资产 | 用途推断 |
|---|---|
| `vue2.min.js` / `vue3.global.prod.js` | 前端框架，**双版本并存**（可能迁移中或兼容并存） |
| `pinia.iife.prod.js` | 状态管理 |
| `vue-router3.min.js` / `vue-router4.global.prod.js` | 路由（与 Vue 版本对应） |
| `jquery.min.js` / `lodash.min.js` | 工具库 |
| `zod.min.js` | **schema 校验**（用于 manifest / 数据契约） |
| `elkjs` | **血缘图布局**（对应「本次请求」血缘图） |
| `toastr.min.js` | 通知 |

关键选择：
- **依赖全部 vendored**，不在运行时依赖 CDN → 对 Tauri 移动端 / 离线场景稳定，规避网络与中国大陆访问问题。
- `package.json` 的 `scripts` 有极多 `test:*`（chat-ui / chat-generation / chat-context / chat-memory / maid-* / agent / sessions / migration / theme / transfer …），外加 `check:fast` / `smoke:app` / `audit:theme` / `audit:ui` → 推测有 headless 自动化（很可能 CDP / Playwright），质量门禁前置。

→ AIRP 对应点：AIRP 桌面端已选 **Vite + Vue 构建链**（见 ACKNOWLEDGEMENTS 第 3 节依赖表），不回退到 vendored 全局 build。但其「**插件/Widget 运行时可 vendored / 锁定版本、不依赖运行期 CDN**」的思路，值得在 Widget 沙箱运行时设计中借鉴。其 headless 自动化门禁（audit:theme/audit:ui）也值得 AIRP 桌面端 UI 回归参考。

---

## 2. 插件 / 扩展系统（`docs/plugin-system.md`）—— 与 AIRP Widget / consent / sandbox 直接对标

结构：`my-plugin/{manifest.json, index.js}`，入口 `module.exports = function(api){ ... }`。

manifest 字段（理念级，非照搬）：
- 必须：`id`（建议反向域名）、`name`、`version`（semver）、`apiVersion`、`main`、`permissions`
- 可选：`description`、`author`、`mode`（`safe` / `power` / `legacy`）

**权限分级思想（直采为理念，AIRP 必须自行命名实现）：**
- 默认 `safe` 即可安装；**高危权限必须 `mode: "power"`，否则禁止安装/启用**。
- 细粒度权限清单示例：`chat.read` / `chat.write`、`worldbook.read` / `worldbook.write`（需 power）、`storage`、`network`（需 power）、`prompt.modify`（需 power）、`ui.inject`、`variables.read` / `variables.write`、`system.settings`（需 power）。

**事件钩子体系（可作 AIRP hook 类型对照）：**
`message.before_send` / `after_send` / `after_receive` / `before_render` / `after_render`、`variable.changed`、`command.parsed`、`session.changed`、`prompt.before_build` / `prompt.after_build`。

**API 面（注入点模型）：**
`api.storage.get/set/remove/keys`；`api.variables.get/getAll/set/patch/watch`；`api.chat.getMessages/getMessage/updateMessage/sendMessage`；`api.ui.registerSidebar` / `registerChatCard` / `openModal`。

**SillyTavern 兼容层（互操作经验）：**
全局函数 `eventOn` / `eventEmit` / `eventRemove`、`getVariables` / `setVariables` / `updateVariablesWith`、`getChatMessages` / `setChatMessage` / `setChatMessages`、`SillyTavern.extensionSettings`；legacy 事件别名（如 `message_received` → `message.after_receive`、`message_sent` → `message.after_send`）。

→ AIRP 对应点：
- AIRP 桌面端协议已有 **consent / sandbox**（前序 AIRP-State-Protocol）。AiChat 的 `safe/power/legacy` 三级 + 细粒度权限清单，是可参考的「**权限分级具体化样本**」，可直接启发 **Widget manifest 的 permissions schema**；但 AIRP 必须用自身命名、RFC6902 patch 作用域与 sandbox 声明实现，不复用其 manifest 字段名或 api 形状。
- `ui.inject` + `registerSidebar` / `registerChatCard` / `openModal` ≈ AIRP 的「**Widget 作为第三方前端能力核心载体**」的声明式挂载面；iframe-host 是其沙箱化落地形态。
- 事件钩子体系 ≈ AIRP 的 hook / envelope 体系，可对照丰富事件类型（AIRP 已有干净提示词 / 有界 Agent 不变式，钩子设计需与之兼容）。
- ST 兼容层 = 对「酒馆迁移一级需求」的互操作经验：保留 `SillyTavern.extensionSettings` 等全局桥 + legacy 事件别名映射，使旧扩展平滑过渡。AIRP 已有 `TAVERN-PARITY.md`，可借鉴其「全局 shim + 别名映射」模式，而非复制字段。

---

## 3. iframe-host（沙箱化第三方 Widget 的低层形态）

- `src/iframe-host.html` 极简：仅 `<script src="./iframe-host.js"></script>`，body 无框架内容 → 推断是一个**独立 WebView / iframe 容器**，用于承载第三方 UI（很可能是「女仆」助手或插件 UI），与权限系统配合做隔离。
- `iframe-host.js` 约 90KB（未读取内容，仅确认其存在与体量）。

→ AIRP 对应点：AIRP 把 Widget 定义为第三方前端能力核心载体，L1 声明式 Widget 可由 AI 自动生成。iframe-host 是「第三方前端能力核心载体」的**低层承载参考**；AIRP 应走自己的 sandbox 声明 + envelope 协议（canvas relay 路径），不复制其 90KB 实现。

---

## 4. Agent Center 与「女仆」助手（`docs/agent-center-agent-card-migration-plan.md`）—— 与 AIRP Agent / Widget 中心化、审计对标

**中心化理念：** 把所有 Agent 能力（生图 / 记忆表格 / 血缘图 / 执行泳道 / 各协议 Agent / 摘要 / 手机格式）统一收拢到 **Agent Center**，以「**翻面卡片**」管理：
- 正面：名称、简介、状态、快速启用/禁用。
- 反面：详细说明、设置项、提示词编辑、相关资源入口、最近运行记录。

**数据模型亮点（范式级，非照搬）：**
- **per-profile 存储**：`profiles` 以 `presetType:presetId` 为 key，**保留多套 preset 差异**，避免压成单一全局配置。
- **迁移三件套**：一次性迁移（写 `migrations.*.completed` 版本标记）+ **lazy migrate**（运行时从旧字段补齐）+ **fallback 链**（Agent Center profile → 旧 preset 字段 → default）。统一 resolver 函数避免各处手写 fallback。
- **不覆盖原则**：迁移只对「不存在的 profile」写入；已存在则只补缺失字段，不覆盖用户已编辑值。导入旧包触发 lazy migrate。
- 旧字段 UI 删除时只移除 UI 采集，**不主动 `delete` 或清空旧字段**（避免保存时误清用户数据）。

**「女仆」助手（Agent 产品化样板）：**
- App 内 NL Agent，用自然语言替用户操作：建联系人 / 群聊 / 世界书 / 角色卡、改配置、代发消息、生图设头像壁纸；任务中断可跨轮续作。
- **所有写操作需确认**；Agent Center 留存运行记录与审计。
- 有**长期记忆**：自动记偏好/决定，能自查、归档自身记忆。

**存储韧性（README）：** 本地存储迁移到**原子写入的 KV 文件**，异常断电不丢数据。

→ AIRP 对应点：
- AIRP 把 Widget 定义为第三方前端能力核心载体，且 L1 声明式 Widget 可由 AI 自动生成。AiChat 的 **Agent Center 翻面卡片 + 运行记录/审计 + 权限确认**，正是「**AIRP Agent/Widget 中心化管理 + 审计轨迹 + 用户确认门禁**」的可参考产品形态。
- **per-preset profile + 迁移三件套** = AIRP session/角色级 profile 迁移与兼容的成熟范式，可直接启发 AIRP 的 profile 版本化与 lazy migration（AIRP 已有 session 自包含快照 / 版本化 data design，可叠加此模式）。
- 「女仆」的「NL 操作 + 全程权限确认 + 审计」= AIRP consent/sandbox 在 Agent 形态下的产品化样板，但 AIRP 必须按自身 envelope/guard 实现。
- **原子写 KV** = 直接建议：AIRP 本地持久化应引入 `temp-write + rename` 原子提交，避免损坏（对齐 PulsarAI 的安全取向，但更简单务实）。

---

## 5. 可吸收的「理念清单」（必须 AIRP 独立实现）

| 编号 | 理念 | AIRP 落点建议 |
|---|---|---|
| A | 插件权限三级（`safe`/`power`/`legacy`）+ 细粒度权限清单 | Widget manifest 的 `permissions` schema 启发；AIRP 用自身命名 + sandbox 声明实现 |
| B | `ui.inject` + `registerSidebar`/`registerChatCard`/`openModal` 声明式注入点 | Widget 挂载面设计 |
| C | before/after send/receive/render + `prompt.before/after_build` 钩子体系 | 丰富 AIRP hook / envelope 事件类型 |
| D | Agent Center 翻面卡片 + 运行记录/审计 + 权限确认 | Agent/Widget 中心化管理的产品形态 |
| E | per-preset profile + 一次性迁移 + lazy migrate + fallback resolver | AIRP profile 版本化与向后兼容迁移范式 |
| F | 原子写 KV 存储 | AIRP 本地持久化防损（`temp + rename`） |
| G | iframe-host 沙箱承载第三方 UI | Widget 低层承载参考（AIRP 走自身 sandbox+envelope） |
| H | SillyTavern 兼容层（全局 shim + legacy 事件别名） | 酒馆迁移平滑过渡模式 |
| I | vendored / 锁定版本、不依赖运行期 CDN | 插件/Widget 沙箱运行时可借鉴（主链仍 Vite） |

---

## 6. 明确排除（不可吸收）

- 不复制其 **Vue2/3 双版本混用**结构（AIRP 单链 Vue + Vite）。
- 不复制 `plugin-system.md` 的字段名 / api 形状 / manifest 结构——只采「权限分级」理念。
- 不复制 `agent-center` 迁移计划里的任何字段名 / 数据结构——只采迁移三件套模式。
- 不复制 `iframe-host.js` 任何实现。
- 不复制其 prompt / 正则 / 世界书脚本 / 角色卡内容。
- 许可证 **AGPL-3.0 与 AIRP Apache-2.0 不兼容**，任何代码片段（含「看起来无害」的工具函数）都禁止直接采用。

---

## 7. 下一步建议（可选，待用户决定）

1. AIRP 桌面端 **Widget manifest schema** 设计阶段，把 [A][B][C] 作为对照输入。
2. **session/角色 profile 迁移**设计中，把 [E] 作为范式参考。
3. **consent/sandbox + Agent 中心化**（类 Agent Center）设计中，把 [D][G] 作为产品形态参考。
4. 若建立**酒馆扩展兼容层**，把 [H] 的形态作为对照。
5. 本地持久化引入 [F] 原子写。
