# PR #88 独立审计报告

> 日期：2026-07-07
> 审计对象：PR #88 — `feat(webui): 全新 WebUI V2 设计 — 3 页面 Light 模式 Claude Code 风格`
> 分支：`feat/webui-redesign-v2` → `main`，commit `e2d36cf`
> 范围：9 files, +1946/-0
> 审计立场：独立审计（AGENTS.md 守则：独立审计 / 可提出自己的想法 / 质疑历史决策并主动查证）

---

## 0. 审计方法

- **源码直读**：3 个 HTML 页面 + `colors_and_type.css` + `.design` 项目 + `orchestration-summary.json` + 3 份文档
- **独立验证**（不附和需求文档结论）：
  - `engine/src/daemon/mod.rs` 路由表逐条核对（确认哪些端点真实存在）
  - `engine/src/daemon/handlers.rs` import 逻辑核对（确认 RR-001 / card_path 门控）
  - `webui/app.js` 旧实现对比（确认 import 用 JSON body + base64，非 multipart）
  - npm registry 验证 `lucide@1.8.0` 真实存在（`registry.npmjs.org/lucide/1.8.0` 返回有效包元数据）
- **不跑 cargo test**：本 PR 为设计稿（HTML + CSS + .design），无 Rust 代码

---

## 1. PR 内容概览

| 文件 | 行数 | 说明 |
|------|------|------|
| `airp-engine-console/airp-engine-console.design` | +82 | .design 画布项目元数据 |
| `airp-engine-console/colors_and_type.css` | +80 | 共享 CSS 变量定义 |
| `airp-engine-console/orchestration-summary.json` | +58 | 3 页面导航关系定义 |
| `airp-engine-console/pages/characters.html` | +519 | 页面1：角色列表 |
| `airp-engine-console/pages/session.html` | +428 | 页面2：对话空间 |
| `airp-engine-console/pages/workbench.html` | +398 | 页面3：工作台 |
| `docs/WEBUI-DESIGN-DOC-AUDIT.md` | +127 | WebUI 设计文档合规审计 |
| `docs/audits/PR-85-secondary-audit.md` | +142 | PR #85 二次审计报告 |
| `docs/audits/PR84-AUDIT.md` | +112 | PR #84 独立审计 |

**关键事实**：3 个 HTML 页面均为纯静态设计稿，**无任何 fetch / XMLHttpRequest / axios 调用**（已用 Grep 全量确认）。所有 JS 仅含：`lucide.createIcons()`、折叠面板 toggle、Tab 切换。因此本审计重点为：(1) 设计是否覆盖需求文档要求的所有 UI 行为；(2) 设计稿中展示的 API 路径是否正确；(3) 文档准确性。

---

## 2. 设计需求符合度（对照 `WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md` §2/§3/§4）

### 2.1 页面1：角色列表页（characters.html）— 符合度良好

| 需求（§2） | 设计覆盖 | 证据 |
|------------|---------|------|
| §2.1 API Endpoint 输入 | ✅ | characters.html:196 |
| §2.1 API Key 输入 | ✅ | characters.html:207 |
| §2.1 /v1/ 自动追加提示 | ✅ | characters.html:201 "API 路径 /v1/ 由引擎自动追加" |
| §2.1 Model 下拉 + 手动输入 | ✅ | characters.html:217-236 |
| §2.1 Max Tokens | ✅ | characters.html:241 |
| §2.1 "链接" 按钮 + 绿色状态 | ✅ | characters.html:254-262 |
| §2.2 角色卡片（name + description + 头像占位） | ✅ | characters.html:285-372 |
| §2.3 导入拖放区 | ✅ | characters.html:391-398 |
| §2.4 无鉴权警告条 | ✅ | characters.html:266-270 "Engine 未配置 API Key，仅本地开发可用" |

### 2.2 页面2：对话空间页（session.html）— 符合度良好

| 需求（§3） | 设计覆盖 | 证据 |
|------------|---------|------|
| §3.1 Session 列表（title + preview + time） | ✅ | session.html:187-206 |
| §3.1 "新建对话" 按钮 | ✅ | session.html:183-185 |
| §3.2 Chat 消息区 + 流式光标 | ✅ | session.html:278-325（blink 动画 :323） |
| §3.2 发送 + 停止按钮 | ✅ | session.html:332-337 |
| §3.2 Ctrl+Enter 提示 | ✅ | session.html:330 placeholder |
| §3.3 刷新历史 / Regenerate / Rollback | ✅ | session.html:338-349 |
| §3.4 Agent Run（input + max_steps + Run/Clear） | ✅ | session.html:237-246 |
| §3.5 State 预览 + State History | ✅ | session.html:216-225 |
| §3.6 诊断（一键诊断 + 复制摘要） | ✅ | session.html:266-270 |

### 2.3 页面3：工作台（workbench.html）— 缺删除功能

| 需求（§4） | 设计覆盖 | 证据 |
|------------|---------|------|
| §4.1 角色卡编辑表单（name/desc/personality/scenario/first_mes/system_prompt/example） | ✅ | workbench.html:204-250 |
| §4.1 "保存角色卡" 按钮 | ✅ | workbench.html:256-258 |
| §4.1 **"删除角色" 按钮** | ❌ **缺失** | 全 3 页面无 DELETE 角色入口 |
| §4.1 重新提取（reextract） | 🟡 在 session.html:173-175，非工作台 | 无确认流程 |
| §4.2 世界书条目（keys/content/priority/enabled/comment） | ✅ | workbench.html:271-349 |
| §4.2 "添加条目" + "保存世界书" | ✅ | workbench.html:353-360 |
| §4.3 Dirty 追踪指示器 | ✅ | workbench.html:172 |
| §4.3 ESC 关闭提示 | ✅ | workbench.html:369 |

---

## 3. 审计发现

### A 项（阻塞）— 错误结论会导致实施失败

#### A1 — Event Log 展示了不存在的后端端点

**文件**：`airp-engine-console/pages/characters.html`

Event Log 是设计稿中唯一出现 API 路径的位置，但 4 条事件中有 3 条路径错误：

| 行号 | Event Log 显示 | engine 实际路由（mod.rs 确认） | 问题 |
|------|---------------|------------------------------|------|
| 465 | `POST /v1/messages` | `/v1/chat/completions`（mod.rs:199） | `/v1/messages` 是 Anthropic 上游 API 格式，**engine 无此路由**，fetch 会 404 |
| 493 | `POST /v1/sessions` | `/v1/sessions/:character_id`（mod.rs:253） | 缺 `:character_id` 路径参数，裸 `/v1/sessions` 会 404 |
| 507 | `GET /v1/config` | `/v1/settings`（mod.rs:256） | `/v1/config` **不存在**，fetch 会 404 |

**影响**：设计稿的职责之一是传达正确的 API 契约。实现者若参照 Event Log 中的路径编写 fetch 调用，将直接导致 404。需求文档 §8 有正确的路径映射，但设计稿与文档不一致本身即为实施风险。

**修复**：
- `/v1/messages` → `/v1/chat/completions`
- `/v1/sessions` → `/v1/sessions/{character_id}`（或用具体角色名如 `/v1/sessions/linwanqing`）
- `/v1/config` → `/v1/settings`

#### A2 — `WEBUI-DESIGN-DOC-AUDIT.md` 严重过时，审计结论与 PR 实际内容矛盾

**文件**：`docs/WEBUI-DESIGN-DOC-AUDIT.md`

该文档作为 PR 的一部分提交，但其审计对象和结论与 PR #88 实际内容全面矛盾：

| 文档声称 | PR 实际 | 矛盾性质 |
|---------|--------|---------|
| §1："审计对象：`airp-engine-console/` .design 项目（**console.html + workbench.html**）" | 实际为 3 页面：**characters.html + session.html + workbench.html** | 审计对象不存在 |
| §2.1 D-01："**深色背景** #1A1A1E、温暖黏土/琥珀色 **#D97757**" | 实际为**浅色背景** #FAFAF8、主色 **#C4653A**（PR 标题"Light 模式"） | 主题完全相反 |
| §2.2 D-06 / §4 DS-01："**无鉴权警告缺失** ❌" | characters.html:266-270 **已包含**警告条"Engine 未配置 API Key，仅本地开发可用" | 结论错误 |
| §4 DS-04："console.html 默认 `class='light'`...workbench.html 是 `class='dark'`，**不一致**" | 3 页面统一 `class="light"`，与"Light-first"设计一致 | 问题不存在 |
| §3："refreshAvatar ❌ 设计稿只有首字母占位头像" | 仍然如此 | ✅ 此条仍准确 |

**影响**：该文档以"独立审计"身份给出 DS-01（补无鉴权警告）和 DS-04（统一 dark-first class）两条"可操作建议"，但这两条在当前 PR 中均已解决。若实施者照此文档行动，会重复添加已存在的警告条，或误将浅色主题改为深色。文档作为仓库永久记录会持续误导。

**修复**：要么更新该文档以反映 3 页面 Light 模式设计的实际状态，要么从 PR 中移除（它审计的是一个已不存在的旧设计版本）。

---

### B 项（重要）— 影响设计质量

#### B1 — `.design` JSON 文件重复 `styleConstraints` 键

**文件**：`airp-engine-console/airp-engine-console.design:29-30`

```json
"styleConstraints": { "radiusMax": 14, "staticShadowAlphaMax": 0.03 },
"styleConstraints": { "spacingBase": 8, "fontSizeBody": 13, "fontSizeMin": 11, "controlHeightDefault": 32, "controlHeightLarge": 36 }
```

JSON 标准不允许重复键。大多数解析器静默取最后一个值，导致第一个 `styleConstraints`（含 `radiusMax` 和 `staticShadowAlphaMax`）被丢弃。若工具链读取此文件期望 `radiusMax`，将无法找到。

**修复**：合并为一个对象：`"styleConstraints": { "radiusMax": 14, "staticShadowAlphaMax": 0.03, "spacingBase": 8, ... }`

#### B2 — CSS 变量块在 3 个 HTML 中完全重复，`colors_and_type.css` 未被任何页面引用

**文件**：`characters.html:7-89`、`session.html:7-89`、`workbench.html:7-89`

三个 HTML 各自在 `<style id="theme-vars">` 中内联了完全相同的 80 行 CSS 变量定义（`:root` + `.dark`）。同时 `colors_and_type.css` 文件（80 行，内容与内联块完全相同）存在于同一目录但**未被任何 HTML 通过 `<link>` 引用**。

**影响**：DRY 违反。若需调整一个颜色值，需要修改 4 个位置（3 个 HTML + 1 个 CSS 文件）。这与 AGENTS.md "更易修正、更易迭代更新"的取向相悖。

**修复**：3 个 HTML 用 `<link rel="stylesheet" href="../colors_and_type.css">` 引用共享 CSS，删除内联 `<style id="theme-vars">` 块。

#### B3 — 工作台缺少"删除角色"按钮

**文件**：`airp-engine-console/pages/workbench.html`

需求文档 §4.1 明确列出 `DELETE /v1/characters/:character_id` 为已存在端点，engine 路由确认存在（mod.rs:213 `.delete(delete_character_endpoint)`）。但工作台角色卡编辑面板（workbench.html:194-260）只有"保存角色卡"按钮，无删除入口。全 3 页面均无删除角色的 UI。

**影响**：用户无法通过 UI 删除角色。旧 `webui/app.js` 有此功能。

**修复**：在工作台角色卡 tab 的操作栏（workbench.html:254 附近）增加"删除角色"按钮，配 `confirm()` 确认。

#### B4 — "重解"（reextract）按钮缺少确认流程

**文件**：`airp-engine-console/pages/session.html:173-175`

需求文档 §8 明确要求"重解"操作需 `POST /v1/characters/:character_id/reextract` + `confirm()`。设计稿的"重解"按钮无 `data-dom-id`、无 `onclick`、无确认弹窗或内联确认状态的视觉设计。此问题在 `WEBUI-DESIGN-DOC-AUDIT.md` DS-02 中已指出但未修复。

**修复**：补充内联确认状态设计（如点击后按钮变为"确定重新提取？" + 取消链接）。

#### B5 — 外部 CDN 脚本无 SRI integrity 属性

**文件**：3 个 HTML 页面均存在
- `characters.html:90-91`
- `session.html:90-91`
- `workbench.html:90-91`

```html
<script src="https://cdn.jsdelivr.net/npm/@tailwindcss/browser@4.3.1/dist/index.global.js"></script>
<script src="https://unpkg.com/lucide@1.8.0/dist/umd/lucide.min.js"></script>
```

两个外部脚本均未带 `integrity`（SRI）属性。若 CDN 被入侵或中间人篡改，任意 JS 将在页面上下文执行。对于可访问 engine API（含 bearer token、可触发角色卡导入/删除）的 console 而言，这是中等安全风险。

**修复**：添加 `integrity="sha384-..."` + `crossorigin="anonymous"` 属性。（注：`lucide@1.8.0` 的 integrity 可从 npm registry 返回的 `_integrity` 字段获取。）

#### B6 — 工作台副标题"card.json + raw.json"耦合文件布局

**文件**：`airp-engine-console/pages/workbench.html:198`

```html
<p>编辑角色卡。保存后写回 card.json + raw.json。</p>
```

需求文档 §4.1 明确指出前端应通过 `PUT /v1/characters/:character_id` API 操作，**不应依赖文件路径**。engine 实际读取 `card/card.json`（兼容旧 `card.json`）。设计稿在 UI 文案中暴露内部文件名（且 `raw.json` 未必准确），耦合了实现细节，违反"UI 不依赖文件路径"原则。

**修复**：改为"编辑角色卡。保存后通过 API 写回。"或类似不涉及文件名的描述。

#### B7 — session.html 顶部显示部分令牌 `Bearer sk-...abc`

**文件**：`airp-engine-console/pages/session.html:132`

```html
<input type="text" value="Bearer sk-...abc" ... readonly>
```

需求文档 §7.3 强调 API Key 脱敏原则：`SettingsView` 用 `api_key_set: bool` 表示，**不返回 key 本体**（连脱敏字符串都不返回）。虽然此处的 `sk-...abc` 是 mockup 假数据，但它暗示了"在 UI 中显示部分令牌"的模式，与脱敏原则的意图相悖。characters.html 的 Bearer token 输入框（:137）用 `placeholder="Bearer token..."` 是正确做法。

**修复**：将 session.html:132 的 `value="Bearer sk-...abc"` 改为 `placeholder="Bearer token..."`，与 characters.html 一致。

#### B8 — "链接"按钮 onclick 使用 innerHTML

**文件**：`airp-engine-console/pages/characters.html:259`

```html
onclick="document.getElementById('llm-link-status').innerHTML='<span class=&quot;...&quot;>...链接成功</span>'; ..."
```

虽然当前内容为硬编码（非用户输入，不可利用），但使用 `innerHTML` 设置含 HTML 的内容是不安全模式。若后续实现将状态文本改为包含后端返回的字符串（如模型名），将引入 XSS。旧 `webui/app.js` 全程使用 `textContent`（:75）和 `document.createElement`，更安全。

**修复**：改用 `textContent` 或预建 DOM 节点切换可见性，不使用 `innerHTML`。

---

### C 项（建议）— 设计层面建议

#### C1 — `[data-icon]` CSS 为死代码

**文件**：3 个 HTML 的 `<style>` 块（如 characters.html:105-116）

```css
[data-icon] { display: inline-flex; ... mask-size: contain; ... background-color: currentColor; }
```

3 个页面均使用 lucide 图标库（`<i data-lucide="...">`），无任何元素使用 `data-icon` 属性。此 CSS 块为死代码，应删除。

#### C2 — 角色计数徽章 "6" 硬编码

**文件**：`characters.html:278`

```html
<span ...>6</span>
```

角色数量硬编码为 6，实际应由 `GET /v1/characters` 返回的列表长度决定。设计稿可接受，但实现时需动态填充。

#### C3 — 头像仅显示首字母，无真实头像渲染设计

**文件**：`characters.html:290-291`（及 6 张卡片）

设计稿用首字母圆形占位（`林`/`陈`/`赵`等），但需求文档 §2.2 要求通过 `GET /v1/characters/:character_id/avatar` 获取 PNG 并用 blob URL 渲染。`WEBUI-DESIGN-DOC-AUDIT.md` §3 已指出"refreshAvatar ❌"。建议在设计稿中注明"占位头像，实现时替换为真实 avatar blob URL"。

#### C4 — Agent Run 事件颜色未完整覆盖 spec

**文件**：`session.html:247-248`

需求文档 §3.4 / D-23 要求 Agent Run 事件按类型颜色编码：plan(amber) / tool_call(blue) / tool_result(green) / delta(gray) / done(purple)。设计稿仅展示 `[info]`（蓝）和 `[done]`（绿）两种，未覆盖完整色彩方案。建议补全为 5 种颜色的事件示例。

#### C5 — Event Log 中 `GET /v1/characters` 状态码 101 不合理

**文件**：`characters.html:475-479`

Event 2 显示 `GET /v1/characters` 返回状态码 `101`（Switching Protocols）。该状态码仅用于 HTTP 协议升级（如 WebSocket 握手），对 `GET /v1/characters`（应返回 200 + 角色列表）无意义。应为 `200`。

#### C6 — `orchestration-summary.json` 时间戳差一年

**文件**：`airp-engine-console/orchestration-summary.json:8,30,52`

`createdAt: 1751870400000` 对应 2025-07-07，但 PR 日期为 2026-07-07。应为 `1783406400000`（2026-07-07）或实际创建时间。

#### C7 — State 预览 JSON 用 `"sess_7f3a"` 格式，非 UUID

**文件**：`session.html:217`

```json
"session_id": "sess_7f3a"
```

PR #85 二次审计 O1 确认 session_id 是 UUID 格式（如 `28d96f9f-...`），非 `sess_xxxx`。mockup 数据应反映真实格式。

#### C8 — PR84/PR85 审计文档与设计 PR 混合提交

**文件**：`docs/audits/PR-85-secondary-audit.md`、`docs/audits/PR84-AUDIT.md`

这两份审计文档审计的是 PR #84（需求文档修正）和 PR #85（engine A5/A6 修复），与 PR #88（WebUI 设计稿）无直接关系。将它们混入设计 PR 提交模糊了 PR 边界。建议分开提交或在 PR 描述中说明为何一并提交（如"此前未跟踪文件，此 PR 一并归档"）。

**注**：两份文档本身内容经独立验证准确——PR84-AUDIT 的 A1-A6 复核与 engine 源码一致；PR-85-secondary-audit 的 A5/A6 修复验证与 mod.rs/handlers.rs 一致。

---

## 4. 安全审查

| 检查项 | 结果 | 说明 |
|--------|------|------|
| XSS（innerHTML 使用） | ⚠️ B8 | characters.html:259 onclick 用 innerHTML，当前硬编码不可利用但模式不安全 |
| 路径遍历 | ✅ | 无文件路径操作 |
| CRLF 注入 | ✅ | 无 HTTP 头操作 |
| Null byte | ✅ | 无字符串处理 |
| RR-001 import（JSON body + base64，非 multipart） | ✅ | 设计稿无 fetch 调用，无 card_path/multipart 用法。导入区仅拖放 UI。旧 webui/app.js:944-949 已正确实现 JSON body + card_png_base64/card_json |
| 角色卡裸路径（非 /card 子路径） | ✅ | 设计稿无 API 路径调用角色卡 CRUD，无 /card 子路径误用 |
| CDN 脚本 SRI | ⚠️ B5 | 3 页面 2 个外部脚本无 integrity 属性 |
| 令牌脱敏 | ⚠️ B7 | session.html:132 显示部分令牌 |

---

## 5. 需求专项检查

| 专项要求 | 结果 | 证据 |
|---------|------|------|
| import 用 JSON body + base64（RR-001） | ✅ 无违反 | 设计稿无 API 调用；handlers.rs:341 确认 card_path 被环境变量门控拒绝 |
| 角色卡 CRUD 用裸路径 `/v1/characters/:character_id` | ✅ 无违反 | 设计稿无 API 调用；mod.rs:210 确认裸路径路由存在 |
| session 管理 chat/history 带 session_id（A6 修复后） | ✅ 设计支持 | session.html 有 session 列表 + 点击切换 UI；A6 已由 PR #85 修复（handlers.rs 确认 session_id 字段已加） |
| 无鉴权警告检查 access_api_key_set（A5 修复后） | ✅ 已实现 | characters.html:266-270 警告条已存在；A5 已由 PR #85 修复（SettingsView 加 access_api_key_set） |

---

## 6. 文档审查

### 6.1 `WEBUI-DESIGN-DOC-AUDIT.md` — 严重过时（见 A2）

该文档审计的是一个已不存在的旧版设计（2 页面深色 console.html + workbench.html），与 PR #88 的 3 页面浅色设计全面矛盾。**不建议以当前形式合入仓库**。

### 6.2 `docs/audits/PR84-AUDIT.md` — 准确

经独立验证，A1-A6 六条审计结论均与 engine 源码一致。A1（import 不接受 multipart）、A2（角色卡裸路径已存在）、A3（世界书已存在）、A6（session API 割裂）均经 mod.rs/handlers.rs 确认。文档准确。

### 6.3 `docs/audits/PR-85-secondary-audit.md` — 准确

A5（`access_api_key_set: bool`）和 A6（session_id 字段）修复验证均与 engine 源码一致。O1（ChatLog.session_id 与 scope session_id 不一致）的观察已创建 issue #86 跟踪。文档准确。

---

## 7. 正面发现

| # | 发现 | 证据 |
|---|------|------|
| P1 | 无鉴权警告条已存在 | characters.html:266-270 |
| P2 | /v1/ 自动追加提示已存在 | characters.html:201 |
| P3 | 3 页面浅色主题一致（class="light"） | 全部 HTML `<html lang="zh" class="light">` |
| P4 | `prefers-reduced-motion` 已尊重 | session.html:414-417、workbench.html:376-378 |
| P5 | 流式光标动画（blink keyframe） | session.html:323、410-413 |
| P6 | Dirty 追踪指示器（红点） | workbench.html:172 |
| P7 | Tab 切换、折叠面板可交互 | workbench.html:381-392、session.html:419-424 |
| P8 | lucide@1.8.0 版本真实存在 | npm registry 确认，unpkg 路径 `dist/umd/lucide.min.js` 与包 `unpkg` 字段匹配 |
| P9 | 3 页面导航流定义清晰 | orchestration-summary.json 定义 characters → session → workbench 导航关系 |
| P10 | 导入区无 multipart/card_path 误导文案 | characters.html:399 仅"PNG / JSON / V2 卡片" |

---

## 8. 审计结论

### 8.1 问题汇总

| 级别 | 数量 | 编号 |
|------|------|------|
| A（阻塞） | 2 | A1（Event Log 端点错误）、A2（审计文档过时矛盾） |
| B（重要） | 8 | B1-B8 |
| C（建议） | 8 | C1-C8 |

### 8.2 最终结论：**request changes**

设计稿本身质量良好——3 页面完整覆盖需求文档 §2/§3/§4 的 UI 行为，无鉴权警告、流式光标、dirty 追踪、Tab 切换等均已实现，安全方面无 RR-001 违反。但以下问题需修复后方可合入：

**必须修复（A 项）**：
1. **A1**：修正 characters.html Event Log 中的 3 个错误 API 路径（`/v1/messages` → `/v1/chat/completions`、`/v1/sessions` → `/v1/sessions/:character_id`、`/v1/config` → `/v1/settings`）
2. **A2**：更新或移除 `docs/WEBUI-DESIGN-DOC-AUDIT.md`（当前版本审计的是已不存在的旧设计，结论与 PR 矛盾）

**建议修复（B 项，优先处理 B1/B3/B5）**：
- B1：合并 `.design` 重复 JSON 键
- B3：工作台补"删除角色"按钮
- B5：外部 CDN 脚本加 SRI integrity
- B2/B6/B7/B8 可一并修复

修复 A 项后可合入。B/C 项可按 AGENTS.md 审计遗留项处理规则，PR 合并后整理为 GitHub issue 跟进。

---

## 9. 审计遗留项预清单（PR 合并后转 issue）

以下为本次审计中"未改动但写出来的修改意见"，按 AGENTS.md 规则，PR 合并后应整理为 GitHub issue：

| 编号 | 严重度 | 模块 | 内容 | 建议时机 |
|------|--------|------|------|---------|
| B1 | 中 | design | `.design` 重复 `styleConstraints` 键 | 合并前修 |
| B2 | 中 | webui | CSS 变量块重复 3 次，`colors_and_type.css` 未引用 | 后续迭代 |
| B3 | 中 | webui | 工作台缺"删除角色"按钮 | 合并前修 |
| B4 | 低 | webui | "重解"按钮缺确认流程 | 后续迭代 |
| B5 | 中 | webui | CDN 脚本无 SRI | 合并前修 |
| B6 | 低 | webui | 工作台副标题耦合文件布局 | 后续迭代 |
| B7 | 低 | webui | session.html 显示部分令牌 | 后续迭代 |
| B8 | 低 | webui | onclick 用 innerHTML | 后续迭代 |
| C1-C8 | 极低 | 各模块 | 死代码/硬编码/mockup 数据格式 | 后续迭代 |
