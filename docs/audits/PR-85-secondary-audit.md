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

---

## 7. O1 commit (73eeb1a) 补充审计

**审计日期**：2026-07-07
**审计对象**：commit `73eeb1a` —— `fix(engine): O1 ChatLog.scope_session_id 在 HTTP 响应中暴露 (#86)`
**审计依据**：AGENTS.md 审计 agent 守则（独立审计 / 可提出自己想法 / 质疑历史决策）
**验证方式**：源码直读 + 全量 lib 测试

### 7.1 改动概述

本 commit 修复 §5 O1 观察项——`ChatLog.scope_session_id` 此前被 `#[serde(skip, default)]` 完全隐藏，前端无法从 history 响应中获取 scope session 标识，导致 session 列表 id 与 history `session_id`（内部 UUID）无法关联。

改动两处：

1. `engine/src/chat_store.rs`：`scope_session_id` 的 serde 属性从 `#[serde(skip, default)]` 改为 `#[serde(skip_serializing_if = "Option::is_none", default)]`，并补充详细的文档注释说明字段语义、持久化行为与向后兼容性。
2. `engine/src/daemon/mod.rs`：新增 2 个测试覆盖 session-scoped 暴露与 legacy 省略。

### 7.2 改动审查

#### 7.2.1 serde 属性变更正确性

`#[serde(skip_serializing_if = "Option::is_none", default)]` 的语义：
- **序列化**：`Some(x)` 时字段出现（值为 `x`）；`None` 时字段完全省略。
- **反序列化**：字段缺失时 `#[serde(default)]` 给 `None`；字段存在时正常解析。

serde 不关心字段可见性——`scope_session_id` 虽是私有字段（无 `pub`），但 `#[derive(Serialize)]` 仍会序列化它。改动正确。

文档注释清晰说明了：
- `scope_session_id` 与 `session_id`（内部 UUID）的区别
- 持久化时不写入（jsonl 用 `StoredMessage`，meta 用 `ChatLogMeta`，均不含此字段）
- 反序列化时 `#[serde(default)]` 给 `None`，legacy JSON 迁移安全

#### 7.2.2 向后兼容

- **legacy ChatLog**（`scope_session_id = None`）：序列化时字段被 `skip_serializing_if` 省略，JSON 中不出现 `scope_session_id`。旧客户端不受影响。
- **新客户端**：可选地读取 `scope_session_id` 字段，缺失时按 legacy 行为处理。
- 这是纯增量改动，不破坏任何现有 API 契约。

#### 7.2.3 持久化影响

独立验证源码：
- `StoredMessage`（`chat_store.rs:80-85`）：仅含 `msg`（flatten ChatMessage）+ `ts`，不含 `scope_session_id`。✓
- `ChatLogMeta`（`chat_store.rs:62-67`）：仅含 `session_id` / `character_id` / `created_at` / `updated_at`，不含 `scope_session_id`。✓
- legacy JSON 迁移路径（`chat_store.rs:299`）显式 `log.scope_session_id = None`，确保 legacy 数据不会错误携带 scope 标识。✓

反序列化安全：`ChatLog` 从 JSON 反序列化时，若 JSON 无 `scope_session_id` 字段，`#[serde(default)]` 给 `None`。legacy `chat_log.json` 迁移路径安全。

#### 7.2.4 测试覆盖

新增 2 个测试：

| 测试名 | 验证内容 | 结果 |
|---|---|---|
| `test_o1_session_scoped_history_exposes_scope_session_id` | 传 `session_id` 调 `/v1/chat/history` → 响应 JSON 含 `scope_session_id` 且值与传入一致 | ✅ 通过 |
| `test_o1_legacy_history_omits_scope_session_id` | 不传 `session_id` 调 `/v1/chat/history` → 响应 JSON 不含 `scope_session_id` 字段 | ✅ 通过 |

测试用 `make_state_no_key()` 创建临时 data_root，通过 `tower::ServiceExt::oneshot` 直接对 router 发 HTTP 请求，验证完整序列化链路（handler → `load_or_create_for_session` → `Json<ChatLog>` → serde 序列化）。覆盖了 O1 的核心行为。

#### 7.2.5 安全

`scope_session_id` 是由 `POST /v1/sessions/:character_id` 返回的 UUID，用于目录命名（`characters/{id}/sessions/{session_id}/history/`）。它与内部 `session_id` 一样都是会话标识符，不是敏感信息（不含密钥、用户数据、认证凭据）。暴露给已通过鉴权的前端是合理的。

### 7.3 测试结果

#### O1 专项测试

```
running 2 tests
test daemon::tests::test_o1_legacy_history_omits_scope_session_id ... ok
test daemon::tests::test_o1_session_scoped_history_exposes_scope_session_id ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 361 filtered out
```

#### 全量 lib 测试

```
test result: ok. 362 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out
```

对比 b3b29ad 审计时的 360 lib，新增 2 个 O1 测试后总数 362，完全吻合。1 个 ignored 是 `orchestrator::lorebook::tests::bench_aho_corasick_vs_naive`（性能基准测试，与本次改动无关）。

### 7.4 独立发现的问题

#### C1（C 级，非阻塞）— legacy 省略测试的断言不够严格

`test_o1_legacy_history_omits_scope_session_id` 的断言：

```rust
assert!(
    v.get("scope_session_id").is_none() || v["scope_session_id"].is_null(),
    "legacy 响应不应包含 scope_session_id 字段"
);
```

`skip_serializing_if = "Option::is_none"` 确保 `None` 时字段**完全不出现**在 JSON 中，所以 `v.get("scope_session_id")` 返回 `None`，第一个条件即满足。第二个条件 `v["scope_session_id"].is_null()` 是多余的防御——当字段不存在时 `v["scope_session_id"]` 返回 `Value::Null`，`is_null()` 为 `true`。

**问题**：如果未来有人误把 serde 属性从 `skip_serializing_if = "Option::is_none"` 退化为纯 `#[serde(default)]`（无 skip），legacy 响应会变成 `"scope_session_id": null`。此时 `v.get("scope_session_id")` 返回 `Some(Value::Null)`（不是 `None`），但 `v["scope_session_id"].is_null()` 仍为 `true`，断言仍通过——测试无法捕获此 regression。

**建议**：改为严格断言 `assert!(v.get("scope_session_id").is_none(), ...)`，确保字段完全不出现。

**严重度**：C 级（不影响当前正确性，仅降低 regression 捕获能力）

#### C2（C 级，非阻塞）— 测试未覆盖 rollback / regen 端点的 scope_session_id 暴露

`rollback_chat` 和 `regen_chat` 同样返回 `Json<ChatLog>`，当 `session_id` 指定时它们的响应也会包含 `scope_session_id`。但 O1 测试只覆盖了 `chat/history`。

**风险评估**：三个 handler 共享同一 `load_or_create_for_session` 路径和 `Json<ChatLog>` 序列化，行为一致。从风险角度看，覆盖 history 即可代表性地验证序列化逻辑。但严格来说，若未来有人改变 rollback/regen 的返回类型或包装，测试不会捕获。

**建议**：可在后续迭代中补充 rollback/regen 的 `scope_session_id` 暴露测试。

**严重度**：C 级（共享路径已覆盖，盲区风险低）

### 7.5 审计结论

| 项 | 状态 | 说明 |
|---|---|---|
| serde 属性变更 | ✅ 通过 | `skip_serializing_if + default` 正确，`Some` 暴露 / `None` 省略 |
| 向后兼容 | ✅ 通过 | legacy 响应省略字段，旧客户端零影响 |
| 持久化影响 | ✅ 通过 | `StoredMessage` / `ChatLogMeta` 均不含此字段，反序列化 `default` 给 `None` |
| 测试覆盖 | ✅ 通过 | 2 个专项测试覆盖 session-scoped 暴露 + legacy 省略 |
| 安全 | ✅ 通过 | `scope_session_id` 是 UUID 会话标识，非敏感信息 |
| 全量测试 | ✅ 通过 | 362 lib 测试 0 失败 |

**总体结论**：O1 修复实现正确、向后兼容、持久化安全、测试覆盖充分。**建议合并 PR #85**。

C1/C2 为非阻塞观察项，可在后续迭代中按需处理。
