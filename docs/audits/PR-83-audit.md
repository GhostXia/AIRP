# PR #83 独立审计报告

**审计源 LLM**：GLM-5.2
**审计日期**：2026-07-07
**审计范围**：#81 W-03 message_timestamps 传递（commit 158d559）
**审计依据**：AGENTS.md 三条守则

## 改动清单

2 文件 +16 行，纯增量，无逻辑改动。

| 文件 | 改动 |
|---|---|
| `ui/src-tauri/src/bus.rs` | chat.history 转换逻辑读 message_timestamps + 按 index 合并 ts |
| `ui/src/widgets/ChatWidget.vue` | Msg interface 加 ts?: string |

## 审计

### bus.rs 转换逻辑 ✅

**index 对齐验证**：
- `messages` 和 `message_timestamps` 都是 array，按 index 一一对应（`ChatLog` struct 注释明确："长度始终等于 messages.len()"）
- `timestamps.get(i)` 按 index 取，i 从 messages.iter().enumerate() 来
- 旧 jsonl 无 ts → `message_timestamps` 为空 Vec 或长度 < messages → `timestamps.get(i)` 返回 None → `unwrap_or(Value::Null)` → null
- 新 jsonl 有 ts → `Some(string)` → 传递到 message object

**边界情况**：
- messages.len()=0 → 循环不执行，scope_messages 空，order 空 ✅
- messages.len()=10, timestamps.len()=10 → 正常合并 ✅
- messages.len()=10, timestamps.len()=5（旧 jsonl 部分有 ts）→ i=0..4 取 ts，i=5..9 取 null ✅
- messages.len()=10, timestamps.len()=0（旧 jsonl 完全无 ts）→ i=0..9 全取 null ✅

### ChatWidget Msg interface ✅

加 `ts?: string`（可选），不影响现有渲染（template 没读 ts）。注释说明"新消息当前无 ts，留 undefined"。

### 设计取舍（同意开发决策）

**只传递 ts，不显示 ts**：ChatWidget 当前不显示时间戳，传递 ts 是能力闭合。未来 ChatWidget 想显示时，ts 已在 state 里。最小步，避免改视觉布局。✅

## A 项（阻塞）

**无。**

## W 项

**无。** 纯增量改动，index 对齐正确，边界情况覆盖。

## 测试

- Tauri cargo check: pass
- UI vitest: 97/97 pass
- vue-tsc --noEmit: 0 errors

## 结论

**推荐合并。** 纯增量改动，index 对齐正确，向后兼容（旧 jsonl 无 ts → null）。
