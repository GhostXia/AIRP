import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';

const assetsUrl = new URL('../assets/', import.meta.url);
const routerSource = await readFile(
  new URL('../../engine/src/daemon/mod.rs', import.meta.url),
  'utf8',
);

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

function engineRoutes(source) {
  const routes = new Set();
  for (const call of callArguments(source, '.route(')) {
    const args = splitTopLevel(call);
    const path = stringLiteral(args[0] || '');
    if (!path || !args[1]) continue;
    for (const match of args[1].matchAll(/\b(get|post|put|delete|patch)\s*\(/g)) {
      routes.add(`${match[1].toUpperCase()} ${canonicalPath(path)}`);
    }
  }
  return routes;
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
]);
const genericFetchTransports = new Set(['api-client.js', 'agent-test-harness.js']);

test('every WebUI API client route is registered by the Engine router', async () => {
  const registered = engineRoutes(routerSource);
  const used = [];
  const consumedDynamicCalls = new Set();
  const consumedFetchCalls = new Set();
  const files = (await readdir(assetsUrl)).filter(file => file.endsWith('.js')).sort();

  for (const file of files) {
    const source = await readFile(new URL(file, assetsUrl), 'utf8');
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
        consumedDynamicCalls.add(key);
        for (const route of routes) used.push([route, `${file}: ${pathExpression}`]);
      }
    }

    for (const call of callArguments(source, 'fetch(')) {
      if (genericFetchTransports.has(file)) continue;
      const expression = splitTopLevel(call)[0];
      const key = `${file}|${expression}`;
      const routes = dynamicFetchCalls.get(key);
      assert.ok(routes, `${file}: unresolved direct fetch path: ${expression}`);
      consumedFetchCalls.add(key);
      for (const route of routes) used.push([route, `${file}: fetch(${expression})`]);
    }
  }

  assert.ok(used.length >= 100, `route extraction unexpectedly found only ${used.length} calls`);
  assert.deepEqual(
    [...consumedDynamicCalls].sort(),
    [...dynamicCalls.keys()].sort(),
    'dynamic route declarations must correspond to a live unresolved client call',
  );
  assert.deepEqual(
    [...consumedFetchCalls].sort(),
    [...dynamicFetchCalls.keys()].sort(),
    'direct fetch declarations must correspond to a live non-transport fetch call',
  );

  const missing = used.filter(([route]) => (
    ![...registered].some(engineRoute => routesAreCompatible(route, engineRoute))
  ));
  assert.deepEqual(
    missing,
    [],
    `WebUI routes missing from Engine router:\n${missing
      .map(([route, source]) => `- ${route} (${source})`)
      .join('\n')}`,
  );
});
