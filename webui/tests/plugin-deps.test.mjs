// #498 §7.1/§7.4：trusted plugin 软依赖状态测试。
//
// 覆盖 plugin-deps.js 的权威缓存与缺失依赖计算：
//   1. initFromEngine 全量替换（含脏数据过滤）
//   2. missingDependencies 四态：未安装 / 已停止 / 版本过低 / 正常运行
//   3. 无声明 / 引擎不可达（空缓存）→ fail-closed 全提示
//   4. widget-host render 的非阻塞降级提示条（缺失才出现，满足则不出现）
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const deps = require('../assets/widgets/plugin-deps.js');
const manifests = require('../assets/widgets/manifests.js');
const consent = require('../assets/widgets/consent.js');
const sandbox = require('../assets/widgets/sandbox-bridge.js');
const host = require('../assets/widgets/widget-host.js');

function createFakeDom() {
  function make(tag) {
    const el = {
      tagName: tag.toUpperCase(),
      children: [],
      attributes: {},
      style: {},
      className: '',
      textContent: '',
      listeners: {},
      appendChild(child) { el.children.push(child); return child; },
      remove() { /* no-op for transport.destroy() */ },
      append(...kids) { for (const k of kids) el.appendChild(k); },
      replaceChildren(...kids) { el.children = []; for (const k of kids) el.appendChild(k); },
      setAttribute(name, value) { el.attributes[name] = String(value); },
      getAttribute(name) { return name in el.attributes ? el.attributes[name] : null; },
      addEventListener(type, fn) { (el.listeners[type] = el.listeners[type] || []).push(fn); },
      dispatch(type, event) { for (const fn of el.listeners[type] || []) fn(event); },
      querySelectorAll(selector) {
        const m = /^\[([a-z-]+)\]$/.exec(selector);
        if (!m) throw new Error('fake dom only supports [attr] selectors: ' + selector);
        const out = [];
        const walk = node => {
          for (const c of node.children || []) {
            if (c.getAttribute && c.getAttribute(m[1]) != null) out.push(c);
            walk(c);
          }
        };
        walk(el);
        return out;
      },
    };
    return el;
  }
  const doc = {
    createElement: tag => make(tag),
    createTextNode: text => ({ nodeType: 3, textContent: text }),
  };
  doc.body = make('body');
  doc.querySelectorAll = selector => doc.body.querySelectorAll(selector);
  return doc;
}

function textOf(el) {
  const parts = [];
  const walk = node => {
    const isLeaf = node.nodeType === 3 || !node.children || node.children.length === 0;
    if (isLeaf && node.textContent) parts.push(node.textContent);
    for (const c of node.children || []) walk(c);
  };
  walk(el);
  return parts.join('');
}

// ── 1. initFromEngine ────────────────────────────────────────────────
test('plugin-deps: initFromEngine replaces the cache and filters dirty rows', () => {
  deps.initFromEngine({
    plugins: [
      { id: 'com.example.tts', version: '1.0.0', host_api: '1', status: 'running' },
      { id: 'com.example.stt', version: '2.0.0', host_api: '1', status: 'stopped' },
      { id: '', version: 'x' }, // 脏行：无 id → 忽略
      null,
    ],
  });
  assert.equal(deps.allInstalled().length, 2);
  const tts = deps.allInstalled().find(p => p.version === '1.0.0');
  assert.deepEqual(tts, { version: '1.0.0', host_api: '1', status: 'running' });
  // 再次 init 全量替换（不是增量合并）。
  deps.initFromEngine({ plugins: [{ id: 'com.example.tts', status: 'running' }] });
  assert.equal(deps.allInstalled().length, 1);
});

// ── 2. missingDependencies 四态 ─────────────────────────────────────
test('plugin-deps: missingDependencies distinguishes not-installed / stopped / running', () => {
  deps.initFromEngine({
    plugins: [
      // engine `/v1/plugins` 保证带 host_api（manifest 必填字段）。
      { id: 'com.example.tts', host_api: '1', status: 'running' },
      { id: 'com.example.stt', status: 'stopped' },
    ],
  });
  const manifest = {
    type: 'acme.tts-ui',
    trusted_plugins: [
      { id: 'com.example.tts', min_host_api: '1' },      // 满足
      { id: 'com.example.stt' },                          // 已停止
      { id: 'com.example.ocr' },                          // 未安装
    ],
  };
  const missing = deps.missingDependencies(manifest);
  assert.equal(missing.length, 2);
  assert.deepEqual(missing[0], { id: 'com.example.stt', min_host_api: null, reason: 'stopped' });
  assert.deepEqual(missing[1], { id: 'com.example.ocr', min_host_api: null, reason: 'not-installed' });
});

test('plugin-deps: running but host_api below min_host_api is version-too-low', () => {
  deps.initFromEngine({
    plugins: [
      { id: 'com.example.tts', host_api: '1', status: 'running' },
      { id: 'com.example.tts2', host_api: '1', status: 'running' },
      { id: 'com.example.stt', host_api: '1.2', status: 'running' },
      { id: 'com.example.ocr', host_api: 'garbage', status: 'running' },
      { id: 'com.example.any', host_api: '1', status: 'running' },
    ],
  });
  const manifest = {
    type: 'acme.tts-ui',
    trusted_plugins: [
      { id: 'com.example.tts', min_host_api: '1' },   // 满足
      { id: 'com.example.tts2', min_host_api: '2' },  // major 不足
      { id: 'com.example.stt', min_host_api: '1.3' }, // patch 不足
      { id: 'com.example.ocr', min_host_api: '1' },   // 脏数据 fail-closed
      { id: 'com.example.any' },                      // 未声明 → 不比对
    ],
  };
  const missing = deps.missingDependencies(manifest);
  assert.equal(missing.length, 3);
  assert.deepEqual(missing[0], { id: 'com.example.tts2', min_host_api: '2', reason: 'version-too-low' });
  assert.deepEqual(missing[1], { id: 'com.example.stt', min_host_api: '1.3', reason: 'version-too-low' });
  assert.deepEqual(missing[2], { id: 'com.example.ocr', min_host_api: '1', reason: 'version-too-low' });
});

test('plugin-deps: versionAtLeast compares segment-wise and fails closed on dirty data', () => {
  assert.equal(deps.versionAtLeast('1', '1'), true);
  assert.equal(deps.versionAtLeast('1.2', '1'), true);
  assert.equal(deps.versionAtLeast('2', '1.9'), true);
  assert.equal(deps.versionAtLeast('1', '2'), false);
  assert.equal(deps.versionAtLeast('1.2', '1.3'), false);
  assert.equal(deps.versionAtLeast('', '1'), false);  // 空 installed 不满足
  assert.equal(deps.versionAtLeast('x', '1'), false); // 脏数据 fail-closed
  assert.equal(deps.versionAtLeast('1', null), true); // 未声明不比对
  assert.equal(deps.versionAtLeast('1', ''), true);   // 空 min 不比对
  // 审计 #507 N2：hex / 科学计数法 / 带符号段一律拒绝（旧 Number() 误接受）。
  assert.equal(deps.versionAtLeast('0x10', '1'), false);  // hex → 不是 16
  assert.equal(deps.versionAtLeast('1e2', '1'), false);   // 科学计数法 → 不是 100
  assert.equal(deps.versionAtLeast(' 1', '1'), false);    // 前导空格 → 拒绝
  assert.equal(deps.versionAtLeast('+1', '1'), false);    // 带符号 → 拒绝
  assert.equal(deps.versionAtLeast('1', '0x10'), false);  // min 侧同样拒绝
  assert.equal(deps.versionAtLeast('1', '1e2'), false);
  assert.equal(deps.versionAtLeast('01', '1'), true);     // 前导零合法
});

test('plugin-deps: no declaration → no missing; empty cache → all missing (fail-closed)', () => {
  deps.initFromEngine({ plugins: [{ id: 'com.example.tts', status: 'running' }] });
  assert.deepEqual(deps.missingDependencies({ type: 'x', trusted_plugins: [] }), []);
  assert.deepEqual(deps.missingDependencies({ type: 'x' }), []);

  // 引擎不可达（boot 失败）→ 缓存保持空 → 全部声明按缺失提示。
  deps.initFromEngine({ plugins: [] });
  const missing = deps.missingDependencies({ type: 'x', trusted_plugins: [{ id: 'com.example.tts' }] });
  assert.equal(missing.length, 1);
  assert.equal(missing[0].reason, 'not-installed');
});

// ── 3. widget-host 非阻塞降级提示条 ─────────────────────────────────
test('widget-host: missing trusted plugin renders a non-blocking hint above the widget', async () => {
  consent.clearGrants();
  manifests.clearManifests();
  const DEP_MANIFEST = {
    type: 'acme.dep-widget',
    version: '1.0.0',
    entry: { kind: 'esm', source: 'https://example.test/w.js', sandbox: true },
    trusted_plugins: [{ id: 'com.example.tts' }, { id: 'com.example.ocr' }],
  };
  manifests.registerManifest(DEP_MANIFEST);
  consent.grant(DEP_MANIFEST);
  deps.initFromEngine({ plugins: [{ id: 'com.example.tts', status: 'running' }] });

  const doc = createFakeDom();
  const container = doc.createElement('div');
  const transport = {
    sent: [], handlers: new Set(), destroyed: false,
    postMessage(msg) { transport.sent.push(msg); },
    onMessage(cb) {
      transport.handlers.add(cb);
      // 模拟真实 iframe：引导脚本加载后立即回发 ready（早于宿主 mount），
      // 避免 mount() 的 5s 超时定时器挂起测试进程（审计 #507）。
      cb({ kind: 'ready' });
      return () => transport.handlers.delete(cb);
    },
    destroy() { transport.destroyed = true; },
  };
  host.mountWidget(container, { id: 'w1', type: DEP_MANIFEST.type }, null, {
    doc, sandbox, pluginDeps: deps, transportFactory: () => transport,
  });
  await new Promise(r => setTimeout(r, 0));

  const text = textOf(container);
  assert.match(text, /依赖的 trusted plugin 不可用/, '缺失依赖必须出现提示条');
  assert.match(text, /com\.example\.ocr/, '未安装的插件 id 出现在提示中');
  assert.doesNotMatch(text, /com\.example\.tts（/, '已运行的插件不得出现在提示中');
  // 非阻塞：提示条之外 widget 仍挂载（sandbox target 存在）。
  assert.ok(container.children.some(c => c.className === 'widget-sandbox'), 'widget 必须仍加载');
});

test('widget-host: satisfied dependencies render no hint', async () => {
  consent.clearGrants();
  manifests.clearManifests();
  const DEP_MANIFEST = {
    type: 'acme.dep-ok',
    version: '1.0.0',
    entry: { kind: 'esm', source: 'https://example.test/w.js', sandbox: true },
    trusted_plugins: [{ id: 'com.example.tts' }],
  };
  manifests.registerManifest(DEP_MANIFEST);
  consent.grant(DEP_MANIFEST);
  deps.initFromEngine({ plugins: [{ id: 'com.example.tts', status: 'running' }] });

  const doc = createFakeDom();
  const container = doc.createElement('div');
  const transport = {
    sent: [], handlers: new Set(), destroyed: false,
    postMessage(msg) { transport.sent.push(msg); },
    onMessage(cb) {
      transport.handlers.add(cb);
      // 同前：回发 ready，让 mount() 立即完成，不挂 5s 超时定时器。
      cb({ kind: 'ready' });
      return () => transport.handlers.delete(cb);
    },
    destroy() { transport.destroyed = true; },
  };
  host.mountWidget(container, { id: 'w1', type: DEP_MANIFEST.type }, null, {
    doc, sandbox, pluginDeps: deps, transportFactory: () => transport,
  });
  await new Promise(r => setTimeout(r, 0));
  assert.doesNotMatch(textOf(container), /依赖的 trusted plugin/, '依赖满足时不得出现提示条');
});
