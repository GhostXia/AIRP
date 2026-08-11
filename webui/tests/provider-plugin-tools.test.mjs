import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const asset = name => readFile(new URL('../assets/' + name, import.meta.url), 'utf8');
const screen = name => readFile(new URL('../screens/' + name, import.meta.url), 'utf8');
const [providerScript, pluginScript, providerHtml, pluginHtml] = await Promise.all([
  asset('provider-management.js'),
  asset('plugin-tools.js'),
  screen('43-provider-management.html'),
  screen('44-plugin-tools.html'),
]);

class FakeElement {
  constructor(tagName = 'div', selectorChildren = {}) {
    this.tagName = tagName.toUpperCase();
    this.children = [];
    this.selectorChildren = new Map(Object.entries(selectorChildren));
    this.listeners = new Map();
    this.className = '';
    this.classList = this.makeClassList();
    this.dataset = {};
    this.style = {};
    this.textContent = '';
    this._value = '';
    this.checked = false;
    this.disabled = false;
    this.hidden = false;
    this.open = false;
    this.parentElement = null;
    this._innerHTML = '';
  }

  makeClassList() {
    const names = new Set();
    return {
      add: (...values) => values.forEach(value => names.add(value)),
      remove: (...values) => values.forEach(value => names.delete(value)),
      toggle: (value, force) => {
        const next = force === undefined ? !names.has(value) : Boolean(force);
        if (next) names.add(value); else names.delete(value);
        return next;
      },
      contains: value => names.has(value),
    };
  }

  set innerHTML(value) {
    this._innerHTML = String(value);
    if (this._innerHTML === '') this.children = [];
  }

  get innerHTML() { return this._innerHTML; }

  set value(value) { this._value = String(value ?? ''); }
  get value() { return this._value; }

  append(...children) {
    for (const child of children) {
      this.children.push(child);
      if (child && typeof child === 'object') child.parentElement = this;
    }
  }

  appendChild(child) {
    this.append(child);
    return child;
  }

  replaceChildren(...children) {
    this.children = [];
    this.append(...children);
  }

  querySelector(selector) {
    if (this.selectorChildren.has(selector)) return this.selectorChildren.get(selector);
    for (const child of this.children) {
      if (child && typeof child.querySelector === 'function') {
        const match = child.querySelector(selector);
        if (match) return match;
      }
    }
    return null;
  }

  addEventListener(type, listener) {
    const listeners = this.listeners.get(type) || [];
    listeners.push(listener);
    this.listeners.set(type, listeners);
  }

  async dispatch(type, event = {}) {
    const dispatched = {
      target: this,
      preventDefault() {},
      ...event,
    };
    const results = [];
    for (const listener of this.listeners.get(type) || []) {
      results.push(listener(dispatched));
    }
    await Promise.all(results);
  }

  showModal() { this.open = true; }
  close() { this.open = false; }
  scrollIntoView() {}
}

function makeRow(selectors) {
  const row = new FakeElement('tr');
  for (const [selector, element] of Object.entries(selectors)) {
    row.selectorChildren.set(selector, element);
  }
  row.cloneNode = () => makeRow(Object.fromEntries(
    [...row.selectorChildren.entries()].map(([selector]) => [selector, new FakeElement('span')]),
  ));
  return row;
}

function makeDocument(elements) {
  const document = {
    readyState: 'complete',
    querySelector(selector) {
      return elements.get(selector) || null;
    },
    createElement(tagName) { return new FakeElement(tagName); },
    addEventListener() {},
  };
  return document;
}

function makeFieldRow() {
  const parent = new FakeElement('div');
  const input = new FakeElement('input');
  input.parentElement = parent;
  parent.selectorChildren.set('input', input);
  return parent;
}

function createProviderDom() {
  const elements = new Map();
  const add = (selector, element = new FakeElement()) => {
    elements.set(selector, element);
    return element;
  };
  add('#console-nav');
  add('#related-links');
  add('#engine-address');
  add('#engine-status');
  add('#runtime-status');
  add('#pm-rows', new FakeElement('tbody'));
  add('#pm-count');
  add('#pm-enabled-bar');
  add('#pm-enabled-tag');
  add('#pm-enabled-hint');
  add('#pm-table');
  add('#pm-empty');
  const providerRow = makeRow({
    '.pm-cell-name': new FakeElement('td'),
    '.pm-cell-endpoint code': new FakeElement('code'),
    '.pm-cell-model': new FakeElement('td'),
    '.pm-cell-engine': new FakeElement('td'),
    '.pm-cell-default': new FakeElement('td'),
    '.pm-cell-key': new FakeElement('td'),
    '.pm-edit': new FakeElement('button'),
    '.pm-delete': new FakeElement('button'),
  });
  const rowTemplate = add('#pm-row-template', new FakeElement('template'));
  rowTemplate.content = { firstElementChild: providerRow };
  for (const selector of [
    '#pm-default-provider', '#pm-bc-value', '#pm-bsr-value', '#pm-btk-value',
  ]) {
    const field = add(selector, new FakeElement('select'));
    field.parentElement = makeFieldRow();
  }
  for (const selector of ['#pm-bc-key', '#pm-bsr-key', '#pm-btk-key']) add(selector, new FakeElement('input'));
  for (const selector of ['#pm-bc-list', '#pm-bsr-list', '#pm-btk-list']) add(selector, new FakeElement('ul'));
  add('#pm-editor', new FakeElement('dialog'));
  add('#pm-editor-title');
  add('#pm-editor-hint');
  add('#pm-ed-name', new FakeElement('input'));
  add('#pm-ed-endpoint', new FakeElement('input'));
  add('#pm-ed-model', new FakeElement('input'));
  add('#pm-ed-engine', new FakeElement('select'));
  add('#pm-ed-default', new FakeElement('input'));
  add('#pm-ed-apikey', new FakeElement('input'));
  add('#pm-ed-clear-apikey', new FakeElement('button'));
  add('#pm-ed-save', new FakeElement('button'));
  add('#pm-ed-cancel', new FakeElement('button'));
  add('#pm-editor-form', new FakeElement('form'));
  add('#pm-add', new FakeElement('button'));
  add('#pm-reload', new FakeElement('button'));
  add('#pm-save-routing', new FakeElement('button'));
  add('.pm-section-actions', new FakeElement('div'));
  for (const selector of ['#pm-bc-add', '#pm-bsr-add', '#pm-btk-add']) add(selector, new FakeElement('button'));
  for (const selector of ['#pm-rs-char', '#pm-rs-role', '#pm-rs-task']) add(selector, new FakeElement('input'));
  for (const selector of ['#pm-rs-result', '#pm-rs-status', '#pm-rs-rule', '#pm-rs-entry']) add(selector);
  add('#pm-rs-run', new FakeElement('button'));
  add('#pm-reset-routing', new FakeElement('button'));
  return makeDocument(elements);
}

function providerResponse() {
  return {
    entries: [{
      name: 'primary',
      endpoint: 'https://example.test/v1',
      model: 'test-model',
      engine: 'direct',
      is_default: true,
      api_key_set: true,
    }],
    routing: { default_provider: 'primary', by_character: {}, by_scene_role: {}, by_task_kind: {} },
    enabled: true,
  };
}

async function startProvider({ confirmResult = false } = {}) {
  const document = createProviderDom();
  const requests = [];
  const confirmations = [];
  const response = providerResponse();
  const client = {
    base: 'http://engine.test',
    async request(method, path, body) {
      requests.push({ method, path, body });
      if (method === 'GET' && path === '/health') return { ok: true };
      if (method === 'GET' && path === '/v1/providers') return response;
      if (method === 'POST' && path === '/v1/providers') return response;
      if (method === 'GET' && path.startsWith('/v1/providers/resolve')) return { matched: false };
      throw new Error(`unexpected request ${method} ${path}`);
    },
  };
  const context = {
    document,
    location: { origin: 'http://console.test', href: 'http://console.test/43-provider-management.html', search: '' },
    URL,
    URLSearchParams,
    sessionStorage: { getItem: () => null, setItem() {} },
    AIRPApi: { createClient: () => client },
    AIRPConfirm: {
      async confirm(...args) {
        confirmations.push(args);
        return confirmResult;
      },
    },
    setInterval() {},
    console,
    globalThis: null,
  };
  context.globalThis = context;
  vm.runInNewContext(providerScript, context);
  await new Promise(resolve => setImmediate(resolve));
  return { document, requests, confirmations };
}

async function openProviderEditor(env) {
  const row = env.document.querySelector('#pm-rows').children[0];
  assert.ok(row, 'provider row must be rendered');
  await row.querySelector('.pm-edit').dispatch('click');
}

async function saveProviderDraft(env) {
  await env.document.querySelector('#pm-ed-save').dispatch('click');
  return saveAllProviderDraft(env);
}

async function saveAllProviderDraft(env) {
  const actions = env.document.querySelector('.pm-section-actions').children;
  const saveAll = actions.at(-1);
  assert.ok(saveAll, 'save-all button must be injected');
  await saveAll.dispatch('click');
  return env.requests.find(request => request.method === 'POST' && request.path === '/v1/providers');
}

test('provider api_key preserves blank edits, updates non-empty edits, and clears explicitly', async () => {
  const preserve = await startProvider();
  await openProviderEditor(preserve);
  assert.equal(preserve.document.querySelector('#pm-ed-clear-apikey').hidden, false);
  preserve.document.querySelector('#pm-ed-apikey').value = '';
  const preserveRequest = await saveProviderDraft(preserve);
  assert.equal(Object.hasOwn(preserveRequest.body.entries[0], 'api_key'), false);

  const update = await startProvider();
  await openProviderEditor(update);
  update.document.querySelector('#pm-ed-apikey').value = '  replacement-secret  ';
  const updateRequest = await saveProviderDraft(update);
  assert.equal(updateRequest.body.entries[0].api_key, 'replacement-secret');

  const clear = await startProvider({ confirmResult: true });
  await openProviderEditor(clear);
  await clear.document.querySelector('#pm-ed-clear-apikey').dispatch('click');
  assert.equal(clear.confirmations.length, 1);
  assert.match(clear.confirmations[0][0], /清空/);
  const clearRequest = await saveProviderDraft(clear);
  assert.equal(clearRequest.body.entries[0].api_key, '');
});

test('cancelling provider api_key clear leaves the draft unchanged', async () => {
  const env = await startProvider({ confirmResult: false });
  await openProviderEditor(env);
  const keyInput = env.document.querySelector('#pm-ed-apikey');
  keyInput.value = 'unsaved-replacement';
  await env.document.querySelector('#pm-ed-clear-apikey').dispatch('click');
  assert.equal(keyInput.value, 'unsaved-replacement');
  await env.document.querySelector('#pm-ed-cancel').dispatch('click');
  const request = await saveAllProviderDraft(env);
  assert.equal(Object.hasOwn(request.body.entries[0], 'api_key'), false);
});

function createPluginDom() {
  const elements = new Map();
  const add = (selector, element = new FakeElement()) => {
    elements.set(selector, element);
    return element;
  };
  for (const selector of [
    '#console-nav', '#related-links', '#engine-address', '#engine-address-ctx',
    '#runtime-status', '#engine-status', '#pt-count', '#pt-enabled-bar',
    '#pt-enabled-tag', '#pt-enabled-hint', '#pt-table', '#pt-empty',
  ]) add(selector);
  add('#pt-rows', new FakeElement('tbody'));
  const pluginRow = makeRow({
    '.pt-cell-name': new FakeElement('td'),
    '.pt-cell-invocation code': new FakeElement('code'),
    '.pt-cell-side-effect': new FakeElement('td'),
    '.pt-toggle': new FakeElement('button'),
    '.pt-test': new FakeElement('button'),
    '.pt-edit': new FakeElement('button'),
    '.pt-delete': new FakeElement('button'),
  });
  const rowTemplate = add('#pt-row-template', new FakeElement('template'));
  rowTemplate.content = { firstElementChild: pluginRow };
  add('#pt-test-name', new FakeElement('select'));
  add('#pt-test-params', new FakeElement('textarea'));
  add('#pt-test-confirm', new FakeElement('input'));
  add('#pt-test-output');
  add('#pt-test-run', new FakeElement('button'));
  add('#pt-test-clear', new FakeElement('button'));
  add('#pt-add', new FakeElement('button'));
  add('#pt-reload', new FakeElement('button'));
  add('#pt-editor', new FakeElement('dialog'));
  add('#pt-editor-title');
  add('#pt-editor-hint');
  add('#pt-editor-form', new FakeElement('form'));
  add('#pt-ed-name', new FakeElement('input'));
  add('#pt-ed-description', new FakeElement('textarea'));
  add('#pt-ed-side-effect', new FakeElement('select'));
  add('#pt-ed-enabled', new FakeElement('input'));
  add('#pt-ed-kind', new FakeElement('select'));
  add('#pt-inv-webhook', new FakeElement('fieldset'));
  add('#pt-inv-script', new FakeElement('fieldset'));
  add('#pt-ed-wh-url', new FakeElement('input'));
  add('#pt-ed-wh-timeout', new FakeElement('input'));
  add('#pt-ed-wh-headers', new FakeElement('textarea'));
  add('#pt-ed-sc-path', new FakeElement('input'));
  add('#pt-ed-sc-args', new FakeElement('textarea'));
  add('#pt-ed-sc-timeout', new FakeElement('input'));
  add('#pt-ed-cancel', new FakeElement('button'));
  return makeDocument(elements);
}

async function startPluginTools() {
  const document = createPluginDom();
  const requests = [];
  const response = {
    tools: [{
      name: 'signed_hook',
      description: 'signed webhook',
      side_effect: 'readonly',
      enabled: true,
      invocation: {
        kind: 'webhook',
        url: 'https://example.test/hook',
        headers_set: true,
        headers_keys: ['Authorization', 'X-Request-ID'],
        timeout_secs: 10,
      },
    }],
    total: 1,
    enabled: 1,
  };
  const client = {
    base: 'http://engine.test',
    async request(method, path, body) {
      requests.push({ method, path, body });
      if (method === 'GET' && path === '/health') return { ok: true };
      if (method === 'GET' && path === '/v1/plugin-tools') return response;
      if (method === 'POST' && path === '/v1/plugin-tools') return response.tools[0];
      throw new Error(`unexpected request ${method} ${path}`);
    },
  };
  const context = {
    document,
    location: { origin: 'http://console.test', href: 'http://console.test/44-plugin-tools.html', search: '' },
    URL,
    URLSearchParams,
    sessionStorage: { getItem: () => null, setItem() {} },
    AIRPApi: { createClient: () => client },
    AIRPConfirm: { confirm: async () => true },
    setInterval() {},
    console,
    globalThis: null,
  };
  context.globalThis = context;
  vm.runInNewContext(pluginScript, context);
  await new Promise(resolve => setImmediate(resolve));
  return { document, requests };
}

test('plugin tool editor shows header names while preserving headers on blank edit', async () => {
  const env = await startPluginTools();
  const row = env.document.querySelector('#pt-rows').children[0];
  await row.querySelector('.pt-edit').dispatch('click');

  const hint = env.document.querySelector('#pt-editor-hint').textContent;
  assert.match(hint, /Authorization/);
  assert.match(hint, /X-Request-ID/);
  assert.doesNotMatch(hint, /Bearer|secret|value/i);
  assert.equal(env.document.querySelector('#pt-ed-wh-headers').value, '');

  await env.document.querySelector('#pt-editor-form').dispatch('submit');
  const request = env.requests.find(item => item.method === 'POST' && item.path === '/v1/plugin-tools');
  assert.equal(Object.hasOwn(request.body.invocation, 'headers'), false);
});

test('plugin tool editor sends entered headers for replacement', async () => {
  const env = await startPluginTools();
  const row = env.document.querySelector('#pt-rows').children[0];
  await row.querySelector('.pt-edit').dispatch('click');
  env.document.querySelector('#pt-ed-wh-headers').value = 'Authorization: replacement-token';

  await env.document.querySelector('#pt-editor-form').dispatch('submit');
  const request = env.requests.find(item => item.method === 'POST' && item.path === '/v1/plugin-tools');
  assert.deepEqual(Object.keys(request.body.invocation.headers), ['Authorization']);
  assert.equal(request.body.invocation.headers.Authorization, 'replacement-token');
});

test('plugin tool contracts expose only sorted header names', () => {
  assert.match(providerHtml, /id="pm-ed-clear-apikey"/);
  assert.match(pluginHtml, /显示已有 header 名/);
  assert.match(pluginScript, /headers_keys/);
});
