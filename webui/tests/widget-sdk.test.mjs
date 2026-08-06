// C-P4-4：Widget SDK 单元测试。
//
// 覆盖：
//   1. createWidget —— 工厂包装 / 错误捕获 / async mount / debug 日志 / 无 unmount
//   2. defineManifest —— 合法校验 / host_api 语义（与 engine 同语义）/ sandbox 强制 / 冻结
//   3. h —— DOM 构建 / 属性 / 事件 / style / dataset / null 过滤
//   4. 示例包自洽（审计 #489 W3）—— example-manifest.json 按自身 files 清单可走通安装校验
//
// SDK 是纯 ESM，测试直接 import。h 函数依赖 document，用最小 fake DOM。
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, access } from 'node:fs/promises';
import { createWidget, defineManifest, h } from '../assets/widgets/sdk/widget-sdk.js';

// ── 最小 fake DOM（只覆盖 h 函数用到的面） ───────────────────────────────
function installFakeDom() {
  function make(tag) {
    const el = {
      tagName: tag.toUpperCase(),
      children: [],
      attributes: {},
      style: {},
      dataset: {},
      listeners: {},
      append(...kids) {
        for (const k of kids) {
          if (k == null) continue;
          if (typeof k === 'string') {
            el.children.push({ nodeType: 3, textContent: k });
          } else {
            el.children.push(k);
          }
        }
      },
      appendChild(child) { el.children.push(child); return child; },
      setAttribute(name, value) { el.attributes[name] = String(value); },
      getAttribute(name) { return name in el.attributes ? el.attributes[name] : null; },
      addEventListener(type, fn) { (el.listeners[type] = el.listeners[type] || []).push(fn); },
      removeAttribute(name) { delete el.attributes[name]; },
    };
    Object.defineProperty(el, 'className', {
      get() { return el.attributes.class || ''; },
      set(v) { el.attributes.class = String(v); },
    });
    return el;
  }
  const doc = { createElement: (tag) => make(tag) };
  const origDocument = globalThis.document;
  globalThis.document = doc;
  return () => { globalThis.document = origDocument; };
}

function textOf(el) {
  const parts = [];
  const walk = (node) => {
    if (node.nodeType === 3 && node.textContent) parts.push(node.textContent);
    for (const c of node.children || []) walk(c);
  };
  walk(el);
  return parts.join('');
}

// ══ 1. createWidget ═════════════════════════════════════════════════════
test('createWidget: wraps factory and returns WidgetFactory returning WidgetModule', () => {
  const factory = () => ({ mount() {}, unmount() {} });
  const wrapped = createWidget(factory);
  assert.equal(typeof wrapped, 'function');
  const mod = wrapped();
  assert.equal(typeof mod.mount, 'function');
  assert.equal(typeof mod.unmount, 'function');
});

test('createWidget: mount errors are caught and forwarded to onError', () => {
  const errors = [];
  const factory = () => ({
    mount() { throw new Error('mount boom'); },
  });
  const wrapped = createWidget(factory, { onError: (e) => errors.push(e) });
  const mod = wrapped();
  // 不抛错——SDK 捕获并转 onError。
  assert.doesNotThrow(() => mod.mount(null, { instance: { type: 't' } }));
  assert.equal(errors.length, 1);
  assert.match(String(errors[0].message || errors[0]), /mount boom/);
});

test('createWidget: unmount errors are caught and forwarded to onError', () => {
  const errors = [];
  const factory = () => ({
    mount() {},
    unmount() { throw new Error('unmount boom'); },
  });
  const mod = createWidget(factory, { onError: (e) => errors.push(e) })();
  mod.mount(null, { instance: { type: 't' } });
  assert.doesNotThrow(() => mod.unmount());
  assert.equal(errors.length, 1);
  assert.match(String(errors[0].message || errors[0]), /unmount boom/);
});

test('createWidget: async mount rejection is caught and forwarded to onError', async () => {
  const errors = [];
  const factory = () => ({
    mount() { return Promise.reject(new Error('async boom')); },
  });
  const mod = createWidget(factory, { onError: (e) => errors.push(e) })();
  const r = mod.mount(null, { instance: { type: 't' } });
  // async mount 返回 promise；await 后错误已转 onError。
  await r;
  assert.equal(errors.length, 1);
  assert.match(String(errors[0].message || errors[0]), /async boom/);
});

test('createWidget: missing unmount is a no-op (no error)', () => {
  const errors = [];
  const factory = () => ({ mount() {} }); // 无 unmount
  const mod = createWidget(factory, { onError: (e) => errors.push(e) })();
  assert.doesNotThrow(() => mod.unmount());
  assert.equal(errors.length, 0, 'missing unmount must not trigger onError');
});

test('createWidget: debug mode logs lifecycle via console.debug', () => {
  const restore = installFakeDom();
  const logs = [];
  const origDebug = console.debug;
  console.debug = (...args) => logs.push(args.join(' '));
  try {
    const factory = () => ({ mount() {}, unmount() {} });
    const mod = createWidget(factory, { debug: true })();
    mod.mount(null, { instance: { type: 'acme.test' } });
    mod.unmount();
    assert.ok(logs.some((l) => l.includes('mount') && l.includes('acme.test')));
    assert.ok(logs.some((l) => l.includes('unmount')));
  } finally {
    console.debug = origDebug;
    restore();
  }
});

test('createWidget: default onError is a no-op (no throw when onError omitted)', () => {
  const factory = () => ({ mount() { throw new Error('silent'); } });
  const mod = createWidget(factory)();
  assert.doesNotThrow(() => mod.mount(null, { instance: { type: 't' } }));
});

// 审计 #489 W1：onError 自身抛异常不得把原始生命周期失败重新泄漏给宿主。
test('createWidget: throwing onError on sync mount error is swallowed (containment holds)', () => {
  const factory = () => ({
    mount() { throw new Error('mount boom'); },
  });
  const mod = createWidget(factory, {
    onError: () => { throw new Error('reporter boom'); },
  })();
  // 同步路径：mount 不得抛（onError 的异常被 guarded helper 吞掉）。
  assert.doesNotThrow(() => mod.mount(null, { instance: { type: 't' } }));
});

test('createWidget: throwing onError on async mount rejection is swallowed', async () => {
  const factory = () => ({
    mount() { return Promise.reject(new Error('async boom')); },
  });
  const mod = createWidget(factory, {
    onError: () => { throw new Error('reporter boom'); },
  })();
  // async 路径：返回的 promise 必须 resolve（而非因 onError 抛变 rejected）。
  await assert.doesNotReject(async () => {
    await mod.mount(null, { instance: { type: 't' } });
  });
});

test('createWidget: throwing onError on unmount error is swallowed', () => {
  const factory = () => ({
    mount() {},
    unmount() { throw new Error('unmount boom'); },
  });
  const mod = createWidget(factory, {
    onError: () => { throw new Error('reporter boom'); },
  })();
  mod.mount(null, { instance: { type: 't' } });
  assert.doesNotThrow(() => mod.unmount());
});

// ══ 2. defineManifest ════════════════════════════════════════════════════
test('defineManifest: valid manifest returns frozen object', () => {
  const m = defineManifest({
    type: 'acme.demo',
    version: '1.0.0',
    capabilities: ['read:state'],
    host_api: '1',
    entry: { kind: 'esm', source: './x.js', sandbox: true },
  });
  assert.equal(m.type, 'acme.demo');
  assert.equal(m.host_api, '1');
  assert.ok(Object.isFrozen(m), 'defineManifest must return a frozen object');
});

test('defineManifest: rejects non-object / missing type / missing version', () => {
  assert.throws(() => defineManifest(null), /must be an object/);
  assert.throws(() => defineManifest({}), /type is required/);
  assert.throws(() => defineManifest({ type: 'a.b' }), /version is required/);
  assert.throws(() => defineManifest({ type: 'a.b', version: '1.0.0', entry: 'bad' }), /entry must be an object/);
});

test('defineManifest: host_api accepts "1" / "1.0" / "1.2.3" and omits as "1"', () => {
  for (const v of ['1', '1.0', '1.2.3']) {
    const m = defineManifest({ type: 'a.b', version: '1.0.0', host_api: v });
    assert.equal(m.host_api, v);
  }
  // 缺省通过（视为 "1"，由 engine 处理；SDK 不填默认值）。
  const m = defineManifest({ type: 'a.b', version: '1.0.0' });
  assert.equal(m.host_api, undefined);
});

test('defineManifest: host_api rejects invalid formats (aligned with engine parse_host_api_major)', () => {
  // 与 engine/src/extensions/mod.rs parse_host_api_major 同语义的边界。
  for (const bad of ['0', '01', 'abc', '1.x', '1.', '.1', '1.0.0-beta', '99999999999']) {
    assert.throws(
      () => defineManifest({ type: 'a.b', version: '1.0.0', host_api: bad }),
      /host_api/,
      `host_api=${bad} should be rejected`,
    );
  }
});

test('defineManifest: esm entry must have sandbox === true (BUG-6 fail-closed)', () => {
  // esm 缺 sandbox → 抛错。
  assert.throws(
    () => defineManifest({ type: 'a.b', version: '1.0.0', entry: { kind: 'esm', source: './x.js' } }),
    /sandbox === true/,
  );
  // esm sandbox:false → 抛错。
  assert.throws(
    () => defineManifest({ type: 'a.b', version: '1.0.0', entry: { kind: 'esm', source: './x.js', sandbox: false } }),
    /sandbox === true/,
  );
  // esm sandbox:true → 通过。
  const m = defineManifest({ type: 'a.b', version: '1.0.0', entry: { kind: 'esm', source: './x.js', sandbox: true } });
  assert.equal(m.entry.sandbox, true);
  // builtin 无 sandbox 要求 → 通过。
  const m2 = defineManifest({ type: 'a.b', version: '1.0.0', entry: { kind: 'builtin' } });
  assert.equal(m2.entry.kind, 'builtin');
});

// 审计 #489 W2：嵌套 entry / capabilities 必须同样冻结，校验后不得可 mutate。
test('defineManifest: nested entry and capabilities are deep-frozen (mutation fails)', () => {
  const m = defineManifest({
    type: 'acme.demo',
    version: '1.0.0',
    capabilities: ['read:state'],
    entry: { kind: 'esm', source: './x.js', sandbox: true },
  });
  assert.ok(Object.isFrozen(m.entry), 'entry 必须被冻结');
  assert.ok(Object.isFrozen(m.capabilities), 'capabilities 必须被冻结');
  // ESM strict mode：对冻结对象赋值 / 增删必须抛 TypeError。
  assert.throws(() => { m.entry.sandbox = false; }, TypeError);
  assert.throws(() => { m.entry.source = '/evil.js'; }, TypeError);
  assert.throws(() => { m.capabilities.push('write:memory'); }, TypeError);
  assert.throws(() => { m.capabilities[0] = 'write:memory'; }, TypeError);
  // 冻结后值不变。
  assert.equal(m.entry.sandbox, true);
  assert.deepEqual([...m.capabilities], ['read:state']);
});

test('defineManifest: mutating the input after validation does not leak into the result', () => {
  const entry = { kind: 'esm', source: './x.js', sandbox: true };
  const caps = ['read:state'];
  const m = defineManifest({ type: 'a.b', version: '1.0.0', capabilities: caps, entry });
  // 输入侧 mutation 不影响已返回的 manifest（clone 语义）。
  entry.sandbox = false;
  caps.push('write:memory');
  assert.equal(m.entry.sandbox, true);
  assert.deepEqual([...m.capabilities], ['read:state']);
});

// ══ 3. h (DOM helper) ════════════════════════════════════════════════════
test('h: builds element with tag and children', () => {
  const restore = installFakeDom();
  try {
    const el = h('div', null, 'hello', null, 'world');
    assert.equal(el.tagName, 'DIV');
    assert.equal(textOf(el), 'helloworld');
    // null children 被过滤。
    assert.equal(el.children.length, 2);
  } finally {
    restore();
  }
});

test('h: applies className / style / dataset / setAttribute', () => {
  const restore = installFakeDom();
  try {
    const el = h('div', {
      className: 'my-class',
      style: { color: 'red', fontSize: '14px' },
      dataset: { widgetId: 'w1' },
      'aria-label': 'demo',
    });
    assert.equal(el.className, 'my-class');
    assert.equal(el.style.color, 'red');
    assert.equal(el.style.fontSize, '14px');
    assert.equal(el.dataset.widgetId, 'w1');
    assert.equal(el.getAttribute('aria-label'), 'demo');
  } finally {
    restore();
  }
});

test('h: onXxx props are registered as event listeners', () => {
  const restore = installFakeDom();
  try {
    let clicked = 0;
    const el = h('button', { onClick: () => clicked++ });
    assert.equal(el.listeners.click.length, 1);
    el.listeners.click[0]();
    assert.equal(clicked, 1);
  } finally {
    restore();
  }
});

test('h: null/undefined prop values are skipped', () => {
  const restore = installFakeDom();
  try {
    const el = h('div', { className: null, id: undefined, title: 't' });
    assert.equal(el.className, '');
    assert.equal(el.getAttribute('id'), null);
    assert.equal(el.getAttribute('title'), 't');
  } finally {
    restore();
  }
});

// ══ 4. 示例包自洽（审计 #489 W3） ═══════════════════════════════════════
// engine 安装面（validate_and_decode_files）拒绝无 index.js 的包，且第三方
// esm 安装后 entry.source 被强制改写为 /extensions/<digest>/index.js。
// SDK 示例包必须按自身 example-manifest.json 就能走通安装校验：files 清单
// 含 index.js 且每个文件真实存在；index.js 可加载并暴露默认工厂。

const sdkDirUrl = new URL('../assets/widgets/sdk/', import.meta.url);

test('example package: example-manifest.json passes defineManifest validation', async () => {
  const raw = JSON.parse(await readFile(new URL('example-manifest.json', sdkDirUrl), 'utf8'));
  // files 清单是包级声明（安装请求的 files payload 单独携带内容 + sha256），
  // defineManifest 只校验 widget 合同面，不应因 files 字段报错。
  const m = defineManifest(raw);
  assert.equal(m.type, 'acme.sdk-example');
  assert.equal(m.entry.kind, 'esm');
  assert.equal(m.entry.sandbox, true);
});

test('example package: files inventory lists index.js and every entry exists on disk', async () => {
  const raw = JSON.parse(await readFile(new URL('example-manifest.json', sdkDirUrl), 'utf8'));
  assert.ok(Array.isArray(raw.files) && raw.files.length > 0, 'manifest 必须声明 files 清单');
  assert.ok(raw.files.includes('index.js'), 'files 清单必须含安装入口 index.js');
  for (const file of raw.files) {
    await assert.doesNotReject(
      () => access(new URL(file, sdkDirUrl)),
      `manifest files 声明了不存在的包内文件: ${file}`,
    );
  }
});

test('example package: index.js loads and re-exports the widget factory and manifest', async () => {
  const mod = await import(new URL('index.js', sdkDirUrl).href);
  assert.equal(typeof mod.default, 'function', 'index.js 必须 re-export 默认 WidgetFactory');
  assert.ok(mod.manifest && mod.manifest.type === 'acme.sdk-example');
  const instance = mod.default();
  assert.equal(typeof instance.mount, 'function');
  assert.equal(typeof instance.unmount, 'function');
});
