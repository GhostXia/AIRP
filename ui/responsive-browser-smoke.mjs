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
  // Vite redirects its configured `/desktop/` base there even when the smoke
  // starts at `/`; keep the preview fixture explicit so this layout-only test
  // never attempts to connect to an Engine.
  await page.goto(`${origin}/?airp_agent_test=1&airp_fixture=1`, { waitUntil: 'domcontentloaded' });
  try {
    await page.waitForFunction(() => window.__AIRP_AGENT_TEST__?.version === 1, null, { timeout: 15_000 });
  } catch (error) {
    throw new Error(`${error.message}\npage errors: ${pageErrors.join(' | ')}\nbody: ${(await page.locator('body').textContent())?.slice(0, 500)}`);
  }
  try {
    await page.locator('.blueprint').waitFor({ state: 'visible' });
  } catch (error) {
    const layout = await page.evaluate(() => [...document.querySelectorAll('.app, .surface, .surface__body, .blueprint')]
      .map(node => ({
        selector: node.className,
        clientWidth: node.clientWidth,
        clientHeight: node.clientHeight,
        scrollWidth: node.scrollWidth,
        scrollHeight: node.scrollHeight,
      })));
    throw new Error(`${error.message}\nlayout: ${JSON.stringify(layout)}\nbody: ${(await page.locator('body').textContent())?.slice(0, 500)}`);
  }

  // Add enough messages to force a real scroll owner at the short viewport.
  await page.evaluate(() => {
    const harness = window.__AIRP_AGENT_TEST__;
    const order = Array.from({ length: 24 }, (_, index) => `responsive-${index}`);
    const messages = Object.fromEntries(order.map((id, index) => [id, {
      id,
      role: "narrator",
      text: `responsive viewport message ${index}`,
    }]));
    harness.setWidgetState("w-chat", {
      order,
      messages,
      context: {
        character_id: "responsive-character-with-a-very-long-stable-identifier",
        session_id: "00000000-0000-4000-8000-000000000003",
        persona_id: "responsive-persona-with-a-very-long-stable-identifier",
        scene_id: "responsive-scene-with-a-very-long-stable-identifier",
        worldbook_source_ids: ["character:responsive-character-with-a-very-long-stable-identifier"],
      },
    });
    harness.setWidgetOperation("w-chat", { status: "streaming" });
  });
  await page.waitForFunction(() => document.querySelectorAll('.w-chat-log .msg').length > 0
    && window.__AIRP_AGENT_TEST__.getState("w-chat")?.order?.length === 24);

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

  const contextOverflow = await page.locator('.context-strip').evaluate(strip => ({
    clientWidth: strip.clientWidth,
    scrollWidth: strip.scrollWidth,
    labels: [...strip.querySelectorAll('.context-chip')].map(chip => chip.getAttribute('aria-label')),
  }));
  assert.ok(contextOverflow.scrollWidth > contextOverflow.clientWidth,
    `context strip must own long-ID overflow: ${JSON.stringify(contextOverflow)}`);
  assert.equal(contextOverflow.labels.length, 5, 'all fixture context chips must remain available at short width');
  const stop = page.getByRole('button', { name: '停止', exact: true });
  await stop.waitFor({ state: 'visible' });
  assert.equal(await stop.isEnabled(), true, 'stream cancellation must remain enabled at short height');
  await stop.click();

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

  const projectionSource = {
    kind: "character_state",
    scope: "character",
    character_id: `responsive-${"character-".repeat(10)}`,
  };
  await page.evaluate(({ emotion, inventory, source }) => {
    const result = window.__AIRP_AGENT_TEST__.applySurface({
      kind: "snapshot",
      protocol: { major: 2, minor: 0 },
      surfaceId: "responsive-projections",
      revision: "0",
      blueprint: {
        version: 2,
        root: {
          type: "split",
          id: "projection-split",
          orientation: "horizontal",
          children: [
            { type: "widget", id: "emotion-node", instanceId: emotion },
            { type: "widget", id: "inventory-node", instanceId: inventory },
          ],
        },
        widgets: [
          { id: emotion, type: "core.emotion" },
          { id: inventory, type: "core.inventory" },
        ],
      },
    });
    if (result.status !== "applied") throw new Error(`projection Surface rejected: ${JSON.stringify(result)}`);
    window.__AIRP_AGENT_TEST__.setWidgetState(emotion, {
      available: true,
      emotion: 64,
      label: "平静",
      revision: 12,
      timestamp: "2026-08-30T00:00:00Z",
      source,
    });
    window.__AIRP_AGENT_TEST__.setWidgetState(inventory, {
      available: true,
      items: [],
      revision: 12,
      timestamp: "2026-08-30T00:00:00Z",
      source,
    });
  }, { emotion: "responsive-emotion", inventory: "responsive-inventory", source: projectionSource });
  await page.locator('[data-widget-instance="responsive-emotion"] .w-emotion').waitFor({ state: 'visible' });
  await page.locator('[data-widget-instance="responsive-inventory"] .w-inventory').waitFor({ state: 'visible' });

  const projectionSplit = page.locator('[data-node-id="projection-split"]');
  const meter = page.getByRole('meter', { name: '情绪值' });
  assert.equal(await meter.getAttribute('aria-valuemin'), '0');
  assert.equal(await meter.getAttribute('aria-valuemax'), '100');
  assert.equal(await meter.getAttribute('aria-valuenow'), '64');
  assert.equal(await page.locator('.readonly-badge').filter({ hasText: '只读' }).count(), 2);
  assert.equal(await page.locator('[data-widget-instance="responsive-emotion"] button').count(), 0);
  assert.equal(await page.locator('[data-widget-instance="responsive-inventory"] button').count(), 0);
  await page.getByText('暂无物品', { exact: true }).waitFor({ state: 'visible' });

  for (const ratio of [1_000, 9_000]) {
    await projectionSplit.evaluate((split, leading) => {
      split.style.setProperty('--split-leading', `${leading}fr`);
      split.style.setProperty('--split-trailing', `${10_000 - leading}fr`);
      split.style.setProperty('--split-position', `${leading / 100}%`);
    }, ratio);
    for (const width of [320, 760, 761]) {
      await page.setViewportSize({ width, height: 800 });
      const layout = await page.evaluate(() => {
        const split = document.querySelector('[data-node-id="projection-split"]');
        const children = split ? [...split.children].filter(node => node.classList.contains('layout-widget')) : [];
        const widgetInternals = [
          ...document.querySelectorAll('.w-emotion, .w-emotion header, .w-inventory, .w-inventory header, .w-inventory .grid'),
        ];
        const tracked = [
          document.documentElement,
          document.body,
          document.querySelector('.app'),
          document.querySelector('.blueprint'),
          split,
          ...children,
          ...widgetInternals,
        ].filter(Boolean);
        return {
          columns: split ? getComputedStyle(split).gridTemplateColumns.split(/\s+/).filter(Boolean) : [],
          childRects: children.map(node => {
            const rect = node.getBoundingClientRect();
            return { top: rect.top, left: rect.left, width: rect.width };
          }),
          overflow: tracked.map(node => ({
            name: node.className || node.tagName,
            clientWidth: node.clientWidth,
            scrollWidth: node.scrollWidth,
          })),
        };
      });
      for (const entry of layout.overflow) {
        assert.ok(entry.scrollWidth <= entry.clientWidth + 1,
          `${width}px at ${ratio / 100}% projection layout has horizontal overflow: ${JSON.stringify(entry)}`);
      }
      assert.equal(layout.childRects.length, 2, `${width}px projection widgets were not both mounted`);
      if (width <= 760) {
        assert.equal(layout.columns.length, 1, `${width}px horizontal split did not stack: ${layout.columns}`);
        assert.ok(layout.childRects[1].top > layout.childRects[0].top,
          `${width}px projection widgets did not stack vertically: ${JSON.stringify(layout.childRects)}`);
      } else {
        assert.equal(layout.columns.length, 2, `761px horizontal split did not restore columns: ${layout.columns}`);
        const share = layout.childRects[0].width / (layout.childRects[0].width + layout.childRects[1].width);
        const expected = ratio / 10_000;
        assert.ok(Math.abs(share - expected) <= 0.01,
          `761px ${ratio / 100}/${100 - ratio / 100} split ratio was not preserved: ${JSON.stringify(layout.childRects)}`);
      }
    }
  }

  await page.evaluate(({ emotion, inventory, source }) => {
    window.__AIRP_AGENT_TEST__.setWidgetState(emotion, {
      available: false,
      reason: "missing",
      revision: 13,
      timestamp: null,
      source,
    });
    window.__AIRP_AGENT_TEST__.setWidgetState(inventory, {
      available: false,
      reason: "invalid",
      revision: 13,
      timestamp: null,
      source,
    });
  }, { emotion: "responsive-emotion", inventory: "responsive-inventory", source: projectionSource });
  await page.locator('[data-widget-instance="responsive-emotion"]').getByText('未配置', { exact: true })
    .waitFor({ state: 'visible' });
  await page.locator('[data-widget-instance="responsive-inventory"]').getByText('不可用', { exact: true })
    .waitFor({ state: 'visible' });
  console.log(`Responsive short-viewport browser regression passed at ${origin}`);
} finally {
  await browser.close();
  stopServer();
  if (server.exitCode === null) await once(server, 'exit').catch(() => {});
}
