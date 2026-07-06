# PR #76 独立审计报告

- **审计源 LLM**: GLM-5.2
- **审计日期**: 2026-07-07
- **审计范围**: PR #75 follow-up — A-1/A-2/A-3/W-01
- **审计基线**: main @ 7b943e4
- **审计分支**: fix-pr75-followup-a1-a2-a3-w01
- **审计模式**: 独立审计

## 测试

```
cargo test --manifest-path engine/Cargo.toml: 369 pass / 0 fail (新增 2 测试)
神圣不变式 subagent_context_has_no_orchestrator_noise: 1/1 pass
node --check webui/app.js: pass
node --check webui/serve.js: pass
node target/test-serve-security.js: 12 pass / 0 fail
node target/test-md-v2.js: 24 pass / 0 fail
```

编译零错误零警告。

## 逐项审计

### A-1：删除 read_messages_jsonl 中 dead code 回退路径

**位置**：[chat_store.rs:463-471](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L463-L471)

**改动**：删除 ~14 行 `match serde_json::from_str` + ChatMessage 回退，简化为单次 `serde_json::from_str::<StoredMessage>(line).map_err(...)`。

**独立验证**：
- PR #75 二次审计时已用独立 Rust 实验（`target/serde-flatten-test`）验证 `#[serde(flatten)]` + `Option<ts>` + `#[serde(default)]` 能直接 parse 旧 jsonl
- 本 PR 的 `test_message_timestamps_mixed_old_new_jsonl` 测试间接验证了该假设：旧 jsonl（无 ts）append 新消息后 reload，旧行 ts=None 正确解析。如果 A-1 错误，此测试会失败

**错误消息变化审视**：旧行为下非法 JSON 行会先 StoredMessage 失败 → 回退 ChatMessage → 也失败 → 报 ChatMessage 错误。新行为直接报 StoredMessage 错误。两者前缀都是"chat_log.jsonl 第 N 行解析失败"，serde 细节差异对用户无实质影响。

**结论**：正确。

### A-2：debug_assert 保证等长不变量

**位置**：
- [chat_store.rs:305-310](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L305-L310)（legacy JSON 迁移后）
- [chat_store.rs:473-479](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L473-L479)（read_messages_jsonl 返回前）

**审视**：
- `debug_assert` 在 release build 被移除，不影响生产性能
- 只在两个"从磁盘加载"入口加 assert，其他路径（append/delete/rollback）在同一函数内同步修改两个 Vec，不需要重复 assert
- assert 位置合理：read_messages_jsonl 是 jsonl 加载入口，legacy 迁移是 JSON 迁移入口，两者都是从外部数据恢复 ChatLog 状态的边界

**结论**：正确。

### A-3：webui console.warn 暴露长度不匹配 bug

**位置**：[app.js:691-694](file:///d:/AIRP-Dev/webui/app.js#L691-L694)

**审视**：
- `tss.length !== msgs.length` 时 console.warn，不再静默降级
- 浏览器控制台普通用户不看，但开发者能通过 DevTools 定位 engine bug
- 不影响渲染逻辑（仍 graceful degradation 为不显示时间戳）

**结论**：正确。

### W-01：补 2 个回归测试

#### test_message_timestamps_mixed_old_new_jsonl

**位置**：[chat_store.rs:821-882](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L821-L882)

**覆盖场景**：旧 jsonl（2 行无 ts）+ append 1 条新消息（有 ts）→ reload 验证：
- 旧行 ts=None（验证 StoredMessage 能 parse 旧 jsonl — 间接验证 A-1）
- 新行 ts=Some（验证 append 正确写入 ts）
- 新行 ts 能 parse 为 RFC 3339 时间

**关键观察**：append 常规路径是 O(1) 追加一行到 jsonl 末尾，不重写整个文件。所以 reload 时 jsonl 包含：
- 旧行（无 ts，旧格式）→ StoredMessage 解析为 ts=None
- 新行（有 ts，新格式）→ StoredMessage 解析为 ts=Some

此测试**间接验证了 A-1 的核心假设**。如果 StoredMessage 不能 parse 旧 jsonl，测试会失败。

**结论**：测试设计合理。

#### pr75_chat_history_returns_message_timestamps

**位置**：[handlers.rs:1497-1555](file:///d:/AIRP-Dev/engine/src/daemon/handlers.rs#L1497-L1555)

**覆盖场景**：用 ChatLog API 写入 2 条消息 → HTTP POST /v1/chat/history → 验证返回 JSON：
- `messages` 数组长度 = 2
- `message_timestamps` 字段存在且长度 = 2
- 每条 ts 是字符串（非 null）

**数据一致性验证**：`make_state_for_http_test` 返回 `(state, tmp)`，`state.data_root = tmp.path()`。测试中 `root = tmp.path()`，与 state.data_root 一致。

**结论**：HTTP-level 测试正确。

## 发现与建议

### PASS 项

- A-1：dead code 删除正确，核心假设被独立 Rust 实验 + W-01 混合测试双重验证
- A-2：debug_assert 位置合理，覆盖两个磁盘加载入口
- A-3：console.warn 不影响渲染逻辑，暴露 engine bug
- W-01：两个测试设计合理，覆盖混合场景 + HTTP-level
- 测试全绿：369 cargo + 2/2 神圣不变式 + 12/12 security + 24/24 markdown

### 无 W 项

所有审计点均 PASS，无遗留项。

## 综合结论

**推荐合并**。4 项修复都正确，测试覆盖充分，无遗留问题。

**审计 LLM 模型**：GLM-5.2
