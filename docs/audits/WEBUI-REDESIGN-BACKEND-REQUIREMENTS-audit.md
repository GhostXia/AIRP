# WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md 独立审计报告

**审计源 LLM**：GLM-5.2
**审计日期**：2026-07-07
**审计范围**：`docs/WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md`（UI 重构后端对接需求报告）
**审计依据**：AGENTS.md 三条守则（独立审计 / 可提自己想法 / 对历史决策质疑）
**验证方式**：agent-browser + fetch() 同源实测（http://127.0.0.1:8000），辅以源码直读

## 引擎测试环境

- airp-core 0.1.0，端口 8000，无 `access_api_key`（settings.json 中未设）
- settings.json：`api_key=""`、`endpoint=https://api.openai.com/v1/chat/completions`、`model=gpt-4o`
- 既有角色：`听歌文模拟器` / `大乾风华录`（均无 card.json / avatar.png / lorebook.json / state/）
- 临时测试角色：`AuditTestChar`（导入后删除，已清理）

## 改动清单

| 文件 | 改动 |
|---|---|
| `docs/WEBUI-REDESIGN-BACKEND-REQUIREMENTS.md` | 修正 5 条 A 项阻塞错误 + 5 条 B 项描述错误，新增 A6 session API 割裂问题，重排 §9 实施顺序，新增 §10 审计追溯 |
| `docs/audits/WEBUI-REDESIGN-BACKEND-REQUIREMENTS-audit.md` | 本审计报告（含浏览器实测证据） |

---

## 路由存在性判别器

用 `%3A`（URL 编码的 `:`）作为 character_id。`CharacterId::new` 拒绝 `:` → 返回 400（handler 跑过）；路由不存在时 axum fallback 返回 404（空 body）。两者可区分。

| 路径 | 方法 | 状态 | body | 结论 |
|---|---|---|---|---|
| `/v1/characters/%3A` | GET | 400 | `{"error":{"code":"bad_request","message":"...ID 含非法字符 ':'..."}}` | ✅ 路由存在（A2） |
| `/v1/characters/%3A` | PUT | 400 | 同上 | ✅ 路由存在（A2） |
| `/v1/characters/%3A` | DELETE | 400 | 同上 | ✅ 路由存在（B4） |
| `/v1/characters/%3A/card` | GET | 404 | 空 | ✅ 路由不存在（A2） |
| `/v1/characters/%3A/meta` | GET | 404 | 空 | ✅ 路由不存在 |
| `/v1/characters/%3A/avatar` | GET | 400 | 空（StatusCode::BAD_REQUEST 直返） | ✅ 路由存在 |
| `/v1/characters/%3A/lorebook` | GET | 400 | envelope | ✅ 路由存在（A3） |
| `/v1/characters/%3A/lorebook` | PUT | 422 | `missing field 'entries'` | ✅ 路由存在（A3），body 解析已到 |
| `/v1/characters/%3A/state` | GET | 400 | 空 | ✅ 路由存在 |
| `/v1/characters/%3A/state/history` | GET | 400 | 空 | ✅ 路由存在 |
| `/v1/characters/%3A/state/schema` | GET | 400 | 空 | ✅ 路由存在（B4） |
| `/v1/characters/%3A/bogus_extra_seg` | GET | 404 | 空 | 对照：确实不存在的路由 |

---

## A 项（阻塞 — 错误结论会导致实施失败）

### A1 — `POST /v1/characters/import` 不接受 multipart

| 测试 | Content-Type | 状态 | 响应 |
|---|---|---|---|
| multipart FormData | multipart/form-data | **415** | `Expected request with 'Content-Type: application/json'` |
| JSON + card_path | application/json | 400 | `card_path 任意路径读已禁用（AIRP_ALLOW_LOCAL_PATH 未设）` |
| JSON + 有效 card_json | application/json | **200** | `{"character_id":"AuditTestChar","card_format":"json"}` |

**结论**：import **只接受 JSON body**，不接受 multipart。文档 §2.3/§6/§7 三处声称 multipart 全部错误。

**证据**：[handlers.rs:504-520](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L504-L520) — handler 签名 `Json(req): Json<ImportCharacterRequest>`，请求类型 [handlers.rs:39-49](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L39-L49)。Body limit 10 MiB（[mod.rs:200](file:///d:/AIRP-Dev/engine/src/daemon/mod.rs#L200)）。

**附加发现**：import handler 的错误消息本身写着 "Web/远端调用方请用 multipart 上传或 card_png_base64/card_json" — 这是误导性错误消息，因为 multipart 实际不被接受。建议改消息为 "请用 card_png_base64 或 card_json"。

### A2 — 角色卡 GET/PUT/DELETE 端点已存在

| 操作 | 路径 | 状态 | 响应 |
|---|---|---|---|
| GET 裸路径（有卡时） | `/v1/characters/AuditTestChar` | **200** | `{"data":{...},"spec":"chara_card_v2","spec_version":"2.0"}` |
| PUT 裸路径回写 | `/v1/characters/AuditTestChar` | **200** | `{"character_id":"AuditTestChar","status":"ok"}` |
| DELETE | `/v1/characters/AuditTestChar` | **200** | `{"deleted":"AuditTestChar","status":"ok"}` |
| GET /card 子路径 | `/v1/characters/:character_id/card` | **404**（axum fallback，空 body） | 路由不存在 |

**结论**：角色卡 CRUD 在裸路径 `/v1/characters/:character_id` 上完整存在（GET/PUT/DELETE）。文档 §4.1/§5 声称 "需要新增 GET/PUT /v1/characters/:character_id/card" 全部错误。

**证据**：[handlers.rs:783-792](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L783-L792) `get_character_card`、[handlers.rs:816-843](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L816-L843) `update_character_card`、[handlers.rs:796-806](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L796-L806) `delete_character_endpoint`；路由注册 [mod.rs:202-207](file:///d:/AIRP-Dev/engine/src/daemon/mod.rs#L202-L207)。

### A3 — 世界书 GET/PUT 端点已存在

| 操作 | 路径 | 状态 | 响应 |
|---|---|---|---|
| GET 无文件 | `/v1/characters/AuditTestChar/lorebook` | 404 | `{"error":{"code":"not_found","message":"...lorebook...not found"}}` |
| PUT 空 entries | `/v1/characters/AuditTestChar/lorebook` | **200** | `{"character_id":"AuditTestChar","entries_count":0,"status":"ok"}` |
| GET 写入后 | `/v1/characters/AuditTestChar/lorebook` | **200** | `{"entries":[]}` |

**结论**：lorebook GET/PUT 已存在。文档 §4.2/§5 声称 "需要新增" 错误。PUT 请求体需 `entries` 字段。

**证据**：[handlers.rs:847-898](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L847-L898) `get_character_lorebook` / `update_character_lorebook`；路由注册 [mod.rs:216-220](file:///d:/AIRP-Dev/engine/src/daemon/mod.rs#L216-L220)。

### A4 — P0 端点汇总表全部错误

文档 §5 原列 4 条 P0 端点 "当前 engine 不存在，建议新增"：

1. `GET /v1/characters/:character_id/card` — 实际不存在此子路径，但裸路径 `GET /v1/characters/:character_id` 已返回角色卡 JSON
2. `PUT /v1/characters/:character_id/card` — 实际 `PUT /v1/characters/:character_id` 已存在
3. `GET /v1/characters/:character_id/lorebook` — **已存在**
4. `PUT /v1/characters/:character_id/lorebook` — **已存在**

**结论**：4 条 P0 全部判错。真正的 P0 工作不是 "新增端点" 而是 "前端对接现有端点"。

### A5 — `access_api_key` 字段在 `SettingsView` 中不存在

`GET /v1/settings` 实测返回：
```json
{
  "provider": "OpenAI",
  "endpoint": "https://api.openai.com/v1/chat/completions",
  "api_key_set": false,
  "model": "gpt-4o",
  "volume_config": {...},
  "engine": "direct",
  "quota": {"max_requests_per_day":0, "max_tokens_per_day":0}
}
```

**字段清单**：`provider, endpoint, api_key_set, model, volume_config, engine, quota`
**无 `access_api_key` 字段**。文档 §2.4 "检查 `access_api_key` 字段" 无法实现。

**证据**：[mod.rs:69-92](file:///d:/AIRP-Dev/engine/src/daemon/mod.rs#L69-L92) — `SettingsView` 字段定义，`api_key_set` 是 bool（`cfg.api_key.as_deref().is_some_and(|s| !s.is_empty())`），不返回 key 字符串。`access_api_key` 在 `MutableConfig`（[mod.rs:59](file:///d:/AIRP-Dev/engine/src/daemon/mod.rs#L59)）中存在但**未在 HTTP 视图层暴露**。

**修复**：`SettingsView` 加 `access_api_key_set: bool` 字段，`from_config` 用 `cfg.access_api_key.as_deref().is_some_and(|s| !s.is_empty())` 填充。

### A6 — 多 session API 设计割裂（新发现，浏览器实测）

**这是浏览器实测才发现的新问题，原审计报告未覆盖，文档也未提及。**

#### 证据链

1. `POST /v1/sessions/AuditTestChar` 创建 session，返回 UUID_A = `48548c41-09a8-4f00-8d08-1a36a5efc337`
2. `POST /v1/chat/history {character_id:"AuditTestChar"}` 返回 ChatLog，其 `session_id` = UUID_B = `c28c1c43-fac1-4d72-956b-56b3ad025714` — **与 UUID_A 不同**
3. `POST /v1/chat/history {character_id, session_id: UUID_A}` → **422** `unknown field 'session_id', expected 'character_id'`（`HistoryQuery` 用 `#[serde(deny_unknown_fields)]`）
4. `POST /v1/chat/rollback {character_id, message_index, session_id}` → **422** `unknown field 'session_id'`
5. `POST /v1/chat/regen {character_id, session_id}` → **422** `unknown field 'session_id'`
6. `POST /v1/chat/completions {..., session_id}` → **422** `missing field 'variables'`（错误是 user_profile.variables 缺失，**不是** "unknown field session_id"）— 证明 `session_id` 被 chat/completions 接受

#### 源码根因

- `ChatCompletionRequest`（[types.rs:10-52](file:///d:/AIRP-Dev/engine/src/daemon/types.rs#L10-L52)）有 `session_id: Option<SessionId>` 字段 → chat/completions 可写入指定 session 路径 `characters/{id}/sessions/{session_id}/history/`
- `HistoryQuery` / `RollbackRequest` / `RegenRequest`（[types.rs:71-95](file:///d:/AIRP-Dev/engine/src/daemon/types.rs#L71-L95)）全部 `#[serde(deny_unknown_fields)]` 且**无 `session_id` 字段** → 只能操作 legacy per-character 路径 `characters/{id}/history/`
- `ChatLog::load_or_create`（[chat_store.rs:174](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L174)）调用 `load_or_create_for_session(data_root, character_id, None)` → 总是用 legacy 路径，不读 session 目录
- `load_or_create_for_session` 函数存在（[chat_store.rs:183](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L183)）但**仅被 agent/tools.rs 调用，无 HTTP 端点暴露**

#### 影响

文档 §3.1 描述的多 session UI 流程（"点击 session → 加载该 session 的聊天记录"）**在当前 API 下无法实现**：
- `POST /v1/sessions/:character_id` 创建的 session 目录会被 `POST /v1/chat/completions`（带 session_id）写入
- 但 `POST /v1/chat/history` 无法指定 session_id 来加载该 session 的历史 — 它只返回 legacy per-character log
- rollback/regen 同样无法操作指定 session

文档 §8 "Session 点击 → POST /v1/chat/history {character_id} → 渲染消息" 的调用链：无论点哪个 session，加载的都是同一个 legacy log。

#### 修复方案（二选一）

- **方案 1**（推荐，改动小）：在 `HistoryQuery` / `RollbackRequest` / `RegenRequest` 中加 `session_id: Option<SessionId>` 字段（保留 `deny_unknown_fields` 但显式列出 session_id）；handler 改用 `ChatLog::load_or_create_for_session(data_root, character_id, session_id.as_ref())`
- **方案 2**：新增 session-scoped 端点 `POST /v1/sessions/:character_id/:session_id/chat/history|rollback|regen`

---

## B 项（重要 — 影响设计质量，已修复）

### B1 — `api_path_prefix` 字段必要性存疑

engine 已有 `models_url_from_endpoint`（[handlers.rs:1090-1111](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L1090-L1111)）自动检测 `/v1/` 前缀。规则：路径含 `/v1/` → 截到 `/v1/` + `/models`；否则保守替换最后段为 `models`。

**建议**：除非有真实用户反馈无法连接某 provider，否则不必增加这个表面灵活实则易出错的开关。建议从 P2 标记 "暂不实现，待用户反馈"。

### B2 — "api_key 脱敏"描述不准确

文档原说 "只返回 `sk-...****`"，**实际返回 `api_key_set: bool`**，根本不返回 key 字符串（连脱敏字符串都不返回）。

### B3 — RR-001 `card_path` 门控条件理解错误

文档原："card_path 参数仅限 Tauri 本地 IPC 调用"。

**源码事实**：`card_path` 由环境变量 `AIRP_ALLOW_LOCAL_PATH=1` 启用（[handlers.rs:323-330](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L323-L330)），**与是否 Tauri 无关**。Tauri 走 HTTP 同样受这个环境变量门控。文档把"运行时环境变量门控"误描述为"调用方身份门控"。

### B4 — §6 现有端点不变确认表漏列与错误

错误项：
- "POST /v1/characters/import ✅ multipart" — 见 A1
- "GET /v1/settings ✅ api_key 脱敏" — 见 B2

漏列端点（文档完全未提，实测全部存在）：
- `DELETE /v1/characters/:character_id` — 工作台若支持"删除角色"需要
- `GET /v1/characters/:character_id/state/schema` — state schema 查询
- `GET/POST /v1/scenes` / `GET /v1/scenes/:scene_id` / `POST /v1/scenes/:scene_id/characters`
- `GET /v1/presets` / `GET /v1/presets/:preset_id`
- `GET /health` — 与 `/version` 同级无鉴权端点，诊断面板连通性检查应优先用此

### B5 — meta 扩展的设计取舍可优化

文档原建议 `GET /v1/characters/:character_id/meta` + `GET /v1/sessions/:character_id/detail`。

**独立建议**：
- 对角色列表：扩展 `GET /v1/characters` 返回 `[{id, name, description, avatar_url}]` 优于新增 `/meta`（避免 N+1 反模式，角色卡通常 <50 个，payload 可控）。需用 Accept header 或 `?with_meta=true` 做版本兼容。
- 对 session 列表：同样建议扩展现有端点而非新增 `/detail`。但更关键的是 session 当前**根本无 title 字段持久化** — `SessionId` 只是 UUID。光扩展返回结构不够，需要先在 `POST /v1/sessions/:character_id` 时支持传 title 并落盘到 session meta 文件（如 `session.json`），列表端点才能返回。文档 §3.1 提了"方案 B：传 title 参数"但没强调这是 A 的前置依赖。

---

## C 项（设计层面建议 — 独立提议，未在文档中体现）

### C1 — Tauri IPC 路径完全未讨论

项目记忆显示 PR #78/#82/#83 都在做 Tauri shell（`ui/src-tauri/src/bus.rs`）。Tauri 的 `invoke` IPC 可绕过 HTTP 直接调用 Rust 函数。文档完全围绕 HTTP API 设计，未讨论：
- 新 UI 是否在 Tauri 环境运行？如果是，许多"端点"可走 IPC 不必经 HTTP
- 如果同时要支持浏览器直连 + Tauri 包裹，需要明确双路径策略
- bus.rs 当前已实现 `settings.get`/`settings.update` intents（PR #78），文档应引用这些已有 IPC 通道而非重新设计 HTTP 端点

### C2 — `/version` vs `/health` 语义混淆

文档 §3.6 / §6 把 `/version` 当 health ping 用。实际 engine 有专门的 `/health`（[mod.rs:261, 288-307](file:///d:/AIRP-Dev/engine/src/daemon/mod.rs#L288-L307)），返回 `{"engine":"ok","provider_configured":bool,"data_root_writable":bool}`。诊断面板的"连通性检查"应优先用 `/health`，`/version` 只用于显示版本号。

### C3 — §9 推荐实施顺序完全失效

由于 A2/A3/A4（4 个 P0 端点已存在），整个 §9 实施顺序需要重排（已在文档中完成）。

---

## D 项（次要 — 文档表述问题，已修复）

- **D1**：路径参数文档统一写 `:id`，实际 axum 模式串是 `:character_id`。影响小但应准确。
- **D2**：§2.2 "从 card.json 解析" 表述过时，实际优先读 `card/card.json`，`card.json` 仅兼容回退（[handlers.rs:781-782](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L781-L782) 注释）。
- **D3**：§7 #2 "Bearer token 持久化：前端用 sessionStorage" 是 UI 行为，不应放在"后端实现时必须遵守"的安全约束里。
- **D4**：§3.5 "404 = 无 state，需区分处理" 准确，但未提 404 envelope 格式 `{"error":{"code":"not_found",...}}`（前端需要按 code 而非 HTTP 状态判断）。

---

## 测试覆盖矩阵

| 审计项 | 浏览器实测 | 证据 |
|---|---|---|
| A1 import 不接受 multipart | ✅ | 415 + 200 对比 |
| A2 角色卡 GET/PUT/DELETE 已存在 | ✅ | 路由判别器 + 实际 200 响应 |
| A3 世界书 GET/PUT 已存在 | ✅ | 路由判别器 + 实际 200 响应 |
| A4 P0 端点汇总全错 | ✅ | A2+A3 综合证据 |
| A5 access_api_key 字段不存在 | ✅ | GET /v1/settings 实际返回字段 |
| **A6（新）session API 割裂** | ✅ | session_id 不匹配 + 422 deny_unknown_fields + 源码 |
| B1 api_path_prefix 必要性存疑 | ✅ | 源码 models_url_from_endpoint |
| B2 api_key_set 是 bool | ✅ | GET /v1/settings 实际返回 |
| B3 card_path 环境变量门控 | ✅ | 400 + 源码 |
| B4 漏列端点 | ✅ | /health /scenes /presets /state/schema /DELETE 全部 200/400 |
| B5 session 列表无 meta | ✅ | POST+GET 实际返回结构 |

---

## 清理验证

测试结束后 `DELETE /v1/characters/AuditTestChar` 已清理。
`GET /v1/characters` 返回 `["听歌文模拟器","大乾风华录"]` — 与测试前一致。
`data/characters/AuditTestChar` 目录不存在。
无数据污染。

---

## 测试与证据说明

本次审计使用 agent-browser + fetch() 同源实测，辅以源码直读。关键证据文件：
- [engine/src/daemon/mod.rs](file:///d:/AIRP-Dev/engine/src/daemon/mod.rs) — 路由注册（172-265 行）、`SettingsView`（69-92 行）
- [engine/src/daemon/handlers.rs](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs) — 所有 handler 实现
- [engine/src/daemon/types.rs](file:///d:/AIRP-Dev/engine/src/daemon/types.rs) — 请求/响应类型
- [engine/src/chat_store.rs](file:///d:/AIRP-Dev/engine/src/chat_store.rs) — `ChatLog` 结构（30-51 行）、`load_or_create_for_session`（183 行）

浏览器实测原始日志（含完整请求/响应）见本地文件 `docs/audits/WEBUI-REDESIGN-BACKEND-REQUIREMENTS-verify.log`（注：`*.log` 被全局 gitignore 忽略，不入仓；如需归档可改扩展名）。

---

## 结论

| 项 | 数量 | 处置 |
|---|---|---|
| A 阻塞 | 6（含新发现 A6） | 已在文档中修正 |
| B 重要 | 5 | 已在文档中修正 |
| C 设计建议 | 3 | 可后续迭代 |
| D 表述 | 4 | 已在文档中修正 |

**结论**：文档原版本对 engine 现状的核实工作不充分，把 4 个已存在的端点判为"需要新增"，把 JSON body 端点描述为 multipart，把不存在的字段当作 UI 判断依据。经浏览器实测还发现了原审计未覆盖的 A6 session API 割裂问题。修正后的文档已可作为后端实施依据；C 项可后续迭代。
