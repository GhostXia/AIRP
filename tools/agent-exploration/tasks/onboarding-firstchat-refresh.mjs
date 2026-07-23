// 任务集 1: onboarding + 首聊 + 刷新恢复
// 参考 issue #273 MVP 必备项 1

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const cardPath = join(__dirname, '..', 'fixtures', 'character-card.json');

export const DESCRIPTION = `从空数据目录完成 onboarding 流程；导入合成角色卡 fixture；选择模型并完成首次聊天；
发送一轮用户消息并等待 assistant 流式回复完成；刷新页面后确认聊天历史一致。

步骤提示：
1. await ctx.harness.navigate('16-onboarding.html') — onboarding 入口（空数据目录会自动跳到这）
2. 完成每一步的"下一步"按钮点击（6 步），跳过/选择角色/配置 provider 都按默认走
3. 在"给角色的第一句话"输入框填 'Hello Aria, what books do you recommend?'
4. 点击"发送首轮消息"，等待"进入对话空间"按钮出现
5. 进入 02-chat-space.html，再发一条用户消息 'Tell me about sci-fi books.'
6. 等待 assistant 流式回复完成（send-button 不再含 stop class）
7. 调用 ctx.harness.getApiSnapshot('/v1/chat/history', 'POST', {character_id, session_id, limit: 50})
   记录 messages 数组
8. 刷新页面：await ctx.page.reload()
9. 再次调用 getApiSnapshot 取 history，对比 messages 完全一致
10. ASSERT: 两次 history 的 message_ids 顺序与内容完全相等`;

export const EXPECTED = `刷新页面后，/v1/chat/history 返回的 message_ids 顺序与 messages 内容与刷新前完全一致；
页面 DOM 上显示的对话条数也与 API 返回一致。`;

export async function check(harness, result) {
  // 二次校验：刷新前后 history 一致性由 Agent 脚本断言；此处只做兜底
  const consoleErrors = result.consoleErrors || [];
  const severeErrors = consoleErrors.filter(e => !/Deprecation|harvest|analytics/i.test(e.message || ''));
  if (severeErrors.length > 0) {
    return {
      ok: false,
      actual: severeErrors.length + ' severe console errors after refresh: ' + JSON.stringify(severeErrors.slice(0, 3)),
      suspectedArea: 'onboarding or chat-space runtime; check network/console evidence',
    };
  }
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx failed requests: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine 5xx during onboarding/firstchat; check engine logs',
    };
  }
  return { ok: true };
}

// 通过 ctx.fixtures 把解析好的角色卡 JSON 传给 Agent 脚本。
// 不要传 runner-local 路径：engine server 和 Agent 脚本都不应读 runner 文件系统。
// runner 在 runTask 里读取该 fixture JSON 并放入 ctx.fixtures.characterCard。
export const FIXTURES = { characterCard: JSON.parse(await readFile(cardPath, 'utf8')) };
