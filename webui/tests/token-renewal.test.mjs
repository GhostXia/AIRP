import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { createClient } = require('../assets/api-client.js');
const desktopSession = require('../assets/desktop-session.js');

// C-P2 token 续期链路（webui 侧）契约测试：
// - api-client bearer 函数形态：每次请求解析，rotation 后新值即刻生效；
// - 401 → onUnauthorized → truthy 单次重试 / falsy 不重试 / 至多重试一次；
// - desktop-session.js rotation 成功写 sessionStorage + 派发事件；失败返回
//   false；并发 401 以 in-flight Promise 去重。

function response(body, options = {}) {
  return new Response(body, { status: options.status || 200, headers: options.headers });
}

function memorySessionStorage() {
  const store = new Map();
  return {
    getItem: (key) => (store.has(key) ? store.get(key) : null),
    setItem: (key, value) => { store.set(key, String(value)); },
    removeItem: (key) => { store.delete(key); },
  };
}

// 审计 #485 W2：replace globalThis.sessionStorage 后直接 delete，若环境预置了
// 该 global（如未来 jsdom 预设 / 并行测试垫片）会清掉其状态，产生顺序依赖。
// 保存并还原原 descriptor，让每个测试对 global 的介入可逆。
function installSessionStorage(storage) {
  const prev = Object.getOwnPropertyDescriptor(globalThis, 'sessionStorage');
  globalThis.sessionStorage = storage;
  return () => {
    if (prev) Object.defineProperty(globalThis, 'sessionStorage', prev);
    else delete globalThis.sessionStorage;
  };
}

test('C-P2: function bearer is resolved per request (rotation-ready)', async () => {
  let token = 'token-1';
  const seen = [];
  const client = createClient({
    base: 'http://engine.test',
    bearer: () => token,
    trustedOrigins: ['http://engine.test'],
    fetchImpl: async (_url, init) => { seen.push(init.headers.Authorization); return response('{}'); },
  });
  await client.request('GET', '/version');
  token = 'token-2'; // 模拟 rotation 后 sessionStorage 已换新值
  await client.request('GET', '/version');
  assert.deepEqual(seen, ['Bearer token-1', 'Bearer token-2']);
});

test('C-P2: 401 triggers onUnauthorized and a single retry with the renewed bearer', async () => {
  let token = 'stale';
  const calls = [];
  let renewed = false;
  const client = createClient({
    base: 'http://engine.test',
    bearer: () => token,
    trustedOrigins: ['http://engine.test'],
    onUnauthorized: () => { renewed = true; token = 'fresh'; return true; },
    fetchImpl: async (_url, init) => {
      calls.push(init.headers.Authorization);
      return init.headers.Authorization === 'Bearer fresh'
        ? response('{"ok":true}')
        : response('{"detail":"expired"}', { status: 401 });
    },
  });
  const result = await client.request('GET', '/v1/settings');
  assert.deepEqual(result, { ok: true });
  assert.equal(renewed, true);
  assert.deepEqual(calls, ['Bearer stale', 'Bearer fresh']);
});

test('C-P2: retry happens at most once (no renewal loop on persistent 401)', async () => {
  let hookCalls = 0;
  const client = createClient({
    base: 'http://engine.test',
    bearer: 'whatever',
    trustedOrigins: ['http://engine.test'],
    onUnauthorized: () => { hookCalls += 1; return true; },
    fetchImpl: async () => response('{"detail":"expired"}', { status: 401 }),
  });
  await assert.rejects(() => client.request('GET', '/v1/settings'));
  assert.equal(hookCalls, 1, '持续 401 时钩子只允许触发一次');
});

test('C-P2: onUnauthorized returning falsy skips the retry (keyless fallback)', async () => {
  const calls = [];
  const client = createClient({
    base: 'http://engine.test',
    bearer: 'whatever',
    trustedOrigins: ['http://engine.test'],
    onUnauthorized: () => false, // 无 key 模式：renew 403 → 回退既有行为
    fetchImpl: async (_url, init) => {
      calls.push(init.headers.Authorization);
      return response('{"detail":"expired"}', { status: 401 });
    },
  });
  await assert.rejects(() => client.request('GET', '/v1/settings'));
  assert.equal(calls.length, 1, 'falsy 钩子不得触发重试');
});

test('C-P2: stream start also honors 401 renewal retry', async () => {
  let token = 'stale';
  const auths = [];
  const sse = 'data: {"type":"delta","text":"hi"}\n\ndata: {"type":"done"}\n\n';
  const client = createClient({
    base: 'http://engine.test',
    bearer: () => token,
    trustedOrigins: ['http://engine.test'],
    onUnauthorized: () => { token = 'fresh'; return true; },
    fetchImpl: async (_url, init) => {
      auths.push(init.headers.Authorization);
      if (init.headers.Authorization === 'Bearer fresh') {
        return response(sse, { headers: { 'Content-Type': 'text/event-stream' } });
      }
      return response('{"detail":"expired"}', { status: 401 });
    },
  });
  const chunks = [];
  let done = false;
  await client.stream('/v1/chat/continue', {}, {
    onChunk: (payload) => chunks.push(payload),
    onDone: () => { done = true; },
  });
  assert.deepEqual(auths, ['Bearer stale', 'Bearer fresh']);
  assert.equal(chunks.length, 1);
  assert.equal(done, true);
});

test('C-P2: renewDesktopSession rotates, stores the new bearer and dispatches the event', async () => {
  const storage = memorySessionStorage();
  storage.setItem(desktopSession.STORAGE_KEY, 'old-token');
  const restoreStorage = installSessionStorage(storage);
  // Node 无浏览器全局事件 API：以 EventTarget 垫片承接 dispatchEvent。
  const bus = new EventTarget();
  const prevDispatch = globalThis.dispatchEvent;
  globalThis.dispatchEvent = (event) => bus.dispatchEvent(event);
  let dispatched = null;
  const listener = (event) => { dispatched = event; };
  bus.addEventListener('airp-bearer-renewed', listener);
  let call;
  try {
    const ok = await desktopSession.renewDesktopSession({
      base: 'http://engine.test/',
      fetchImpl: async (url, init) => {
        call = { url, init };
        return response(JSON.stringify({ token: 'new-token', token_type: 'Bearer', expires_in: 28800 }));
      },
    });
    assert.equal(ok, true);
    assert.equal(call.url, 'http://engine.test/v1/desktop-session/renew');
    assert.equal(call.init.method, 'POST');
    assert.equal(call.init.headers.Authorization, 'Bearer old-token');
    assert.equal(storage.getItem(desktopSession.STORAGE_KEY), 'new-token');
    assert.ok(dispatched, 'airp-bearer-renewed 事件必须派发');
    assert.equal(dispatched.detail.expires_in, 28800);
  } finally {
    bus.removeEventListener('airp-bearer-renewed', listener);
    if (prevDispatch) globalThis.dispatchEvent = prevDispatch; else delete globalThis.dispatchEvent;
    restoreStorage();
  }
});

test('C-P2: renewDesktopSession returns false on failure and keeps the old bearer', async () => {
  const storage = memorySessionStorage();
  storage.setItem(desktopSession.STORAGE_KEY, 'old-token');
  const restoreStorage = installSessionStorage(storage);
  try {
    const ok = await desktopSession.renewDesktopSession({
      base: 'http://engine.test',
      fetchImpl: async () => response('{"error":{"code":"token_invalid"}}', { status: 401 }),
    });
    assert.equal(ok, false);
    assert.equal(storage.getItem(desktopSession.STORAGE_KEY), 'old-token');
  } finally {
    restoreStorage();
  }
});

test('C-P2: renewDesktopSession returns false without a bearer (no-key mode)', async () => {
  const storage = memorySessionStorage();
  const restoreStorage = installSessionStorage(storage);
  try {
    const ok = await desktopSession.renewDesktopSession({
      base: 'http://engine.test',
      fetchImpl: async () => { throw new Error('must not be called'); },
    });
    assert.equal(ok, false);
  } finally {
    restoreStorage();
  }
});

test('C-P2: concurrent 401s share one in-flight renewal (dedupe)', async () => {
  const storage = memorySessionStorage();
  storage.setItem(desktopSession.STORAGE_KEY, 'old-token');
  const restoreStorage = installSessionStorage(storage);
  let fetchCount = 0;
  try {
    const fetchImpl = async () => {
      fetchCount += 1;
      await new Promise((resolve) => setTimeout(resolve, 10));
      return response(JSON.stringify({ token: 'new-token', expires_in: 100 }));
    };
    const [a, b, c] = await Promise.all([
      desktopSession.renewDesktopSession({ base: 'http://engine.test', fetchImpl }),
      desktopSession.renewDesktopSession({ base: 'http://engine.test', fetchImpl }),
      desktopSession.renewDesktopSession({ base: 'http://engine.test', fetchImpl }),
    ]);
    assert.deepEqual([a, b, c], [true, true, true]);
    assert.equal(fetchCount, 1, 'rotation 语义下并发续期必须去重为单次请求');
  } finally {
    restoreStorage();
  }
});
