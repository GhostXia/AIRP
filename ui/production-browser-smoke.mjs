import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright-core';

const origin = process.env.AIRP_SMOKE_ORIGIN;
const username = process.env.AIRP_SMOKE_ADMIN_USER;
const password = process.env.AIRP_SMOKE_ADMIN_PASSWORD;
const resultFile = process.env.AIRP_SMOKE_RESULT_FILE;
const executablePath = process.env.AIRP_CHROME_PATH || '/usr/bin/google-chrome';
const chromeSpki = process.env.AIRP_CHROME_SPKI;

for (const [name, value] of Object.entries({ origin, username, password, resultFile, chromeSpki })) {
  assert.ok(value, `${name} is required`);
}
assert.match(chromeSpki, /^[A-Za-z0-9+/]{43}=$/, 'chromeSpki must be one SHA-256 hash');

const persisted = JSON.parse(readFileSync(resultFile, 'utf8'));
const browser = await chromium.launch({
  headless: true,
  executablePath,
  // Trust only the disposable Caddy leaf key; never disable TLS verification globally.
  args: [`--ignore-certificate-errors-spki-list=${chromeSpki}`],
});
try {
  const context = await browser.newContext({
    httpCredentials: { username, password },
    ignoreHTTPSErrors: false,
  });
  const page = await context.newPage();
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript(() => {
    window.__airpCspViolations = [];
    window.__airpXss = 0;
    document.addEventListener('securitypolicyviolation', event => {
      window.__airpCspViolations.push({
        directive: event.effectiveDirective,
        blocked: event.blockedURI,
      });
    });
  });

  const response = await page.goto(origin, { waitUntil: 'domcontentloaded' });
  assert.equal(response?.status(), 200);
  await page.waitForFunction(() => document.querySelector('#conn-text')?.textContent?.startsWith('已连接'), null, { timeout: 15_000 });

  const headers = response.headers();
  assert.match(headers['content-security-policy'] || '', /script-src 'self'/);
  assert.doesNotMatch(headers['content-security-policy'] || '', /unsafe-inline|unsafe-eval/);
  assert.equal(headers['x-frame-options'], 'DENY');
  assert.equal(headers['x-content-type-options'], 'nosniff');
  assert.equal(headers['cache-control'], 'no-store');
  assert.equal(await page.locator('#production-connection').isVisible(), true);
  assert.equal(await page.locator('#engine-url').isVisible(), false);
  assert.equal(await page.locator('#bearer-token').isVisible(), false);
  assert.deepEqual(await page.evaluate(() => window.__airpCspViolations), []);

  const injectionName = '<img src=x onerror="window.__airpXss=1">';
  const importStatus = await page.evaluate(async ({ injectionName }) => {
    const card = {
      spec: 'chara_card_v2',
      spec_version: '2.0',
      data: {
        name: injectionName,
        description: '<script>window.__airpXss=2</script>',
        personality: '',
        scenario: '',
        first_mes: 'hello',
        mes_example: '',
        creator_notes: 'synthetic browser injection fixture',
        system_prompt: '',
        post_history_instructions: '',
        alternate_greetings: [],
        character_book: null,
        tags: ['smoke'],
        creator: 'airp-smoke',
        character_version: '1',
        extensions: {},
      },
    };
    const imported = await fetch('/v1/characters/import', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ character_id: 'smoke-xss', card_json: JSON.stringify(card) }),
    });
    return imported.status;
  }, { injectionName });
  assert.equal(importStatus, 200);
  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => document.querySelector('#conn-text')?.textContent?.startsWith('已连接'), null, { timeout: 15_000 });
  await page.waitForFunction(name => document.body.textContent.includes(name), injectionName);
  assert.equal(await page.locator('img[src="x"]').count(), 0);
  assert.equal(await page.evaluate(() => window.__airpXss), 0);

  // Initial UI hydration and the injection fixture share the engine's burst bucket.
  // Let it refill so this assertion measures stream cancellation rather than rate limiting.
  await page.waitForTimeout(4_000);
  const cancellation = await page.evaluate(async ({ characterId, sessionId }) => {
    const controller = new AbortController();
    const startedAt = performance.now();
    const response = await fetch('/v1/chat/completions', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        character_id: characterId,
        session_id: sessionId,
        user_profile: { name: 'Tester', variables: {} },
        message: 'cancel this synthetic stream',
      }),
      signal: controller.signal,
    });
    if (!response.ok || !response.body) return { status: response.status, firstDone: true, elapsedMs: performance.now() - startedAt };
    const reader = response.body.getReader();
    const first = await reader.read();
    controller.abort();
    const drainUntilCanceled = async () => {
      while (true) {
        try {
          if ((await reader.read()).done) return true;
        } catch (error) {
          if (error?.name === 'AbortError') return true;
          throw error;
        }
      }
    };
    const canceled = await Promise.race([
      drainUntilCanceled(),
      new Promise(resolve => setTimeout(() => resolve(false), 2_000)),
    ]);
    return { status: response.status, firstDone: first.done, canceled, elapsedMs: performance.now() - startedAt };
  }, { characterId: persisted.character_id, sessionId: persisted.session_id });
  assert.equal(cancellation.status, 200);
  assert.equal(cancellation.firstDone, false);
  assert.equal(cancellation.canceled, true);
  assert.ok(cancellation.elapsedMs < 2_000, `stream cancellation took ${cancellation.elapsedMs}ms`);

  const cspViolations = await page.evaluate(() => window.__airpCspViolations);
  assert.deepEqual(cspViolations, []);
  assert.deepEqual(pageErrors, []);
  await context.close();
  console.log('production browser smoke passed');
} finally {
  await browser.close();
}
