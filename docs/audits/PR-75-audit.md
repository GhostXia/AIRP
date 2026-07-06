# PR #75 独立审计报告

- **审计源 LLM**: GLM-5.2
- **审计日期**: 2026-07-06
- **审计范围**: issue #73 方案 B — 消息级时间戳
- **审计基线**: main @ fb5a355
- **审计分支**: fix-issue-73-msg-timestamps
- **审计模式**: 独立审计

## 测试

```
cargo test --manifest-path engine/Cargo.toml chat_store: 7 pass / 0 fail
cargo test --manifest-path engine/Cargo.toml: 367 pass / 0 fail
node --check webui/app.js: pass
node --check webui/serve.js: pass
node target/test-serve-security.js: 12 pass / 0 fail
node target/test-md-v2.js: 24 pass / 0 fail
```

神圣不变式 **2/2 pass**。编译零错误零警告。

## 逐项审计

### 1. 设计：StoredMessage + ChatLog.message_timestamps

**实现**（[chat_store.rs:62-77](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L62-L77)）：
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMessage {
    #[serde(flatten)]
    msg: ChatMessage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ts: Option<String>,
}
```

**分析**：
- `#[serde(flatten)]` 让 `ChatMessage` 的 `role` / `content` 平铺到同一 JSON 对象，OpenAI 协议兼容
- `ts` 可选，旧 jsonl 无此字段 → `None`（向后兼容）
- `skip_serializing_if = "Option::is_none"`：旧消息仍写为 `{role, content}`，不污染 jsonl

**结论**：设计正确。

### 2. 数据一致性：messages 与 message_timestamps 等长

所有修改 `messages` 的路径都同步维护 `message_timestamps`：
- `append`：push msg + push ts（[chat_store.rs:364-365](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L364-L365)）
- `append` 滚动截断：`drain` 两者同步（[chat_store.rs:370-371](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L370-L371)）
- `delete_last_n`：`clear` / `truncate` 两者同步（[chat_store.rs:413-417](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L413-L417)）
- `rollback_to`：`truncate` 两者同步（[chat_store.rs:428-429](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L428-L429)）
- `save`：按 index 取 ts（[chat_store.rs:334-335](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L334-L335)）
- `read_messages_jsonl`：返回两个等长 Vec（[chat_store.rs:450-455](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L450-L455)）
- legacy JSON 迁移：补齐长度（[chat_store.rs:302-304](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L302-L304)）

**结论**：一致。

### 3. 向后兼容性

- **旧 jsonl（无 ts）**：`read_messages_jsonl` 先尝试 `StoredMessage`，失败回退 `ChatMessage`，`ts: None`（[chat_store.rs:438-450](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L438-L450)）
- **旧 ChatLog JSON（legacy 迁移）**：`#[serde(default)]` 给空 Vec，迁移时补齐为全 `None`（[chat_store.rs:302-304](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L302-L304)）
- **webui 无 ts 数据**：`tsRaw ? new Date(tsRaw) : null` 传给 `appendMsg`，非 Date 不显示时间戳（[app.js:694-697](file:///d:/AIRP-Dev/webui/app.js#L694-L697)）

**结论**：向后兼容正确，不强制迁移。

### 4. webui 渲染

`loadHistory` 现在读取 `data.message_timestamps`，与 `messages` 一一对应（[app.js:690](file:///d:/AIRP-Dev/webui/app.js#L690)）。历史消息有时间戳时显示 `HH:MM:SS`，与流式新消息一致。

**边界**：`tss[i]` 为 `null` / `undefined` 时安全降级。

### 5. 测试覆盖

4 个新增测试：
- `test_message_timestamps_persisted_after_append`：append + reload 验证 ts 持久化
- `test_message_timestamps_back_compat_old_jsonl`：旧格式无 ts → None
- `test_message_timestamps_delete_last_n_keeps_sync`：删除后等长
- `test_message_timestamps_rollback_keeps_sync`：rollback 后等长

**建议补强（W-01）**：
- 补一个测试：旧 jsonl append 新消息后，save 写入的 jsonl 混合旧行（无 ts）+ 新行（有 ts），reload 后仍保持对应关系
- 补一个 HTTP-level 测试：调用 `POST /v1/chat/history` 返回的 JSON 包含 `message_timestamps` 字段，且长度等于 `messages`

W-01 属于 nice-to-have，不影响当前 PR 合并。

## 发现与建议

### PASS 项

- StoredMessage 设计
- messages / message_timestamps 一致性维护
- 向后兼容性
- webui 渲染
- 测试全绿

### W 项（可后续 issue）

- **W-01**：补混合旧新 jsonl + HTTP-level 回归测试（nice-to-have）

## 综合结论

**推荐合并**。设计正确、向后兼容、测试覆盖、全量测试通过。

**审计 LLM 模型**：GLM-5.2
