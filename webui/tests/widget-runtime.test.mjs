// C-P1 widget 运行时测试：webui/assets/widgets/ 的行为镜像
// ui/src/registry/*.test.ts（行为基准）。覆盖：
//   1. registry  —— 注册/解析/esm 加载/importer 注入
//   2. manifests —— set 全替换 / patch upsert / esm 自动注册
//   3. consent   —— 身份绑定 grantKey / builtin 免同意 / 持久化
//   4. sandbox-bridge —— ready 竞态 / mount 超时 / intent·error 转发 / state 缓冲
//   5. widget-host —— 四态（failed/gated/sandboxed·module/missing）+ BUG-6 拒载
//   6. slots     —— slot 注册/机器可读计划/data-slot 挂载/state 推送
// widget 运行时文件是 UMD 脚本（浏览器挂全局、node 走 module.exports），
// 这里用 createRequire 直接载入。
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const registry = require('../assets/widgets/registry.js');
const manifests = require('../assets/widgets/manifests.js');
const consent = require('../assets/widgets/consent.js');
const sandbox = require('../assets/widgets/sandbox-bridge.js');
const host = require('../assets/widgets/widget-host.js');
const slots = require('../assets/widgets/slots.js');

// ── 极简 fake DOM（只覆盖 widget 宿主用到的面） ────────────────────────────
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

// 可驱动的沙箱假 transport（镜像 sandbox-bridge.test.ts 的注入手法）。
function createFakeTransport() {
  const transport = {
    sent: [],
    handlers: new Set(),
    destroyed: false,
    postMessage(msg) { transport.sent.push(msg); },
    onMessage(cb) { transport.handlers.add(cb); return () => transport.handlers.delete(cb); },
    destroy() { transport.destroyed = true; },
    emit(msg) { for (const cb of [...transport.handlers]) cb(msg); },
  };
  return transport;
}

function mockStorage(initial) {
  const map = new Map(Object.entries(initial || {}));
  return {
    getItem: key => (map.has(key) ? map.get(key) : null),
    setItem: (key, value) => { map.set(key, value); },
    removeItem: key => { map.delete(key); },
    _map: map,
  };
}

// ══ 1. registry ══════════════════════════════════════════════════════════
test('registry: module widget register / resolve / unregister / registeredTypes', () => {
  registry.registerModuleWidget('t.mod', () => ({ mount() {} }));
  assert.ok(registry.registeredTypes().includes('t.mod'));
  assert.equal(registry.resolveWidget('t.mod').kind, 'module');
  registry.unregisterWidget('t.mod');
  assert.equal(registry.resolveWidget('t.mod'), undefined);
});

test('registry: esm widget resolves function module and { default } module via injectable importer', async () => {
  const asFunction = () => ({ mount() {} });
  registry.registerEsmWidget('t.esm-fn', 'src-a', { sandbox: true, importer: async () => asFunction });
  const modFn = await registry.resolveWidget('t.esm-fn').load();
  assert.equal(typeof modFn.mount, 'function');

  registry.registerEsmWidget('t.esm-default', 'src-b', { sandbox: true, importer: async () => ({ default: () => ({ mount() {} }) }) });
  const modDefault = await registry.resolveWidget('t.esm-default').load();
  assert.equal(typeof modDefault.mount, 'function');

  registry.registerEsmWidget('t.esm-bad', 'src-c', { sandbox: true, importer: async () => ({}) });
  await assert.rejects(() => registry.resolveWidget('t.esm-bad').load(), /WidgetFactory/);

  registry.unregisterWidget('t.esm-fn');
  registry.unregisterWidget('t.esm-default');
  registry.unregisterWidget('t.esm-bad');
});

test('registry: setDefaultEsmImporter overrides the global importer', async () => {
  const seen = [];
  registry.setDefaultEsmImporter(async source => { seen.push(source); return () => ({ mount() {} }); });
  registry.registerEsmWidget('t.esm-global', 'global-src', { sandbox: true });
  await registry.resolveWidget('t.esm-global').load();
  assert.deepEqual(seen, ['global-src']);
  registry.setDefaultEsmImporter(source => import(source)); // 还原
  registry.unregisterWidget('t.esm-global');
});

test('registry: W2 — esm registration requires a manifest reference or sandbox:true; builtin unaffected', () => {
  // 无标记 / 空 options / 遗留 function 形态：注册面即拒绝（BUG-6 纵深设防）。
  assert.throws(() => registry.registerEsmWidget('t.nope', 'src'), /sandbox/);
  assert.throws(() => registry.registerEsmWidget('t.nope', 'src', {}), /sandbox/);
  assert.throws(() => registry.registerEsmWidget('t.nope', 'src', async () => ({})), /sandbox/);
  assert.equal(registry.resolveWidget('t.nope'), undefined, 'rejected registration must not leak into the registry');

  // 显式 sandbox:true 标记放行。
  registry.registerEsmWidget('t.sbx', 'src', { sandbox: true, importer: async () => ({ mount() {} }) });
  assert.ok(registry.resolveWidget('t.sbx'));
  registry.unregisterWidget('t.sbx');

  // manifest 引用形态放行（manifests.js / C-P2 catalog 的合法路径）。
  registry.registerEsmWidget('t.with-manifest', 'src', { manifest: ESM_MANIFEST });
  assert.ok(registry.resolveWidget('t.with-manifest'));
  registry.unregisterWidget('t.with-manifest');

  // 首方 builtin（module widget）不受此门禁影响。
  registry.registerModuleWidget('t.builtin-safe', () => ({ mount() {} }));
  assert.equal(registry.resolveWidget('t.builtin-safe').kind, 'module');
  registry.unregisterWidget('t.builtin-safe');
});

// ══ 2. manifests ═════════════════════════════════════════════════════════
const ESM_MANIFEST = {
  type: 't.esm-widget', version: '1.0.0',
  entry: { kind: 'esm', source: 'https://example.test/w.js', sandbox: true },
};

test('manifests: register/get/all and esm auto-registration', () => {
  manifests.clearManifests();
  manifests.registerEsmWidgetsFromManifests([ESM_MANIFEST]);
  assert.equal(manifests.getManifest('t.esm-widget').version, '1.0.0');
  assert.equal(manifests.allManifests().length, 1);
  assert.ok(registry.resolveWidget('t.esm-widget'), 'esm manifest must auto-register into the component registry');
  manifests.clearManifests();
  assert.equal(registry.resolveWidget('t.esm-widget'), undefined, 'clearManifests must unregister esm widgets it brought in');
});

test('manifests: op:"set" full-replaces, op:"patch" upserts by type', () => {
  manifests.applyManifestMessage('set', [ESM_MANIFEST]);
  assert.equal(manifests.allManifests().length, 1);

  const bumped = { ...ESM_MANIFEST, version: '2.0.0' };
  manifests.applyManifestMessage('patch', [bumped]);
  assert.equal(manifests.getManifest('t.esm-widget').version, '2.0.0', 'patch must upsert the same type');
  assert.equal(manifests.allManifests().length, 1);

  const other = { type: 't.other', version: '1.0.0', entry: { kind: 'builtin' } };
  manifests.applyManifestMessage('set', [other]);
  assert.equal(manifests.getManifest('t.esm-widget'), undefined, 'set must clear the previous full set');
  assert.equal(registry.resolveWidget('t.esm-widget'), undefined, 'set must drop stale esm registrations');
  manifests.clearManifests();
});

test('manifests: builtin entries are recorded but not registered as esm', () => {
  manifests.applyManifestMessage('set', [{ type: 't.builtin', version: '1.0.0', entry: { kind: 'builtin' } }]);
  assert.ok(manifests.getManifest('t.builtin'));
  assert.equal(registry.resolveWidget('t.builtin'), undefined);
  manifests.clearManifests();
});

test('manifests: W2 — esm entry without sandbox:true is recorded but never registered', () => {
  manifests.clearManifests();
  manifests.registerEsmWidgetsFromManifests([
    { type: 't.nosbx', version: '1.0.0', entry: { kind: 'esm', source: 'x.js' } },
  ]);
  assert.ok(manifests.getManifest('t.nosbx'), 'manifest must stay recorded so render can show the BUG-6 refusal');
  assert.equal(registry.resolveWidget('t.nosbx'), undefined, 'no-sandbox esm must not reach the component registry');
  manifests.clearManifests();
});

// ══ 3. consent ═══════════════════════════════════════════════════════════
test('consent: builtin needs no consent; esm gates until granted', () => {
  consent.clearGrants();
  assert.equal(consent.needsConsent({ entry: { kind: 'builtin' } }), false);
  assert.equal(consent.canMount({ type: 'x', version: '1', entry: { kind: 'builtin' } }), true);
  assert.equal(consent.needsConsent(ESM_MANIFEST), true);
  assert.equal(consent.canMount(ESM_MANIFEST), false);
  consent.grant(ESM_MANIFEST);
  assert.equal(consent.canMount(ESM_MANIFEST), true);
  consent.revoke(ESM_MANIFEST);
  assert.equal(consent.canMount(ESM_MANIFEST), false);
});

test('consent: grant is identity-bound (type@version#source); source or version change requires re-consent', () => {
  consent.clearGrants();
  consent.grant(ESM_MANIFEST);
  const swappedSource = { ...ESM_MANIFEST, entry: { ...ESM_MANIFEST.entry, source: 'https://evil.test/w.js' } };
  const bumped = { ...ESM_MANIFEST, version: '1.0.1' };
  assert.equal(consent.canMount(swappedSource), false, 'source swap must not carry the old approval');
  assert.equal(consent.canMount(bumped), false, 'version bump must not carry the old approval');
  consent.clearGrants();
});

test('consent: effectiveCapabilities empty until mountable', () => {
  consent.clearGrants();
  const m = { ...ESM_MANIFEST, capabilities: ['read:state'] };
  assert.deepEqual(consent.effectiveCapabilities(m), []);
  consent.grant(m);
  assert.deepEqual(consent.effectiveCapabilities(m), ['read:state']);
  consent.clearGrants();
});

test('consent: grants persist to injectable storage and restore via initGrants', () => {
  consent.clearGrants();
  const storage = mockStorage();
  consent.initGrants(storage);
  consent.grant(ESM_MANIFEST);
  const raw = storage.getItem(consent.STORAGE_KEY);
  assert.ok(raw && raw.includes('t.esm-widget@1.0.0#https://example.test/w.js'));

  // 新会话（模块单例下先清空再恢复）模拟跨刷新。
  consent.clearGrants();
  consent.initGrants(storage);
  assert.equal(consent.canMount(ESM_MANIFEST), true, 'saved grants must survive reload');
  consent.clearGrants();
});

test('consent: corrupted storage never throws and starts fresh', () => {
  consent.clearGrants();
  consent.initGrants(mockStorage({ [consent.STORAGE_KEY]: '{not-json' }));
  assert.equal(consent.canMount(ESM_MANIFEST), false);
  consent.clearGrants();
});

// ══ 4. sandbox-bridge ════════════════════════════════════════════════════
const INSTANCE = { id: 'w1', type: 't.esm-widget' };

test('sandbox bridge: ready BEFORE mount resolves immediately and sends the mount message', async () => {
  const transport = createFakeTransport();
  const bridge = new sandbox.SandboxBridge(transport, () => {}, () => {});
  transport.emit({ kind: 'ready' }); // 抢跑竞态：ready 先于 mount()
  await bridge.mount(INSTANCE, ['read:state']);
  assert.deepEqual(transport.sent, [{ kind: 'mount', instance: INSTANCE, capabilities: ['read:state'] }]);
  bridge.destroy();
});

test('sandbox bridge: ready AFTER mount still resolves and sends mount once', async () => {
  const transport = createFakeTransport();
  const bridge = new sandbox.SandboxBridge(transport, () => {}, () => {});
  const pending = bridge.mount(INSTANCE, []);
  transport.emit({ kind: 'ready' });
  await pending;
  assert.equal(transport.sent.filter(m => m.kind === 'mount').length, 1);
  bridge.destroy();
});

test('sandbox bridge: mount rejects when the iframe never signals ready (5s default, shortened here)', async () => {
  const transport = createFakeTransport();
  const bridge = new sandbox.SandboxBridge(transport, () => {}, () => {});
  const pending = bridge.mount(INSTANCE, [], 10);
  await assert.rejects(() => pending, /did not signal ready in time/);
  for (const w of bridge.readyWaiters) if (w.timer) clearTimeout(w.timer); // 清理未触发定时器
  // 晚到的 ready 不能让已拒绝的 mount 复活发出 mount 消息。
  transport.emit({ kind: 'ready' });
  assert.equal(transport.sent.length, 0);
  bridge.destroy();
});

test('sandbox bridge: intent / error surfaced, state pushed, destroy stops everything', () => {
  const transport = createFakeTransport();
  const intents = []; const errors = [];
  const bridge = new sandbox.SandboxBridge(transport, (n, p) => intents.push([n, p]), m => errors.push(m));
  transport.emit({ kind: 'intent', name: 'status.toggle', params: { id: 'w1' } });
  transport.emit({ kind: 'error', message: 'boom' });
  assert.deepEqual(intents, [['status.toggle', { id: 'w1' }]]);
  assert.deepEqual(errors, ['boom']);
  bridge.pushState({ label: 'x' });
  assert.deepEqual(transport.sent.at(-1), { kind: 'state', state: { label: 'x' } });
  bridge.destroy();
  assert.equal(transport.destroyed, true);
  const sentBefore = transport.sent.length;
  bridge.pushState({ label: 'y' });
  transport.emit({ kind: 'intent', name: 'late' });
  assert.equal(transport.sent.length, sentBefore, 'destroyed bridge must stop forwarding');
  assert.equal(intents.length, 1, 'destroyed bridge must stop surfacing intents');
});

test('sandbox bridge: destroyed bridge rejects mount', async () => {
  const transport = createFakeTransport();
  const bridge = new sandbox.SandboxBridge(transport, () => {}, () => {});
  bridge.destroy();
  await assert.rejects(() => bridge.mount(INSTANCE, []), /destroyed/);
});

test('sandbox frame: external bootstrap signals ready, handles mount/state, buffers state, targets host precisely', () => {
  // srcdoc 内联引导被父页 CSP 拦截（实证 2026-08-05），改为同源静态页 +
  // 外链经典脚本；行为契约不变（对译 SANDBOX_BOOTSTRAP）。
  const js = require('node:fs').readFileSync(new URL('../assets/widgets/sandbox-frame.js', import.meta.url), 'utf8');
  assert.match(js, /send\(\{ kind: 'ready' \}\)/, 'bootstrap must signal ready');
  assert.match(js, /msg\.kind === 'mount'/, 'bootstrap must handle mount');
  assert.match(js, /msg\.kind === 'state'/, 'bootstrap must handle state');
  assert.match(js, /hasState/, 'bootstrap must buffer state for late onState registration');
  assert.match(js, /params\.get\('origin'\)/, 'bootstrap must use the host-supplied origin as targetOrigin');
  assert.match(js, /parent\.postMessage\(msg, TARGET\)/, 'bootstrap must post back with a precise targetOrigin');
  assert.match(js, /new URLSearchParams\(window\.location\.search\)/, 'widget source must arrive via the frame URL, never be baked into code');
});

test('sandbox frame: html page is CSP-clean (no inline script/style)', () => {
  const html = require('node:fs').readFileSync(new URL('../assets/widgets/sandbox-frame.html', import.meta.url), 'utf8');
  const scriptTags = html.match(/<script\b[^>]*>/g) || [];
  assert.ok(scriptTags.length > 0, 'frame page must load its bootstrap');
  for (const tag of scriptTags) assert.match(tag, /\bsrc=/, 'frame page must not contain inline scripts');
  assert.ok(!/<style[\s>]/.test(html), 'frame page must not contain inline styles');
  assert.match(html, /<link rel="stylesheet" href="sandbox-frame\.css">/, 'frame styles must be an external stylesheet');
});

test('sandbox transport: createIframeTransport builds a sandboxed same-origin frame carrying the absolute source', () => {
  const doc = createFakeDom();
  doc.baseURI = 'http://127.0.0.1:8765/screens/03-workbench.html';
  doc.defaultView = {
    addEventListener() {},
    removeEventListener() {},
  };
  const container = doc.createElement('div');
  const transport = sandbox.createIframeTransport(container, '../assets/widgets/status.module.js', doc);
  const iframe = container.children[0];
  assert.equal(iframe.attributes.sandbox, 'allow-scripts');
  const src = iframe.attributes.src;
  assert.ok(src.startsWith('http://127.0.0.1:8765/assets/widgets/sandbox-frame.html?src='), src);
  const passed = decodeURIComponent(new URL(src).searchParams.get('src'));
  assert.equal(passed, 'http://127.0.0.1:8765/assets/widgets/status.module.js', 'source must be resolved against the host page base');
  assert.equal(new URL(src).searchParams.get('origin'), 'http://127.0.0.1:8765', 'frame must learn the host origin from the host itself');
  transport.destroy();
});

test('sandbox transport: gates on event.source === iframe.contentWindow (contract markers)', () => {
  // createIframeTransport 需要真实 DOM；此处断言门控契约存在于源码（浏览器行为由冒烟覆盖）。
  const src = require('node:fs').readFileSync(new URL('../assets/widgets/sandbox-bridge.js', import.meta.url), 'utf8');
  assert.match(src, /ev\.source !== iframe\.contentWindow/, 'host must gate messages on the iframe window identity');
  assert.match(src, /sandbox.*allow-scripts/, 'iframe must run with allow-scripts');
  // 契约只看代码：剥离注释（说明性文字允许提及被禁的旗标）。
  const code = src.replace(/\/\/[^\n]*/g, '').replace(/\/\*[\s\S]*?\*\//g, '');
  assert.doesNotMatch(code, /allow-same-origin/, 'iframe must NOT get allow-same-origin');
  assert.match(code, /setAttribute\('sandbox', 'allow-scripts'\)/, 'sandbox attribute must be exactly allow-scripts');
  // host → iframe 只能 '*'（Chrome 拒收 'null' targetOrigin，实证 2026-08-05）；
  // 安全由 event.source 门控承担，绝不得反向退化门控。
  assert.match(code, /postMessage\(msg, '\*'\)/, 'host posts to the sandbox frame window directly');
});

// ══ 5. widget-host 四态 + BUG-6 ═══════════════════════════════════════════
function hostDeps(extra) {
  return Object.assign({
    registry, manifests, consent, sandbox,
    doc: createFakeDom(),
  }, extra || {});
}

test('widget-host: missing type renders the missing state', () => {
  const doc = createFakeDom();
  const container = doc.createElement('div');
  host.mountWidget(container, { id: 'i', type: 'no.such' }, null, { doc });
  assert.match(textOf(container), /未注册的 widget：no\.such/);
});

test('widget-host: builtin module widget mounts in-process with host-enforced context', async () => {
  consent.clearGrants();
  const doc = createFakeDom();
  const container = doc.createElement('div');
  const seen = {};
  registry.registerModuleWidget('t.builtin-mod', () => ({
    mount(el, ctx) {
      seen.el = el; seen.ctx = ctx;
      ctx.onState(s => { seen.lastState = s; });
    },
    unmount() { seen.unmounted = true; },
  }));
  const handle = host.mountWidget(container, { id: 'i1', type: 't.builtin-mod', capabilities: ['read:state'] }, { label: 'init' }, { doc });
  await new Promise(r => setTimeout(r, 0)); // load() 是 async
  assert.ok(seen.el, 'widget mount must receive a container element');
  assert.equal(seen.ctx.instance.id, 'i1');
  assert.deepEqual(seen.ctx.capabilities, ['read:state'], 'builtin without manifest falls back to instance capabilities');
  assert.equal(seen.lastState.label, 'init', 'initial state must be delivered after mount');
  handle.pushState({ label: 'next' });
  assert.equal(seen.lastState.label, 'next');
  handle.destroy();
  assert.equal(seen.unmounted, true);
  registry.unregisterWidget('t.builtin-mod');
});

test('widget-host: module widget mount failure surfaces the failed state', async () => {
  const doc = createFakeDom();
  const container = doc.createElement('div');
  registry.registerModuleWidget('t.throws', () => ({ mount() { throw new Error('widget exploded'); } }));
  host.mountWidget(container, { id: 'i', type: 't.throws' }, null, { doc });
  await new Promise(r => setTimeout(r, 0));
  assert.match(textOf(container), /widget exploded/);
  registry.unregisterWidget('t.throws');
});

test('widget-host: BUG-6 — esm entry without sandbox:true is refused (no in-process third-party path)', async () => {
  consent.clearGrants();
  const doc = createFakeDom();
  for (const entry of [{ kind: 'esm', source: 'x.js' }, { kind: 'esm', source: 'x.js', sandbox: false }]) {
    manifests.clearManifests();
    manifests.registerManifest({ type: 't.unsafe', version: '1.0.0', entry });
    consent.grant({ type: 't.unsafe', version: '1.0.0', entry }); // 即便已 consent 也必须拒载
    const container = doc.createElement('div');
    host.mountWidget(container, { id: 'i', type: 't.unsafe' }, null, { doc });
    await new Promise(r => setTimeout(r, 0));
    assert.match(textOf(container), /sandbox:true/, 'missing sandbox:true must be refused: ' + JSON.stringify(entry));
  }
  consent.clearGrants();
  manifests.clearManifests();
});

test('widget-host: unconsented esm renders the consent gate; approving mounts into the sandbox', async () => {
  consent.clearGrants();
  manifests.clearManifests();
  manifests.registerManifest(ESM_MANIFEST);
  const doc = createFakeDom();
  const container = doc.createElement('div');
  const transport = createFakeTransport();
  const handle = host.mountWidget(container, { id: 'w1', type: ESM_MANIFEST.type }, { label: 's' }, {
    doc, transportFactory: () => transport,
  });
  await new Promise(r => setTimeout(r, 0));
  assert.match(textOf(container), /第三方 widget/, 'gated state must show the consent UI');
  assert.match(textOf(container), /example\.test/, 'consent UI must show the source');
  assert.equal(transport.sent.length, 0, 'nothing may load before consent');

  const approveButton = container.children.flatMap(c => c.children || []).find(c => c.tagName === 'BUTTON');
  assert.ok(approveButton, 'consent UI must expose an approve button');
  approveButton.dispatch('click', {});
  await new Promise(r => setTimeout(r, 0));
  assert.equal(consent.isGranted(ESM_MANIFEST), true, 'approving must persist the grant');
  transport.emit({ kind: 'ready' });
  await new Promise(r => setTimeout(r, 0));
  const mountMsg = transport.sent.find(m => m.kind === 'mount');
  assert.ok(mountMsg, 'after consent the sandbox must receive mount');
  assert.deepEqual(mountMsg.capabilities, [], 'manifest declared no capabilities → none granted');
  handle.destroy();
  assert.equal(transport.destroyed, true);
  consent.clearGrants();
  manifests.clearManifests();
});

test('widget-host: sandbox runtime error surfaces the failed state', async () => {
  consent.clearGrants();
  manifests.clearManifests();
  manifests.registerManifest(ESM_MANIFEST);
  consent.grant(ESM_MANIFEST);
  const doc = createFakeDom();
  const container = doc.createElement('div');
  const transport = createFakeTransport();
  host.mountWidget(container, { id: 'w1', type: ESM_MANIFEST.type }, null, { doc, transportFactory: () => transport });
  transport.emit({ kind: 'ready' });
  await new Promise(r => setTimeout(r, 0));
  transport.emit({ kind: 'error', message: 'import failed: CORS' });
  await new Promise(r => setTimeout(r, 0));
  assert.match(textOf(container), /import failed: CORS/);
  consent.clearGrants();
  manifests.clearManifests();
});

test('widget-host: W1 — sandbox failure terminal states destroy the bridge (no leaked listeners)', async () => {
  consent.clearGrants();
  manifests.clearManifests();
  manifests.registerManifest(ESM_MANIFEST);
  consent.grant(ESM_MANIFEST);
  const doc = createFakeDom();

  // (a) onError 终态：widget 上报运行错误 → failed + destroy。
  const container = doc.createElement('div');
  const transport = createFakeTransport();
  host.mountWidget(container, { id: 'w1', type: ESM_MANIFEST.type }, null, { doc, transportFactory: () => transport });
  transport.emit({ kind: 'ready' });
  await new Promise(r => setTimeout(r, 0));
  transport.emit({ kind: 'error', message: 'import failed: CORS' });
  await new Promise(r => setTimeout(r, 0));
  assert.match(textOf(container), /import failed: CORS/);
  assert.equal(transport.destroyed, true, 'onError terminal state must destroy the bridge (W1)');

  // (b) catch 终态（mount 超时/发送异常同走此分支）：mount 成功后 pushState
  // 发送抛错 → try 块内 reject → failed + 幂等销毁。
  const container2 = doc.createElement('div');
  const transport2 = createFakeTransport();
  transport2.postMessage = (msg) => {
    if (msg.kind === 'state') throw new Error('frame was detached');
    transport2.sent.push(msg);
  };
  host.mountWidget(container2, { id: 'w2', type: ESM_MANIFEST.type }, null, { doc, transportFactory: () => transport2 });
  transport2.emit({ kind: 'ready' });
  await new Promise(r => setTimeout(r, 0));
  assert.match(textOf(container2), /frame was detached/);
  assert.equal(transport2.destroyed, true, 'catch terminal state must destroy the bridge (W1)');

  consent.clearGrants();
  manifests.clearManifests();
});

test('widget-host: a misbehaving widget must not break teardown', () => {
  const doc = createFakeDom();
  const container = doc.createElement('div');
  registry.registerModuleWidget('t.misbehave', () => ({ mount() {}, unmount() { throw new Error('refuse teardown'); } }));
  const handle = host.mountWidget(container, { id: 'i', type: 't.misbehave' }, null, { doc });
  assert.doesNotThrow(() => handle.destroy(), 'teardown must contain widget errors');
  registry.unregisterWidget('t.misbehave');
});

// ══ 6. slots ═════════════════════════════════════════════════════════════
test('slots: registerSlot derives screen/region from the id and validates input', () => {
  slots.clearSlots();
  slots.registerSlot({ id: 'chat.sidebar', widgets: [] });
  const slot = slots.getSlot('chat.sidebar');
  assert.equal(slot.screen, 'chat');
  assert.equal(slot.region, 'sidebar');
  assert.throws(() => slots.registerSlot({}), /slot id is required/);
});

test('slots: plan() is machine-readable JSON; applySlotPlan merge vs replace', () => {
  slots.clearSlots();
  slots.applySlotPlan({ slots: [{ id: 'a.one' }, { id: 'b.two' }] });
  assert.deepEqual(slots.slotNames().sort(), ['a.one', 'b.two']);
  const json = JSON.parse(JSON.stringify(slots.plan()));
  assert.equal(json.version, 1);
  assert.equal(json.slots.length, 2);

  slots.applySlotPlan({ slots: [{ id: 'c.three' }] }); // merge（upsert）
  assert.equal(slots.slotNames().length, 3);
  slots.applySlotPlan({ slots: [{ id: 'd.only' }] }, 'replace');
  assert.deepEqual(slots.slotNames(), ['d.only']);
  assert.throws(() => slots.applySlotPlan({}), /slots/);
  slots.clearSlots();
});

test('slots: mountSlots scans [data-slot], mounts registered widgets, marks empty slots', () => {
  slots.clearSlots();
  consent.clearGrants();
  const doc = createFakeDom();
  const filled = doc.createElement('div'); filled.setAttribute('data-slot', 'chat.sidebar');
  const empty = doc.createElement('div'); empty.setAttribute('data-slot', 'unknown.slot');
  doc.body.append(filled, empty);

  registry.registerModuleWidget('t.slot-widget', () => ({ mount(el) { el.textContent = 'mounted'; } }));
  slots.registerSlot({ id: 'chat.sidebar', widgets: [{ instance: { id: 's1', type: 't.slot-widget' }, state: { label: 'x' } }] });

  const handles = slots.mountSlots(doc);
  assert.equal(handles.length, 1);
  assert.equal(handles[0].slotId, 'chat.sidebar');
  assert.equal(filled.getAttribute('data-slot-state'), 'mounted');
  assert.equal(empty.getAttribute('data-slot-state'), 'empty');
  assert.match(textOf(filled), /mounted/);

  // 重复挂载：只清空 slot DOM 并重新挂载（旧句柄不由 mountSlots 销毁，见 S2 注释修正）。
  slots.mountSlots(doc);
  assert.equal(filled.children.length, 1);

  // pushSlotState 按 slotId 过滤。
  const pushed = [];
  const stubHandle = { slotId: 'chat.sidebar', pushState: s => pushed.push(s) };
  const otherHandle = { slotId: 'other', pushState: s => pushed.push(s) };
  assert.equal(slots.pushSlotState([stubHandle, otherHandle], 'chat.sidebar', { label: 'y' }), 1);
  assert.deepEqual(pushed, [{ label: 'y' }]);

  registry.unregisterWidget('t.slot-widget');
  slots.clearSlots();
});

test('slots: the shipped slots.json registers exactly the five C-P1 slots with sandboxed esm entries', async () => {
  const { readFile } = await import('node:fs/promises');
  const plan = JSON.parse(await readFile(new URL('../assets/widgets/slots.json', import.meta.url), 'utf8'));
  assert.deepEqual(
    plan.slots.map(s => s.id).sort(),
    ['chat.panel-right', 'chat.sidebar', 'diagnostics.context', 'settings.context', 'workbench.grid'],
  );
  // BUG-6：计划里所有 esm entry 必须声明 sandbox:true。
  for (const manifest of plan.manifests) {
    if (manifest.entry && manifest.entry.kind === 'esm') {
      assert.equal(manifest.entry.sandbox, true, manifest.type + ' must declare sandbox:true');
    }
  }
  // 每个 slot 的 widget type 都有对应 manifest（机器可读闭环）。
  const types = new Set(plan.manifests.map(m => m.type));
  for (const slot of plan.slots) {
    for (const widget of slot.widgets) {
      assert.ok(types.has(widget.instance.type), slot.id + ' references unregistered type ' + widget.instance.type);
    }
  }
});
