# PR #85 二次审计报告

**审计源 LLM**：Kimi-K2.7-Code  
**审计日期**：2026-07-07  
**审计对象**：commit `b3b29ad` —— `fix(engine): A5+A6 修复后端阻塞项（session API 割裂 + access_api_key_set）`  
**审计依据**：AGENTS.md 审计 agent 守则（独立审计 / 可提出自己想法 / 质疑历史决策）  
**验证方式**：源码直读 + 全量测试 + agent-browser fetch 实测

---

## 1. 审计范围

本次二次审计针对 PR #85 已合并到 `main` 的 commit `b3b29ad`，独立审查其对 PR #84 审计报告所列 A5/A6 阻塞项的修复实现。

涉及文件：
- `engine/src/daemon/types.rs` — 请求类型加字段
- `engine/src/daemon/handlers.rs` — handler 实现与错误消息
- `engine/src/daemon/mod.rs` — `SettingsView` 与新增测试

---

## 2. 改动审查

### 2.1 A6 — 多 session API 割裂修复

**问题回顾**：`POST /v1/chat/completions` 已支持 `session_id`，但 `chat/history`、`chat/rollback`、`chat/regen` 三个端点因 `#[serde(deny_unknown_fields)]` 且类型无 `session_id` 字段，会返回 422 `unknown field 'session_id'`，导致 UI 无法读取指定 session 的历史。

**实现审查**：
- `HistoryQuery`、`RollbackRequest`、`RegenRequest` 均新增 `pub session_id: Option<SessionId>` 字段，保留 `#[serde(deny_unknown_fields)]`。
- `get_chat_history`、`rollback_chat`、`regen_chat` 三个 handler 全部从 `ChatLog::load_or_create` 改为 `ChatLog::load_or_create_for_session(..., req.session_id.as_ref())`。
- `session_id` 为 `None` 时回退到 legacy per-character 路径，旧客户端零迁移。

**独立判断**：修复方案与 PR #84 审计推荐的"方案 1"一致，改动面最小，充分利用了 `chat_store.rs` 中已有的 `load_or_create_for_session` 基础设施。实现正确。

### 2.2 A5 — `SettingsView` 无 `access_api_key` 相关字段

**问题回顾**：无鉴权警告 UI 需要判断 daemon 是否配置了自鉴权 key，但 `GET /v1/settings` 仅返回 `api_key_set`（上游 provider key），不反映 `access_api_key`。

**实现审查**：
- `SettingsView` 新增 `pub access_api_key_set: bool` 字段。
- `from_config` 用 `cfg.access_api_key.as_deref().is_some_and(|s| !s.is_empty())` 填充。
- 仅返回 bool，不返回 key 本体，脱敏模式与 `api_key_set` 一致。

**独立判断**：实现正确，符合 RR-001 与安全约束。

### 2.3 附带修复 — import 错误消息误导

**问题回顾**：`card_path` 被禁用时错误消息写"请用 multipart 上传"，但 `/v1/characters/import` 实际只接受 JSON body，multipart 会返回 415。

**实现审查**：错误消息已改为 `"card_path 任意路径读已禁用（AIRP_ALLOW_LOCAL_PATH 未设）。Web/远端调用方请用 card_png_base64 或 card_json 字段（JSON body，非 multipart）。"`。

**独立判断**：消息准确，消除了误导。

---

## 3. 测试验证

### 3.1 新增 A5/A6 测试

PR #85 新增 7 个测试：
- `test_a5_settings_exposes_access_api_key_set_false_when_none`
- `test_a5_settings_exposes_access_api_key_set_true_when_set`
- `test_a6_chat_history_accepts_session_id_field`
- `test_a6_chat_history_session_scoped_vs_legacy_diverge`
- `test_a6_chat_rollback_accepts_session_id`
- `test_a6_chat_regen_accepts_session_id`
- `test_a6_chat_history_without_session_id_still_works`

### 3.2 测试结果

| 测试集合 | 数量 | 结果 |
|---|---|---|
| lib tests | 360 | 全部通过 |
| `tests/agent_run.rs` | 3 | 全部通过 |
| `tests/openai_compat.rs` | 11 | 全部通过 |
| `tests/sse_wiremock.rs` | 5 | 全部通过 |
| **合计** | **379** | **0 失败，1 ignored** |

A5/A6 专项 7 个测试全部通过。

> 注：commit 消息自称 "360 lib + 5 state_protocol = 365"，与实际测试目标不符（不存在 `state_protocol` 测试目标，实际为 3+11+5=19 个集成测试）。此为 commit message 的小瑕疵，不影响代码正确性。

---

## 4. 浏览器实测

使用 agent-browser 对本地 engine（`http://127.0.0.1:8000`）执行同源 fetch 验证：

| # | Method | Path | Status | 关键结论 |
|---|--------|------|--------|----------|
| 1 | GET | `/version` | 200 | engine 存活 |
| 2 | GET | `/v1/settings` | 200 | 含 `access_api_key_set: false` |
| 3 | POST | `/v1/sessions/AuditTestChar` | 200 | 返回裸 UUID 字符串 |
| 4 | POST | `/v1/chat/history`（带 `session_id`） | 200 | A6 修复：不再 422 |
| 5 | POST | `/v1/chat/history`（无 `session_id`） | 200 | legacy 兼容 |
| 6 | POST | `/v1/chat/rollback`（带 `session_id`） | 200 | A6 修复 |
| 7 | POST | `/v1/chat/regen`（带 `session_id`） | 200 | A6 修复 |
| 8 | POST | `/v1/characters/import`（`card_path`） | 400 | 错误消息含 "JSON body"，不含 "multipart" |
| 9 | DELETE | `/v1/characters/AuditTestChar` | 200 | 清理成功 |

实测结论：**A5/A6 修复均生效**。

---

## 5. 独立发现的观察项（非阻塞）

以下项不属于 PR #85 引入的问题，但属于 A6 相关设计，可作为后续迭代参考：

### O1 — `ChatLog.session_id` 与 scope session_id 不一致

浏览器实测显示：
- `POST /v1/sessions/AuditTestChar` 返回 scope session_id = `28d96f9f-...`
- 用该 scope id 调 `POST /v1/chat/history` 返回的 `ChatLog.session_id` = `14508e49-...`（不同 UUID）

原因：`ChatLog.session_id` 是内部 UUID（`ChatLog::new` 生成），而 scope session_id 只用于目录命名（`scope_session_id` 字段被 `#[serde(skip)]`）。HTTP 响应只暴露内部 `session_id`。

影响：UI 拿到 history 后，若用响应中的 `session_id` 与 session 列表做关联，会出现不匹配。前端应使用调用 `/v1/sessions/:character_id` 时得到的 scope session_id 作为 session 标识，而非 `ChatLog.session_id`。

建议：可在文档或 API 注释中明确这一约定；若未来需要，可在 `ChatLog` 序列化时额外暴露 `scope_session_id` 以减少前端困惑。

> 已按审计遗留项处理规则创建 issue #86 跟踪。

### O2 — commit message 测试计数不准确

commit message 写 "全量 360 lib + 5 state_protocol = 365 测试通过"，实际集成测试目标为 `agent_run`/`openai_compat`/`sse_wiremock`（共 19 个），总计 379 个测试。建议后续 commit message 与实际测试目标保持一致。

---

## 6. 审计结论

| 项 | 状态 | 说明 |
|---|---|---|
| A5 修复 | ✅ 通过 | `SettingsView` 正确暴露 `access_api_key_set: bool`，不泄露 key 本体 |
| A6 修复 | ✅ 通过 | history/rollback/regen 均支持 `session_id`，legacy 路径兼容 |
| import 错误消息 | ✅ 通过 | 已修正为 JSON body 提示 |
| 新增测试 | ✅ 通过 | 7 个专项测试覆盖 A5/A6 行为 |
| 全量测试 | ✅ 通过 | 379 测试 0 失败 |
| 浏览器实测 | ✅ 通过 | A5/A6 行为与错误消息均符合预期 |

**总体结论**：PR #85 修复实现正确、测试覆盖充分、向后兼容无破坏。**建议保持已合并状态，无需回滚**。

O1/O2 为非阻塞观察项，可在后续 P1/P2 迭代中按需处理。O1 已创建 issue #86 跟踪。
