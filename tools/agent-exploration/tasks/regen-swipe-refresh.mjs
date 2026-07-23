// 任务集 2: Regen + Swipe + 刷新恢复
// 参考 issue #273 MVP 必备项 2

export const DESCRIPTION = `前置：已完成 onboarding 并有至少一轮 assistant 回复（可直接调 /v1/characters/import
和 /v1/chat/completions API 准备数据，不必走 onboarding UI）。

步骤提示：
1. 导航到 02-chat-space.html?character=<id>&session=<id>
2. 记录当前最后一条 assistant 消息的 message_id 和 content
3. 调 /v1/chat/regen 重新生成最后一条 assistant 消息，等待流式完成
4. 调 /v1/chat/history 确认 assistant 消息已被替换（message_id 不变或新生成；
   按 engine 当前契约：regen 替换最后一条，durable ID 行为以 engine 实现为准）
5. 对该 assistant 消息调 /v1/chat/swipe 至少 3 次，每次 index 0/1/2
   等待每次切换后 history 反映新候选
6. 切换到候选 1，发一条新用户消息 'Continue the story.'
7. 等待 assistant 流式回复完成
8. 刷新页面：await ctx.page.reload()
9. 再次调 /v1/chat/history
10. ASSERT: 当前激活候选与刷新前一致（通过 history 的 active candidate 字段或 message 内容比对）
11. ASSERT: 后续对话上下文连续（用户消息 'Continue the story.' 后跟 assistant 回复）`;

export const EXPECTED = `Swipe 切换后，刷新页面应保持当前激活候选；
后续对话上下文基于当前激活候选继续，不串扰其他候选内容。`;

export async function check(harness, result) {
  // 兜底：检查是否有 5xx 或严重 console 错误
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx during swipe/regen: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine /v1/chat/swipe or /v1/chat/regen handler',
    };
  }
  // 检查是否有 unhandledrejection
  const unhandled = (result.consoleErrors || []).filter(e => e.type === 'unhandledrejection');
  if (unhandled.length > 0) {
    return {
      ok: false,
      actual: unhandled.length + ' unhandled promise rejections: ' + JSON.stringify(unhandled.slice(0, 3)),
      suspectedArea: 'webui swipe runtime; check chat-space.js candidate switching',
    };
  }
  return { ok: true };
}
