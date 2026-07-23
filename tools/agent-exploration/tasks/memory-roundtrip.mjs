// 任务集 4: Memory 任务
// 参考 issue #273 任务集 4（用户选择包含在 MVP 中）

export const DESCRIPTION = `前置：用 API 准备角色 + session，并完成至少 1 轮对话。

步骤提示：
1. 导航到 02-chat-space.html?character=<id>&session=<id>
2. 发一条用户消息包含明确事实: 'My name is Agent-Tester and I live in Taipei.'
3. 等待 assistant 回复完成
4. 等待 memory 抽取（如果 engine 是异步抽取，轮询 /v1/memory/resident 至少 10 秒）
5. 导航到 17-memory-state.html?character=<id>&session=<id>
6. ASSERT: resident memory 中能找到 'Agent-Tester' 或 'Taipei' 相关条目
7. 手动编辑 resident memory: 调 PUT /v1/memory/resident 添加一条 'User prefers concise answers.'
8. 导航回 02-chat-space.html，发一条用户消息 'What do you know about me?'
9. 等待 assistant 回复
10. ASSERT: assistant 回复中应体现新写入的记忆（'concise' 或相关词）
   注: mock provider 可能不真实反映；如果用 mock provider, ASSERT 改为:
   调 /v1/chat/preview 确认 prompt 装配摘要中包含 'User prefers concise answers.'
11. 刷新 17-memory-state.html
12. ASSERT: resident memory 仍包含手动添加的条目
13. 终止 engine (如果 runner 支持) → 重启 engine → 重新访问 17-memory-state.html
14. ASSERT: resident memory 持久化（仍包含手动条目）

注: 步骤 13-14 的 engine 重启由 runner 控制；如果 runner 不支持, 改为只测刷新恢复。`;

export const EXPECTED = `Memory 抽取能捕获对话中的事实；
手动修改 resident memory 后，后续对话的 prompt 装配包含新记忆；
刷新和（如可测）重启 engine 后，resident memory 持久化。`;

export async function check(harness, result) {
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx during memory ops: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine /v1/memory/resident handler or memory extraction pipeline',
    };
  }
  // PUT 4xx 是合理的（body 超限等），不算 task 失败
  return { ok: true };
}
