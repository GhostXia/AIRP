import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';

const require = createRequire(import.meta.url);
const { buildRequest, describeEvent, run } = require('../assets/agent-run.js');
const runtime = await readFile(new URL('../assets/console-runtime.js', import.meta.url), 'utf8');
const agentPage = await readFile(new URL('../screens/07-agent-runs.html', import.meta.url), 'utf8');

const tools = [
  { name: 'read_state', description: 'Read state', side_effect: 'readonly' },
  { name: 'delete_session', description: 'Delete session', side_effect: 'destructive' },
  { name: 'append_message', description: 'Append message', side_effect: 'append' },
];

function input(overrides = {}) {
  return {
    characterId: 'alice',
    sessionId: 'session-1',
    userId: 'operator',
    message: 'inspect state',
    maxSteps: 4,
    toolAuthorityEnabled: true,
    tools,
    selectedTools: [],
    confirmedTools: [],
    ...overrides,
  };
}

test('ordinary generation grants no tool capability or allowlist', () => {
  const body = buildRequest(input({ selectedTools: ['read_state'], toolAuthorityEnabled: false }));
  assert.deepEqual(body.capabilities, []);
  assert.deepEqual(body.allowed_tools, []);
  assert.deepEqual(body.confirm_tools, []);
});

test('Agent page delegates its live run to the tested request/SSE controller', () => {
  assert.ok(
    agentPage.indexOf('assets/agent-run.js') < agentPage.indexOf('assets/console-runtime.js'),
    'agent-run controller must load before the page runtime',
  );
  assert.match(runtime, /AIRPAgentRun\.run\(client,/);
  assert.match(runtime, /Boolean\(settings\.access_api_key_set\)/);
  assert.doesNotMatch(runtime, /client\.stream\('\/v1\/agent\/run'/);
});

test('selected tools produce the minimum call:tool request body', () => {
  const body = buildRequest(input({ selectedTools: ['read_state'] }));
  assert.deepEqual(body, {
    character_id: 'alice',
    session_id: 'session-1',
    user_id: 'operator',
    user_profile: { name: 'operator', variables: {} },
    message: 'inspect state',
    max_steps: 4,
    capabilities: ['call:tool'],
    allowed_tools: ['read_state'],
    confirm_tools: [],
  });
});

test('confirm_tools only contains selected destructive tools', () => {
  const body = buildRequest(input({
    selectedTools: ['read_state', 'delete_session'],
    confirmedTools: ['read_state', 'delete_session', 'unknown'],
  }));
  assert.deepEqual(body.allowed_tools, ['read_state', 'delete_session']);
  assert.deepEqual(body.confirm_tools, ['delete_session']);
});

test('run forwards the real body and exposes readable SSE state', async () => {
  let captured;
  const seen = [];
  const client = {
    async stream(path, body, handlers) {
      captured = { path, body, signal: handlers.signal };
      handlers.onChunk({ type: 'plan', step: 1, action: 'call_tool' });
      handlers.onChunk({ type: 'tool_call', step: 1, tool: 'read_state', params: {} });
      handlers.onChunk({ type: 'tool_result', step: 1, tool: 'read_state', output: { ok: true }, dry_run: false });
      handlers.onChunk({ type: 'delta', step: 2, chunk: '完成' });
      handlers.onDone({ type: 'done', stop_reason: 'completed', steps_taken: 2, tokens_estimated: 12 });
      return { completed: true };
    },
  };
  const signal = new AbortController().signal;
  await run(client, input({ selectedTools: ['read_state'] }), {
    signal,
    onEvent: (event, text) => seen.push([event.type, text]),
  });

  assert.equal(captured.path, '/v1/agent/run');
  assert.equal(captured.signal, signal);
  assert.deepEqual(captured.body.capabilities, ['call:tool']);
  assert.deepEqual(captured.body.allowed_tools, ['read_state']);
  assert.deepEqual(seen.map(([type]) => type), ['plan', 'tool_call', 'tool_result', 'delta', 'done']);
  assert.match(seen[1][1], /调用工具 read_state/);
  assert.match(seen[2][1], /已执行/);
  assert.equal(seen[3][1], '完成');
  assert.match(seen[4][1], /2 步/);
});

test('dry-run SSE state is explicit before destructive confirmation', () => {
  assert.match(describeEvent({
    type: 'tool_result', step: 1, tool: 'delete_session', output: { would_delete: true }, dry_run: true,
  }), /仅演练，尚未执行/);
});
