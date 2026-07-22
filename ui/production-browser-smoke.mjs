import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { chromium } from 'playwright-core';

const origin = process.env.AIRP_SMOKE_ORIGIN;
const username = process.env.AIRP_SMOKE_ADMIN_USER;
const password = process.env.AIRP_SMOKE_ADMIN_PASSWORD;
const resultFile = process.env.AIRP_SMOKE_RESULT_FILE;
const browserStateFile = process.env.AIRP_SMOKE_BROWSER_STATE_FILE;
const browserResultFile = process.env.AIRP_SMOKE_BROWSER_RESULT_FILE;
const executablePath = process.env.AIRP_CHROME_PATH || '/usr/bin/google-chrome';
const chromeSpki = process.env.AIRP_CHROME_SPKI;
for (const [name, value] of Object.entries({ origin, username, password, resultFile, browserStateFile, browserResultFile, chromeSpki })) assert.ok(value, `${name} is required`);
assert.match(chromeSpki, /^[A-Za-z0-9+/]{43}=$/);

const apiResult = JSON.parse(readFileSync(resultFile, 'utf8'));
const characterId = apiResult.character_id;
const sessionId = apiResult.session_id;
assert.ok(characterId && sessionId, 'API smoke must provide a durable character/session');

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
    window.__airpXss = 0;
    document.addEventListener('securitypolicyviolation', event => window.__airpCspViolations.push({ directive: event.effectiveDirective, blocked: event.blockedURI }));
  });

  await page.goto(origin + '/screens/16-onboarding.html', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => document.querySelector('#onboarding-card')?.textContent?.includes('检查 AIRP Engine'));
  assert.equal(await page.locator('#onboarding-steps .step').count(), 6);
  await page.getByRole('button', { name: '下一步 →' }).click();
  await page.getByRole('button', { name: '保存并下一步 →' }).click();
  await page.getByRole('button', { name: '下一步 →' }).waitFor({ state: 'visible' });
  await page.waitForFunction(() => document.querySelector('#onboarding-card .btn-primary')?.disabled === false);
  await page.getByRole('button', { name: '下一步 →' }).click();
  const characterChoice = page.getByRole('button', { name: characterId, exact: true });
  if (await characterChoice.count()) await characterChoice.click();
  await page.getByRole('button', { name: '下一步 →' }).click();
  await page.getByRole('button', { name: '下一步 →' }).click();
  await page.getByLabel('给角色的第一句话').fill('onboarding production smoke ' + Date.now());
  await page.getByRole('button', { name: '发送首轮消息' }).click();
  await page.getByRole('button', { name: '进入对话空间 →' }).waitFor({ state: 'visible', timeout: 20_000 });

  const chatUrl = origin + '/screens/02-chat-space.html?character=' + encodeURIComponent(characterId) + '&session=' + encodeURIComponent(sessionId);
  const response = await page.goto(chatUrl, { waitUntil: 'domcontentloaded' });
  assert.equal(response?.status(), 200);
  const headers = response.headers();
  assert.match(headers['content-security-policy'] || '', /script-src 'self'/);
  assert.doesNotMatch(headers['content-security-policy'] || '', /unsafe-inline|unsafe-eval/);
  assert.equal(headers['x-frame-options'], 'DENY');
  assert.equal(headers['x-content-type-options'], 'nosniff');
  assert.equal(headers['cache-control'], 'no-store');

  await page.waitForFunction(() => document.querySelector('#message-input')?.disabled === false);
  const before = await context.request.post(origin + '/v1/chat/history', { data: { character_id: characterId, session_id: sessionId, limit: 200 } });
  assert.equal(before.status(), 200);
  const beforeHistory = await before.json();

  const message = 'production browser continuity ' + Date.now();
  await page.locator('#message-input').fill(message);
  await page.locator('#send-message').click();
  await page.waitForFunction(() => document.querySelector('#send-message')?.classList.contains('stop'), null, { timeout: 5_000 });
  await page.waitForFunction(() => !document.querySelector('#send-message')?.classList.contains('stop'), null, { timeout: 20_000 });
  const afterHistory = await waitForHistory(
    context,
    { character_id: characterId, session_id: sessionId, limit: 200 },
    history => history.total >= beforeHistory.total + 2 && history.messages.some(item => item.role === 'user' && item.content === message),
  );

  await page.goto(origin + '/screens/23-diagnostics.html?character=' + encodeURIComponent(characterId), { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => document.querySelector('#view pre')?.textContent?.includes('version'));
  assert.equal(await page.locator('#engine-status').evaluate(node => node.classList.contains('danger')), false);

  const injectionName = '<img src=x onerror="window.__airpXss=1">';
  const card = { spec: 'chara_card_v2', spec_version: '2.0', data: { name: injectionName, description: '<script>window.__airpXss=2</script>', personality: '', scenario: '', first_mes: 'hello', mes_example: '', creator_notes: 'browser xss fixture', system_prompt: '', post_history_instructions: '', alternate_greetings: [], character_book: null, tags: ['smoke'], creator: 'airp-smoke', character_version: '1', extensions: {} } };
  const imported = await context.request.post(origin + '/v1/characters/import', { data: { character_id: 'browser-xss', card_json: JSON.stringify(card) } });
  assert.equal(imported.status(), 200);
  await page.goto(origin + '/screens/01-role-list.html', { waitUntil: 'domcontentloaded' });
  await page.waitForFunction(name => document.body.textContent.includes(name), injectionName);
  assert.equal(await page.locator('img[src="x"]').count(), 0);
  assert.equal(await page.evaluate(() => window.__airpXss), 0);

  await context.storageState({ path: browserStateFile });
  writeFileSync(browserResultFile, JSON.stringify({ characterId, sessionId, total: afterHistory.total, message }, null, 2));
  assert.deepEqual(await page.evaluate(() => window.__airpCspViolations), []);
  assert.deepEqual(pageErrors, []);
  await context.close();
  console.log('production WebUI browser smoke passed');
} finally {
  await browser.close();
}
