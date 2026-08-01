import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright-core';

const origin = process.env.AIRP_SMOKE_ORIGIN;
const username = process.env.AIRP_SMOKE_ADMIN_USER;
const password = process.env.AIRP_SMOKE_ADMIN_PASSWORD;
const resultFile = process.env.AIRP_SMOKE_RESULT_FILE;
const executablePath = process.env.AIRP_CHROME_PATH || '/usr/bin/google-chrome';
const chromeSpki = process.env.AIRP_CHROME_SPKI;
for (const [name, value] of Object.entries({ origin, username, password, resultFile, chromeSpki })) assert.ok(value, `${name} is required`);
assert.match(chromeSpki, /^[A-Za-z0-9+/]{43}=$/);

const apiResult = JSON.parse(readFileSync(resultFile, 'utf8'));
const characterId = apiResult.character_id;
const sessionId = apiResult.session_id;
assert.ok(characterId && sessionId, 'API smoke must provide a durable character/session');

const pages = [
  { file: '34-relationship-graph.html', title: '角色关系图谱', ready: '#graph-character', refresh: '#graph-refresh' },
  { file: '35-plot-arc.html', title: '剧情弧编辑器', ready: '#arc-character' },
  { file: '36-image-gen.html', title: '场景插图生成', ready: '#image-character', refresh: '#image-refresh-btn' },
  { file: '37-character-templates.html', title: '角色卡模板库', ready: '#template-grid' },
  { file: '38-style-learn.html', title: '风格迁移', ready: '#learn-character', refresh: '#profiles-refresh' },
  { file: '39-dialogue-gen.html', title: '对话示例生成器', ready: '#gen-character' },
  { file: '40-worldbook-graph.html', title: '世界书知识图谱', ready: '#graph-character', refresh: '#graph-refresh' },
  { file: '41-timeline-export.html', title: '剧情时间线导出', ready: '#tl-character' },
  { file: '42-card-diff.html', title: '角色卡版本对比', ready: '#cd-character' },
  { file: '43-provider-management.html', title: '多 Provider 路由', ready: '#pm-rows', refresh: '#pm-reload' },
  { file: '44-plugin-tools.html', title: '插件工具', ready: '#pt-rows', refresh: '#pt-reload' },
];

function pageUrl(file) {
  const url = new URL('/screens/' + file, origin);
  url.searchParams.set('character', characterId);
  url.searchParams.set('session', sessionId);
  return url.href;
}

async function assertSecurityHeaders(response) {
  assert.equal(response?.status(), 200);
  const headers = response.headers();
  assert.match(headers['content-security-policy'] || '', /script-src 'self'/);
  assert.doesNotMatch(headers['content-security-policy'] || '', /unsafe-inline|unsafe-eval/);
  assert.equal(headers['x-frame-options'], 'DENY');
  assert.equal(headers['x-content-type-options'], 'nosniff');
  assert.equal(headers['cache-control'], 'no-store');
}

const browser = await chromium.launch({
  headless: true,
  executablePath,
  args: [`--ignore-certificate-errors-spki-list=${chromeSpki}`],
});
try {
  const context = await browser.newContext({ httpCredentials: { username, password }, ignoreHTTPSErrors: false });
  const page = await context.newPage();
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript(() => {
    window.__airpCspViolations = [];
    document.addEventListener('securitypolicyviolation', event => window.__airpCspViolations.push({ directive: event.effectiveDirective, blocked: event.blockedURI }));
  });

  for (const advancedPage of pages) {
    const response = await page.goto(pageUrl(advancedPage.file), { waitUntil: 'domcontentloaded' });
    await assertSecurityHeaders(response);
    await page.locator('#page-title').filter({ hasText: advancedPage.title }).waitFor({ state: 'visible', timeout: 15_000 });
    await page.locator(advancedPage.ready).waitFor({ state: 'attached', timeout: 15_000 });
    await page.waitForFunction(() => document.querySelector('#engine-status')?.classList.contains('ok'), null, { timeout: 15_000 });
    assert.equal(await page.locator('#runtime-status').count(), 1, `${advancedPage.file} must retain its runtime status region`);

    if (advancedPage.refresh) {
      await page.locator(advancedPage.refresh).click();
      await page.waitForFunction(() => document.querySelector('#engine-status')?.classList.contains('ok'), null, { timeout: 5_000 });
    }
  }

  assert.deepEqual(await page.evaluate(() => window.__airpCspViolations), []);
  assert.deepEqual(pageErrors, []);
  await context.close();

  // A browser-visible failure state is part of the runtime contract: a broken
  // Engine connection must not leave any advanced page as an apparent
  // successful empty result.
  const errorContext = await browser.newContext({ httpCredentials: { username, password }, ignoreHTTPSErrors: false });
  const errorPage = await errorContext.newPage();
  const errorPageErrors = [];
  errorPage.on('pageerror', error => errorPageErrors.push(error.message));
  await errorPage.route('**/health', route => route.abort('failed'));
  for (const advancedPage of pages) {
    await errorPage.goto(pageUrl(advancedPage.file), { waitUntil: 'domcontentloaded' });
    await errorPage.waitForFunction(() => {
      const classes = document.querySelector('#engine-status')?.classList;
      return classes?.contains('danger') || classes?.contains('error');
    }, null, { timeout: 15_000 });
  }
  assert.deepEqual(errorPageErrors, []);
  await errorContext.close();
  console.log('production advanced WebUI pages smoke passed');
} finally {
  await browser.close();
}
