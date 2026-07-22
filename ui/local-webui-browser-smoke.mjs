import assert from 'node:assert/strict';
import { chromium } from 'playwright-core';

const origin = process.env.AIRP_SMOKE_ORIGIN || 'http://127.0.0.1:8765';
const executablePath = process.env.AIRP_CHROME_PATH;
assert.ok(executablePath, 'AIRP_CHROME_PATH is required');

const browser = await chromium.launch({ headless: true, executablePath });
try {
  const page = await browser.newPage();
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript(() => {
    window.__airpCspViolations = [];
    document.addEventListener('securitypolicyviolation', event => window.__airpCspViolations.push({ directive: event.effectiveDirective, blocked: event.blockedURI }));
  });

  const response = await page.goto(origin, { waitUntil: 'domcontentloaded' });
  assert.equal(response?.status(), 200);
  const headers = response.headers();
  assert.match(headers['content-security-policy'] || '', /script-src 'self'/);
  assert.equal(headers['x-frame-options'], 'DENY');
  assert.equal(headers['x-content-type-options'], 'nosniff');
  assert.equal(headers['cache-control'], 'no-store');
  await page.waitForURL('**/screens/01-role-list.html');
  await page.waitForFunction(() => document.querySelector('#engine-status')?.textContent?.includes('连接'));
  assert.equal(await page.locator('#character-grid').count(), 1);

  await page.goto(origin + '/screens/23-diagnostics.html', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => document.querySelector('#view pre')?.textContent?.includes('version'));
  assert.equal(await page.locator('#console-nav .nav-link').count() >= 10, true);

  for (const path of [
    '03-workbench.html', '04-world-book.html', '05-presets-models.html',
    '06-user-persona.html', '07-agent-runs.html', '08-settings.html',
    '17-memory-state.html', '18-group-chat.html', '19-branch-tree.html',
    '20-assembly-preview.html', '21-usage-quota.html', '22-backup-restore.html',
    '23-diagnostics.html', '24-plugins.html', '25-notes-connections.html',
  ]) {
    // The Engine deliberately rate-limits /v1. Keep this broad navigation
    // smoke representative of a human operator instead of bursting 15 pages.
    await page.waitForTimeout(1_500);
    await page.goto(origin + '/screens/' + path + '?character=webui-smoke', { waitUntil: 'domcontentloaded' });
    await page.waitForFunction(() => document.querySelector('#view')?.children.length > 0);
    assert.ok((await page.locator('#heading-title').textContent())?.trim(), path + ' must render a title');
    assert.equal(await page.locator('#engine-status').evaluate(node => node.classList.contains('danger')), false, path + ' must stay connected');
    assert.doesNotMatch((await page.locator('#runtime-status').textContent()) || '', /失败/, path + ' must not report a load failure');
  }
  assert.deepEqual(await page.evaluate(() => window.__airpCspViolations), []);
  assert.deepEqual(pageErrors, []);
  console.log(`Local WebUI browser smoke passed at ${origin}`);
} finally {
  await browser.close();
}
