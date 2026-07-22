import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright-core';

const origin = process.env.AIRP_SMOKE_ORIGIN;
const username = process.env.AIRP_SMOKE_ADMIN_USER;
const password = process.env.AIRP_SMOKE_ADMIN_PASSWORD;
const browserResultFile = process.env.AIRP_SMOKE_BROWSER_RESULT_FILE;
const executablePath = process.env.AIRP_CHROME_PATH || '/usr/bin/google-chrome';
const chromeSpki = process.env.AIRP_CHROME_SPKI;
for (const [name, value] of Object.entries({ origin, username, password, browserResultFile, chromeSpki })) assert.ok(value, `${name} is required`);
assert.match(chromeSpki, /^[A-Za-z0-9+/]{43}=$/);
const expected = JSON.parse(readFileSync(browserResultFile, 'utf8'));

async function waitForHistory(context, payload, predicate, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  let latest;
  while (Date.now() < deadline) {
    const response = await context.request.post(origin + '/v1/chat/history', { data: payload });
    assert.equal(response.status(), 200);
    latest = await response.json();
    if (predicate(latest)) return latest;
    await new Promise(resolve => setTimeout(resolve, 200));
  }
  throw new Error(`history did not reach the expected state within ${timeoutMs}ms; latest total=${latest?.total ?? 'unknown'}`);
}

const browser = await chromium.launch({ headless: true, executablePath, args: [`--ignore-certificate-errors-spki-list=${chromeSpki}`] });
try {
  const context = await browser.newContext({ httpCredentials: { username, password }, ignoreHTTPSErrors: false });
  const page = await context.newPage();
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript(() => {
    window.__airpCspViolations = [];
    document.addEventListener('securitypolicyviolation', event => window.__airpCspViolations.push({ directive: event.effectiveDirective, blocked: event.blockedURI }));
  });

  const chatUrl = origin + '/screens/02-chat-space.html?character=' + encodeURIComponent(expected.characterId) + '&session=' + encodeURIComponent(expected.sessionId);
  const response = await page.goto(chatUrl, { waitUntil: 'domcontentloaded' });
  assert.equal(response?.status(), 200);
  await page.waitForFunction(() => document.querySelector('#message-input')?.disabled === false, null, { timeout: 15_000 });
  await page.waitForFunction(message => document.querySelector('#message-flow')?.textContent?.includes(message), expected.message, { timeout: 10_000 });

  const before = await waitForHistory(
    context,
    { character_id: expected.characterId, session_id: expected.sessionId, limit: 200 },
    history => history.total >= expected.total && history.messages.some(item => item.role === 'user' && item.content === expected.message),
  );
  assert.ok(before.total >= expected.total, 'history must not shrink across the production restart');

  const secondMessage = 'restart browser continuity ' + Date.now();
  await page.locator('#message-input').fill(secondMessage);
  await page.locator('#send-message').click();
  await page.waitForFunction(() => document.querySelector('#send-message')?.classList.contains('stop'), null, { timeout: 5_000 });
  await page.waitForFunction(() => !document.querySelector('#send-message')?.classList.contains('stop'), null, { timeout: 20_000 });
  await waitForHistory(
    context,
    { character_id: expected.characterId, session_id: expected.sessionId, limit: 200 },
    history => history.total >= before.total + 2 && history.messages.some(item => item.role === 'user' && item.content === secondMessage),
  );
  assert.deepEqual(await page.evaluate(() => window.__airpCspViolations), []);
  assert.deepEqual(pageErrors, []);
  await context.close();
  console.log('production browser restart continuity smoke passed');
} finally {
  await browser.close();
}
