import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { AirpHttpError, AirpStreamError, consumeSse, createClient, errorMessage, parseSseBlock } = require('../assets/api-client.js');

function response(body, options = {}) {
  return new Response(body, { status: options.status || 200, headers: options.headers });
}

test('request sends JSON to the configured same-origin base', async () => {
  let call;
  const client = createClient({
    base: 'http://engine.test/',
    fetchImpl: async (url, init) => {
      call = { url, init };
      return response(JSON.stringify(['alice']));
    },
  });
  const result = await client.request('POST', '/v1/chat/history', { character_id: 'alice' });
  assert.deepEqual(result, ['alice']);
  assert.equal(call.url, 'http://engine.test/v1/chat/history');
  assert.equal(call.init.body, '{"character_id":"alice"}');
  assert.equal(call.init.headers.Authorization, undefined);
});

test('bearer is only added as an Authorization header', async () => {
  let call;
  const client = createClient({
    base: 'http://engine.test', bearer: 'secret-value',
    fetchImpl: async (url, init) => { call = { url, init }; return response('{}'); },
  });
  await client.request('GET', '/version');
  assert.equal(call.url, 'http://engine.test/version');
  assert.equal(call.init.headers.Authorization, 'Bearer secret-value');
  assert.ok(!call.url.includes('secret-value'));
});

test('request applies its configurable default timeout', async () => {
  const client = createClient({
    base: 'http://engine.test',
    requestTimeoutMs: 5,
    fetchImpl: async (_url, init) => new Promise((resolve, reject) => {
      init.signal.addEventListener('abort', () => reject(init.signal.reason), { once: true });
    }),
  });
  await assert.rejects(client.request('GET', '/health'), error => error && error.name === 'TimeoutError');
});

test('stream applies its separately configurable default timeout', async () => {
  const client = createClient({
    base: 'http://engine.test',
    streamTimeoutMs: 5,
    fetchImpl: async (_url, init) => new Promise((resolve, reject) => {
      init.signal.addEventListener('abort', () => reject(init.signal.reason), { once: true });
    }),
  });
  await assert.rejects(client.stream('/v1/chat/completions', {}, {}), error => error && error.name === 'TimeoutError');
});

test('caller signal takes precedence over the default timeout', async () => {
  const controller = new AbortController();
  let receivedSignal;
  const client = createClient({
    base: 'http://engine.test',
    requestTimeoutMs: 5,
    fetchImpl: async (_url, init) => {
      receivedSignal = init.signal;
      return response('{}');
    },
  });
  await client.request('GET', '/health', undefined, { signal: controller.signal });
  assert.equal(receivedSignal, controller.signal);
});

test('non-2xx JSON becomes a structured AirpHttpError', async () => {
  const client = createClient({
    base: 'http://engine.test',
    fetchImpl: async () => response(JSON.stringify({ detail: '角色不存在' }), { status: 404 }),
  });
  await assert.rejects(
    client.request('GET', '/v1/characters/missing'),
    error => error instanceof AirpHttpError && error.status === 404 && error.message === '角色不存在',
  );
});

test('SSE parser supports event name and multiline data', () => {
  assert.deepEqual(parseSseBlock('event: message\ndata: {"type":"body_chunk",\ndata: "text":"hi"}'), {
    event: 'message', data: '{"type":"body_chunk",\n"text":"hi"}',
  });
});

test('consumeSse emits body chunks and completion', async () => {
  const chunks = [];
  let done = 0;
  const body = 'event: message\ndata: {"type":"body_chunk","text":"你"}\n\n' +
    'event: message\ndata: {"type":"body_chunk","text":"好"}\n\n' +
    'event: message\ndata: {"type":"done"}\n\n';
  const result = await consumeSse(response(body), {
    onChunk: chunk => chunks.push(chunk.text),
    onDone: () => { done += 1; },
  });
  assert.deepEqual(chunks, ['你', '好']);
  assert.equal(done, 1);
  assert.equal(result.completed, true);
});

test('SSE error preserves commit_state so UI never suggests blind retry', async () => {
  const body = 'event: error\ndata: {"type":"error","message":"上游失败","retryable":false,"commit_state":"partially_committed"}\n\n';
  await assert.rejects(
    consumeSse(response(body), {}),
    error => error instanceof AirpStreamError && error.commitState === 'partially_committed' && error.retryable === false,
  );
});

test('SSE ending without a done event is an uncertain non-retryable failure', async () => {
  const body = 'event: message\ndata: {"type":"body_chunk","text":"partial"}\n\n';
  await assert.rejects(
    consumeSse(response(body), {}),
    error => error instanceof AirpStreamError && error.code === 'stream_incomplete' && error.commitState === 'unknown' && error.retryable === false,
  );
});

test('stream transport failure is reported with unknown commit state', async () => {
  const body = new ReadableStream({
    start(controller) {
      controller.enqueue(new TextEncoder().encode('event: message\ndata: {"type":"body_chunk","text":"partial"}\n\n'));
      controller.error(new Error('connection reset'));
    },
  });
  const client = createClient({ base: 'http://engine.test', fetchImpl: async () => response(body) });
  await assert.rejects(
    client.stream('/v1/chat/completions', {}, {}),
    error => error instanceof AirpStreamError && error.code === 'stream_transport' && error.commitState === 'unknown' && error.retryable === false,
  );
});

test('errorMessage exposes only a useful public string', () => {
  assert.equal(errorMessage({ message: '失败', api_key: 'secret' }), '失败');
  assert.equal(errorMessage(null, '请求失败'), '请求失败');
});
