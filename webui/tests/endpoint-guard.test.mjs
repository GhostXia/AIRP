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
  // 审计 #485 W1：扫描面改为递归后，widgets/ 子目录的 fetch 调用点由本 map
  // 解析（engineUrl() 包装无法静态提路径）。替代原 subdirectoryConsumers
  // 标记机制：新增子目录 fetch 若未登记会直接报 unresolved，不再静默漏扫。
  ["widgets/boot.js|engineUrl('/v1/extensions/catalog')", ['GET /v1/extensions/catalog']],
  ["widgets/boot.js|engineUrl('/v1/extensions/grants')", ['GET /v1/extensions/grants']],
  ["widgets/boot.js|engineUrl('/v1/plugins')", ['GET /v1/plugins']],
  ["widgets/boot.js|engineUrl('/v1/widget-intents')", ['POST /v1/widget-intents']],
  // 降级读静态 slots.json：非 /v1 端点，不产生路由消费（空数组 = 已解析）。
  ["widgets/boot.js|new URL('./slots.json', import.meta.url)", []],
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

// 审计 #485 W1：原非递归 readdir 扫不到 assets/ 子目录（如 widgets/）内的
// fetch，子目录新增调用点时 ui:false 端点不告警。现改为递归扫描；子目录
// 消费点若无法静态解析，照旧在 dynamicFetchCalls / dynamicCalls 登记，
// 不再需要单独的 subdirectoryConsumers 标记面。

// ── 扫描 WebUI 调用点 ────────────────────────────────────────────────────

async function collectUsedRoutes() {
  const used = [];
  const fileSources = new Map();
  const files = (await readdir(assetsUrl, { recursive: true }))
    .map(file => file.replaceAll('\\', '/'))
    .filter(file => file.endsWith('.js'))
    .sort();

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
      const hasRetentionMetadata = (
        typeof entry.owner === 'string'
        && entry.owner.length > 0
        && entry.provenance
        && typeof entry.provenance === 'object'
        && typeof entry.provenance.source === 'string'
        && typeof entry.provenance.ref === 'string'
        && /^\d{4}-\d{2}-\d{2}$/.test(entry.reviewAfter || '')
      );
      const hasExternalContract = (
        entry.externalContract
        && typeof entry.externalContract === 'object'
        && typeof entry.externalContract.source === 'string'
        && entry.externalContract.source.length > 0
      );
      assert.ok(
        hasRetentionMetadata || hasExternalContract,
        `${key}: ui:false 必须给出 provenance/owner/reviewAfter 或 externalContract`,
      );
    }
    if (entry.bodyLimit !== undefined) {
      assert.match(entry.bodyLimit, /^\d+(B|KB|MB)$/, `${key}: bodyLimit 格式非法`);
    }
  }
});

test('ui:false retention metadata stays synchronized with the external compatibility contract', async () => {
  const retained = golden.endpoints.filter(entry => !entry.ui);
  const contractSources = new Map();
  for (const entry of retained) {
    const source = entry.externalContract?.source;
    assert.ok(source, `${entry.method} ${entry.path}: 缺少 externalContract.source`);
    assert.match(source, /^(docs|engine)\/[A-Za-z0-9._/-]+\.md$/, `${entry.method} ${entry.path}: externalContract.source 必须是仓库内 Markdown 文件`);
    if (!contractSources.has(source)) {
      contractSources.set(source, await readFile(new URL(`../../${source}`, import.meta.url), 'utf8'));
    }
  }

  const contractRows = new Map();
  for (const [source, text] of contractSources) {
    const rows = text.matchAll(/^\|\s*(GET|POST|PUT|DELETE|PATCH)\s*\|\s*`([^`]+)`\s*\|\s*([^|]+?)\s*\|\s*(\d{4}-\d{2}-\d{2})\s*\|/gm);
    for (const [, method, path, owner, reviewAfter] of rows) {
      const route = `${method} ${canonicalPath(path)}`;
      assert.equal(contractRows.has(route), false, `compatibility contract 重复条目: ${route}`);
      contractRows.set(route, { source, owner: owner.trim(), reviewAfter });
    }
  }

  const retainedRoutes = new Set(retained.map(entry => `${entry.method} ${canonicalPath(entry.path)}`));
  assert.deepEqual(
    [...contractRows.keys()].sort(),
    [...retainedRoutes].sort(),
    'compatibility contract 路由集合必须与 fixture 的全部 ui:false 条目一致（不得漏掉 /v1/conversations*）',
  );

  const provenanceSources = new Map();
  for (const entry of retained) {
    const route = `${entry.method} ${canonicalPath(entry.path)}`;
    const contract = contractRows.get(route);
    assert.ok(contract, `${route}: externalContract 未在合同表中登记`);
    assert.equal(entry.owner, contract.owner, `${route}: fixture owner 与合同 owner 不同步`);
    assert.equal(entry.reviewAfter, contract.reviewAfter, `${route}: fixture reviewAfter 与合同 review-after 不同步`);

    const sourceText = contractSources.get(contract.source);
    assert.ok(
      sourceText.includes(`| ${entry.method} | \`${entry.path}\` |`),
      `${route}: externalContract source 未包含精确方法/路径行`,
    );

    const provenance = entry.provenance;
    assert.ok(provenance, `${route}: 缺少 provenance`);
    assert.equal(provenance.source, golden.generatedFrom.file, `${route}: provenance.source 必须与 generatedFrom.file 同步`);
    assert.equal(provenance.ref, golden.generatedFrom.ref, `${route}: provenance.ref 必须与 generatedFrom.ref 同步`);
    if (!provenanceSources.has(provenance.source)) {
      provenanceSources.set(
        provenance.source,
        await readFile(new URL(`../../${provenance.source}`, import.meta.url), 'utf8'),
      );
    }
    assert.ok(
      provenanceSources.get(provenance.source).includes(`"${entry.path}"`),
      `${route}: provenance source 未包含路由声明`,
    );
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
