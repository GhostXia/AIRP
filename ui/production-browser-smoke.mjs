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
  await page.waitForFunction(() => document.querySelector('#persona-select option[value=""]'));
  assert.equal(await page.locator('#persona-effective-hint').getAttribute('role'), 'status');
  assert.equal(await page.locator('#persona-effective-hint').getAttribute('aria-live'), 'polite');
  await page.locator('#persona-select').selectOption('');
  assert.equal(await page.locator('#btn-save-persona').isDisabled(), true);
  assert.equal(await page.locator('#btn-delete-persona').isDisabled(), true);

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
  assert.equal(await page.locator('#persona-select').inputValue(), '');
  await page.waitForFunction(name => document.body.textContent.includes(name), injectionName);
  assert.equal(await page.locator('img[src="x"]').count(), 0);
  assert.equal(await page.evaluate(() => window.__airpXss), 0);

  // #126 D-PR2: lorebook-section DOM wiring（主面板迁移后 workbench 不再有 lorebook tab）
  assert.equal(await page.locator('#lorebook-section').isVisible(), true);
  assert.equal(await page.locator('#lore-entries').count(), 1);
  assert.equal(await page.locator('#btn-lore-add').count(), 1);
  assert.equal(await page.locator('#btn-lore-save').count(), 1);
  assert.equal(await page.locator('#btn-refresh-lorebook').count(), 1);
  // workbench lorebook tab 已移除
  assert.equal(await page.locator('#wb-tab-lorebook').count(), 0);
  assert.equal(await page.locator('[data-tab="lorebook"]').count(), 0);

  // S1/S2: 通过 API 注入带 advisory 字段的 lorebook entry，验证 selective toggle
  // 启用/禁用 secondary_keys input，以及 advisory 区域只读（span 非 input）。
  await page.evaluate(async () => {
    const payload = {
      entries: [
        {
          keys: ['dragon'],
          content: 'dragons are cool',
          enabled: true,
          priority: 10,
          constant: false,
          comment: null,
          selective: false,
          secondary_keys: [],
          case_sensitive: true,
          extensions: { position: 'before_char', depth: 4, probability: 80, selective: false },
        },
      ],
    };
    const r = await fetch('/v1/characters/smoke-xss/lorebook', {
      method: 'PUT',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!r.ok) throw new Error('PUT lorebook failed: ' + r.status);
  });
  // 第三方审计反馈：Playwright selectOption 无条件 dispatch change，"相同值不触发 change"
  // 根因假设不成立。更可能是 change handler 中 await refreshSessions() 等步骤抛异常导致
  // loadLorebook() 永不执行。先 selectOption 触发 change，再用 try/catch 加诊断输出
  // 捕获真实失败原因（pageErrors / lore-entries innerHTML / lore-status text）。
  // 按钮已从 <summary> 挪到 <details> 内部（方案 C，可访问性改进），避免 summary toggle 拦截。
  page.on('dialog', dialog => dialog.accept());
  await page.locator('#char-select').selectOption('smoke-xss');
  try {
    await page.waitForFunction(() => document.querySelector('#lore-entries .wb-lore-entry'), null, { timeout: 10_000 });
  } catch (err) {
    // 诊断输出：超时时打印 pageErrors、lore-entries 内容、lore-status 文本，
    // 便于在 CI 日志中直接看到真实失败原因，而非反复猜测事件是否触发。
    const diagPageErrors = await page.evaluate(() => window.__airpCspViolations);
    const diagLoreEntriesHtml = await page.locator('#lore-entries').innerHTML().catch(() => '<unavailable>');
    const diagLoreStatus = await page.locator('#lore-status').textContent().catch(() => '<unavailable>');
    const diagSelectedChar = await page.locator('#char-select').inputValue().catch(() => '<unavailable>');
    console.error('DIAG pageErrors(csp):', JSON.stringify(diagPageErrors));
    console.error('DIAG lore-entries innerHTML:', diagLoreEntriesHtml);
    console.error('DIAG lore-status text:', diagLoreStatus);
    console.error('DIAG char-select value:', diagSelectedChar);
    throw err;
  }
  // S1: selective=false 时 secondary_keys input disabled
  const secDisabledBefore = await page.locator('#lore-entries .wb-lore-secondary').first().isDisabled();
  assert.equal(secDisabledBefore, true);
  // S1: 勾选 selective 后 secondary_keys input 启用
  await page.locator('#lore-entries .wb-lore-selective input').first().check();
  const secDisabledAfter = await page.locator('#lore-entries .wb-lore-secondary').first().isDisabled();
  assert.equal(secDisabledAfter, false);
  // S5: aria-expanded 在展开后为 true
  await page.locator('#lore-entries .wb-lore-toggle').first().click();
  const ariaExpanded = await page.locator('#lore-entries .wb-lore-toggle').first().getAttribute('aria-expanded');
  assert.equal(ariaExpanded, 'true');
  // S2: advisory 区域用 span 渲染，不存在可输入元素（input/textarea）
  const advInputCount = await page.locator('#lore-entries .wb-lore-advisory input, #lore-entries .wb-lore-advisory textarea').count();
  assert.equal(advInputCount, 0);
  const advSpanCount = await page.locator('#lore-entries .wb-lore-advisory-value').count();
  assert.ok(advSpanCount > 0, 'advisory 区域应有 span 渲染的值');
  // S2/CR-nitpick: top-level case_sensitive 与 extensions.position 都展示，selective 跳过
  const advText = await page.locator('#lore-entries .wb-lore-advisory').first().textContent();
  assert.ok(advText.includes('case_sensitive'), 'advisory 应含 case_sensitive');
  assert.ok(advText.includes('position'), 'advisory 应含 position');
  assert.ok(advText.includes('depth'), 'advisory 应含 depth');
  assert.ok(!advText.includes('selective'), 'advisory 不应含 selective');

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
