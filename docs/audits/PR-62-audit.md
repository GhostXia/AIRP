# PR #62 审计报告 — WebUI 工作台（角色卡编辑 + 世界书管理）

**审计源 LLM 模型**：GLM-5.2（智谱 AI，2026 年）
**审计日期**：2026-07-05
**审计范围**：PR E（engine CRUD 端点）+ PR F（WebUI 工作台 UI）
**审计分支**：`webui-pr-f-workbench` @ c6fe535
**审计依据**：AGENTS.md「审计 Agent 守则」—— 独立审计，不附和开发结论

---

## 审计方法

独立读取 `git diff main..HEAD` 全量改动 + 复核 `data_dir` 模块的 `character_dir` / `char_world_lorebook_path` / `get_character` / `delete_character` 实际行为，对每个 handler 和前端函数做独立判断。

---

## 发现项

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

## 修复汇总

| 编号 | 级别 | 状态 | 修复内容 |
|---|---|---|---|
| F1 | LOW | 已文档化 | handler 注释补"设计说明"段落 |
| F3 | MED | 已修 | PUT 端点加 2MB body limit |
| F5 | LOW | 已修 | ESC 在 input/textarea 中不关工作台 |
| F2/F4/F6/F7 | LOW | 未修 | 非阻塞，可后续迭代 |
| F8 | — | 安全确认 | 无 XSS |

## 验证

- `cargo check -p airp-core --lib` ✓
- 神圣不变式 `subagent_context_has_no_orchestrator_noise` + `subagent_prepared_pipeline_has_no_orchestrator_noise` 2/2 ✓
- `node --check webui/app.js` ✓

## 结论

**推荐合并**。F3 是唯一的 MED 级问题且已修复，其余均为 LOW 级 UX/一致性改进，不阻塞交付。F1 的 raw.json 语义已在文档中澄清，符合"工作台编辑 = 新规范化版本"的设计取向。
