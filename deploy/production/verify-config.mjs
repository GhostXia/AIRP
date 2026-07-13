import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = dirname(fileURLToPath(import.meta.url));
const read = name => readFileSync(join(root, name), 'utf8');
const compose = read('compose.yaml');
const caddy = read('Caddyfile.common');
const engineImage = read('Dockerfile.engine');
const gatewayImage = read('Dockerfile.gateway');

assert.doesNotMatch(compose, /^\s+ports:\s*$[\s\S]*?engine:/m, 'engine must not publish a host port');
const engineBlock = compose.slice(compose.indexOf('  engine:'), compose.indexOf('  gateway:'));
assert.doesNotMatch(engineBlock, /^\s+ports:/m, 'engine service publishes a host port');
assert.match(engineBlock, /cap_drop:\s*\n\s*- ALL/);
assert.match(engineBlock, /no-new-privileges:true/);
assert.match(compose, /backend:\s*\n\s+internal: true/);
assert.match(compose, /AIRP_HTTPS_PORT:-443}:\$\{AIRP_HTTPS_PORT:-443}/);
assert.match(compose, /engine_access_key:\s*\n\s+file: \.\/secrets\/engine_access_key/);
assert.doesNotMatch(compose, /AIRP_ACCESS_KEY:\s*\$/);
assert.doesNotMatch(compose, /latest/);
assert.doesNotMatch(gatewayImage, /COPY webui \/srv/);
assert.doesNotMatch(gatewayImage, /mock-provider|smoke\.mjs|serve\.js|start\.bat/);

assert.match(caddy, /basic_auth/);
assert.match(caddy, /header_up Authorization "Bearer \{\$AIRP_ACCESS_KEY\}"/);
assert.match(caddy, /max_size 10MB/);
assert.match(caddy, /Content-Security-Policy/);
assert.match(caddy, /frame-ancestors 'none'/);
assert.match(caddy, /request>headers>Authorization delete/);
assert.match(caddy, /response>headers>Set-Cookie delete/);
assert.match(caddy, /handle \{\s*\n\s*header Cache-Control "no-store"/);
assert.doesNotMatch(caddy, /unsafe-inline|unsafe-eval/);

for (const dockerfile of [engineImage, gatewayImage]) {
  assert.match(dockerfile, /FROM [^\n]+@sha256:[a-f0-9]{64}/, 'base images must be digest pinned');
  assert.doesNotMatch(dockerfile, /:latest/);
}

console.log('production deployment static checks passed');
