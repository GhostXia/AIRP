# RUNBOOK：会话被 TurnCommit marker 锁死后的恢复

> 适用版本：v0.0.3+（BUG-2 缓解切片）。
> 关联登记：CURRENT-BASELINE #409（已知残留：非终态 TurnCommit marker 使会话 fail-closed）。

## 背景：为什么会话会被“锁死”

AIRP 在写入一轮对话（消息 / 状态 / 卷）前，会先在会话 history 目录落下一个
`turn_commit.json` **提交标记（marker）**，全部阶段完成后才删除它。如果进程
在写入中途崩溃或被强杀，会残留一个**非终态 / 不可读**的 marker。引擎无法确定
这次写入到底完成了多少，因此在 payload-aware replay（按载荷重放）交付之前，
该会话会**保守地拒绝一切新写入**（`409 session_recovery_required`），
这就是 fail-closed 锁死。

**重要：锁死不是数据损坏。** 已有消息仍然可读（历史查询不受影响），只是不能
继续写入。

## 恢复方式一：WebUI 一键恢复（推荐）

1. 打开**对话空间**（02-chat-space）。被锁死的会话会在输入框下方显示红色标签
   **「会话：恢复中」**，旁边出现 **「尝试恢复会话」** 按钮。
2. 点击按钮并确认。引擎会把残留 marker **归档（quarantine）** 而不是删除，
   随后会话解除锁定。
3. 成功时事件日志记录 `session.recover` 与归档路径，页面自动刷新 session-state
   与历史；失败时记录 `session.recover.error`，可按提示重试。

备选入口：**控制台 → 诊断**（23-diagnostics）页面底部「会话恢复（写入中断锁死）」
卡片，可先查看当前会话状态（`recovering` 即被锁定），再点击「尝试恢复会话」。

## 恢复方式二：直接调用端点

```
POST /v1/chat/session-recover
Authorization: Bearer <access_api_key>（如已启用鉴权）
Content-Type: application/json

{
  "character_id": "<角色目录名>",
  "session_id": "<命名会话 UUID，可省略（legacy 会话）>",
  "user_id": "<多租户用户 ID，可省略>"
}
```

响应形状：

```json
{
  "status": "recovered",
  "character_id": "...",
  "session_id": "...",
  "generation_id": "被中断的生成 ID（marker 不可读时为空）",
  "phase": "message_committed",
  "quarantined_marker": "<data_root>/quarantine/turn-commit/<character>/<session>/turn_commit.quarantined.<时间戳>.json"
}
```

错误语义：

| 状态码 | 含义 | 处理 |
| ------ | ---- | ---- |
| 404 | 该会话没有 pending marker，无需恢复 | 检查 character_id / session_id / user_id 是否指向正确数据根 |
| 409 `session_busy` | 会话正在生成/提交中 | 等待当前操作结束再重试 |
| 409 `turn commit marker was resolved concurrently` | marker 已被并发清理 | 直接重试即可 |

## 恢复方式三：手工处理（引擎不可用时）

1. 停止引擎进程。
2. 找到 marker：`<data_root>/characters/<角色>/history/turn_commit.json`
   （命名会话：`<data_root>/characters/<角色>/sessions/<会话>/history/turn_commit.json`；
   多租户：`<data_root>/users/<user_id>/...` 同构）。
3. **不要删除**。把它移动到
   `<data_root>/quarantine/turn-commit/<角色>/<会话或 legacy>/`，
   重命名为 `turn_commit.quarantined.<时间戳>.json`（与端点行为一致）。
4. 重启引擎，会话即可继续写入。

## quarantine 文件如何处理

- 归档文件是那次被中断写入的**原始凭据**，保留它们不会占用可观空间，也**不
  影响会话正常运行**。
- **payload-aware replay 尚未交付**：目前引擎不会自动重放被中断的那一轮。未来
  replay 切片落地后，将依据这些归档 marker 尝试补齐/回退未完成的写入。
- 在确认不需要追责或 replay 之前，请勿清空 `quarantine/` 目录；确需清理时建议
  先纳入备份范围再删除。

## 已知限制

- 被中断的那一轮写入本身**不会被自动补全**：最后一条消息可能缺失或半写
  （由会话历史自身的原子写契约兜底）。恢复后请刷新历史确认内容，再决定是否
  重新发送那一轮消息。
- 本 runbook 的端点只处理 TurnCommit marker 锁死；`session_busy`（有活跃生成）
  不属于此场景，请用取消生成端点或等待其结束。
