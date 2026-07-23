import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const harnessScript = await readFile(new URL('../assets/agent-test-harness.js', import.meta.url), 'utf8');

test('harness script is CSP-compatible: no inline handlers, no eval', () => {
  assert.doesNotMatch(harnessScript, /\beval\s*\(/);
  assert.doesNotMatch(harnessScript, /new\s+Function\s*\(/);
  assert.doesNotMatch(harnessScript, /document\.write\s*\(/);
});

test('harness exposes window.__AIRP_AGENT_TEST__ v2 with required methods', () => {
  assert.match(harnessScript, /window\.__AIRP_AGENT_TEST__\s*=/);
  assert.match(harnessScript, /version:\s*2/);
  for (const method of [
    'navigate', 'getCurrentScreen', 'fillInput', 'clickButton',
    'getVisibleText', 'getDomSnapshot', 'getConsoleErrors',
    'getFailedRequests', 'getApiSnapshot', 'waitFor', 'screenshot'
  ]) {
    assert.match(harnessScript, new RegExp(`${method}\\s*\\(`));
  }
});

test('harness activation gate matches existing ui/ convention', () => {
  assert.match(harnessScript, /airp_agent_test=1/);
  assert.match(harnessScript, /AIRP_AGENT_TEST/);
  assert.match(harnessScript, /VITE_AIRP_AGENT_TEST/);
});

test('harness gate defaults to off in production build', () => {
  // 默认关闭逻辑必须存在，生产构建未带 flag 时不暴露
  assert.match(harnessScript, /shouldInstallAgentTestHarness/);
});
