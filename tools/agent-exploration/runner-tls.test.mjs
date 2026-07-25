// runner-tls.test.mjs — 防回归契约测试
//
// 背景：agent-browser-exploration.yml 的 runner 在 GitHub Actions 上连续 3 次 run
// 因 net::ERR_CERT_AUTHORITY_INVALID 失败。根因是 runner.mjs 没把 Caddy 自签证书的
// SPKI 传给 Chrome，导致 page.goto 拒绝 https://localhost:9443。
//
// 本测试静态扫描 + 单元测试 runner.mjs 的 TLS 处理逻辑：
//   1. 静态扫描：runner.mjs 必须读 AIRP_CHROME_SPKI、必须传给 chromium.launch args、
//      newContext 必须显式 ignoreHTTPSErrors: false（防未来默认值变化）
//   2. 单元测试：buildLaunchArgs(spki) 对合法 spki / 非法 spki / 空 spki 的行为
//   3. workflow 扫描：agent-browser-exploration.yml 必须把 AIRP_CHROME_SPKI 传给 runner step
//   4. bootstrap 扫描：bootstrap-topology.sh 必须把 chrome_spki 写到 $GITHUB_ENV

import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { buildLaunchArgs } from './tls-args.mjs';

const runnerSrc = await readFile(new URL('./runner.mjs', import.meta.url), 'utf8');
const tlsArgsSrc = await readFile(new URL('./tls-args.mjs', import.meta.url), 'utf8');
const workflowSrc = await readFile(new URL('../../.github/workflows/agent-browser-exploration.yml', import.meta.url), 'utf8');
const bootstrapSrc = await readFile(new URL('../../deploy/production/bootstrap-topology.sh', import.meta.url), 'utf8');

// 合法 SPKI：base64 编码的 SHA-256 公钥哈希，固定 32 字节 → base64 编码 43 字符 + 1 个 '=' 填充
// 构造 43 字符 base64 串 + '=' 填充
const VALID_SPKI = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq=';

test('runner.mjs 静态契约：读取 AIRP_CHROME_SPKI env', () => {
  assert.match(runnerSrc, /AIRP_CHROME_SPKI/, 'runner 必须读取 AIRP_CHROME_SPKI 环境变量');
  assert.match(runnerSrc, /args\['chrome-spki'\]\s*\|\|\s*process\.env\.AIRP_CHROME_SPKI/, '必须同时支持 --chrome-spki CLI 参数和 env');
});

test('runner.mjs 静态契约：chromium.launch 传 buildLaunchArgs 结果到 args', () => {
  assert.match(
    runnerSrc,
    /chromium\.launch\(\s*{\s*headless:\s*true,\s*executablePath:\s*CHROME,\s*args:\s*buildLaunchArgs\(CHROME_SPKI\)\s*}\s*\)/,
    'chromium.launch 必须显式传 args: buildLaunchArgs(CHROME_SPKI)',
  );
});

test('runner.mjs 静态契约：newContext 显式 ignoreHTTPSErrors: false', () => {
  assert.match(runnerSrc, /ignoreHTTPSErrors:\s*false/, 'newContext 必须显式 ignoreHTTPSErrors: false（防 Playwright 默认值变化导致安全降级）');
});

test('runner.mjs 静态契约：从 ./tls-args.mjs import buildLaunchArgs', () => {
  assert.match(runnerSrc, /import\s*{\s*buildLaunchArgs\s*}\s*from\s*['"]\.\/tls-args\.mjs['"]/, 'runner 必须从 ./tls-args.mjs import buildLaunchArgs');
});

test('tls-args.mjs 静态契约：export function buildLaunchArgs', () => {
  assert.match(tlsArgsSrc, /export\s+function\s+buildLaunchArgs\s*\(/, 'tls-args.mjs 必须有 export function buildLaunchArgs');
});

test('buildLaunchArgs 单元：合法 SPKI 生成 --ignore-certificate-errors-spki-list 参数', () => {
  const args = buildLaunchArgs(VALID_SPKI);
  assert.deepEqual(args, ['--ignore-certificate-errors-spki-list=' + VALID_SPKI]);
});

test('buildLaunchArgs 单元：空 SPKI 返回空数组（保持兼容旧 http 场景）', () => {
  assert.deepEqual(buildLaunchArgs(''), [], '空 spki 不应生成任何 args（兼容 http 同源场景）');
  assert.deepEqual(buildLaunchArgs(undefined), [], 'undefined spki 不应生成任何 args');
  assert.deepEqual(buildLaunchArgs(null), [], 'null spki 不应生成任何 args');
});

test('buildLaunchArgs 单元：格式非法的 SPKI 返回空数组（不传给 Chrome）', () => {
  // 缺少末尾 '=' 填充（长度 43 但无填充）
  assert.deepEqual(buildLaunchArgs('ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq'), [], '缺少填充的 spki 应被拒绝');
  // 长度不对（过短）
  assert.deepEqual(buildLaunchArgs('short='), [], '过短的 spki 应被拒绝');
  // 长度对但含非法字符 '!'（正则要求 [A-Za-z0-9+/]）
  assert.deepEqual(buildLaunchArgs('ABCDEFGHIJKLMNOPQRSTUVWXYZ!bcdefghijklmnopq='), [], '含非法字符的 spki 应被拒绝');
});

test('workflow 静态契约：agent-browser-exploration.yml 把 AIRP_CHROME_SPKI 传给 runner step', () => {
  assert.match(
    workflowSrc,
    /AIRP_CHROME_SPKI:\s*\$\{\{\s*env\.AIRP_CHROME_SPKI\s*\}\}/,
    'workflow 必须把 env.AIRP_CHROME_SPKI 传给 Run agent exploration step',
  );
});

test('workflow 静态契约：bootstrap-topology.sh 把 chrome_spki 写到 $GITHUB_ENV', () => {
  assert.match(
    bootstrapSrc,
    /AIRP_CHROME_SPKI=%s.*\$chrome_spki.*>>\s*"\$GITHUB_ENV"/s,
    'bootstrap-topology.sh 必须把 chrome_spki 写到 $GITHUB_ENV',
  );
});
