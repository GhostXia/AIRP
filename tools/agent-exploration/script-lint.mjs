// 脚本预检：拦截 LLM 反复犯的反模式。
//
// 拆到独立模块便于单元测试（runner.mjs 是 CLI 入口，import 它会触发整个
// 脚本执行，无法在测试中独立 import lintScript）。
//
// 检测项：
//   1. 手写 'session-...' 字面量（必须用 ctx.createSession）
//   2. 直接调 POST /v1/sessions（必须用 ctx.createSession）
//   3. 直接调 GET /v1/characters 并解析响应（必须用 ctx.characterExists）
//   4. PUT /v1/memory/resident body 包含 user_id 字段（引擎会拒绝未知字段或忽略）
//   5. character_id 用 'test-' 前缀（必须用 ctx.uuid()）
//
// 注：lint 是兜底，不是安全边界。脚本仍可能在执行中违反其他契约。
// 返回违例字符串数组（空数组表示通过）。

/**
 * 检测 LLM 生成脚本中的反模式。
 * @param {string} src - 脚本源码
 * @returns {string[]} 违例描述数组，空数组表示通过
 */
export function lintScript(src) {
  const v = [];
  // 1. 'session-...' / "session-..." / `session-${...}` 字面量
  if (/(['"`])session-/.test(src)) {
    v.push("Found 'session-...' string literal — use ctx.createSession(characterId) instead of hand-writing session_id");
  }
  // 2. 直接调 POST /v1/sessions/:character_id
  if (/apiCall\s*\(\s*['"`][^'"`]*\/v1\/sessions\//.test(src) ||
      /fetch\s*\(\s*[^,)]*\/v1\/sessions\//.test(src)) {
    v.push("Found direct call to /v1/sessions — use ctx.createSession(characterId) instead");
  }
  // 3. 直接调 GET /v1/characters 并解析
  if (/apiCall\s*\(\s*['"`]\/v1\/characters['"`]/.test(src) ||
      /fetch\s*\(\s*[^,)]*\/v1\/characters['"`]/.test(src)) {
    v.push("Found direct call to /v1/characters — use ctx.characterExists(characterId) instead");
  }
  // chars.characters / chars?.characters 模式（误以为返回对象数组）
  if (/chars\s*\??\.\s*characters/.test(src)) {
    v.push("Found chars.characters access — /v1/characters returns string array, use ctx.characterExists instead");
  }
  // 4. PUT /v1/memory/resident body 含 user_id
  // 简单启发：在 /v1/memory/resident 附近的 PUT body 中检测 user_id 字段
  if (/\/v1\/memory\/resident[\s\S]{0,500}user_id/.test(src)) {
    v.push("Found user_id field in /v1/memory/resident body — UpdateResidentMemoryRequest has no user_id field, remove it");
  }
  // 5. 'test-' + ... 拼接模式（用于生成 character_id 等唯一 ID），应改用 ctx.uuid()
  // 检测：'test-' + 或 "test-" + 或 `test-${...} 形式
  // 覆盖 charId = 'test-' + randomHex(16)、characterId = 'test-' + Date.now() 等变体
  if (/['"`]test-['"`]\s*\+/.test(src) || /\\\`test-\$\{/.test(src)) {
    v.push("Found 'test-' + ... ID generation pattern — use ctx.uuid() instead");
  }
  // 6. 错误的 DOM 选择器：#chat-input / #send-button（实际是 #message-input / #send-message）
  if (/#chat-input/.test(src)) {
    v.push("Found #chat-input selector — the correct selector is #message-input");
  }
  if (/#send-button/.test(src)) {
    v.push("Found #send-button selector — the correct selector is #send-message");
  }
  // 7. 依赖 sseCall 返回的 messageId/message_id（引擎 SSE 流不发 message_id）
  // sseCall 返回 { content } — 只有 content，没有 message_id/messageId。
  // 需 message_id 时必须调 /v1/chat/history 取最后一条 assistant 消息的 message_id。
  // 7a. .messageId (camelCase) 属性访问 — 任何 API 都不返回此字段名
  if (/\.\s*messageId\b/.test(src)) {
    v.push("Found .messageId access — sseCall returns { content } only. Query /v1/chat/history for message_id instead.");
  }
  // 7b. .message_id (snake_case) 属性访问 — 检测 sseCall 结果变量上的访问。
  // 启发式：变量名匹配 reply/response/result 或以 Reply/Response 结尾。
  // history 消息上的 .message_id 是合法的，不误报。
  if (/\b(reply|response|result|initialReply|regenReply|userReply|newReply|chatReply|aiReply|assistantReply|\w*Reply|\w*Response)\s*\.\s*message_id\b/.test(src)) {
    v.push("Found .message_id access on sseCall result — sseCall returns { content } only. Use /v1/chat/history to get message_id.");
  }
  // 7c. 解构 messageId (camelCase) — 任何 API 都不返回此字段名
  if (/\{\s*messageId\b/.test(src)) {
    v.push("Found destructuring of messageId — sseCall returns { content } only. Query /v1/chat/history for message_id instead.");
  }
  return v;
}
