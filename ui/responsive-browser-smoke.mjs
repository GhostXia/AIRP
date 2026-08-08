// Short-viewport regression for the Vue workbench layout.
//
// This deliberately drives the same MockBus used by the dev UI rather than
// relying on a screenshot: the assertions cover the scroll/overflow contract
// that is easy to regress when the app root owns no page scroll.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { once } from 'node:events';
import { chromium } from 'playwright-core';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';

const port = Number(process.env.AIRP_RESPONSIVE_PORT || 4174);
const origin = `http://127.0.0.1:${port}`;
const executablePath = process.env.AIRP_CHROME_PATH || 'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe';
assert.ok(existsSync(executablePath), `AIRP_CHROME_PATH or Chrome executable is required: ${executablePath}`);

const uiDir = dirname(fileURLToPath(import.meta.url));
const isWindows = process.platform === 'win32';
const serverCommand = isWindows ? (process.env.ComSpec || 'cmd.exe') : 'npm';
const serverArgs = isWindows
  ? ['/d', '/s', '/c', `npm run dev -- --host 127.0.0.1 --port ${port}`]
  : ['run', 'dev', '--', '--host', '127.0.0.1', '--port', String(port)];
const server = spawn(serverCommand, serverArgs, {
  cwd: uiDir,
  env: { ...process.env, BROWSER: 'none' },
  stdio: ['ignore', 'pipe', 'pipe'],
});
let serverOutput = '';
server.stdout.on('data', chunk => { serverOutput += chunk.toString(); });
server.stderr.on('data', chunk => { serverOutput += chunk.toString(); });

function stopServer() {
  if (server.exitCode !== null) return;
  if (process.platform === 'win32') {
    try {
      execFileSync('taskkill', ['/pid', String(server.pid), '/t', '/f'], { stdio: 'ignore' });
    } catch {
      // The wrapper may have exited while Vite was shutting down.
    }
  } else {
    server.kill();
  }
}

async function waitForServer(timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const response = await fetch(origin, { signal: AbortSignal.timeout(1_000) });
      if (response.ok) return;
    } catch {
      // Vite is still starting.
    }
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  throw new Error(`Vite did not start at ${origin}\n${serverOutput}`);
}

const browser = await (async () => {
  try {
    await waitForServer();
    return await chromium.launch({ headless: true, executablePath });
  } catch (error) {
    stopServer();
    throw error;
  }
})();

try {
  const page = await browser.newPage({ viewport: { width: 360, height: 320 } });
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.goto(`${origin}/?airp_agent_test=1`, { waitUntil: 'domcontentloaded' });
  try {
    await page.waitForFunction(() => window.__AIRP_AGENT_TEST__?.version === 1, null, { timeout: 15_000 });
  } catch (error) {
    throw new Error(`${error.message}\npage errors: ${pageErrors.join(' | ')}\nbody: ${(await page.locator('body').textContent())?.slice(0, 500)}`);
  }
  await page.locator('.blueprint').waitFor({ state: 'visible' });

  // Add enough messages to force a real scroll owner at the short viewport.
  await page.evaluate(() => {
    for (let i = 0; i < 24; i += 1) {
      window.__AIRP_AGENT_TEST__.sendChat(`responsive viewport message ${i}`);
    }
  });
  await page.waitForFunction(() => document.querySelectorAll('.w-chat-log .msg').length >= 10);

  const widths = await page.evaluate(() => {
    const nodes = [document.documentElement, document.body, document.querySelector('.app'), document.querySelector('.blueprint')];
    return nodes.map(node => ({
      name: node?.className || node?.tagName,
      clientWidth: node?.clientWidth ?? 0,
      scrollWidth: node?.scrollWidth ?? 0,
    }));
  });
  for (const entry of widths) {
    assert.ok(entry.scrollWidth <= entry.clientWidth + 1, `${entry.name} has horizontal overflow: ${JSON.stringify(entry)}`);
  }

  const scrollability = await page.locator('.w-chat-log').evaluate(log => ({
    clientHeight: log.clientHeight,
    scrollHeight: log.scrollHeight,
  }));
  assert.ok(scrollability.scrollHeight > scrollability.clientHeight, `chat log must have a bounded scroll owner: ${JSON.stringify(scrollability)}`);

  // Drive the actual scroll owner to its logical end. Virtualized rows are
  // recycled, so a DOM `.last()` assertion alone could only prove that the
  // currently rendered slice has an end—not that message 23 is reachable.
  const chatLog = page.locator('.w-chat-log');
  await chatLog.evaluate(log => {
    log.scrollTop = log.scrollHeight;
    log.dispatchEvent(new Event('scroll', { bubbles: true }));
  });
  await page.waitForFunction(() => [...document.querySelectorAll('.w-chat-log .msg')]
    .some(node => node.textContent?.includes('responsive viewport message 23')), null, { timeout: 15_000 });
  const tailMessage = page.locator('.w-chat-log .msg').filter({ hasText: 'responsive viewport message 23' }).first();
  const tailRect = await chatLog.evaluate((log, targetText) => {
    const target = [...log.querySelectorAll('.msg')].find(node => node.textContent?.includes(targetText));
    const rect = target?.getBoundingClientRect();
    const bounds = log.getBoundingClientRect();
    return {
      scrollTop: log.scrollTop,
      targetTop: rect?.top ?? null,
      targetBottom: rect?.bottom ?? null,
      logTop: bounds.top,
      logBottom: bounds.bottom,
    };
  }, 'responsive viewport message 23');
  assert.equal(await tailMessage.count(), 1, 'logical tail message must be rendered after scrolling to scrollHeight');
  assert.ok(tailRect.scrollTop > 0, `chat log did not scroll: ${JSON.stringify(tailRect)}`);
  assert.ok(tailRect.targetTop >= tailRect.logTop && tailRect.targetBottom <= tailRect.logBottom,
    `logical tail message is outside chat-log bounds: ${JSON.stringify(tailRect)}`);
  console.log(`Responsive short-viewport browser regression passed at ${origin}`);
} finally {
  await browser.close();
  stopServer();
  if (server.exitCode === null) await once(server, 'exit').catch(() => {});
}
