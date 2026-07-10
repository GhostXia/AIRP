# WebUI 重构 — 后端对接需求报告

> **PR #88 设计输入，不是运行时完成证明**：设计稿曾落在 `airp-engine-console/`；未合并 PR #106 只部分迁移到 `webui/`，且仍有 CSS 损坏和 acceptance 缺口。当前结论见 [PROJECT-AUDIT-2026-07-10.md](PROJECT-AUDIT-2026-07-10.md) A-08 与 issue #105。

> 日期：2026-07-07
> 给：后端开发 Agent
> 由：UI 设计审计
> 目的：列出 UI 重构后需要后端支持的所有 API 调用、行为约定和新增需求

---

## 1. 背景

WebUI 已完成视觉重构，从原来的单页三栏 harness 改为 **3 页 SPA 式导航结构**：

1. **角色列表页** (`characters.html`) — LLM 配置 + 角色卡片列表 + 导入
2. **对话空间页** (`session.html`) — 角色信息 + session 列表 + chat + agent run + 诊断
3. **工作台页** (`workbench.html`) — 角色卡编辑 + 世界书编辑

设计稿位置：`airp-engine-console/` 目录（`.design` 项目 + 3 个 HTML 页面 + `colors_and_type.css`）。

旧 webui/ 的功能逻辑（`app.js` 1402 行）作为 API 调用参考实现，新 UI 的 JS 行为需基于相同端点重新编写。

---

## 2. 页面 1：角色列表页 — 后端需求

### 2.1 LLM 配置 "链接" 流程

**UI 行为**：
- 用户填入 API Endpoint + API Key
- 点击 "链接" 按钮
- 成功后常驻显示绿色状态 "链接成功"
- 自动拉取模型列表填充 Model 下拉框

**需要的后端 API**：

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 连接测试 | `/v1/models` | GET | 用用户填的 endpoint + key 代理请求上游 `/models`，成功 = 链接成功，返回模型列表填充下拉框 |
| 写入配置 | `/v1/settings` | POST | 将 endpoint / api_key / model / max_tokens 写入 engine settings，热重载 |

**关键细节**：

- API Endpoint 输入框**不含 `/v1/` 后缀**。用户输入 `http://127.0.0.1:8889`，engine 自动拼接 `/v1/` 前缀调用上游。UI placeholder 已标注此规则。
- 但 engine 需要支持**自定义 API 路径前缀**的设置项。因为不是所有 LLM 提供商都用 `/v1/`（虽然 OpenAI 兼容的都如此）。建议在 settings 中增加 `api_path_prefix` 字段，默认 `/v1/`。
- API Key 写入后，后续 GET `/v1/settings` 返回时 **不返回 key 本体**——当前引擎已实现：`SettingsView` 用 `api_key_set: bool` 字段表示是否已设置（`true`/`false`），**不返回** `sk-...****` 脱敏字符串。前端按布尔值判断即可。
- Model 下拉框：优先显示 `/v1/models` 返回的列表，同时支持手动输入（UI 有独立输入框）。手动输入的模型名优先级高于下拉选择。
- "链接" 按钮的 onclick 目前只是演示用的前端状态切换，实际实现需要调用 `/v1/models` + `/v1/settings` POST。

### 2.2 角色列表

**需要的后端 API**：

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 获取角色列表 | `/v1/characters` | GET | 返回 `string[]` 角色 slug 列表 |
| 获取角色详情 | `/v1/characters/:character_id` | GET | **已存在** — 返回完整 TavernCardV2 JSON。前端从中提取 `data.name` / `data.description`。但需 N+1 请求（每个角色一次），建议扩展列表端点返回 meta，见 §5 P1 |
| 获取角色头像 | `/v1/characters/:character_id/avatar` | GET | 返回 PNG bytes，UI 用 blob URL 渲染 |

**关键细节**：

- 当前 `/v1/characters` 只返回 slug 列表（`string[]`），不包含 name/description。但新 UI 的角色卡片需要显示 **角色名 + 描述 + 头像**。
- `GET /v1/characters/:character_id` 已存在（返回完整角色卡 JSON），前端可从中提取 name/description，但每个角色一次请求 = N+1。
- **建议扩展** `GET /v1/characters` 返回完整列表（含 meta），避免 N+1 请求。用 `?with_meta=true` query 参数做版本兼容（不传 = 旧行为 `string[]`，传 = 新行为 `[{id, name, description, avatar_url}]`）。
- 如果不改后端，前端 fallback：对每个 slug 调 `GET /v1/characters/:character_id` 取 name/description + 调 avatar 端点（显示首字母 fallback）。但这对用户体验不友好（卡片逐个加载）。

### 2.3 角色导入

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 导入角色卡 | `/v1/characters/import` | POST | **JSON body**（`Content-Type: application/json`），不接受 multipart。Body 字段：`character_id?` / `card_path?` / `card_json?` / `card_png_base64?`。Body limit 10 MiB。 |

**UI 行为**：拖放或点击上传 PNG/JSON/V2 卡片文件。前端读取文件后：
- PNG 文件 → `FileReader.readAsDataURL` 或 `arrayBuffer` → base64 编码 → 填入 `card_png_base64` 字段
- JSON 文件 → `FileReader.readAsText` → 填入 `card_json` 字段
- 组装成 JSON body POST 到 `/v1/characters/import`

**永禁 `card_path` 参数**（RR-001）：`card_path` 由 engine 启动时的 `AIRP_ALLOW_LOCAL_PATH=1` 环境变量门控，未设时 handler 返回 400。Web/远端调用方即使伪造 JSON body 带 `card_path` 也被拒。**门控条件是环境变量，不是"调用方是否 Tauri"**——Tauri sidecar 启动脚本带此变量，对外暴露的 engine 不带。

### 2.4 无鉴权警告

- UI 检测到 engine 无 `access_api_key` 配置时，显示黄色警告条 "Engine 未配置 API Key，仅本地开发可用"。
- **当前后端缺口**：`GET /v1/settings` 返回的 `SettingsView` 字段为 `provider / endpoint / api_key_set / model / volume_config / engine / quota`，**不包含 `access_api_key` 相关字段**（`api_key_set` 只反映上游 provider 的 `api_key`，不反映 daemon 自鉴权的 `access_api_key`）。
- **需要后端新增**：在 `SettingsView` 中增加 `access_api_key_set: bool` 字段（脱敏，同 `api_key_set` 模式），前端检查此字段。详见 §5 P0 端点表。

---

## 3. 页面 2：对话空间页 — 后端需求

### 3.1 Session 管理

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 获取 session 列表 | `/v1/sessions/:character_id` | GET | 返回 `SessionId[]`（UUID 字符串数组，无 title/last_message/updated_at） |
| 创建 session | `/v1/sessions/:character_id` | POST | 返回新 `SessionId`（裸 UUID 字符串，非对象） |
| 获取历史消息 | `/v1/chat/history` | POST | `{character_id}` → `ChatLog`（**当前不支持 session_id 参数**，见下方 A6） |

**UI 行为**：
- 点击角色卡片 → 进入对话空间，左栏显示 session 列表
- "新建对话" → POST 创建 session → 自动切换到新 session
- 点击已有 session → 加载该 session 的聊天记录

**关键细节**：

- 当前 `/v1/sessions/:character_id` 返回 `SessionId[]`（只有 UUID），**不包含 session 标题或最后消息预览**。
- 新 UI 的 session 列表需要显示：session 标题（描述性名称）、最后一条消息预览、时间戳。
- **建议**（A6 修复后方案 C 成本最低，推荐先用 C，后续按需升级）：
  - **方案 C（推荐，A6 修复后可行）**：前端从 `POST /v1/chat/history {character_id, session_id}` 推导 title（取第一条用户消息前 30 字符）。A6 修复前不可行（history 不能指定 session_id）；修复后零后端工作量。
  - 方案 A：新增 `GET /v1/sessions/:character_id/detail` 返回每个 session 的 title + last_message + updated_at。需后端新增端点。
  - 方案 B：在 `POST /v1/sessions/:character_id` 时支持传入 title 参数（需后端持久化 session meta，当前 `SessionId` 只是 UUID，无 title 字段）。是方案 A 的前置依赖。
  - 推荐路径：先用方案 C（零后端工作量），若 UX 不佳再考虑 A+B 组合。

#### A6 — 多 session API 设计割裂（P0 阻塞，浏览器实测发现）

**这是当前后端的根本性缺口，必须在多 session UI 实现前修复。**

**实测证据**（agent-browser + fetch，2026-07-07）：
1. `POST /v1/sessions/AuditTestChar` 创建 session，返回 UUID_A
2. `POST /v1/chat/history {character_id}` 返回的 ChatLog `session_id` 是 UUID_B — **与 UUID_A 不同**
3. `POST /v1/chat/history {character_id, session_id}` → **422** `unknown field 'session_id', expected 'character_id'`
4. `POST /v1/chat/rollback {character_id, message_index, session_id}` → **422** `unknown field 'session_id'`
5. `POST /v1/chat/regen {character_id, session_id}` → **422** `unknown field 'session_id'`
6. `POST /v1/chat/completions {..., session_id}` → 422 是 `missing field 'variables'`（**不是** unknown field）— 证明 `chat/completions` 接受 `session_id`

**根因**：
- `ChatCompletionRequest` 有 `session_id: Option<SessionId>` 字段 → chat/completions 可写入指定 session 路径 `characters/{id}/sessions/{session_id}/history/`
- `HistoryQuery` / `RollbackRequest` / `RegenRequest` 全部 `#[serde(deny_unknown_fields)]` 且**无 `session_id` 字段** → 只能操作 legacy per-character 路径 `characters/{id}/history/`
- `ChatLog::load_or_create_for_session` 函数存在但**仅被 agent/tools.rs 调用，无 HTTP 端点暴露**

**影响**：UI "点击 session → 加载该 session 历史" 流程在当前 API 下无法实现——无论点哪个 session，`chat/history` 加载的都是同一个 legacy per-character log。

**修复方案**（二选一）：
- **方案 1**（推荐，改动小）：在 `HistoryQuery` / `RollbackRequest` / `RegenRequest` 中加 `session_id: Option<SessionId>` 字段（保留 `deny_unknown_fields` 但显式列出 session_id）；handler 改用 `ChatLog::load_or_create_for_session(data_root, character_id, session_id.as_ref())`
- **方案 2**：新增 session-scoped 端点 `POST /v1/sessions/:character_id/:session_id/chat/history|rollback|regen`

详见审计报告 `docs/audits/WEBUI-REDESIGN-BACKEND-REQUIREMENTS-audit.md` A6 节。

### 3.2 Chat Streaming

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 发送消息 | `/v1/chat/completions` | POST | SSE 流式返回 `data: {text}` |

**UI 行为**：
- 用户输入消息 → POST chat/completions
- 流式 token 逐个追加到消息气泡（用 textContent 流式追加，完成后切 innerHTML 做 markdown 渲染）
- streaming 期间显示 "停止" 按钮，调用 `AbortController.abort()`
- 支持 Ctrl+Enter 发送

**与旧 webui 一致**，无新增后端需求。

### 3.3 Chat 操作

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 刷新历史 | `/v1/chat/history` | POST | `{character_id}` |
| Regenerate | `/v1/chat/regen` | POST | `{character_id}` |
| Rollback | `/v1/chat/rollback` | POST | `{character_id, message_index}` |

**无新增后端需求**，UI 行为与旧 webui 一致。

### 3.4 Agent Run

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 运行 Agent | `/v1/agent/run` | POST | SSE 返回 `AgentEvent` |

**UI 行为**：
- 输入 agent 指令 + max_steps
- SSE 事件按类型颜色编码显示：plan(amber) / tool_call(blue) / tool_result(green) / delta(gray) / done(purple)
- 二次点击 Run 时先 abort 前一个请求

**无新增后端需求**。

### 3.5 State / State History

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 获取 live state | `/v1/characters/:character_id/state` | GET | 404 = 无 state，需区分处理 |
| 获取 state history | `/v1/characters/:character_id/state/history` | GET | `?limit=N` clamp 1..1000 |

**无新增后端需求**。

### 3.6 诊断

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| Engine 版本 | `/version` | GET | 无鉴权 |
| Settings | `/v1/settings` | GET | |
| Models | `/v1/models` | GET | |

**无新增后端需求**，UI 行为与旧 webui 一致。

---

## 4. 页面 3：工作台 — 后端需求

### 4.1 角色卡编辑

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 读取角色卡 | `/v1/characters/:character_id` | GET | **已存在** — 返回 TavernCardV2 JSON（`serde_json::Value`，优先读 `card/card.json`，兼容旧 `card.json`） |
| 保存角色卡 | `/v1/characters/:character_id` | PUT | **已存在** — 接收角色卡 JSON 写回。响应 `{"character_id":"...","status":"ok"}` |
| 删除角色 | `/v1/characters/:character_id` | DELETE | **已存在** — 响应 `{"deleted":"...","status":"ok"}` |
| 重新提取 | `/v1/characters/:character_id/reextract` | POST | 已存在 — 触发 CF-7 资产解包 |

**关键细节**：

- 旧 webui 的 workbench 通过 GET engine 内部文件路径读 card.json，然后 PUT 写回。新 UI **不应**依赖文件路径，直接用 `GET/PUT /v1/characters/:character_id` 即可。
- 路径参数名是 `:character_id`（不是 `:id`），axum 路由模式串为 `/v1/characters/:character_id`。
- **不存在** `/v1/characters/:character_id/card` 子路径——卡片 CRUD 在裸路径上。前端若误调 `/card` 会收到 axum fallback 的 404（空 body）。
- PUT 请求体是完整的 TavernCardV2 JSON（`{spec, spec_version, data:{...}}`）。
- 实测：导入测试角色后 GET 返回 `{"data":{...},"spec":"chara_card_v2","spec_version":"2.0"}`；PUT 回写返回 `{"character_id":"...","status":"ok"}`；DELETE 返回 `{"deleted":"...","status":"ok"}`。

### 4.2 世界书编辑

| 操作 | 端点 | Method | 说明 |
|------|------|--------|------|
| 读取世界书 | `/v1/characters/:character_id/lorebook` | GET | **已存在** — 返回 lorebook JSON（`{entries:[...]}`）。文件不存在时 404 envelope `{"error":{"code":"not_found",...}}` |
| 保存世界书 | `/v1/characters/:character_id/lorebook` | PUT | **已存在** — 请求体需 `entries` 字段（缺失时 422 `missing field 'entries'`）。响应 `{"character_id":"...","entries_count":N,"status":"ok"}` |

**关键细节**：

- 旧 webui 读写 lorebook 的逻辑直接操作文件。新 UI 通过 `GET/PUT /v1/characters/:character_id/lorebook` API。
- UI 当前覆盖的字段：keys / content / priority / enabled / comment
- 后续需要覆盖的字段（按 TAVERN-PARITY.md）：secondary_keys / position / depth / order / probability / selective / constant / sticky / cooldown / delay / recursive / group
- 实测：PUT `{entries:[]}` 后 GET 返回 `{entries:[]}`；空状态 GET 返回 404 envelope（**不是**空对象 `{}`，前端需按 `error.code === "not_found"` 区分）。

### 4.3 Dirty 追踪 + ESC 关闭

纯前端行为，不需要后端支持。但 "保存" 按钮需要调用 PUT 端点。

---

## 5. 新增端点汇总

**修订说明**（2026-07-07 审计后）：原版本列出 4 条 P0 端点（角色卡 GET/PUT + 世界书 GET/PUT）声称"engine 不存在"，经浏览器实测全部已存在（见 §4.1/§4.2）。真正的 P0 工作是下表两项。

| 优先级 | 端点 / 改动 | Method | 说明 | 依赖 |
|--------|------|--------|------|------|
| **P0** | `chat/history` / `chat/rollback` / `chat/regen` 加 `session_id` 字段 | POST (扩展) | **A6 阻塞**：当前 `HistoryQuery`/`RollbackRequest`/`RegenRequest` 用 `deny_unknown_fields` 且无 `session_id`，无法操作指定 session。多 session UI 必需。 | `types.rs` 加字段 + handler 改用 `load_or_create_for_session` |
| **P0** | `SettingsView` 加 `access_api_key_set: bool` | GET `/v1/settings` (扩展) | **A5 阻塞**：当前 `SettingsView` 无 `access_api_key` 相关字段，无鉴权警告 UI 无判断依据。 | `mod.rs` `SettingsView` 加字段 + `from_config` 填充 |
| P1 | `/v1/characters` | GET (扩展) | 返回含 meta 的角色列表（name/description），而非仅 slug[]。避免 N+1。 | 遍历 card.json |
| P1 | `/v1/sessions/:character_id` | GET (扩展) | 返回含 title/last_message/updated_at 的 session 详情。依赖 session title 持久化前置。 | chat log 遍历或 session meta 文件 |
| P1 | `POST /v1/sessions/:character_id` 支持 `title` 参数 | POST (扩展) | 创建 session 时持久化 title 到 session meta（当前 `SessionId` 只是 UUID，无 title）。P1 session 列表扩展的前置依赖。 | session meta 文件 schema |
| P2 | settings 新增 `api_path_prefix` | POST `/v1/settings` | 允许自定义 API 路径前缀（默认 `/v1/`）。**审计建议暂不实现**：engine 已有 `models_url_from_endpoint` 自动检测 `/v1/` 前缀，未收到真实用户反馈前不必增加此开关。 | settings.json schema |

**P0 端点**：A6 session API 割裂 + A5 access_api_key_set 字段。这两项不修复，多 session UI 和无鉴权警告功能无法实现。

**P1 端点**：角色列表和 session 列表的 meta 扩展提升 UX（避免 N+1 请求和不友好的 slug-only 显示），但有 fallback 方案（前端逐个请求或从 history 推导）。

**P2 端点**：API 路径前缀是边缘情况优化，且与既有自动推导逻辑语义重叠，建议暂缓。

**已删除的伪 P0**（原版本错误列出，实测已存在）：~~`GET/PUT /v1/characters/:character_id/card`~~、~~`GET/PUT /v1/characters/:character_id/lorebook`~~。前端直接用裸路径 `GET/PUT /v1/characters/:character_id` 和 `GET/PUT /v1/characters/:character_id/lorebook` 即可。

---

## 6. 现有端点不变确认

以下端点的行为**无需修改**，新 UI 直接复用旧 webui 的调用模式（已浏览器实测确认 2026-07-07）：

| 端点 | 确认 |
|------|------|
| `GET /version` | ✅ 无鉴权，返回 `{"name":"airp-core","version":"0.1.0"}` |
| `GET /health` | ✅ 无鉴权，返回 `{"engine":"ok","provider_configured":bool,"data_root_writable":bool}`。诊断面板的连通性检查应优先用此端点 |
| `POST /v1/chat/completions` | ✅ SSE streaming，**支持 `session_id` 字段**写入指定 session 路径 |
| `POST /v1/agent/run` | ✅ SSE events，同旧实现 |
| `POST /v1/chat/history` | ⚠️ **需扩展**（A6）—— 当前 `deny_unknown_fields` 不接受 `session_id`，只能操作 legacy per-character log |
| `POST /v1/chat/rollback` | ⚠️ **需扩展**（A6）—— 同上 |
| `POST /v1/chat/regen` | ⚠️ **需扩展**（A6）—— 同上 |
| `POST /v1/characters/import` | ✅ **JSON body**（非 multipart），永禁 `card_path`（环境变量门控）。详见 §2.3 |
| `POST /v1/characters/:character_id/reextract` | ✅ 触发 CF-7 资产解包 |
| `GET /v1/characters/:character_id` | ✅ 返回角色卡 JSON（裸路径，非 `/card` 子路径） |
| `PUT /v1/characters/:character_id` | ✅ 写回角色卡 JSON |
| `DELETE /v1/characters/:character_id` | ✅ 删除角色 |
| `GET /v1/characters/:character_id/avatar` | ✅ 返回 PNG bytes，`Content-Type: image/png`，文件缺失 404（空 body） |
| `GET /v1/characters/:character_id/lorebook` | ✅ 返回 lorebook JSON，文件缺失 404 envelope |
| `PUT /v1/characters/:character_id/lorebook` | ✅ 写回 lorebook，请求体需 `entries` 字段 |
| `GET /v1/characters/:character_id/state` | ✅ 404 区分（空 body，非 envelope） |
| `GET /v1/characters/:character_id/state/history` | ✅ `?limit=N` clamp 1..=1000，倒序 |
| `GET /v1/characters/:character_id/state/schema` | ✅ 路由存在（文档原版本漏列） |
| `GET /v1/models` | ✅ 上游代理，用配置的 `api_key` 加 `Authorization: Bearer` 转发 |
| `GET /v1/settings` | ✅ 返回 `api_key_set: bool`（**非** `sk-...****` 脱敏字符串）；**需扩展** 加 `access_api_key_set`（A5） |
| `POST /v1/settings` | ✅ 热重载，落盘 `data/settings.json`（含明文 api_key） |
| `GET/POST /v1/sessions/:character_id` | ✅ 基础 list/create，返回 UUID 字符串数组 / 裸 UUID 字符串 |
| `GET/POST /v1/scenes` / `GET /v1/scenes/:scene_id` / `POST /v1/scenes/:scene_id/characters` | ✅ 场景管理（文档原版本漏列） |
| `GET /v1/presets` / `GET /v1/presets/:preset_id` | ✅ 预设查询（文档原版本漏列） |

---

## 7. 安全约束（不可违反）

以下约束从 AGENTS.md + WEBUI-BACKEND-PLAN 继承，后端实现时必须遵守：

1. **RR-001 永禁 `card_path`（Web/远端）**：浏览器端所有导入操作走 **JSON body**（`card_png_base64` 或 `card_json` 字段），不走文件路径。后端 `/v1/characters/import` 的 `card_path` 参数由 `AIRP_ALLOW_LOCAL_PATH=1` 环境变量门控——**门控条件是环境变量，不是"调用方是否 Tauri"**。Tauri sidecar 启动脚本带此变量，对外暴露的 engine 不带；未设时 handler 返回 400。
2. **Bearer token 持久化**：前端用 sessionStorage（关 tab 即清），后端不提供 token 存储端点。（此为前端行为，列此仅作约定。）
3. **API Key 脱敏**：`GET /v1/settings` 返回的 `SettingsView` 用 `api_key_set: bool` 表示是否已设置，**不返回 key 本体**（连脱敏字符串都不返回）。`access_api_key` 当前未在 `SettingsView` 暴露（A5 需扩展为 `access_api_key_set: bool`）。落盘的 `data/settings.json` **含明文 api_key**（非脱敏）。
4. **CORS**：当前 `Any`，如需对外暴露需收紧。
5. **avatar blob URL**：前端用 `URL.createObjectURL` + `revokeObjectURL` 管理，后端只负责返回 bytes。
6. **import 错误消息误导**：当前 `card_path` 被拒时返回的错误消息写 "请用 multipart 上传或 card_png_base64/card_json"，但 multipart 实际不被接受（`Json` extractor 要求 `application/json`）。后端应修消息为 "请用 card_png_base64 或 card_json"。

---

## 8. UI → 后端 行为映射（给实现者的速查表）

| UI 按钮/操作 | 后端调用链 |
|-------------|-----------|
| "链接" 按钮 | `GET /v1/models`（测试连通 + 获取模型列表）→ `POST /v1/settings`（写入 endpoint/key/model） |
| 角色卡片点击 | `GET /v1/characters`（列表）→ `GET /v1/characters/:character_id`（取角色卡，从中读 name/description）或等 §5 P1 meta 扩展 |
| "导入" 上传 | 前端读文件 → base64/text → `POST /v1/characters/import`（**JSON body**，`card_png_base64` 或 `card_json` 字段） |
| "新建对话" | `POST /v1/sessions/:character_id` → 刷新 session 列表 |
| Session 点击 | `POST /v1/chat/history {character_id, session_id}` → 渲染消息（**需 A6 修复后**；当前只能 `{character_id}` 读 legacy log） |
| "发送" | `POST /v1/chat/completions`（SSE streaming，支持 `session_id` 字段） |
| "停止" | `AbortController.abort()`（纯前端） |
| "刷新历史" | `POST /v1/chat/history {character_id, session_id}`（需 A6） |
| "Regenerate" | `POST /v1/chat/regen {character_id, session_id}`（需 A6） |
| "Rollback" | `POST /v1/chat/rollback {character_id, message_index, session_id}`（需 A6） |
| "重解" | `POST /v1/characters/:character_id/reextract` + `confirm()` |
| "工作台" | `GET /v1/characters/:character_id` → 渲染编辑表单（**裸路径**，非 `/card`） |
| "保存角色卡" | `PUT /v1/characters/:character_id`（JSON body，完整 TavernCardV2） |
| "删除角色" | `DELETE /v1/characters/:character_id` |
| 世界书 tab | `GET /v1/characters/:character_id/lorebook` → 渲染条目列表（404 envelope = 空） |
| "保存世界书" | `PUT /v1/characters/:character_id/lorebook`（JSON body，需 `entries` 字段） |
| Agent "Run" | `POST /v1/agent/run`（SSE events） |
| "诊断" | 优先 `GET /health`（连通性 + 数据目录可写）+ `GET /version`（版本）+ `GET /v1/settings` + `GET /v1/models` |
| 无鉴权警告 | `GET /v1/settings` → 检查 `access_api_key_set`（**需 A5 修复后**；当前 `SettingsView` 无此字段） |

---

## 9. 推荐实施顺序

**修订说明**（2026-07-07 审计后）：原版本前列 4 条 P0 端点经实测已存在，整个顺序需重排。

1. **P0-A**（后端）：`chat/history` / `rollback` / `regen` 加 `session_id` 字段（A6） — 多 session UI 的前置
2. **P0-B**（后端）：`SettingsView` 加 `access_api_key_set: bool`（A5） — 无鉴权警告 UI 的前置
3. **P0-C**（前端）：工作台直接对接现有 `GET/PUT /v1/characters/:character_id` + `GET/PUT /v1/characters/:character_id/lorebook`（零后端工作量）
4. **P0-D**（前端）：修正 import 调用为 JSON body + base64（非 multipart）
5. **P1-A**（后端）：`GET /v1/characters` 扩展返回 meta — 角色列表页面基础
6. **P1-B**（后端）：`POST /v1/sessions/:character_id` 支持 `title` + `GET` 扩展返回 detail — session 列表页面基础
7. **P2**（暂缓）：settings `api_path_prefix` — 已有自动推导逻辑，待用户反馈
8. 其余端点保持不变，前端 JS 行为参照旧 webui/app.js 重新编写

---

## 10. 审计追溯

本文档于 2026-07-07 经 GLM-5.2 独立审计 + agent-browser 浏览器实测，修正了 5 条 A 项阻塞错误（A1-A5）并发现 1 条新阻塞（A6）。完整审计报告与实测证据见 [`docs/audits/WEBUI-REDESIGN-BACKEND-REQUIREMENTS-audit.md`](audits/WEBUI-REDESIGN-BACKEND-REQUIREMENTS-audit.md)。
