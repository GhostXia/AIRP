import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';

// 端点契约守卫（Task #5，审计 H2/RR-007 残留方向）
//
// 与 route-contract.test.mjs 的分工：
// - route-contract：动态解析 engine/src/daemon/mod.rs 的 .route() 链，断言
//   「WebUI 每个调用点都已在 engine 路由器注册」——抓 webui↔engine 实时漂移。
// - 本文件（endpoint-guard）：以人工评审过的 golden 清单
//   （fixtures/v1-endpoints.json）为契约面：
//   1) 正向：WebUI 调用的每个 /v1 端点必须存在于 golden 清单且 method 匹配；
//   2) 反向：golden 中每个 ui:true 端点必须确有 WebUI 消费（防清单腐化），
//      ui:false 端点必须显式给出引擎独有理由（如 Conversation 双轨）。
//   本文件不与 engine 源码比对，engine 新增路由不会误红；清单更新方式见
//   DEV-GUIDE「端点契约守卫」与 webui/tests/extract-v1-endpoints.mjs。
//
// 注意：dynamicCalls / dynamicFetchCalls 映射必须与 route-contract.test.mjs
// 保持同步（两处各自持有副本，route-contract 会校验其与实际调用点对应）。

const assetsUrl = new URL('../assets/', import.meta.url);
const golden = JSON.parse(
  await readFile(new URL('./fixtures/v1-endpoints.json', import.meta.url), 'utf8'),
);

// ── 解析辅助（与 route-contract.test.mjs 同源） ──────────────────────────

function callArguments(source, marker) {
  const calls = [];
  let cursor = 0;
  while ((cursor = source.indexOf(marker, cursor)) !== -1) {
    const open = cursor + marker.length - 1;
    let quote = '';
    let escaped = false;
    let depth = 0;
    for (let index = open; index < source.length; index += 1) {
      const char = source[index];
      if (quote) {
        if (escaped) escaped = false;
        else if (char === '\\') escaped = true;
        else if (char === quote) quote = '';
        continue;
      }
      if (char === "'" || char === '"' || char === '`') {
        quote = char;
      } else if (char === '(') {
        depth += 1;
      } else if (char === ')') {
        depth -= 1;
        if (depth === 0) {
          calls.push(source.slice(open + 1, index));
          cursor = index + 1;
          break;
        }
      }
    }
    if (depth !== 0) throw new Error(`unclosed ${marker} call`);
  }
  return calls;
}

function splitTopLevel(source, separator = ',') {
  const parts = [];
  let start = 0;
  let quote = '';
  let escaped = false;
  let depth = 0;
  for (let index = 0; index < source.length; index += 1) {
    const char = source[index];
    if (quote) {
      if (escaped) escaped = false;
      else if (char === '\\') escaped = true;
      else if (char === quote) quote = '';
      continue;
    }
    if (char === "'" || char === '"' || char === '`') quote = char;
    else if ('([{'.includes(char)) depth += 1;
    else if (')]}'.includes(char)) depth -= 1;
    else if (char === separator && depth === 0) {
      parts.push(source.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(source.slice(start).trim());
  return parts;
}

function stringLiteral(expression) {
  const value = expression.trim();
  if (value.length < 2 || !["'", '"'].includes(value[0]) || value.at(-1) !== value[0]) {
    return null;
  }
  return value.slice(1, -1).replace(/\\(['"\\])/g, '$1');
}

function canonicalPath(path) {
  const pathname = ('/' + path.replace(/^\/+/, '')).split(/[?#]/, 1)[0];
  return pathname
    .replace(/\/:[A-Za-z0-9_]+/g, '/:param')
    .replace(/\/\*[A-Za-z0-9_]+/g, '/:param');
}

function staticPath(expression) {
  let path = '';
  for (const part of splitTopLevel(expression, '+')) {
    const literal = stringLiteral(part);
    if (literal !== null) {
      path += literal;
      if (path.includes('?') || path.includes('#')) break;
    } else if (/^encodeURIComponent\s*\(/.test(part)) {
      path += ':param';
    } else if (path.startsWith('/') && /^\(.+\?.+:.+\)$/s.test(part)) {
      break;
    } else {
      return null;
    }
  }
  return path.startsWith('/') ? canonicalPath(path) : null;
}

function routesAreCompatible(clientRoute, engineRoute) {
  const [clientMethod, clientPath] = clientRoute.split(' ');
  const [engineMethod, enginePath] = engineRoute.split(' ');
  if (clientMethod !== engineMethod) return false;
  const clientSegments = clientPath.split('/');
  const engineSegments = enginePath.split('/');
  return clientSegments.length === engineSegments.length
    && clientSegments.every((segment, index) => (
      segment === engineSegments[index]
      || segment === ':param'
      || engineSegments[index] === ':param'
    ));
}

// ── 动态路径与特殊消费的静态声明 ─────────────────────────────────────────
// 与 route-contract.test.mjs 保持同步。

const dynamicCalls = new Map([
  ['card-diff.js|request|url', ['GET /v1/characters/:param/revisions/diff']],
  ['image-gen.js|request|path', ['GET /v1/characters/:param/images']],
  ['chat-space.js|stream|path', [
    'POST /v1/chat/continue',
    'POST /v1/chat/regen',
  ]],
  ['console-runtime.js|request|path', [
    'GET /version',
    'GET /health',
    'GET /v1/settings',
    'GET /v1/characters',
    'GET /v1/presets',
    'GET /v1/scenes',
    'GET /v1/agent/tools',
  ]],
]);

const dynamicFetchCalls = new Map([
  ["entry.js|'health'", ['GET /health']],
  ['card-diff.js|url', ['GET /v1/characters/:param/revisions/diff']],
  ['timeline-export.js|url', ['GET /v1/sessions/:param/:param/timeline/export']],
  ["desktop-session.js|base + '/v1/desktop-session/renew'", ['POST /v1/desktop-session/renew']],
]);
const genericFetchTransports = new Set(['api-client.js', 'agent-test-harness.js']);

// 不经 client.request、由浏览器直接加载的 /v1 资产端点（<img src>）。
// 用源码标记校验消费仍然成立，防止清单凭空声称 UI 消费。
const browserAssetLoads = [
  {
    route: 'GET /v1/characters/:param/images/:param',
    file: 'image-gen.js',
    markers: [
      `+ '/v1/' + resp.image_path`,
      `'images/' + encodeURIComponent(item.filename)`,
    ],
  },
  {
    route: 'GET /v1/characters/:param/sessions/:param/images/:param',
    file: 'image-gen.js',
    markers: [
      `'sessions/' + encodeURIComponent(sessionId)`,
      `'images/' + encodeURIComponent(item.filename)`,
    ],
  },
];

// C-P2：widgets/ 子目录不在顶层 readdir 扫描面内，其直接 fetch 消费点
// 在此声明并以源码标记校验（同 browserAssetLoads 的防腐化思路）。
const subdirectoryConsumers = [
  {
    route: 'GET /v1/extensions/catalog',
    file: 'widgets/boot.js',
    markers: ["fetch('/v1/extensions/catalog'"],
  },
  {
    route: 'POST /v1/widget-intents',
    file: 'widgets/boot.js',
    markers: ["fetch('/v1/widget-intents'"],
  },
];

// ── 扫描 WebUI 调用点 ────────────────────────────────────────────────────

async function collectUsedRoutes() {
  const used = [];
  const fileSources = new Map();
  const files = (await readdir(assetsUrl)).filter(file => file.endsWith('.js')).sort();

  for (const file of files) {
    const source = await readFile(new URL(file, assetsUrl), 'utf8');
    fileSources.set(file, source);

    // client.request / client.stream（与 route-contract 同规则）
    for (const kind of ['request', 'stream']) {
      for (const call of callArguments(source, `client.${kind}(`)) {
        const args = splitTopLevel(call);
        if (kind === 'request' && args[0]?.startsWith('...')) continue;
        const method = kind === 'stream' ? 'POST' : stringLiteral(args[0] || '')?.toUpperCase();
        const pathExpression = kind === 'stream' ? args[0] : args[1];
        assert.ok(method, `${file}: client.${kind} must use a literal HTTP method`);
        assert.ok(pathExpression, `${file}: client.${kind} has no path`);
        const path = staticPath(pathExpression);
        if (path) {
          used.push([`${method} ${path}`, `${file}: ${pathExpression}`]);
          continue;
        }
        const key = `${file}|${kind}|${pathExpression.trim()}`;
        const routes = dynamicCalls.get(key);
        assert.ok(routes, `${file}: unresolved dynamic client.${kind} path: ${pathExpression}`);
        for (const route of routes) used.push([route, `${file}: ${pathExpression}`]);
      }
    }

    // 注入 transport：options.request('METHOD', path)（如 workbench-reextract.js）
    for (const call of callArguments(source, 'options.request(')) {
      const args = splitTopLevel(call);
      const method = stringLiteral(args[0] || '')?.toUpperCase();
      const path = args[1] ? staticPath(args[1]) : null;
      assert.ok(method && path, `${file}: unresolved injected options.request call: ${call.slice(0, 80)}`);
      used.push([`${method} ${path}`, `${file}: options.request(${args[1]})`]);
    }

    // 直接 fetch（非通用 transport 文件）
    for (const call of callArguments(source, 'fetch(')) {
      if (genericFetchTransports.has(file)) continue;
      const expression = splitTopLevel(call)[0];
      const key = `${file}|${expression}`;
      const routes = dynamicFetchCalls.get(key);
      assert.ok(routes, `${file}: unresolved direct fetch path: ${expression}`);
      for (const route of routes) used.push([route, `${file}: fetch(${expression})`]);
    }
  }

  // <img src> 等浏览器直接加载
  for (const load of browserAssetLoads) {
    const source = fileSources.get(load.file);
    assert.ok(source, `browserAssetLoads 引用了不存在的文件: ${load.file}`);
    for (const marker of load.markers) {
      assert.ok(
        source.includes(marker),
        `${load.file}: 浏览器资产加载标记消失: ${marker}（端点 ${load.route} 的 ui:true 标注需要复核）`,
      );
    }
    used.push([load.route, `${load.file}: <img src> asset load`]);
  }

  // widgets/ 子目录消费点（源码标记校验）
  for (const consumer of subdirectoryConsumers) {
    const source = await readFile(new URL(consumer.file, assetsUrl), 'utf8');
    for (const marker of consumer.markers) {
      assert.ok(
        source.includes(marker),
        `${consumer.file}: 子目录消费标记消失: ${marker}（端点 ${consumer.route} 的 ui:true 标注需要复核）`,
      );
    }
    used.push([consumer.route, `${consumer.file}: ${consumer.markers[0]}`]);
  }

  return used;
}

const METHOD_SET = new Set(['GET', 'POST', 'PUT', 'DELETE', 'PATCH']);

test('golden fixture is well-formed', () => {
  assert.ok(Array.isArray(golden.endpoints) && golden.endpoints.length > 0, 'endpoints 为空');
  const seen = new Set();
  for (const entry of golden.endpoints) {
    const key = `${entry.method} ${canonicalPath(entry.path)}`;
    assert.ok(METHOD_SET.has(entry.method), `非法 method: ${entry.method} ${entry.path}`);
    assert.ok(entry.path.startsWith('/v1/'), `golden 仅收录 /v1 端点: ${entry.path}`);
    assert.ok(!seen.has(key), `golden 重复条目: ${key}`);
    seen.add(key);
    assert.equal(typeof entry.ui, 'boolean', `${key}: ui 必须为布尔值`);
    if (!entry.ui) {
      assert.ok(
        typeof entry.reason === 'string' && entry.reason.length > 0,
        `${key}: ui:false 必须给出引擎独有理由（reason）`,
      );
    }
    if (entry.bodyLimit !== undefined) {
      assert.match(entry.bodyLimit, /^\d+(B|KB|MB)$/, `${key}: bodyLimit 格式非法`);
    }
  }
});

test('every /v1 endpoint called by WebUI exists in the golden inventory', async () => {
  const used = await collectUsedRoutes();
  const usedV1 = used.filter(([route]) => route.split(' ')[1].startsWith('/v1/'));
  const uniqueRoutes = new Set(usedV1.map(([route]) => route));
  assert.ok(uniqueRoutes.size >= 80, `扫描异常：仅发现 ${uniqueRoutes.size} 个 /v1 调用`);

  const missing = usedV1.filter(([route]) => (
    !golden.endpoints.some(entry => (
      entry.method === route.split(' ')[0]
      && routesAreCompatible(route, `${entry.method} ${canonicalPath(entry.path)}`)
    ))
  ));
  assert.deepEqual(
    missing,
    [],
    `WebUI 调用了 golden 清单之外的 /v1 端点（请先更新 fixtures/v1-endpoints.json，`
    + `可用 node webui/tests/extract-v1-endpoints.mjs 对账）:\n`
    + [...new Set(missing.map(([route, source]) => `- ${route} (${source})`))].join('\n'),
  );
});

test('golden ui:true entries all have a live WebUI consumer (no stale inventory)', async () => {
  const used = await collectUsedRoutes();
  const usedRoutes = used.map(([route]) => route);
  const stale = golden.endpoints.filter(entry => entry.ui && !usedRoutes.some(route => (
    routesAreCompatible(route, `${entry.method} ${canonicalPath(entry.path)}`)
  )));
  assert.deepEqual(
    stale.map(entry => `${entry.method} ${entry.path}`),
    [],
    'golden 声称 ui:true 但 WebUI 已无消费点：请移除端点或将 ui 改为 false 并补充 reason',
  );
});
