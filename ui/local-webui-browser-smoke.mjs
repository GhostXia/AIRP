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

  // The entry page now exposes an explicit first-run choice. Older bundles
  // still redirect directly to the wizard, while already-onboarded bundles
  // can land on the main UI. Wait for either final route or the new choice
  // state before deciding which path this smoke should exercise.
  await page.waitForFunction(() => {
    const pathname = window.location.pathname;
    if (pathname.endsWith('/screens/16-onboarding.html') || pathname.endsWith('/screens/01-role-list.html')) {
      return true;
    }
    const actions = document.querySelector('#entry-actions');
    return (pathname.endsWith('/') || pathname.endsWith('/index.html')) && actions && !actions.hidden;
  });

  const entryPath = new URL(page.url()).pathname;
  if (entryPath.endsWith('/') || entryPath.endsWith('/index.html')) {
    const wizardLink = page.locator('#entry-start-wizard');
    assert.equal(await wizardLink.count(), 1);
    assert.match(await wizardLink.getAttribute('href') || '', /screens\/16-onboarding\.html/);
    await wizardLink.click();
    await page.waitForURL('**/screens/16-onboarding.html');
  }

  if (new URL(page.url()).pathname.endsWith('/screens/16-onboarding.html')) {
    await page.waitForFunction(() => document.querySelector('#onboarding-card')?.textContent?.includes('检查 AIRP Engine'));
    assert.equal(await page.locator('#onboarding-steps .step').count(), 6);
    assert.equal(await page.locator('#engine-status').evaluate(node => node.classList.contains('danger')), false);
    await page.locator('#skip-onboarding').click();
    await page.waitForURL('**/screens/01-role-list.html');
  }

  assert.match(new URL(page.url()).pathname, /\/screens\/01-role-list\.html$/);
  await page.waitForFunction(() => document.querySelector('#engine-status')?.textContent?.includes('连接'));
  assert.equal(await page.locator('#character-grid').count(), 1);

  await page.goto(origin + '/screens/23-diagnostics.html', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => document.querySelector('#view pre')?.textContent?.includes('version'));
  assert.equal(await page.locator('#console-nav .nav-link').count() >= 10, true);

  for (const path of [
    '03-workbench.html', '04-world-book.html', '05-presets.html',
    '06-user-persona.html', '07-agent-runs.html', '08-settings.html',
    '17-memory-state.html', '18-group-chat.html', '19-branch-tree.html',
    '20-assembly-preview.html', '21-usage-quota.html', '22-backup-restore.html',
    '23-diagnostics.html', '24-plugins.html', '25-notes-connections.html',
  ]) {
    // The Engine deliberately rate-limits /v1. Keep this broad navigation
    // smoke representative of a human operator instead of bursting 15 pages.
    await page.waitForTimeout(1_500);
    await page.goto(origin + '/screens/' + path + '?character=webui-smoke', { waitUntil: 'domcontentloaded' });
    if (path === '18-group-chat.html') {
      // 18-group-chat.html was rewritten in PR #317 to a custom group-chat
      // layout (scene sidebar + message flow) and no longer uses the
      // console-runtime.js skeleton (#view / #heading-title / #runtime-status).
      // Verify its own boot path finalized #engine-status (boot() always
      // flips the pill to ok or danger). When the engine is in danger,
      // #scene-list may never populate (its data fetch failed), so we must
      // let the predicate return on danger alone — the next assertion will
      // then surface the connectivity failure immediately instead of timing
      // out (CodeRabbit 第五轮 inline).
      await page.waitForFunction(() => {
        const status = document.querySelector('#engine-status');
        const sceneList = document.querySelector('#scene-list');
        if (!status || !sceneList) return false;
        const finalized = status.classList.contains('ok') || status.classList.contains('danger');
        const danger = status.classList.contains('danger');
        const populated = sceneList.textContent.trim().length > 0 && sceneList.textContent.trim() !== '加载中…';
        return finalized && (danger || populated);
      });
      assert.equal(await page.locator('#engine-status').evaluate(node => node.classList.contains('danger')), false, path + ' must stay connected');
      continue;
    }
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
