// 任务集：chat/preview 装配预览 API smoke
// #433 residual：验证 preview 只读、脱敏、顺序和边界，不触发 provider。

export const DESCRIPTION = `通过 API 验证 /v1/chat/preview 的 bounded prompt assembly trace。

步骤提示：
1. 导航到 01-role-list.html，生成 UUID character_id
2. 导入带 synthetic secret marker 的角色卡，确认角色存在；创建命名 session
3. 记录 /v1/chat/history 与 /v1/chat/session-state 的响应
4. 调 /v1/chat/preview，使用不可达 synthetic endpoint、synthetic API key、card marker 和 message marker；不得调用 provider
5. ASSERT: trace 具备 segments/diagnostics/effective 结构，最小 fixture 的 source order 为 card → card → user
6. ASSERT: total_chars 与 segments.chars 之和一致，total_estimated_tokens 与 segments.estimated_tokens 之和一致；位置与字段均 bounded
7. ASSERT: 响应不包含 synthetic API key、endpoint、角色卡内容或用户消息
8. 再次读取 history 与 session-state，确认与预览前完全一致`;

export const EXPECTED = `preview 返回脱敏且有界的 PromptAssemblyTrace；
segments 保持 card → card → user 顺序、totals 与段元数据一致；不可达 provider endpoint 不影响响应；
preview 前后 history 与 session-state 不变。`;

function assert(condition, message) {
  if (!condition) throw new Error('ASSERT: ' + message);
}

export async function run(ctx) {
  await ctx.harness.navigate('01-role-list.html');

  const characterId = ctx.uuid();
  const cardSecret = 'PREVIEW_CARD_SECRET_433';
  const messageSecret = 'PREVIEW_MESSAGE_SECRET_433';
  const apiKey = 'synthetic-preview-api-key-433';
  const endpoint = 'http://127.0.0.1:1/preview-provider';
  const cardJson = JSON.stringify({
    spec: 'chara_card_v2',
    spec_version: '2.0',
    data: {
      name: 'Preview Fixture Character',
      description: cardSecret,
      personality: 'calm and precise',
      scenario: 'A bounded trace smoke test.',
      first_mes: '',
      mes_example: '',
      creator_notes: 'Synthetic fixture for preview smoke.',
      system_prompt: '',
      post_history_instructions: '',
      alternate_greetings: [],
      character_book: null,
      tags: ['synthetic'],
      creator: 'airp-agent-exploration',
      character_version: '1',
      extensions: {},
    },
  });

  const imported = await ctx.apiCall('/v1/characters/import', {
    character_id: characterId,
    card_json: cardJson,
  });
  assert(imported.character_id === characterId, 'character import returned unexpected id');
  assert(await ctx.characterExists(characterId), 'character import did not persist character');
  const sessionId = await ctx.createSession(characterId);

  const historyQuery = { character_id: characterId, session_id: sessionId, limit: 50 };
  const sessionQuery = { character_id: characterId, session_id: sessionId };
  const historyBefore = await ctx.apiCall('/v1/chat/history', historyQuery);
  const sessionBefore = await ctx.apiCall('/v1/chat/session-state', sessionQuery);

  const trace = await ctx.apiCall('/v1/chat/preview', {
    character_id: characterId,
    character_card_id: cardJson,
    session_id: sessionId,
    user_profile: { name: 'Tester', variables: {} },
    message: messageSecret,
    endpoint,
    api_key: apiKey,
    model: 'preview-no-call-model',
  });

  assert(trace && typeof trace === 'object' && !Array.isArray(trace), 'trace must be an object');
  assert(Array.isArray(trace.segments), 'trace.segments must be an array');
  assert(Array.isArray(trace.diagnostics), 'trace.diagnostics must be an array');
  assert(trace.effective && typeof trace.effective === 'object', 'trace.effective must be an object');
  assert(trace.segments.length === 3, 'minimal fixture must produce exactly three segments');

  const sourceKinds = trace.segments.map(segment => segment.source_kind);
  assert(JSON.stringify(sourceKinds) === JSON.stringify(['card', 'card', 'user']), 'unexpected segment source order: ' + JSON.stringify(sourceKinds));

  let totalChars = 0;
  let totalEstimatedTokens = 0;
  let previousPosition = -1;
  for (const segment of trace.segments) {
    assert(segment && typeof segment === 'object', 'segment must be an object');
    assert(typeof segment.source_kind === 'string' && segment.source_kind.length > 0, 'segment source_kind must be non-empty');
    assert(Number.isSafeInteger(segment.position) && segment.position >= previousPosition, 'segment positions must be bounded and ordered');
    assert(Number.isSafeInteger(segment.chars) && segment.chars >= 0, 'segment chars must be bounded');
    assert(Number.isSafeInteger(segment.estimated_tokens) && segment.estimated_tokens >= 0, 'segment estimated_tokens must be bounded');
    assert(!('content' in segment), 'segment must not expose prompt content');
    totalChars += segment.chars;
    totalEstimatedTokens += segment.estimated_tokens;
    previousPosition = segment.position;
  }
  assert(Number.isSafeInteger(trace.total_chars) && trace.total_chars === totalChars, 'total_chars must equal segment chars sum');
  assert(Number.isSafeInteger(trace.total_estimated_tokens) && trace.total_estimated_tokens === totalEstimatedTokens, 'total_estimated_tokens must equal segment token sum');
  assert(trace.effective.character_id === characterId, 'effective character_id mismatch');
  assert(trace.effective.endpoint === 'configured', 'effective endpoint must be redacted');

  const serializedTrace = JSON.stringify(trace);
  for (const marker of [apiKey, endpoint, cardSecret, messageSecret]) {
    assert(!serializedTrace.includes(marker), 'trace leaked synthetic marker: ' + marker);
  }

  const historyAfter = await ctx.apiCall('/v1/chat/history', historyQuery);
  const sessionAfter = await ctx.apiCall('/v1/chat/session-state', sessionQuery);
  assert(JSON.stringify(historyAfter) === JSON.stringify(historyBefore), 'preview changed chat history');
  assert(JSON.stringify(sessionAfter) === JSON.stringify(sessionBefore), 'preview changed session state');
}

export async function check(harness, result) {
  const failed = (result.failedRequests || []).filter(request => request.status && request.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx during preview API smoke: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine /v1/chat/preview handler',
    };
  }
  const unhandled = (result.consoleErrors || []).filter(error => error.type === 'unhandledrejection');
  if (unhandled.length > 0) {
    return {
      ok: false,
      actual: unhandled.length + ' unhandled promise rejections: ' + JSON.stringify(unhandled.slice(0, 3)),
      suspectedArea: 'agent exploration preview smoke script',
    };
  }
  return { ok: true };
}
