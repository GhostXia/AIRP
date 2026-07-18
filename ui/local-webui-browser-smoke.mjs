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
    document.addEventListener('securitypolicyviolation', event => {
      window.__airpCspViolations.push({
        directive: event.effectiveDirective,
        blocked: event.blockedURI,
      });
    });
  });

  const response = await page.goto(origin, { waitUntil: 'domcontentloaded' });
  assert.equal(response?.status(), 200);
  const headers = response.headers();
  assert.match(headers['content-security-policy'] || '', /script-src 'self'/);
  assert.equal(headers['x-frame-options'], 'DENY');
  assert.equal(headers['x-content-type-options'], 'nosniff');
  assert.equal(headers['cache-control'], 'no-store');
  assert.equal(await page.evaluate(() => window.AIRP_WEBUI_CONFIG?.mode), 'local');
  assert.equal(await page.locator('#production-connection').isVisible(), true);
  assert.equal(await page.locator('#production-connection').textContent(), '本机安全连接');
  assert.equal(await page.locator('#engine-url').isVisible(), false);
  assert.equal(await page.locator('#bearer-token').isVisible(), false);
  await page.waitForFunction(() => document.querySelector('#onboarding-root h2'));
  assert.deepEqual(await page.evaluate(() => window.__airpCspViolations), []);
  assert.deepEqual(pageErrors, []);
  console.log(`Local WebUI browser smoke passed at ${origin}`);
} finally {
  await browser.close();
}
