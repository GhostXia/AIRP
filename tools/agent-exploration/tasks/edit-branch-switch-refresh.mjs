// 任务集 3: Edit + Branch + 切换 + 刷新恢复
// 参考 issue #273 MVP 必备项 3

export const DESCRIPTION = `前置：用 API 准备一个角色 + 一个有至少 2 轮对话的 session。

步骤提示：
1. 导航到 02-chat-space.html?character=<id>&session=<id>
2. 记录当前 history 的所有 message_ids 和 active_leaf（通过 /v1/chat/history）
3. 找到第一条 role=user 消息的 message_id
4. 调 PUT /v1/chat/message 编辑该用户消息 content 为 'I changed my question: what is the library policy on late returns?'
5. 由于编辑历史用户消息会触发分支语义（按 engine 当前实现：编辑 user 消息可能创建分支或原地替换；
   以 engine 实现为准），调 /v1/chat/history 确认当前 active path 状态
6. 如果 engine 支持分支（chat_store 有 branch_tree）：调 /v1/chat/branch/switch
   切换到原 active_leaf（编辑前的 leaf）
7. 在原分支继续发一条用户消息 'Thanks.', 等待 assistant 回复
8. 切换回新分支（编辑后的 leaf），发一条用户消息 'And the fines?', 等待 assistant 回复
9. 多次切换两个分支（至少 3 次来回），每次确认 history 只显示当前 active path
10. 刷新页面：await ctx.page.reload()
11. 再次调 /v1/chat/history
12. ASSERT: 当前 active_leaf 与刷新前一致
13. ASSERT: 当前 active path 的消息序列与刷新前一致
14. ASSERT: 另一分支的数据未被删除（切回另一分支验证其消息序列仍在）`;

export const EXPECTED = `编辑历史用户消息后建立的分支，与原分支共存；
多次切换后，刷新页面应保持当前 active path；
另一分支数据未被污染或删除。`;

export async function check(harness, result) {
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx during branch ops: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine chat_store branch_tree or /v1/chat/branch/switch handler',
    };
  }
  const errors = (result.consoleErrors || []).filter(e => e.type === 'error' && !/favicon|networkerror/i.test(e.message || ''));
  if (errors.length > 5) {
    return {
      ok: false,
      actual: errors.length + ' console errors during branch switching: ' + JSON.stringify(errors.slice(0, 3)),
      suspectedArea: 'webui branch-tree rendering or chat-space active path rendering',
    };
  }
  return { ok: true };
}
