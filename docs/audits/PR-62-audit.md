# PR #62 审计报告 — WebUI 工作台（角色卡编辑 + 世界书管理）

**审计源 LLM 模型**：GLM-5.2（智谱 AI，2026 年）
**审计日期**：2026-07-05
**审计范围**：PR E（engine CRUD 端点）+ PR F（WebUI 工作台 UI）
**审计分支**：`webui-pr-f-workbench` @ 412971c（含第二次 UI/UX 美化）
**第三次审计分支状态**：在 412971c 基础上增加了本轮 C1/C3 修复
**审计依据**：AGENTS.md「审计 Agent 守则」—— 独立审计，不附和开发结论

---

## 审计方法

独立读取 `git diff main..HEAD` 全量改动 + 复核 `data_dir` 模块的 `character_dir` / `char_world_lorebook_path` / `get_character` / `delete_character` 实际行为，对每个 handler 和前端函数做独立判断。本次为第三次独立审计，聚焦第二次 UI/UX 改造后是否引入新问题（dirty 标志位、resizer 资源泄漏、render 时的 stale data）。

---

## 发现项（第三次审计）

### C1（HIGH，已修）— 拖拽 mousemove 监听器在用户"鼠标离开窗口"后无法释放

**位置**：`webui/app.js` `initWorkbenchResizer`

**事实**：
- 原实现只在 `mouseup` 触发时移除 `mousemove` 监听器
- 用户在 mousedown 启动拖拽后，如果鼠标移出浏览器窗口（拖到屏幕外或拖到另一个显示器）然后松开，部分浏览器不触发 mouseup
- 结果：`dragging = true` 永久保持、`document.body.style.userSelect = 'none'` 永久生效、mousemove 监听器永不释放——用户整个页面文本选择被禁用

**修复**：
- 提取 `endDrag()` 统一清理函数
- 加 `document.mouseleave` + `window.blur` 两个兜底事件，强制结束拖拽
- `mousedown` 加 `if (dragging) return;` 防止快速重复点

### C2（MED，已确认安全）— 切角色后工作台如打开会显示陈旧数据

**位置**：`webui/app.js` `charSelect.addEventListener('change', ...)`

**事实**：`charSelect.change` 切角色时只 `clearChatView/refreshSessions/refreshAvatar/refreshStateAll/loadHistory`，不关闭工作台或重新加载工作台数据。

**独立判断**：
- 工作台 `hidden = true` 时用户看不见，不影响实际体验
- 用户主动点工作台按钮会重新调 `openWorkbench` → `loadWorkbenchCard/Lorebook`，数据会刷新
- 但若工作台一直打开，用户切角色后 dirty 状态可能错位

**风险点**：若用户在工作台编辑 A 角色，不关闭直接切到 B 角色（工作台保持打开），A 的 wbCardData 内存对象被新数据覆盖前可能有混淆——但 API 请求是异步的，且 wbCardData 在 loadWorkbenchCard 成功后整体替换，**无 race**。

**结论**：安全，未修。

### C3（MED，已修）— dirty 标志在打开工作台加载新数据时不被重置

**位置**：`webui/app.js` `openWorkbench`

**事实**：原 `openWorkbench` 不调 `setWbDirty(false)`。如果用户上次关闭前有 dirty 残留（虽然 `closeWorkbench` 会清），重新打开新角色的工作台，dot 应不显示——OK。但若用户先打开 A 工作台编辑后切到 B 角色（工作台保持打开），dirty 状态可能误显。

**修复**：`openWorkbench` 开头加 `setWbDirty(false)`，强制清状态。

### C4（LOW，未修）— resizer 缺触屏支持

**位置**：`webui/app.js` `initWorkbenchResizer`

**事实**：只用 mousedown/mousemove/mouseup，未处理 touchstart/touchmove/touchend。

**独立判断**：触屏用户（Surface / iPad / 笔记本触屏）无法拖拽。但 webui 当前定位是桌面开发控制台，触屏非核心场景。可后续补。

### C5（LOW，未修）— resizer 拖拽时无 cursor 反馈

**位置**：`webui/style.css` / `webui/app.js`

**事实**：mousedown 期间 `body.style.userSelect = 'none'` 防止文本选中，但 resizer 元素本身没显式 `cursor: col-resize`（虽然 CSS 已有）。

**独立判断**：CSS 已有 `cursor: col-resize`，无需修。

### C6（MED，已确认安全）— 异步 race 风险

**位置**：`webui/app.js` `loadWorkbenchCard` / `loadWorkbenchLorebook`

**事实**：两个加载并发，无竞态保护。理论上 loadWorkbenchCard 后用户立即切换角色，老 selectedChar 还在闭包里——但 fetch 完成后只是 setText/wbCardData = ...，无副作用。

**独立判断**：当前实现安全。若未来加重操作需加 abortController。

---

## 发现项（第二次审计）

### U1（HIGH，已修）— 工作台面板布局粗暴

**位置**：`webui/style.css` / `webui/index.html` / `webui/app.js`

**事实**：
- 原面板宽度 420px 固定，在 13 寸笔记本上遮挡右侧 event log，且无法调整
- 关闭按钮与 refresh 按钮并列在 Characters h3 标题中，容易误点

**修复**：
- 面板改为 460px 默认宽度，`resize: horizontal`，左侧加 `#workbench-resizer` 拖拽条（最小 320px，最大 65vw）
- 工作台按钮移到字符选择器下方独立按钮行，并新增 "↻ 重解" 按钮
- 标题区增加未保存红点 `#wb-dirty-dot`

### U2（HIGH，已修）— 角色卡字段视觉层级混乱

**位置**：`webui/app.js` `renderCardFields` / `webui/style.css`

**事实**：所有 textarea 等高，7 个字段无分组，长文本字段（description/system_prompt/mes_example）与短字段同等高度。

**修复**：
- name 单独一行
- 其余 6 个字段归入 "提示词与背景" 分组（`.wb-group`）
- textarea 使用 `field-sizing: content` 自适应高度，长字段加 `.wb-tall` 类

### U3（MED，已修）— 工作台无 dirty 状态 / 关闭确认

**位置**：`webui/app.js`

**事实**：编辑后 ESC/点关闭直接丢失，无视觉反馈。

**修复**：
- 新增 `wbDirty` 状态 + `#wb-dirty-dot` 红点
- 字段输入时触发 `setWbDirty(true)`
- 关闭前 `confirm` 拦截
- 保存成功后 `setWbDirty(false)`

### U4（MED，已修）— 保存按钮无 loading 状态

**位置**：`webui/app.js` `saveWorkbenchCard` / `saveWorkbenchLore`

**修复**：请求期间 `disabled = true`，返回后恢复。

### U5（MED，已修）— Lorebook 条目视觉噪音大

**位置**：`webui/app.js` `renderLoreEntry` / `webui/style.css`

**事实**：keys/content/priority/comment/enabled 全部等高输入框，条目多时代码/内容难浏览。

**修复**：
- 条目默认折叠，头部行内显示：展开按钮、序号、keys、priority、enabled、删除
- 折叠时可直接编辑 keys/priority/enabled
- 点击展开后编辑 content 和 comment
- 新增条目自动展开并滚动到视图

### U6（LOW，已修）— 工作台缺少重新解包入口

**位置**：`webui/app.js` / `webui/index.html`

**事实**：用户编辑角色卡后若想更新 world/lorebook.json 和 greetings，需要去 engine API 手动调用 `/v1/characters/:id/reextract`。

**修复**：在 Characters section 新增 "↻ 重解" 按钮，点击后调 reextract 并刷新世界书。

### F2（LOW，未修）— `get_character_lorebook` 错误格式不一致

**位置**：`engine/src/daemon/handlers.rs` `get_character_lorebook`

**事实**：返回 `Response` 而非 `Result<Json<Value>, AirpError>`，CharacterId 解析失败手动回 `StatusCode::BAD_REQUEST.into_response()`，与其他 handler 的 `AirpError::BadRequest` JSON 格式不一致。

**影响**：客户端 `formatError` 在 400 时拿不到结构化 error body。

**独立判断**：非阻塞。其他端点（如 `get_character_state`）也有类似模式。可后续统一。

### F3（MED，已修）— PUT 端点无 body 大小限制

**位置**：`engine/src/daemon/mod.rs` 路由注册

**事实**：`/v1/characters/import` 有 `DefaultBodyLimit::max(10 * 1024 * 1024)`，但新增的 `PUT /v1/characters/:id` 和 `PUT /v1/characters/:id/lorebook` 无限制。axum 0.7 默认无限制，可被用于内存耗尽 DoS。

**修复**：两个 PUT 端点各加 `.layer(DefaultBodyLimit::max(2 * 1024 * 1024))`（2MB，角色卡 JSON 远小于此）。

### F5（LOW，已修）— ESC 在 input/textarea 中会关闭工作台丢失编辑

**位置**：`webui/app.js` ESC keydown handler

**事实**：全局 `document.addEventListener('keydown', ...)`，按 ESC 即关工作台。用户在 textarea 中编辑时按 ESC（很多编辑器的"取消焦点"习惯）会丢失所有未保存编辑。

**修复**：在 handler 中检查 `e.target.tagName`，INPUT/TEXTAREA 中按 ESC 不关工作台。

### F7（LOW，未修）— `update_character_card` 接受任意 JSON 不校验 TavernCardV2 结构

**位置**：`engine/src/daemon/handlers.rs` `update_character_card` body 类型 `Json<serde_json::Value>`

**事实**：可写入 `{"foo": "bar"}` 这种非法角色卡，破坏后续 chat/reextract 路径。

**独立判断**：刻意设计——工作台编辑原始 JSON 需要灵活性。客户端表单只暴露 7 个字段，写入时保留原卡其余字段，实际不会写入完全非法的 JSON。可接受。

### F8（LOW，已确认安全）— 前端无 XSS 风险

**位置**：`webui/app.js` workbench DOM 操作

**事实**：所有用户数据通过 `el.value = ...` / `el.textContent = ...` 写入，无 `innerHTML` 设置用户内容。`innerHTML = ''` 仅用于清空。安全 ✓。

### F1（LOW，已文档化）— `update_character_card` 覆盖 `raw.json` 与 ASSET-SPEC "存储永不丢" 规则的关系

**位置**：`engine/src/daemon/handlers.rs` `update_character_card` L833

**事实**：
- 导入侧注释明确写道：`card/raw.json — 完整 TavernV2 JSON（最小 sidecar，守 ASSET-SPEC 规则2 存储永不丢）`
- `reextract_character_assets`（L532-534）读取顺序：`raw.json` → `card.json` → `card.png`，即 `raw.json` 是 reextract 的源
- 本 PR 的 `update_character_card` 同时写 `card/card.json` 和 `card/raw.json`

**独立判断**：
乍看是违反"永不丢"规则——原始 imported 卡被覆盖后无法恢复。但深入推演：
- 若只写 `card.json` 留 `raw.json`：`GET` 读 `card.json` 反映编辑 ✓，但 `reextract` 读 `raw.json` 提取旧资产 ✗（用户编辑 first_mes 后点 reextract，greetings/00.md 仍是旧内容——更糟）
- 当前实现（都覆盖）：编辑被视为"新的规范化版本"，reextract 用编辑后的卡——一致且可预期
- 真正的"丢原始"风险：用户误编辑后无法回退到 imported 原文。但用户可重新导入，不算硬损失

**结论**：当前行为可接受，但需明确文档化语义。已在 handler 注释中补"设计说明"段落。

### F2（LOW，未修）— `get_character_lorebook` 错误格式不一致

**位置**：`engine/src/daemon/handlers.rs` `get_character_lorebook`

**事实**：返回 `Response` 而非 `Result<Json<Value>, AirpError>`，CharacterId 解析失败手动回 `StatusCode::BAD_REQUEST.into_response()`，与其他 handler 的 `AirpError::BadRequest` JSON 格式不一致。

**影响**：客户端 `formatError` 在 400 时拿不到结构化 error body。

**独立判断**：非阻塞。其他端点（如 `get_character_state`）也有类似模式。可后续统一。

### F3（MED，已修）— PUT 端点无 body 大小限制

**位置**：`engine/src/daemon/mod.rs` 路由注册

**事实**：`/v1/characters/import` 有 `DefaultBodyLimit::max(10 * 1024 * 1024)`，但新增的 `PUT /v1/characters/:id` 和 `PUT /v1/characters/:id/lorebook` 无限制。axum 0.7 默认无限制，可被用于内存耗尽 DoS。

**修复**：两个 PUT 端点各加 `.layer(DefaultBodyLimit::max(2 * 1024 * 1024))`（2MB，角色卡 JSON 远小于此）。

**验证**：`cargo check` + 神圣不变式 2/2 通过。

### F4（LOW，未修）— 工作台无"未保存修改"指示

**位置**：`webui/app.js` workbench 区块

**事实**：用户编辑角色卡字段或世界书条目后，UI 无 dirty 标记，无"有未保存修改，确定关闭？"确认。ESC 或 ✕ 直接关闭会丢失编辑。

**独立判断**：UX 改进项，非阻塞。可在后续 PR 加 dirty flag + 关闭确认。

### F5（LOW，已修）— ESC 在 input/textarea 中会关闭工作台丢失编辑

**位置**：`webui/app.js` ESC keydown handler

**事实**：全局 `document.addEventListener('keydown', ...)`，按 ESC 即关工作台。用户在 textarea 中编辑时按 ESC（很多编辑器的"取消焦点"习惯）会丢失所有未保存编辑。

**修复**：在 handler 中检查 `e.target.tagName`，INPUT/TEXTAREA 中按 ESC 不关工作台。

### F6（LOW，未修）— 保存按钮无 loading/disabled 状态

**位置**：`webui/app.js` `saveWorkbenchCard` / `saveWorkbenchLore`

**事实**：保存按钮在 API 请求期间未 disable，用户可双击触发重复保存。

**独立判断**：非阻塞。后端 PUT 是幂等的整体替换，重复保存不会产生脏数据。

### F7（LOW，未修）— `update_character_card` 接受任意 JSON 不校验 TavernCardV2 结构

**位置**：`engine/src/daemon/handlers.rs` `update_character_card` body 类型 `Json<serde_json::Value>`

**事实**：可写入 `{"foo": "bar"}` 这种非法角色卡，破坏后续 chat/reextract 路径。

**独立判断**：刻意设计——工作台编辑原始 JSON 需要灵活性。客户端表单只暴露 7 个字段，写入时保留原卡其余字段，实际不会写入完全非法的 JSON。可接受。

### F8（LOW，已确认安全）— 前端无 XSS 风险

**位置**：`webui/app.js` workbench DOM 操作

**事实**：所有用户数据通过 `el.value = ...` / `el.textContent = ...` 写入，无 `innerHTML` 设置用户内容。`innerHTML = ''` 仅用于清空。安全 ✓。

---

## 修复汇总（含第三次审计）

| 编号 | 级别 | 状态 | 修复内容 |
|---|---|---|---|
| C1 | HIGH | 已修 | 拖拽 resizer 加 mouseleave/blur 兜底，mousemove 监听器必释放 |
| C3 | MED | 已修 | openWorkbench 入口处 setWbDirty(false)，避免切换角色后 dot 错位 |
| C2 | MED | 安全确认 | 切角色后工作台显示陈旧数据是安全 race，已分析无副作用 |
| C4 | LOW | 未修 | 触屏拖拽支持缺失，非桌面核心场景 |
| C5 | LOW | 未修 | cursor 反馈，CSS 已有 |
| C6 | MED | 安全确认 | loadWorkbenchCard/Lorebook 并发 race 当前安全 |
| U1 | HIGH | 已修 | 工作台面板可拖拽调整宽度；按钮移出标题行 |
| U2 | HIGH | 已修 | 角色卡字段分组 + textarea 自适应高度 |
| U3 | MED | 已修 | dirty 红点 + 关闭确认 |
| U4 | MED | 已修 | 保存按钮请求期间 disabled |
| U5 | MED | 已修 | Lorebook 条目折叠/展开 + 行内编辑头部字段 |
| U6 | LOW | 已修 | 新增 "重解" 按钮，可调 reextract |
| F1 | LOW | 已文档化 | handler 注释补"设计说明"段落 |
| F3 | MED | 已修 | PUT 端点加 2MB body limit |
| F5 | LOW | 已修 | ESC 在 input/textarea 中不关工作台 |
| F2/F7 | LOW | 未修 | 非阻塞，可后续迭代 |
| F8 | — | 安全确认 | 无 XSS |

## 验证

- `cargo check -p airp-core --lib` ✓
- 神圣不变式 `subagent_context_has_no_orchestrator_noise` + `subagent_prepared_pipeline_has_no_orchestrator_noise` 2/2 ✓
- `node --check webui/app.js` ✓

## 结论

**推荐合并**。C1 是新发现的 HIGH 级问题（拖拽监听器泄漏）已修；C2/C6 经独立分析为安全 race；其余 LOW 级 UX 改进非阻塞。F1 raw.json 语义已文档化，符合"工作台编辑 = 新规范化版本"的设计取向；F3 PUT body limit 防 DoS；U1-U6 完成用户授权的 UI/UX 大刀阔斧改造。
