# PR #75 二次独立审计报告

- **审计源 LLM**: GLM-5.2
- **审计日期**: 2026-07-06
- **审计范围**: issue #73 方案 B — 消息级时间戳（二次独立审计）
- **审计基线**: main @ fb5a355
- **审计分支**: fix-issue-73-msg-timestamps
- **审计模式**: 独立审计 + 质疑历史决策（按 AGENTS.md 审计守则第 3 条）
- **首轮报告**: [PR-75-audit.md](file:///d:/AIRP-Dev/docs/audits/PR-75-audit.md)

## 触发原因

首轮审计与本次审计相隔 < 1 小时，结论同质化风险高。按 AGENTS.md 审计守则第 3 条"质疑历史决策"重新审视。

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

## 独立 Rust 实验：A-1 验证

为质疑首轮审计未明确验证的"StoredMessage 是否真能 parse 旧 jsonl"问题，运行独立 Rust 实验
（`D:\AIRP-Dev\target\serde-flatten-test`）：

| 输入 jsonl 行 | StoredMessage parse | ChatMessage parse | 结论 |
|---|---|---|---|
| `{"role":"user","content":"legacy1"}` | OK (ts=None) | OK | flatten+default 直接通过 |
| `{"role":"assistant","content":"legacy2"}` | OK (ts=None) | OK | 同上 |
| `{"role":"system","content":"sys"}` | OK (ts=None) | OK | 同上 |
| `{"role":"user","content":"x","unknown_field":"foo"}` | OK (ts=None) | OK | 额外字段不影响 |
| `{"role":"tool","content":"x"}` | FAIL | FAIL | 两者都失败，回退也不救 |

序列化对比：`StoredMessage { ts: None }` 输出与 `ChatMessage` 输出**逐字节一致**（均为 `{"role":"user","content":"hi"}`）。

### A-1 发现：`read_messages_jsonl` 中的 ChatMessage 回退路径是 dead code

**位置**：[chat_store.rs:462-475](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L462-L475)

**证据链**：
1. `#[serde(flatten)]` 把 `ChatMessage` 的 `role`/`content` 平铺到 JSON 同一层
2. `ts: Option<String>` + `#[serde(default)]` → 旧 jsonl 无 ts 时 `ts = None`（不报错）
3. 旧 jsonl 的 JSON 形状 = `{role, content}` ⊂ StoredMessage 的 JSON 形状 = `{role, content, ?ts}`
4. 因此 StoredMessage 在所有 ChatMessage 能 parse 的行上也能 parse
5. 唯一 StoredMessage 失败而 ChatMessage 成功的边界 case：未来 ChatMessage 加 required 字段 → 但 ChatMessage 同样会失败，回退路径不能救

**影响**：约 14 行 dead code（行 462-475 + 行 466 错误处理）。保留无功能影响，但增加维护成本和误读风险。

**建议（不阻塞合并）**：
- **方案 A（保守）**：保留回退 + 注释"防御性回退，理论上 StoredMessage 永不会因 ts 缺失失败；保留以应对未来结构分叉"
- **方案 B（彻底）**：删除回退逻辑，read_messages_jsonl 简化为单次 `serde_json::from_str(line)` 解析

我倾向方案 B（更简单 + 与"避免过度工程"原则对齐），但这是 nice-to-have 改进，**不阻塞当前 PR 合并**。

## 其他审计发现

### 1. `message_timestamps` 公开字段的 API 风险（信息级）

**位置**：[chat_store.rs:44](file:///d:/AIRP-Dev/engine/src/chat_store.rs#L44) `pub message_timestamps: Vec<Option<String>>`

**风险**：外部代码可直接修改 `Vec` 长度（push/truncate），破坏与 `messages` 的等长不变量（PR 的核心保证）。

**现状**：注释明确"长度始终等于 `messages.len()`"，但 `pub` 字段未做长度校验。

**建议**（非阻塞）：
- 短期：维持 `pub`，但加 debug_assert 保证不变量（`debug_assert_eq!(self.message_timestamps.len(), self.messages.len())`）
- 长期：将 `pub` 改为 `pub(crate)` 或封装为 `pub fn message_timestamps(&self) -> &[Option<String>]` 暴露只读视图

**优先级**：低。当前所有改动路径都在 `chat_store.rs` 内部维护，外部代码未触达此字段。后续若开放 API 给 webui 之外的端点再收紧。

### 2. webui `tss[i]` 跨索引的 graceful degradation（信息级）

**位置**：[app.js:694-697](file:///d:/AIRP-Dev/webui/app.js#L694-L697)

```javascript
msgs.forEach((m, i) => {
  const tsRaw = tss[i];  // tss.length < msgs.length 时 → undefined
  const ts = tsRaw ? new Date(tsRaw) : null;
  appendMsg(m.role || 'assistant', m.text || m.content || '', false, ts);
});
```

**观察**：当 `tss.length < msgs.length` 时，缺 ts 的消息被静默设为 `null`（不显示时间戳）。

**风险**：engine 应保证 `message_timestamps.len() == messages.len()`，长度不匹配是 engine bug。webui 静默降级掩盖了 bug。

**建议**（非阻塞）：
- webui 端可加一行 `if (tss.length !== msgs.length) console.warn('engine bug: message_timestamps length mismatch')` 暴露 bug
- engine 端在 `load_or_create` 后加 `debug_assert_eq!(messages.len(), message_timestamps.len())`

**优先级**：低。当前 engine 测试已覆盖等长（`test_message_timestamps_delete_last_n_keeps_sync` / `test_message_timestamps_rollback_keeps_sync`）。

### 3. W-01 重审（首轮报告建议补强测试）

首轮审计 W-01 建议：
> 补一个测试：旧 jsonl append 新消息后，save 写入的 jsonl 混合旧行（无 ts）+ 新行（有 ts），reload 后仍保持对应关系
> 补一个 HTTP-level 测试：调用 `POST /v1/chat/history` 返回的 JSON 包含 `message_timestamps` 字段

**本轮重审**：经独立 Rust 实验验证（Test 5），StoredMessage 序列化与 ChatMessage 序列化在 `ts: None` 时输出**逐字节一致**。这意味着：
- 旧行（无 ts）save 后再 reload → ts 仍 None（不变）
- 新行（有 ts）save 后再 reload → ts 仍 Some（不变）
- 混合场景：jsonl 中旧行 + 新行共存，reload 后两端分别保持原 ts 状态

混合 jsonl 场景**已被现有两个测试隐含覆盖**：
- `test_message_timestamps_back_compat_old_jsonl`：旧行 → None
- `test_message_timestamps_persisted_after_append`：新行 → Some

虽然没有一个测试显式混合，但行为可由两者**组合推断**。HTTP-level 测试同理：handlers.rs 的 `/v1/chat/history` 返回 ChatLog 整体 JSON，其中 `message_timestamps` 字段已包含（由 `Json<Value>` 序列化自动处理），没有额外的服务层处理可能破坏该字段。

**W-01 维持 nice-to-have 评级**，不阻塞合并。

## 发现与建议

### PASS 项

- StoredMessage 设计
- messages / message_timestamps 一致性维护
- 向后兼容性
- webui 渲染
- 测试全绿
- 367 cargo + 2/2 神圣不变式 + 12/12 security + 24/24 markdown

### A 项（建议，nice-to-have）

- **A-1**：`read_messages_jsonl` 中的 ChatMessage 回退路径是 dead code（方案 B：删除以简化代码）
- **A-2**：`message_timestamps` 公开字段的 API 风险（短期加 debug_assert，长期改 `pub(crate)`）
- **A-3**：webui `tss[i]` graceful degradation 掩盖 engine bug（短期 console.warn，长期依赖 engine 端 assert）

### W 项（可后续 issue）

- **W-01**（首轮提出，本轮维持）：补混合旧新 jsonl + HTTP-level 回归测试（nice-to-have）

## 综合结论

**推荐合并**。A-1 ~ A-3 均为 nice-to-have 改进建议，不影响功能正确性。W-01 同属 nice-to-have。

PR 的核心保证——"messages 与 message_timestamps 等长"——由 4 个回归测试覆盖。向后兼容性（`StoredMessage + Option<ts> + flatten(ChatMessage)`）经独立 Rust 实验验证，StoredMessage 能直接 parse 旧 jsonl（不需要回退逻辑）。

**审计 LLM 模型**：GLM-5.2
