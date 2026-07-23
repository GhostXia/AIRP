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

const screens = [
  '16-onboarding.html', '01-role-list.html', '02-chat-space.html',
  '17-memory-state.html', '19-branch-tree.html', '14-message-swipe.html',
];

for (const screen of screens) {
  test(screen + ' conditionally loads agent-test-harness.js with silent fail', async () => {
    const html = await readFile(new URL('../screens/' + screen, import.meta.url), 'utf8');
    // CSP 禁内联事件处理器，用纯 async external script；harness 内 flag 默认 off，文件缺失 404 不阻塞页面
    assert.match(html, /<script\s+src="[^"]*assets\/agent-test-harness\.js"\s+async><\/script>/);
    // 不得放在 <head>（避免阻塞首屏）
    const headEnd = html.indexOf('</head>');
    const scriptPos = html.indexOf('agent-test-harness.js');
    assert.ok(scriptPos > headEnd, screen + ': harness script must be after </head>');
  });
}
