# AIRP WebUI 分析与优化报告

> **历史快照**：本文早于后续 PR #77–#125，其中 UI/IPC 完成度、全量 history 判断和百分比已过期。PR #124/#125 已交付 durable history 与 WebUI window；产品 UI 虚拟化/性能门仍开放。当前结论见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。

> 日期：2026-07-06
> 范围：`webui/`（临时浏览器验证 harness）+ `ui/`（Tauri 桌面产品端）
> 目的：梳理两套 UI 的现状、问题、优化方向，为后续迭代提供工程决策依据

---

## 1. 架构总览

本项目存在 **两套完全独立的 UI 前端**，服务于不同定位：

| 维度 | `webui/` (浏览器 Harness) | `ui/` (Tauri 桌面产品) |
|------|--------------------------|------------------------|
| 定位 | 临时后端验证 harness，非产品 | 长期产品 UI |
| 技术栈 | 原生 HTML + CSS + JS（零构建） | Vue 3 + TypeScript + Vite + Tauri |
| 通信方式 | 直接 `fetch` 调 engine HTTP API | AgentBus 协议（Tauri IPC / MockBus） |
| 状态管理 | 全局变量 + DOM 操作 | Vue reactive（`stateStore`）+ RFC6902 patch |
| 渲染模型 | 命令式 DOM 构建 | 声明式 Blueprint + Widget Registry |
| 功能覆盖 | 全部 engine API（chat/agent/state/diagnostics） | 仅 chat + character select（phase0） |
| 安全模型 | Bearer token + `sessionStorage`（未实现） | Tauri IPC（本机可信通道） |

**核心矛盾**：`webui/` 在功能覆盖上远超 `ui/`（后者仅处于 phase0），但 `webui/` 是临时性质不做产品化；`ui/` 有完善的协议/Widget 架构但功能尚未落地。

---

## 2. `webui/` 详细分析

### 2.1 优点

- **零构建约束**：纯 HTML/CSS/JS，双击 `start.bat` 即可运行，对验证场景极度友好
- **完整 API 覆盖**：chat streaming、agent run event log、state/history、character import、workbench 编辑，后端能力几乎全部暴露
- **错误可见性好**：每个 HTTP 请求的状态码、耗时、错误体都记录在 Event Log；diagnostics 一键扫描
- **安全意识到位**：`card_path` 永禁（RR-001）、avatar 通过 blob URL 渲染避免 bearer 泄露、SSE abort 防竞态
- **markdown 渲染安全**：先 escapeHtml 全转义，再用 private-use Unicode 占位符隔离 code fence，不会 XSS

### 2.2 问题与优化建议

#### P0: 可用性问题

**[W-01] 连接状态不持久**
- 现状：每次刷新页面都重新走一遍连接流程（`setTimeout(connect, 300)`），Bearer token 在 `<input>` 里刷新即丢
- 建议：Bearer token 存 `sessionStorage`（关 tab 即清，降 XSS 持久风险，WEBUI-BACKEND-PLAN §4.4 已提到但未实现）
- 影响：用户体验差，每次开页面都要重输

**[W-02] 并发 stream 测试暴露在主 UI**
- 现状：`#concurrent-test` 区块直接放在 center panel，和正常聊天区域混在一起
- 建议：移到 Diagnostics 折叠区，或改为仅开发者可见的 debug 模式开关
- 影响：普通用户被不相关的测试功能干扰

**[W-03] 左侧面板信息过载**
- 现状：Engine health、Settings、Models、Characters、Sessions、State、State History、Import 全堆在 280px 宽的侧栏
- 建议：
  - Engine/Settings/Models 合并为一个「连接信息」可折叠区
  - State/State History 合并，默认折叠
  - 左侧核心只留 Characters + Sessions + Import
- 影响：信息密度过高，新用户找不到关键操作

**[W-04] Agent Runner 和 Chat 混在同一面板**
- 现状：chat input 下方紧接着 agent runner、concurrent test、diagnostics，垂直空间被大量挤占
- 建议：将 agent runner、concurrent test、diagnostics 统一收进一个「工具箱」标签页或可折叠面板
- 影响：chat 区域被压缩，长时间对话时体验差

#### P1: 功能完善

**[W-05] 缺少 Streaming 时中断/取消的 UI**
- 现状：abort 逻辑存在（`abortController`），但用户没有 UI 按钮来主动中断正在进行的 streaming
- 建议：在 chat input 旁边增加「停止生成」按钮（仅在 streaming 进行时显示），调用 `abortController.abort()`
- 影响：用户无法取消不想要的回复，只能等待完成

**[W-06] Chat 消息没有时间戳**
- 现状：`appendMsg` 只渲染 role + text，不显示时间
- 建议：在 `.msg .role` 同行增加 `toLocaleTimeString()` 时间戳（可默认折叠，hover 显示）
- 影响：无法判断消息时序

**[W-07] History 按钮语义不清**
- 现状：按钮文字为 "History"，实际行为是「从 engine 拉取完整聊天记录覆盖本地视图」
- 建议：改为「刷新历史」或「同步记录」，并增加 tooltip 说明行为

**[W-08] Session 选择器只显示前 12 字符**
- 现状：`id.slice(0, 12)` 截断，UUID 前 12 位无法区分
- 建议：显示后 8 位（更有区分度），或显示创建时间

**[W-09] 缺少 Settings 写入 UI**
- 现状：`/v1/settings` POST 在 engine 侧已实现，但 WebUI 只读不写
- 建议：在 Settings 区域增加「编辑」按钮，弹出一个简单的 endpoint/model/api_key 表单
- 注意：WEBUI-BACKEND-PLAN §10 明确说「不做 access key 管理 UI」，但 endpoint/model 是常用配置
- 折中：只暴露 endpoint 和 model（不含 api_key），api_key 改动走 engine 环境变量

#### P2: 工程质量

**[W-10] `app.js` 单文件 1365 行，无模块化**
- 现状：所有逻辑（API 层、chat、agent、workbench、diagnostics、markdown renderer）都在一个 IIFE 里
- 建议：由于是零构建约束，可以用多个 `<script>` 标签按职责拆分（如 `api.js`、`chat.js`、`workbench.js`、`agent.js`），或用原生 ES modules（`type="module"`）
- 影响：维护成本随功能增长快速上升

**[W-11] markdown renderer 有边界 case**
- 现状：不支持列表（`- item`）、链接（`[text](url)`）、表格、嵌套 code fence
- 建议：按 WEBUI-BACKEND-PLAN 的定位（harness，不做产品），当前覆盖够用。但建议在 renderer 入口加一行注释标明支持范围，避免后续维护者期望完整 GFM
- 影响：低——harness 场景不需要完整 markdown

**[W-12] Workbench 拖拽调整宽度可能有边界问题**
- 现状：`initWorkbenchResizer` 在 `mouseleave`/`blur` 时兜底 endDrag，但缺少 `mouseup` 在非 window 元素上的处理
- 建议：当前实现基本可用。可考虑用 CSS `resize: horizontal`（已设）替代自定义拖拽，简化代码

**[W-13] `serve.js` 无 gzip/brotli 压缩**
- 现状：纯 `fs.readFile` + `res.end`，无压缩
- 影响：低——`app.js` 约 40KB 未压缩，对本地网络几乎无感
- 建议：不优化，保持零依赖

---

## 3. `ui/` (Tauri) 详细分析

### 3.1 优点

- **协议驱动架构**：Blueprint + Widget Registry + Envelope + RFC6902 patch，扩展性强
- **安全纵深**：consent 机制、sandbox iframe（`opaque-origin`）、capability 声明、运行时 guard
- **虚拟滚动**：ChatWidget 实现了 `computeWindow` 虚拟列表，100k 消息也能保持性能
- **TypeScript 全覆盖**：类型安全的协议绑定，Rust wire types ↔ TS types 手动镜像
- **可测试性**：MockBus、vitest 单测、agent-test harness

### 3.2 问题与优化建议

#### P0: 功能落地

**[T-01] 仅 Phase0 — 功能远不如 webui/**
- 现状：只有 chat + character select（列表 + 导入），缺：
  - Agent run 事件展示
  - Session 管理（列表、创建、切换）
  - State/History 查看
  - Workbench（角色卡/世界书编辑）
  - Streaming token 渲染（当前 ChatWidget 只显示完整 text）
  - History/Regen/Rollback
  - Diagnostics
- 建议：这是 Tauri UI 的核心差距。优先级：
  1. Streaming chat（逐 token 渲染 + markdown）
  2. Session CRUD
  3. Agent run event log
  4. State viewer
  5. Workbench
- 影响：产品端无法替代 webui/ 做日常使用

**[T-02] Chat 消息固定 48px 高度不适应变长内容**
- 现状：`ITEM_H = 48` 固定行高，虚拟滚动基于此。消息文本被裁剪
- 建议：
  - 方案 A：动态行高虚拟滚动（需要测量每行高度，复杂度高）
  - 方案 B：取消固定行高，改用 CSS `overflow-y: auto` + 懒加载（简单但放弃虚拟滚动）
  - 方案 C：保持虚拟滚动但允许行高变化（`ResizeObserver` 监听 + 重算 window）
- 推荐：短期用方案 B（RP 聊天场景单次会话很少超 1000 条），长期用方案 C

**[T-03] ChatWidget 无 markdown 渲染**
- 现状：`<span class="text">{{ m.text }}</span>` 纯文本，不支持 markdown
- 建议：引入轻量 markdown 渲染（如 `marked` + DOMPurify），或从 webui 移植手写 renderer
- 影响：AI 回复中的格式化内容（代码块、标题、列表）无法正常显示

**[T-04] 无 streaming 体验**
- 现状：ChatWidget 只在 state `set`/`patch` 到来时渲染完整消息，没有逐 token 流式效果
- 建议：需要在 Bus 层支持 SSE 或增量 delta 推送，ChatWidget 维护一个「正在生成」的临时消息
- 这需要 engine→Tauri IPC 支持 SSE 事件流，是一个较大的跨层改动

**[T-04b] TauriBus ↔ engine IPC 未完成（审计 P0 #3）**
- 现状：`ui/src/protocol/tauri-bus.ts` 有文件骨架但未完成实际实现，产品 UI 只能跑 MockBus（假数据）
- 这是 ui 产品线的**第一阻塞项**——DEV-GUIDE 首要目标「可执行文件双击启动 + 真实对话闭环」因此未验收
- 建议：Sprint 1 任务 1.2 已明确标记——完成 TauriBus 实现 + src-tauri 侧桥接 engine sidecar HTTP/SSE
- 影响：没有 IPC 打通，ui 的所有 widget（ChatWidget、CharactersWidget 等）都无法与真实 engine 交互

#### P1: 架构改进

**[T-05] MINIMAL_BLUEPRINT 硬编码在 App.vue**
- 现状：Tauri 模式下 blueprint 在前端硬编码，engine 不推送 blueprint
- 建议：让 engine 在 Tauri 模式下也推送 blueprint（通过 IPC），UI 完全由引擎驱动
- 影响：当前硬编码意味着 UI 布局无法动态调整

**[T-06] Widget 系统功能丰富但实际注册的 widget 没有接入**
- 现状：有 Card、Map、Quest、Inventory、Memory、Emotion 等 8 个 widget 组件 + manifest，但 MINIMAL_BLUEPRINT 只用了 chat + characters
- 建议：这些 RP 专用 widget 需要等 engine 的 state patch 支持才能落地。优先实现 engine → UI 的完整 state 推送链路

**[T-07] 缺少全局错误处理 UI**
- 现状：`busError` 只在 topbar 下方显示一行红色文字
- 建议：增加 toast/通知系统，区分「连接断开」「权限错误」「provider 错误」等类别

**[T-08] 角色列表只显示 slug，无头像/描述**
- 现状：CharactersWidget 只渲染 `id`（slug），没有 avatar 预览或描述
- 建议：增加角色卡的 name + avatar 显示（从 engine `/v1/characters/:id/avatar` 获取）

#### P2: 工程质量

**[T-09] TypeScript 类型手写镜像 Rust types，无自动化同步**
- 现状：`protocol/types.ts` 手动镜像 `protocol/src/lib.rs`，注释说「keep it in sync」
- 建议：考虑用 `ts-rs` 或 `specta` 从 Rust 自动生成 TS 类型，消除人工同步风险
- 影响：中等——当前类型变化不频繁，但长期会累积 drift

**[T-10] 测试覆盖不均**
- 现状：protocol 层（bus-factory、guard、consent、registry）有较好的 vitest 覆盖，但 Vue 组件和 Widget 零测试
- 建议：至少为 ChatWidget 和 CharactersWidget 加 smoke test（mount + 检查关键 DOM）

---

## 4. 两套 UI 的关系与建议

### 4.1 核心判断

**webui/ 是当前唯一可完整体验后端能力的入口，ui/ 是长期产品方向但功能严重不足。**

两者不应合并或互相替代，但应：
1. 从 webui/ 的完整功能实现中提炼 UI 交互模式，指导 ui/ 的产品化
2. webui/ 的 markdown renderer、SSE 处理、workbench 等实现可作为 ui/ 的参考
3. ui/ 的协议/Widget/sandbox 架构是正确的产品基础设施，应持续投入

### 4.2 路线建议

**短期（1-2 周）— 让 webui/ 更好用**
- [W-01] Bearer token 持久化到 sessionStorage
- [W-05] 增加「停止生成」按钮
- [W-03] 左侧面板信息分组折叠
- [W-04] Agent Runner / Concurrent Test / Diagnostics 收进工具箱

**中期（2-4 周）— 让 ui/ 功能追平 webui/ 核心**
- [T-01] Streaming chat 实现
- [T-02] ChatWidget 动态行高
- [T-03] Markdown 渲染
- Session 管理 widget
- Agent event log widget

**长期（1-2 月）— 产品化**
- Workbench（角色卡/世界书编辑）产品级实现
- State viewer / Map / Quest 等 RP widget 全部接入
- [T-05] Engine 推送 Blueprint，UI 完全声明式
- [T-09] Rust→TS 类型自动生成
- webui/ 降级为 developer-only 诊断页或归档

### 4.3 可复用资产

| webui/ 实现 | ui/ 可复用方式 |
|------------|---------------|
| `renderMarkdown()` | 移植为 Vue composable `useMarkdown()` |
| `streamSse()` SSE 解析 | 封装为 Tauri IPC 的 SSE 事件适配层 |
| Workbench 角色卡/世界书编辑 UI | 产品化重写为 Widget（`core.workbench`） |
| Diagnostics 一键扫描 | 产品化为设置页的「连接测试」入口 |
| Agent event log 渲染 | 产品化为 `core.agent-log` Widget |

---

## 5. 项目文档承诺与兑现对照（UI 相关）

> **历史对照表**：下列结论只反映 2026-07-06 的判断，不能作为当前 UI 状态或 Sprint 计划。当前优先级以 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 为准。

### 5.1 文档承诺的 UI 相关条目兑现情况

| 文档承诺 | 兑现度 | 详情 |
|---------|--------|------|
| 虚拟滚动（Perf Spike 10万条 60fps） | 🔴 代码在，从未真跑 | `virtual-window.ts` 的 `computeWindow` 已实现但无真实压测证据（PLAN §2.5、DEV-GUIDE §7） |
| 流式增量追加渲染，禁每 token 重解析整段 markdown | 🟡 webui 做到，ui 未做 | webui 用 textContent 流式 + 完成后切 innerHTML；ui 只有完整消息渲染（PLAN §2.5 约束 #6） |
| 状态更新 patch 优先，禁全量重灌 | ✅ ui 层已实现 | `stateStore` + RFC6902 patch，含 `test` op 预校验 |
| UI 只渲染声明式 Blueprint，不执行 agent 生成的代码 | ✅ 架构在 | BlueprintRenderer + WidgetHost + esm sandbox |
| Stable ID 做 key | ✅ id-keyed chat 已实现 | PR #6 完成 `{messages, order}` 模型 |
| TauriBus ↔ engine IPC 打通 | 🔴 未完成 | `tauri-bus.ts` 实现未完成，产品 UI 只能跑 MockBus（审计发现 #3） |
| 可执行文件双击启动 + 真实对话闭环 | 🟡 50% | 打包链路通（PR #13），但运行时验收缺失（审计发现 #4） |
| Perf Spike 验证门 | 🔴 0% | 从未执行（PLAN §2.5、DEV-GUIDE §7 均列为硬要求） |
| 端到端闭环（双击→选角色→发消息→收流式回复） | 🟡 80% | webui 可达；Tauri 产品端因 IPC 未通而不完整 |

### 5.2 历史 UI 阻塞项（2026-07-05 审计）

审计报告列出 5 个 P0 阻塞项中，有 3 个与 UI 直接相关：

1. **Agent 内核能力面为空**（兑现度 ~15%）：无 ReAct 规划、无工具结果回灌（M_AGENT-4）、无 memory/skill/hook/macro/subagent。虽然这不直接是 UI 层问题，但意味着即使 UI 做好 streaming/agent event log，后端也无法产生有意义的 agent 事件供 UI 展示。
   - **对 UI 的影响**：Sprint 2 完成后（M_AGENT-4/5），ui 的 Agent event log widget 才有真实数据可渲染。

2. **TauriBus ↔ engine IPC 未完成**（审计发现 #3）：这是 ui 产品线的**第一阻塞项**。`tauri-bus.ts` 虽然有文件骨架，但与 engine sidecar 的 HTTP/SSE 桥接未实现。在 Sprint 1（端到端闭环）中标记为任务 1.2。
   - **对 UI 的影响**：当前 ui 只能用 MockBus（假数据），完全无法替代 webui 做日常使用。

3. **Perf Spike 未验证**（审计发现 #4）：虚拟滚动代码已就位但从未真压测。DEV-GUIDE §7 列为硬约束——「过了才锁 Tauri+Vue」。
   - **对 UI 的影响**：ChatWidget 的固定 48px 行高 + 虚拟滚动方案是否可靠，目前无数据支撑。

### 5.3 历史路线图（Sprint 视角）

按当时的 Sprint 规划，UI 相关优先级如下；当前不再采用此排序：

| Sprint | UI 任务 | 前置依赖 |
|--------|--------|---------|
| Sprint 1（1-2周） | TauriBus ↔ engine IPC 打通（任务 1.2）；可执行文件运行时验收（任务 1.3） | engine sidecar 已打包 |
| Sprint 2（2周） | Agent event log widget 数据流就绪（M_AGENT-4/5 完成后） | Agent 工具结果回灌 |
| Sprint 3（2-3周） | webui 世界书命中展示；ui lorebook widget 骨架 | 世界书引擎（Task 1.3） |
| Sprint 4（2周） | 事件总线 SSE 订阅端点 → ui 实时事件通知 | 扩展接口地基 |
| Sprint 5+ | ChatWidget markdown + 头像 + 时间戳；会话管理 UI；Perf Spike；Workbench | 以上全部 |

### 5.4 PLAN.md §3.7/§4 中未在本文第 2-3 节覆盖的 UI 约束

**性能契约 7 条（PLAN §2.5）中的 UI 层硬约束**：

| 约束 | webui 状态 | ui 状态 |
|------|-----------|---------|
| 1. 聊天/长列表强制虚拟滚动 | ❌ 无虚拟滚动（列表全渲染） | ✅ `computeWindow` 已实现（但未压测） |
| 2. 全量历史真相在引擎，UI 窗口分页拉取 | 🟡 一次拉全量 history | ✅ id-keyed + patch |
| 3. 状态更新 patch 优先 | ❌ 全量替换 chatLog.innerHTML | ✅ RFC6902 patch |
| 4. 稳定 ID 做 key | ❌ 无 ID（直接 appendChild） | ✅ 消息有 id |
| 5. 重计算留 Rust | ✅ 不适用（harness） | ✅ engine sidecar |
| 6. 流式增量追加渲染 | ✅ textContent 流式 + 完成后切 markdown | ❌ 只渲染完整消息 |
| 7. 内存卫生 | 🟡 event log 有 200 条上限 | ❌ 无显式内存管理 |

注意：webui 作为临时 harness，不要求满足全部 7 条性能契约（WEBUI-BACKEND-PLAN §10 明确说不做产品级打磨）。但 ui 作为产品端，必须全部满足。

---

## 6. 安全相关观察

| 项目 | 评估 |
|------|------|
| webui `card_path` 永禁 | 正确 — RR-001 护栏完好 |
| avatar blob URL + revoke | 正确 — 防 bearer 泄漏 + 防 memory leak |
| markdown escapeHtml | 正确 — 先转义再渲染，无 XSS |
| CORS `Any` | 可接受（本地 dev），但跨设备访问需收紧 |
| bearer token 明文 `<input type="password">` | 可接受（本地），但应存 sessionStorage 而非裸露在 DOM |
| Tauri consent + sandbox | 正确 — 纵深防御设计合理 |
| serve.js 路径穿越防护 | 正确 — `path.relative` 检查 + null byte 拦截 |
| serve.js malformed URL 防护 | 正确 — try/catch `decodeURIComponent` |

---

## 7. 不做的事（与 WEBUI-BACKEND-PLAN 对齐）

- 不在 webui/ 做产品级 UI 打磨（主题/响应式/无障碍）
- 不在 webui/ 做 access key 管理 UI
- 不让 webui/ 反向决定 Tauri UI 架构
- 不合并两套 UI 为一套（定位不同）
- 不在 harness 层引入框架/构建链
