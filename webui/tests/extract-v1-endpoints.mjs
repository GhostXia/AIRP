#!/usr/bin/env node
// 一次性脚本：从 engine/src/daemon/mod.rs 抽取全部 /v1 路由，并扫描
// webui/assets/*.js 的实际调用点，生成 golden 清单草案（含 ui 消费标注）。
//
// 用法（仓库根目录）:
//   node webui/tests/extract-v1-endpoints.mjs [--write] [--working-tree]
//     默认解析 git HEAD 版本的 engine/src/daemon/mod.rs（golden 以 main 为准，
//     不受工作区未提交改动影响）；--working-tree 改为解析磁盘上的当前文件。
//     默认仅向 stdout 打印草案 JSON 与统计；
//     --write 时将草案写入 webui/tests/fixtures/v1-endpoints.draft.json
//     （注意：草案中 ui:false 项的 reason 为空，需人工补充后再合入正式 fixture）。
//
// 背景：Task #5 端点契约守卫。正式 fixture 为 webui/tests/fixtures/v1-endpoints.json，
// 本脚本只在 engine 路由大幅变动后用于重新对账，不作为 CI 依赖。

import { readFile, readdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';

const assetsUrl = new URL('../assets/', import.meta.url);
const repoRoot = new URL('../../', import.meta.url);
const routerSource = process.argv.includes('--working-tree')
  ? await readFile(new URL('engine/src/daemon/mod.rs', repoRoot), 'utf8')
  : execFileSync('git', ['show', 'HEAD:engine/src/daemon/mod.rs'], {
    cwd: repoRoot,
    encoding: 'utf8',
    maxBuffer: 16 * 1024 * 1024,
  });

// ── 以下解析函数与 webui/tests/route-contract.test.mjs 保持同源 ──

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

// ── engine 路由抽取（含 body-limit 关联） ──

function evalLimitExpression(expression) {
  const bytes = expression.split('*').map(part => Number(part.trim())).reduce((a, b) => a * b, 1);
  if (!Number.isFinite(bytes)) return expression;
  if (bytes >= 1024 * 1024 && bytes % (1024 * 1024) === 0) return `${bytes / (1024 * 1024)}MB`;
  if (bytes >= 1024 && bytes % 1024 === 0) return `${bytes / 1024}KB`;
  return `${bytes}B`;
}

function engineRouteEntries(source) {
  const entries = [];
  for (const call of callArguments(source, '.route(')) {
    const args = splitTopLevel(call);
    const path = stringLiteral(args[0] || '');
    if (!path || !args[1]) continue;
    const handler = args[1];
    const positions = [];
    for (const match of handler.matchAll(/\b(get|post|put|delete|patch)\s*\(/g)) {
      positions.push({ method: match[1].toUpperCase(), start: match.index, end: match.index + match[0].length });
    }
    positions.forEach((position, index) => {
      const stop = index + 1 < positions.length ? positions[index + 1].start : handler.length;
      const segment = handler.slice(position.end, stop);
      const limit = segment.match(/DefaultBodyLimit::max\(([\d\s*]+)\)/);
      entries.push({
        method: position.method,
        path,
        bodyLimit: limit ? evalLimitExpression(limit[1]) : null,
      });
    });
  }
  return entries;
}

// ── webui 调用点扫描（与 route-contract.test.mjs 同规则） ──

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
  ["entry.js|'/v1/onboarding/complete'", ['POST /v1/onboarding/complete']],
  ['card-diff.js|url', ['GET /v1/characters/:param/revisions/diff']],
  ['timeline-export.js|url', ['GET /v1/sessions/:param/:param/timeline/export']],
]);
const genericFetchTransports = new Set(['api-client.js', 'agent-test-harness.js']);

async function collectWebuiRoutes() {
  const used = [];
  const files = (await readdir(assetsUrl)).filter(file => file.endsWith('.js')).sort();
  for (const file of files) {
    const source = await readFile(new URL(file, assetsUrl), 'utf8');
    for (const kind of ['request', 'stream']) {
      for (const call of callArguments(source, `client.${kind}(`)) {
        const args = splitTopLevel(call);
        if (kind === 'request' && args[0]?.startsWith('...')) continue;
        const method = kind === 'stream' ? 'POST' : stringLiteral(args[0] || '')?.toUpperCase();
        const pathExpression = kind === 'stream' ? args[0] : args[1];
        if (!method || !pathExpression) continue;
        const path = staticPath(pathExpression);
        if (path) {
          used.push(`${method} ${path}`);
          continue;
        }
        const key = `${file}|${kind}|${pathExpression.trim()}`;
        const routes = dynamicCalls.get(key);
        if (!routes) {
          console.error(`未解析的动态调用: ${key}`);
          continue;
        }
        used.push(...routes);
      }
    }
    for (const call of callArguments(source, 'fetch(')) {
      if (genericFetchTransports.has(file)) continue;
      const expression = splitTopLevel(call)[0];
      const key = `${file}|${expression}`;
      const routes = dynamicFetchCalls.get(key);
      if (!routes) {
        console.error(`未解析的 fetch 调用: ${key}`);
        continue;
      }
      used.push(...routes);
    }
  }
  return used;
}

// ── 主流程 ──

const engineEntries = engineRouteEntries(routerSource)
  .filter(entry => entry.path.startsWith('/v1/'))
  .sort((a, b) => a.path.localeCompare(b.path) || a.method.localeCompare(b.method));

const usedRoutes = [...new Set(await collectWebuiRoutes())].filter(route => route.split(' ')[1].startsWith('/v1/'));

const golden = engineEntries.map(entry => {
  const route = `${entry.method} ${canonicalPath(entry.path)}`;
  const consumedBy = usedRoutes.filter(used => routesAreCompatible(used, route));
  return {
    method: entry.method,
    path: entry.path,
    ...(entry.bodyLimit ? { bodyLimit: entry.bodyLimit } : {}),
    ui: consumedBy.length > 0,
    reason: '',
    _consumedBy: consumedBy,
  };
});

// webui 调用了但 engine 清单中不存在的端点（正常情况下应为空）
const orphans = usedRoutes.filter(used => (
  !engineEntries.some(entry => routesAreCompatible(used, `${entry.method} ${canonicalPath(entry.path)}`))
));

const uiCount = golden.filter(entry => entry.ui).length;
console.error(`engine /v1 端点总数: ${golden.length}（method+path 组合）`);
console.error(`有 UI 消费: ${uiCount} / 仅引擎: ${golden.length - uiCount}`);
console.error(`webui 孤儿调用（清单外）: ${orphans.length ? orphans.join(', ') : '无'}`);

const draft = {
  $comment: 'DRAFT — reason 字段需人工补充后方可作为正式 fixture',
  endpoints: golden,
};

if (process.argv.includes('--write')) {
  await writeFile(
    new URL('./fixtures/v1-endpoints.draft.json', import.meta.url),
    JSON.stringify(draft, null, 2) + '\n',
    'utf8',
  );
  console.error('已写入 webui/tests/fixtures/v1-endpoints.draft.json');
} else {
  console.log(JSON.stringify(draft, null, 2));
}
