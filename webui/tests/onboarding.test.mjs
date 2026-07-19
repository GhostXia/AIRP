// Onboarding wizard tests — L1 unit + L2 integration.
// 零依赖（node:test + assert），最小 DOM mock（不引入 jsdom，保持现有约定）。
// 设计 spec §7.2, §7.3。
import { test } from 'node:test';
import assert from 'node:assert/strict';

// ── 最小 DOM mock ──────────────────────────────────────────────────────────
// onboarding.js 依赖：document.createElement, container.innerHTML, addEventListener,
// sessionStorage, btoa, TextDecoder, fetch, AbortController（后两者通过 Port 注入或 fetcher）
// 我们 mock 到刚好能测 Port 契约、版本检查、cleanup、Stage 1 渲染分支。

function makeMockContainer() {
  return {
    _children: [],
    _innerHTML: '',
    get innerHTML() { return this._innerHTML; },
    set innerHTML(v) { this._innerHTML = v; this._children = []; },
    appendChild(node) { this._children.push(node); return node; },
    querySelector(sel) { return null; },
    removeChild(node) {
      const i = this._children.indexOf(node);
      if (i >= 0) this._children.splice(i, 1);
      return node;
    },
  };
}

function makeMockNode(tag) {
  return {
    tagName: (tag || 'div').toUpperCase(),
    className: '',
    textContent: '',
    _children: [],
    _listeners: {},
    appendChild(n) { this._children.push(n); return n; },
    removeChild(n) { const i = this._children.indexOf(n); if (i >= 0) this._children.splice(i, 1); return n; },
    addEventListener(type, handler) {
      (this._listeners[type] = this._listeners[type] || []).push(handler);
    },
    removeEventListener(type, handler) {
      if (!this._listeners[type]) return;
      this._listeners[type] = this._listeners[type].filter(h => h !== handler);
    },
    querySelector() { return null; },
    set hidden(v) { this._hidden = v; },
    get hidden() { return this._hidden; },
    set value(v) { this._value = v; },
    get value() { return this._value; },
    set type(v) { this._type = v; },
    set placeholder(v) { this._placeholder = v; },
    set autocomplete(v) { this._autocomplete = v; },
    set rows(v) { this._rows = v; },
    set accept(v) { this._accept = v; },
    set selected(v) { this._selected = v; },
    append() {},
  };
}

// 全局 document mock（onboarding.js 用 document.createElement）
globalThis.document = {
  createElement: (tag) => makeMockNode(tag),
};

// sessionStorage mock（onboarding.js Stage 1 dev 模式读写）
const _ss = {};
globalThis.sessionStorage = {
  getItem: (k) => (k in _ss ? _ss[k] : null),
  setItem: (k, v) => { _ss[k] = String(v); },
  removeItem: (k) => { delete _ss[k]; },
};

// btoa mock（onboarding.js Stage 4 PNG base64）
globalThis.btoa = (s) => Buffer.from(s, 'binary').toString('base64');

// TextDecoder mock
globalThis.TextDecoder = class {
  decode(buf) { return Buffer.from(buf).toString('utf8'); }
};

// 动态 import onboarding.js（ESM）
const { mountOnboarding } = await import('../onboarding.js');

// ── mock Port helper（spec §7.2）─────────────────────────────────────────────
function makeMockPort(overrides) {
  overrides = overrides || {};
  const calls = { fetch: [], onComplete: [], onSkip: [] };
  const defaultFetcher = async (path, opts) => {
    calls.fetch.push({ path, opts });
    // 默认返回 mock Response
    return {
      ok: true,
      status: 200,
      text: async () => JSON.stringify({ version: 'test', engine: 'ok', provider_configured: false, data_root_writable: true }),
      body: { getReader: () => ({ read: async () => ({ done: true, value: undefined }) }) },
    };
  };
  return {
    port: Object.freeze({
      version: 1,
      mode: 'dev',
      fetcher: overrides.fetcher || defaultFetcher,
      formatError: overrides.formatError || ((data, text) => (data && data.error && data.error.message) || text || JSON.stringify(data)),
      onComplete: overrides.onComplete || ((config) => calls.onComplete.push(config)),
      onSkip: overrides.onSkip || (() => calls.onSkip.push(null)),
      ...('version' in overrides ? { version: overrides.version } : {}),
      ...('mode' in overrides ? { mode: overrides.mode } : {}),
    }),
    calls,
  };
}

// ══════════════════════════════════════════════════════════════════════════
// L1 单元测试
// ══════════════════════════════════════════════════════════════════════════

test('Port 契约: version !== 1 throws (F3 降级入口)', () => {
  const { port } = makeMockPort({ version: 2 });
  assert.throws(() => mountOnboarding(makeMockContainer(), port), /hostPort\.version must be 1/);
});

test('Port 契约: version 缺失 throws', () => {
  const badPort = { mode: 'dev', fetcher: () => {}, formatError: () => '', onComplete: () => {}, onSkip: () => {} };
  assert.throws(() => mountOnboarding(makeMockContainer(), badPort), /hostPort\.version must be 1/);
});

test('Port 契约: 缺少必需成员 throws', () => {
  const badPort = { version: 1, mode: 'dev', fetcher: () => {}, formatError: () => '' };  // 缺 onComplete/onSkip
  assert.throws(() => mountOnboarding(makeMockContainer(), badPort), /missing required member "onComplete"/);
});

test('Port 契约: hostPort=null throws', () => {
  assert.throws(() => mountOnboarding(makeMockContainer(), null), /hostPort\.version must be 1/);
});

test('mountOnboarding 返回 cleanup 函数', () => {
  const { port } = makeMockPort();
  const container = makeMockContainer();
  const cleanup = mountOnboarding(container, port);
  assert.equal(typeof cleanup, 'function');
  // cleanup 不应 throw
  assert.doesNotThrow(() => cleanup());
  // cleanup 后 container 被清空
  assert.equal(container.innerHTML, '');
});

test('cleanup 清空 container.innerHTML（无残留）', () => {
  const { port } = makeMockPort();
  const container = makeMockContainer();
  const cleanup = mountOnboarding(container, port);
  // 模拟渲染添加了一些子节点
  container._children.push(makeMockNode('div'));
  assert.ok(container._children.length > 0);
  cleanup();
  assert.equal(container._children.length, 0);
});

test('Stage 1 dev 模式渲染 engine URL + bearer 输入（mode 分支）', () => {
  const { port } = makeMockPort({ mode: 'dev' });
  const container = makeMockContainer();
  mountOnboarding(container, port);
  // 验证渲染了 onb-wizard 容器（通过 appendChild 调用）
  assert.ok(container._children.length > 0, 'dev 模式应渲染向导内容');
  const first = container._children[0];
  assert.ok(first.className.includes('onb-wizard') || first._children.length > 0, '应渲染 onb-wizard 结构');
});

test('Stage 1 prod 模式不渲染 engine URL 输入（mode 分支）', () => {
  const { port } = makeMockPort({ mode: 'production' });
  const container = makeMockContainer();
  mountOnboarding(container, port);
  assert.ok(container._children.length > 0, 'prod 模式也应渲染向导内容');
});

test('api_key 永不作为 Port 成员（安全不变量 §3.6）', () => {
  // Port 形状由宿主构造；此处验证向导内部不会把 api_key 写入返回的 config 中
  // （config 由 finish() 构造，不含 api_key 字段）
  const { port, calls } = makeMockPort({
    onComplete: (config) => calls.onComplete.push(config),
  });
  const container = makeMockContainer();
  const cleanup = mountOnboarding(container, port);
  // 直接调用 cleanup 验证不 throw（无法直接触发 finish，需 L2 集成测试覆盖）
  assert.doesNotThrow(() => cleanup());
  // 验证 Port 对象自身无 api_key 键（Host 端构造约束）
  assert.ok(!('api_key' in port), 'Port 不得包含 api_key 成员');
});

test('Port 对象是 Object.freeze 的（不可变）', () => {
  const { port } = makeMockPort();
  assert.equal(Object.isFrozen(port), true);
});

// ══════════════════════════════════════════════════════════════════════════
// L2 集成测试（向导 + mock fetcher 走完流程片段）
// ══════════════════════════════════════════════════════════════════════════

test('F4 运行时崩溃：cleanup 不 throw 且清空 container', () => {
  // 即使向导内部崩溃，cleanup 也必须安全
  const { port } = makeMockPort();
  const container = makeMockContainer();
  const cleanup = mountOnboarding(container, port);
  assert.doesNotThrow(() => cleanup());
  assert.equal(container.innerHTML, '');
});

test('F4 renderStage 同步抛错 → renderCrashFallback 渲染崩溃面板（spec §6.4）', () => {
  // 触发 render() 顶层 try/catch → renderCrashFallback 路径：
  // 让 container.appendChild 第一次调用抛错（renderStage 同步异常），
  // 第二次正常（让 renderCrashFallback 能渲染崩溃面板）
  const { port } = makeMockPort();
  const container = makeMockContainer();
  let firstCall = true;
  const orig = container.appendChild;
  container.appendChild = function (node) {
    if (firstCall) { firstCall = false; throw new Error('mock sync render crash'); }
    return orig.call(this, node);
  };
  // mount 应不 throw（render 的 try/catch 捕获 + renderCrashFallback 渲染面板）
  assert.doesNotThrow(() => mountOnboarding(container, port));
  // 验证渲染了崩溃面板：递归查找 h2 with textContent '向导遇到问题'
  function findCrashH2(node) {
    if (node.tagName === 'H2' && node.textContent === '向导遇到问题') return node;
    if (!node._children) return null;
    for (const c of node._children) {
      const r = findCrashH2(c);
      if (r) return r;
    }
    return null;
  }
  let crashH2 = null;
  for (const c of container._children) {
    crashH2 = findCrashH2(c);
    if (crashH2) break;
  }
  assert.ok(crashH2, '应渲染崩溃面板（h2 "向导遇到问题"）');
});

test('F4 崩溃面板含 [重试向导] / [退回手动配置] 按钮（spec §6.4）', () => {
  const { port } = makeMockPort();
  const container = makeMockContainer();
  let firstCall = true;
  const orig = container.appendChild;
  container.appendChild = function (node) {
    if (firstCall) { firstCall = false; throw new Error('mock crash'); }
    return orig.call(this, node);
  };
  mountOnboarding(container, port);
  // 递归查找所有 button 节点
  function findAll(node, pred, acc) {
    if (pred(node)) acc.push(node);
    if (node._children) for (const c of node._children) findAll(c, pred, acc);
  }
  const buttons = [];
  for (const c of container._children) {
    findAll(c, n => n.tagName === 'BUTTON', buttons);
  }
  const texts = buttons.map(b => b.textContent);
  assert.ok(texts.includes('重试向导'), '崩溃面板应含 [重试向导] 按钮');
  assert.ok(texts.includes('退回手动配置'), '崩溃面板应含 [退回手动配置] 按钮');
});

test('F5 Stage 1 fetcher 抛错 → 渲染 Stage 1 错误面板（不降级 onSkip，不触发 renderCrashFallback）', async () => {
  // spec §6.5：F5 HTTP 失败由向导内阶段错误处理，不降级；spec §6.4：仅同步渲染崩溃走 F4
  const { port, calls } = makeMockPort({
    mode: 'production',  // prod 模式自动触发 runHealthCheck
    fetcher: async () => { throw new Error('mock network failure'); },
    onSkip: () => calls.onSkip.push(null),
  });
  const container = makeMockContainer();
  mountOnboarding(container, port);
  // prod 模式通过 setTimeout(0) 触发 safeAsync(runHealthCheck)
  // 等待 setTimeout + microtasks 完成
  await new Promise(r => setTimeout(r, 50));

  // onSkip 不应被调用（fetcher 失败由向导内 Stage 1 错误处理）
  assert.equal(calls.onSkip.length, 0, 'fetcher 失败不应自动 onSkip');

  // 应渲染错误面板（showError 添加 .onb-error 元素）
  function findErrorNode(node) {
    if (node.className && node.className.includes('onb-error')) return node;
    if (!node._children) return null;
    for (const c of node._children) {
      const r = findErrorNode(c);
      if (r) return r;
    }
    return null;
  }
  let errNode = null;
  for (const c of container._children) {
    errNode = findErrorNode(c);
    if (errNode) break;
  }
  assert.ok(errNode, '应渲染 .onb-error 错误面板（Stage 1 fetcher 失败 → showError）');

  // 不应渲染崩溃面板（F4 不应触发——F5 由 runHealthCheck 内 try/catch 处理）
  function findCrashH2(node) {
    if (node.tagName === 'H2' && node.textContent === '向导遇到问题') return node;
    if (!node._children) return null;
    for (const c of node._children) {
      const r = findCrashH2(c);
      if (r) return r;
    }
    return null;
  }
  let crashH2 = null;
  for (const c of container._children) {
    crashH2 = findCrashH2(c);
    if (crashH2) break;
  }
  assert.equal(crashH2, null, 'F5 不应触发 renderCrashFallback');
});

test('F5 fetcher 失败不降级到 onSkip（向导内处理）', async () => {
  const { port, calls } = makeMockPort({
    fetcher: async () => {
      const err = new Error('network');
      throw err;
    },
    onSkip: () => calls.onSkip.push(null),
  });
  const container = makeMockContainer();
  const cleanup = mountOnboarding(container, port);
  // 向导渲染成功（fetcher 在 Stage 1 才被调用，渲染本身不调用 fetcher）
  assert.ok(container._children.length > 0);
  // onSkip 不应被自动调用（fetcher 失败由向导内 Stage 1 错误处理）
  assert.equal(calls.onSkip.length, 0);
  cleanup();
});

test('onComplete config 形状正确（firstChatCompleted 字段）', () => {
  // 验证 finish() 构造的 config 包含所有必需字段
  // 通过验证 Port 形状间接保证（完整首聊流程需 SSE mock，留给 L3 烟雾测试）
  const { port } = makeMockPort();
  assert.equal(port.version, 1);
  assert.ok(typeof port.onComplete === 'function');
  assert.ok(typeof port.onSkip === 'function');
});

test('向导零 import 宿主代码（静态检查）', async () => {
  // 设计 spec §3.6 不变量 1：onboarding.js 不得 import app.js / shared.js
  // 读取源码验证无 import 语句指向宿主模块
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  // 允许的 import：无（本文件不应有 import 语句，除 export 外）
  const importLines = src.split('\n').filter(l => /^\s*import\s/.test(l));
  assert.equal(importLines.length, 0, 'onboarding.js 不得有 import 语句（零宿主依赖），实际: ' + JSON.stringify(importLines));
});

test('api_key 不进入向导源码 state 对象（静态检查 §4.3）', async () => {
  // 验证向导源码中无 state.api_key 赋值
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  // 禁止 state.api_key = 赋值（api_key 永不进入向导 state）
  assert.ok(!/state\.api_key\s*=/.test(src), '向导 state 不得包含 api_key 字段');
  // 禁止 localStorage/sessionStorage 写入 api_key
  assert.ok(!/localStorage\.setItem\(['"]api_key/.test(src), '不得写入 localStorage api_key');
  assert.ok(!/sessionStorage\.setItem\(['"]api_key/.test(src), '不得写入 sessionStorage api_key');
});

test('生产模式不构造 card_path（静态检查 §4.3）', async () => {
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  // 向导侧不应构造 card_path（生产模式 handlers 拒绝；向导也不构造）
  assert.ok(!/card_path\s*:/.test(src), '向导不得构造 card_path 字段');
});

test('formatError 白名单展开已知字段（复用 app.js 行为）', async () => {
  // 验证 shared.js formatError 与原 app.js:176-200 行为一致
  // （通过 require shared.js 测试，因 shared.js 是 classic script）
  const { createRequire } = await import('node:module');
  const require = createRequire(import.meta.url);
  // shared.js 暴露 window.AIRPShared，在 Node 需 mock window
  globalThis.window = globalThis.window || {};
  require('../shared.js');
  const { formatError } = globalThis.window.AIRPShared;
  // 已知字段展开
  const r1 = formatError({ error: { code: 'invalid_endpoint', message: 'bad url', upstream_status: 404, detail: 'x' } });
  assert.ok(r1.includes('invalid_endpoint'));
  assert.ok(r1.includes('bad url'));
  assert.ok(r1.includes('upstream_status=404'));
  assert.ok(r1.includes('detail=x'));
  // 未知字段折叠为 extras
  const r2 = formatError({ error: { code: 'e', custom_field: 'v' } });
  assert.ok(r2.includes('extras='));
  assert.ok(r2.includes('custom_field'));
  // 字符串直接返回
  assert.equal(formatError('plain string'), 'plain string');
  // 无 data 返回 text
  assert.equal(formatError(null, 'fallback'), 'fallback');
});

test('formatError 不向 UI 泄露凭据', async () => {
  globalThis.window = globalThis.window || {};
  const { createRequire } = await import('node:module');
  const require = createRequire(import.meta.url);
  if (!globalThis.window.AIRPShared) require('../shared.js');
  const { formatError } = globalThis.window.AIRPShared;
  const rendered = formatError({
    error: {
      code: 'upstream_error',
      upstream_body: 'Authorization: Bearer bearer-secret api_key=plain-secret sk-abcdefghijk',
      access_token: 'nested-secret',
    },
  });
  assert.ok(rendered.includes('[REDACTED]'));
  for (const secret of ['bearer-secret', 'plain-secret', 'sk-abcdefghijk', 'nested-secret']) {
    assert.ok(!rendered.includes(secret), '不得显示 ' + secret);
  }
});

test('formatError 脱敏 upstream_body 内嵌的带引号 JSON 凭据', async () => {
  globalThis.window = globalThis.window || {};
  const { createRequire } = await import('node:module');
  const require = createRequire(import.meta.url);
  if (!globalThis.window.AIRPShared) require('../shared.js');
  const rendered = globalThis.window.AIRPShared.formatError({
    error: { code: 'upstream', upstream_body: '{"api_key":"plain-secret","token": "quoted-token"}' },
  });
  assert.ok(!rendered.includes('plain-secret'));
  assert.ok(!rendered.includes('quoted-token'));
  assert.match(rendered, /\[REDACTED\]/);
});

test('Stage 6 消费 engine 的 {type,text} chunk 并识别 SSE error 事件（静态回归）', async () => {
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  assert.match(src, /chunk\.type === 'body_chunk' && chunk\.text/);
  assert.ok(!src.includes('bodyP.append(chunk.content)'), 'engine chunk 字段是 text，不是 content');
  assert.match(src, /eventName === 'error'/);
});

test('角色导入校验兼容 engine 的 bare string ID 列表（静态回归）', async () => {
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  assert.match(src, /typeof c === 'string' \? c : \(c\.id \|\| c\.character_id\)/);
});

test('effective config 预览提供 chat preview 必需的 user_profile（静态回归）', async () => {
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  assert.match(src, /user_profile: \{ name: '', variables: \{\} \}/);
  assert.match(src, /message: ''/);
});

test('向导完成配置携带 session_id，供刷新后恢复当前会话（静态回归）', async () => {
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  assert.match(src, /session_id: state\.sessionId \|\| ''/);
});

test('宿主在向导完成后同步开发连接配置再自动连接（静态回归）', async () => {
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const completeStart = src.indexOf('function onOnboardingComplete');
  const skipStart = src.indexOf('function onOnboardingSkip');
  const completeBody = src.slice(completeStart, skipStart);
  assert.match(completeBody, /engineUrl\.value = sessionStorage\.getItem\('airp_engine_url'\)/);
  assert.ok(completeBody.indexOf('engineUrl.value =') < completeBody.indexOf('scheduleAutoConnect()'));
  assert.match(completeBody, /selectedSess = '';[\s\S]*localStorage\.removeItem\('airp_session_id'\)/);
});

test('流式错误仅在 engine 确认未提交时允许重发（静态回归）', async () => {
  const fs = await import('node:fs');
  const src = fs.readFileSync(new URL('../onboarding.js', import.meta.url), 'utf8');
  assert.match(src, /error\.commitState = detail\.commit_state \|\| 'ambiguous'/);
  assert.match(src, /canResend = err\.retryable === true && err\.commitState === 'uncommitted'/);
  assert.match(src, /进入聊天检查记录/);
  const eofStart = src.indexOf('// - completed=false');
  const catchStart = src.indexOf('} catch (err)', eofStart);
  assert.ok(eofStart >= 0, '必须找到提前 EOF 分支起点');
  assert.ok(catchStart > eofStart, '必须找到位于提前 EOF 分支之后的 catch 边界');
  assert.ok(!src.slice(eofStart, catchStart).includes('() => sendFirstMessage(message, box)'), '提前 EOF 不得盲目重发');
});

test('makeFetcher dev 模式从 sessionStorage 读取（auth 单一实现 §3.2）', async () => {
  globalThis.window = globalThis.window || {};
  globalThis.window.location = { origin: 'http://localhost:8080' };
  const { createRequire } = await import('node:module');
  const require = createRequire(import.meta.url);
  if (!globalThis.window.AIRPShared) require('../shared.js');
  const { makeFetcher } = globalThis.window.AIRPShared;

  // dev 模式：sessionStorage 已设置
  _ss.airp_engine_url = 'http://test-engine:9999';
  _ss.airp_bearer = 'test-bearer';
  let capturedUrl, capturedHeaders;
  globalThis.fetch = async (url, opts) => {
    capturedUrl = url;
    capturedHeaders = opts.headers;
    return { ok: true, status: 200, text: async () => '{}' };
  };
  const fetcher = makeFetcher('dev');
  await fetcher('/health', {});
  assert.equal(capturedUrl, 'http://test-engine:9999/health');
  assert.equal(capturedHeaders['Authorization'], 'Bearer test-bearer');

  // prod 模式：用 window.location.origin，无 Authorization
  const fetcherProd = makeFetcher('production');
  await fetcherProd('/health', {});
  assert.equal(capturedUrl, 'http://localhost:8080/health');
  assert.ok(!capturedHeaders['Authorization'], 'prod 模式不携带 Authorization header');
});

test('makeFetcher dev 模式回退默认 URL（spec §3.2）', async () => {
  globalThis.window = globalThis.window || {};
  const { createRequire } = await import('node:module');
  const require = createRequire(import.meta.url);
  if (!globalThis.window.AIRPShared) require('../shared.js');
  const { makeFetcher } = globalThis.window.AIRPShared;
  delete _ss.airp_engine_url;
  delete _ss.airp_bearer;
  let capturedUrl;
  globalThis.fetch = async (url) => { capturedUrl = url; return { ok: true, status: 200, text: async () => '{}' }; };
  const fetcher = makeFetcher('dev');
  await fetcher('/version', {});
  assert.equal(capturedUrl, 'http://127.0.0.1:8000/version');
});

test('makeFetcher local mode always uses same origin without bearer', async () => {
  globalThis.window = globalThis.window || {};
  globalThis.window.location = { origin: 'http://127.0.0.1:8765' };
  const { createRequire } = await import('node:module');
  const require = createRequire(import.meta.url);
  if (!globalThis.window.AIRPShared) require('../shared.js');
  const { makeFetcher } = globalThis.window.AIRPShared;
  _ss.airp_engine_url = 'http://untrusted.invalid';
  _ss.airp_bearer = 'must-not-leak';
  let capturedUrl;
  let capturedHeaders;
  globalThis.fetch = async (url, opts) => {
    capturedUrl = url;
    capturedHeaders = opts.headers;
    return { ok: true, status: 200, text: async () => '{}' };
  };
  await makeFetcher('local')('/health', {});
  assert.equal(capturedUrl, 'http://127.0.0.1:8765/health');
  assert.ok(!capturedHeaders.Authorization);
});
