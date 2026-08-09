import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const groupChatScript = await readFile(new URL('../assets/group-chat.js', import.meta.url), 'utf8');

function createElement(tagName, textContent = '') {
  const element = {
    tagName: tagName.toUpperCase(),
    textContent,
    value: '',
    className: '',
    disabled: false,
    style: {},
    children: [],
    listeners: new Map(),
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    append(...children) {
      children.forEach(child => this.appendChild(child));
    },
    replaceChildren(...children) {
      this.children = [...children];
    },
    addEventListener(type, handler) {
      this.listeners.set(type, handler);
    },
  };
  if (tagName === 'status') element.lastChild = { textContent: '' };
  return element;
}

function createHarness({ sessionResult, sessionError = null, streamHandler = null } = {}) {
  const elements = {
    '#group-flow': createElement('div'),
    '#group-input': createElement('textarea'),
    '#group-send': createElement('button'),
    '#group-status': createElement('div'),
    '#scene-list': createElement('div'),
    '#scene-chars': createElement('div'),
    '#engine-status': createElement('status'),
  };
  const requests = [];
  const streams = [];
  const storage = new Map();
  const client = {
    async request(method, path, body) {
      requests.push({ method, path, body });
      if (path === '/health') return { ok: true };
      if (path === '/v1/scenes') return ['scene-1'];
      if (path === '/v1/scenes/scene-1') {
        return { scene_id: 'scene-1', description: '测试场景', characters: ['char-a'] };
      }
      if (path === '/v1/sessions/char-a') {
        if (sessionError) throw sessionError;
        return sessionResult;
      }
      throw new Error('unexpected request: ' + method + ' ' + path);
    },
    async stream(path, body, handlers) {
      streams.push({ path, body });
      if (streamHandler) await streamHandler(handlers);
    },
  };
  const context = {
    AIRPApi: {
      createClient() { return client; },
      errorMessage(data, fallback) { return (data && data.message) || fallback || '请求失败'; },
    },
    URLSearchParams,
    encodeURIComponent,
    location: { origin: 'https://example.test', search: '' },
    sessionStorage: {
      getItem(key) { return storage.get(key) || null; },
      setItem(key, value) { storage.set(key, String(value)); },
    },
    document: {
      querySelector(selector) { return elements[selector] || null; },
      createElement(tagName) { return createElement(tagName); },
      createTextNode(text) { return createElement('text', text); },
    },
  };
  vm.runInNewContext(groupChatScript, context);
  return { elements, requests, streams };
}

function flush() {
  return new Promise(resolve => setImmediate(resolve));
}

test('group chat keeps the session unavailable and blocks streaming when session creation rejects', async () => {
  const harness = createHarness({ sessionError: new Error('角色不存在') });
  await flush();

  assert.equal(harness.elements['#group-status'].textContent, '会话创建失败，请重试');
  assert.equal(harness.requests.filter(request => request.path === '/v1/sessions/char-a').length, 1);

  harness.elements['#group-input'].value = '无法发送的消息';
  harness.elements['#group-send'].listeners.get('click')();
  await flush();

  assert.equal(harness.streams.length, 0, 'sendGroupMessage must not stream without a sessionId');
  assert.equal(harness.elements['#group-input'].value, '无法发送的消息', 'blocked send must leave input untouched');
});

test('group chat keeps normal session creation and per-character streaming behavior', async () => {
  const harness = createHarness({
    sessionResult: 'session-1',
    streamHandler: async handlers => {
      handlers.onChunk({ type: 'body_chunk', text: '正常回复' });
      handlers.onDone();
    },
  });
  await flush();

  assert.equal(harness.elements['#group-status'].textContent, '场景: scene-1 · 1 个角色');
  assert.equal(harness.requests.filter(request => request.path === '/v1/sessions/char-a').length, 1);

  harness.elements['#group-input'].value = '正常消息';
  harness.elements['#group-send'].listeners.get('click')();
  await flush();

  assert.equal(harness.streams.length, 1);
  assert.equal(harness.streams[0].path, '/v1/chat/completions');
  assert.equal(harness.streams[0].body.session_id, 'session-1');
  assert.equal(harness.elements['#group-input'].value, '');
});
