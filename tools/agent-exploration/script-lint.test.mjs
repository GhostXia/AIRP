import test from 'node:test';
import assert from 'node:assert/strict';
import { lintScript } from './script-lint.mjs';

// 反例脚本：综合 regen-swipe-refresh 和 memory-roundtrip 中 LLM 实际生成的反模式
const BAD_SCRIPT = `export async function run(ctx) {
  const { apiCall, sseCall, harness } = ctx;
  function randomHex(len) { let s = ''; for (let i = 0; i < len; i++) s += Math.floor(Math.random()*16).toString(16); return s; }
  const charId = 'test-' + randomHex(16);
  const sessionId = 'session-' + randomHex(16);
  await apiCall('/v1/characters/import', { character_id: charId, card_json: '{}' });
  const chars = await apiCall('/v1/characters', null, 'GET');
  const found = chars?.characters?.some(c => c.character_id === charId);
  await apiCall('/v1/sessions/' + charId, { session_id: sessionId });
  await apiCall('/v1/memory/resident', { character_id: charId, session_id: sessionId, user_id: '', content: 'x' }, 'PUT');
  const reply = await sseCall('/v1/chat/completions', { character_id: charId, session_id: sessionId, message: 'hi', user_profile: { name: 'T', variables: {} } });
  const mid = reply.messageId;
  await page.fill('#chat-input', 'hello');
  await page.click('#send-button');
}`;

// 正例脚本：使用 ctx.uuid() / ctx.createSession / ctx.characterExists，body 无 user_id，
// 不依赖 sseCall.messageId，不用错误选择器
const GOOD_SCRIPT = `export async function run(ctx) {
  const characterId = ctx.uuid();
  const cardJson = JSON.stringify({ spec: 'chara_card_v2', data: { name: 'TestBot', first_mes: 'Hi' } });
  await ctx.apiCall('/v1/characters/import', { character_id: characterId, card_json: cardJson });
  if (!await ctx.characterExists(characterId)) throw new Error('ASSERT: import failed');
  const sessionId = await ctx.createSession(characterId);
  const reply = await ctx.sseCall('/v1/chat/completions', {
    character_id: characterId, session_id: sessionId, message: 'Hello',
    user_profile: { name: 'Tester', variables: {} }
  });
  const content = reply.content;
  const history = await ctx.apiCall('/v1/chat/history', { character_id: characterId, session_id: sessionId, limit: 10 });
  const assistantMsgId = history.messages[history.messages.length - 1].message_id;
  await ctx.apiCall('/v1/memory/resident', { character_id: characterId, session_id: sessionId, content: 'x' }, 'PUT');
  await ctx.page.fill('#message-input', 'hello');
  await ctx.page.click('#send-message');
}`;

test('lintScript: 正例脚本应通过（无违例）', () => {
  const v = lintScript(GOOD_SCRIPT);
  assert.equal(v.length, 0, 'expected no violations, got: ' + JSON.stringify(v));
});

test('lintScript: 检测 session- 字面量', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes("session-")), 'should detect session- literal');
});

test('lintScript: 检测 test- + ... ID 拼接模式', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes("'test-' + ... ID generation pattern")), 'should detect test- + ... pattern');
});

test('lintScript: 检测直接调 /v1/sessions', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes('direct call to /v1/sessions')), 'should detect direct /v1/sessions call');
});

test('lintScript: 检测直接调 /v1/characters', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes('direct call to /v1/characters')), 'should detect direct /v1/characters call');
});

test('lintScript: 检测 chars.characters 访问模式', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes('chars.characters access')), 'should detect chars.characters access');
});

test('lintScript: 检测 /v1/memory/resident body 中的 user_id', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes('user_id field in /v1/memory/resident')), 'should detect user_id in memory/resident body');
});

test('lintScript: 反例脚本至少触发 6 条违例', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.length >= 6, 'expected >=6 violations, got ' + v.length + ': ' + JSON.stringify(v));
});

test('lintScript: 检测 #chat-input 错误选择器', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes('#chat-input selector')), 'should detect #chat-input');
});

test('lintScript: 检测 #send-button 错误选择器', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes('#send-button selector')), 'should detect #send-button');
});

test('lintScript: 检测 reply.messageId 访问（sseCall 不返回 messageId）', () => {
  const v = lintScript(BAD_SCRIPT);
  assert.ok(v.some(s => s.includes('messageId access')), 'should detect reply.messageId');
});

test('lintScript: 检测 reply.message_id (snake_case) 访问 sseCall 结果', () => {
  const src = `const reply = await ctx.sseCall('/v1/chat/completions', {});
  const mid = reply.message_id;`;
  const v = lintScript(src);
  assert.ok(v.some(s => s.includes('message_id access on sseCall result')), 'should detect reply.message_id: ' + JSON.stringify(v));
});

test('lintScript: 检测 initialReply.message_id 访问', () => {
  const src = `const initialReply = await ctx.sseCall('/v1/chat/completions', {});
  const id = initialReply.message_id;`;
  const v = lintScript(src);
  assert.ok(v.length > 0, 'should detect initialReply.message_id');
});

test('lintScript: 检测解构 messageId from sseCall 结果', () => {
  const src = `const { messageId, content } = await ctx.sseCall('/v1/chat/completions', {});`;
  const v = lintScript(src);
  assert.ok(v.some(s => s.includes('destructuring of messageId')), 'should detect destructuring: ' + JSON.stringify(v));
});

test('lintScript: history 消息的 .message_id 不误报', () => {
  const src = `const history = await ctx.apiCall('/v1/chat/history', {}, 'POST');
  const lastMsg = history.messages[history.messages.length - 1];
  const mid = lastMsg.message_id;`;
  const v = lintScript(src);
  assert.equal(v.length, 0, 'history.message_id should not trigger violations: ' + JSON.stringify(v));
});

test('lintScript: #message-input 和 #send-message 不会误报', () => {
  const v = lintScript("await page.fill('#message-input', 'x'); await page.click('#send-message');");
  assert.equal(v.length, 0, 'correct selectors should not trigger violations');
});

test('lintScript: 模板字符串 session-${...} 也能被检测', () => {
  const src = "const sessionId = `session-${Date.now()}`;";
  const v = lintScript(src);
  assert.ok(v.length > 0, 'should detect template literal session-');
});

test('lintScript: 双引号 "session-..." 也能被检测', () => {
  const src = 'const sessionId = "session-123";';
  const v = lintScript(src);
  assert.ok(v.length > 0, 'should detect double-quoted session-');
});

test('lintScript: 空脚本通过', () => {
  const v = lintScript('');
  assert.equal(v.length, 0);
});
