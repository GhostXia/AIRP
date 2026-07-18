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

  // P1 golden path: a fresh production browser must load the real onboarding
  // module, complete the first chat through visible UI, and retain that session
  // after a page refresh. A missing module used to fall back to the manual
  // console and let this smoke pass without testing onboarding at all.
  const onboardingAsset = await context.request.get(origin + '/onboarding.js');
  assert.equal(onboardingAsset.status(), 200);
  assert.match(onboardingAsset.headers()['content-type'] || '', /javascript/);

  const onboardingRoot = page.locator('#onboarding-root');
  await page.waitForFunction(() => document.querySelector('#onboarding-root h2')?.textContent?.includes('Step 1'));
  await onboardingRoot.getByRole('button', { name: '下一步', exact: true }).click();

  await page.waitForFunction(() => document.querySelector('#onboarding-root h2')?.textContent?.includes('Step 2'));
  await page.waitForFunction(() => {
    const inputs = document.querySelectorAll('#onboarding-root .onb-stage input');
    return inputs.length >= 4 && inputs[0].value && inputs[1].value && inputs[2].value;
  });
  await onboardingRoot.getByRole('button', { name: '保存并验证' }).click();

  await page.waitForFunction(() => document.querySelector('#onboarding-root h2')?.textContent?.includes('Step 3'));
  const modelPicker = onboardingRoot.locator('.onb-model-picker select');
  await page.waitForFunction(() => document.querySelectorAll('#onboarding-root .onb-model-picker select option').length > 1);
  await modelPicker.selectOption({ index: 1 });
  await onboardingRoot.getByRole('button', { name: '保存并下一步' }).click();

  await page.waitForFunction(() => document.querySelector('#onboarding-root h2')?.textContent?.includes('Step 4'));
  const onboardingCard = {
    spec: 'chara_card_v2',
    spec_version: '2.0',
    data: {
      name: 'Onboarding Browser',
      description: '生产向导黄金路径角色',
      personality: '沉稳',
      scenario: '黄昏的旧街区',
      first_mes: '你来了。',
      mes_example: '',
      creator_notes: 'production browser smoke fixture',
      system_prompt: '保持角色回复。',
      post_history_instructions: '',
      alternate_greetings: [],
      character_book: null,
      tags: ['smoke'],
      creator: 'airp-smoke',
      character_version: '1',
      extensions: {},
    },
  };
  await onboardingRoot.locator('input[type="file"]').setInputFiles({
    name: 'onboarding-browser.json',
    mimeType: 'application/json',
    buffer: Buffer.from(JSON.stringify(onboardingCard)),
  });
  await onboardingRoot.getByRole('button', { name: '导入', exact: true }).click();

  await page.waitForFunction(() => document.querySelector('#onboarding-root h2')?.textContent?.includes('Step 5'));
  await page.waitForFunction(() => document.querySelectorAll('#onboarding-root .onb-stage select').length >= 2);
  await onboardingRoot.getByRole('button', { name: '下一步', exact: true }).click();

  const firstMessage = '请带我看看这条旧街。';
  await page.waitForFunction(() => document.querySelector('#onboarding-root h2')?.textContent?.includes('Step 6'));
  await onboardingRoot.locator('textarea').fill(firstMessage);
  await onboardingRoot.getByRole('button', { name: '发送', exact: true }).click();
  await page.waitForFunction(() => document.querySelector('#onboarding-root')?.hidden === true, null, { timeout: 15_000 });
  await page.waitForFunction(() => document.querySelector('#conn-text')?.textContent?.startsWith('已连接'), null, { timeout: 15_000 });

  const onboardingState = await page.evaluate(() => ({
    onboarded: localStorage.getItem('airp_onboarded'),
    characterId: localStorage.getItem('airp_character_id'),
    sessionId: localStorage.getItem('airp_session_id'),
  }));
  assert.equal(onboardingState.onboarded, 'true');
  assert.ok(onboardingState.characterId, 'onboarding must persist the selected character');
  assert.ok(onboardingState.sessionId, 'onboarding must persist the first-chat session');

  // Allow the shared rate-limit bucket to refill, then prove that onboarding
  // finalized a durable assistant turn before testing refresh recovery.
  await page.waitForTimeout(2_500);
  await page.locator('[data-view="session"]').first().click();
  await page.locator('#btn-history').click();
  await page.waitForFunction(() => {
    const turns = document.querySelectorAll('#chat-log .msg.assistant[data-message-id] .text');
    return turns.length > 0 && turns[turns.length - 1].textContent?.trim();
  }, null, { timeout: 10_000 });
  const firstAssistant = await page.locator('#chat-log .msg.assistant[data-message-id]').last().evaluate(node => ({
    messageId: node.dataset.messageId,
    text: node.querySelector('.text')?.textContent || '',
  }));
  assert.ok(firstAssistant.messageId, 'first chat must finalize a durable assistant message');
  assert.ok(firstAssistant.text.trim(), 'first chat assistant response must be non-empty');

  await page.reload({ waitUntil: 'domcontentloaded' });
  await page.waitForFunction(() => document.querySelector('#conn-text')?.textContent?.startsWith('已连接'), null, { timeout: 15_000 });
  await page.waitForFunction(message => document.querySelector('#chat-log')?.textContent?.includes(message), firstMessage, { timeout: 10_000 });
  await page.waitForFunction(expected => Array.from(document.querySelectorAll('#chat-log .msg.assistant[data-message-id]')).some(node =>
    node.dataset.messageId === expected.messageId && node.querySelector('.text')?.textContent === expected.text
  ), firstAssistant, { timeout: 10_000 });
  assert.equal(await page.locator('#char-select').inputValue(), onboardingState.characterId);
  assert.equal(await page.locator('#sess-select').inputValue(), onboardingState.sessionId);
  await page.waitForTimeout(2_500);

  // #182 防回归：确保 production 容器 serve 了 lorebook-utils.js。
  // 根因曾是 Dockerfile.gateway 漏 COPY → Caddy try_files fallback 到 index.html
  // → MIME text/html → 浏览器拒绝执行 → AIRPLorebookUtils undefined。
  assert.equal(await page.evaluate(() => typeof window.AIRPShared), 'object');
  assert.equal(await page.evaluate(() => typeof window.AIRPLorebookUtils), 'object');
  assert.equal(await page.evaluate(() => typeof window.AIRPAssemblyUtils), 'object');
  assert.equal(await page.evaluate(() => typeof window.AIRPHistoryUtils), 'object');
  assert.equal(await page.locator('#assembly-summary').count(), 1);
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

  // #194: hostile trace metadata must flow through the real renderer as text only.
  // #115 Phase 2h: 同时验证 6 个 *_revision 字段都在装配预览面板渲染（· r{N} 后缀）。
  await page.route('**/v1/chat/preview', async route => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        effective: {
          character_id: 'smoke-xss',
          character_revision: 3,
          persona_id: 'smoke-persona',
          persona_revision: 7,
          preset_id: 'smoke-preset',
          preset_revision: 2,
          lorebook_revision: 5,
          state_revision: 42,
          memory_revision: 9,
          model: injectionName,
          provider: 'test',
          // #114 effective config summary：来源字段 + 新增参数
          persona_activation_source: 'explicit',
          provider_source: 'snapshot',
          model_source: 'preset',
          temperature: 0.8,
          temperature_source: 'request',
          max_tokens: 2048,
          max_tokens_source: 'preset',
        },
        total_estimated_tokens: 1,
        segments: [{ source_kind: 'card', source_id: injectionName, chars: 1, estimated_tokens: 1, stable_or_volatile: 'stable' }],
        diagnostics: [{ kind: 'fixture', message: injectionName }],
      }),
    });
  });
  await page.locator('[data-view="session"]').first().click();
  await Promise.all([
    page.waitForResponse(response => response.url().endsWith('/v1/chat/preview') && response.request().method() === 'POST'),
    page.locator('#btn-refresh-assembly').click(),
  ]);
  await page.waitForFunction(name => document.querySelector('#assembly-summary')?.textContent?.includes(name), injectionName);
  assert.equal(await page.locator('#assembly-summary img[src="x"]').count(), 0);
  assert.equal(await page.evaluate(() => window.__airpXss), 0);

  // #115 Phase 2h: 6 个 revision 都应在装配预览面板渲染（· r{N} 后缀）
  await page.waitForFunction(() => {
    const text = document.querySelector('#assembly-summary')?.textContent || '';
    return text.includes('smoke-xss · r3')
      && text.includes('smoke-persona · r7')
      && text.includes('smoke-preset · r2')
      && text.includes('世界书 · r5')
      && text.includes('状态 · r42')
      && text.includes('记忆 · r9');
  }, null, { timeout: 5_000 });
  // #114 effective config summary：模型 chip 应显示 · 预设（model_source=preset），
  // 身份 chip 应显示 · 显式（persona_activation_source=explicit），温度 chip 应显示 0.8 · 请求。
  // #221 L2：用 DOM 选择器精确定位 chip（byLabel），避免 text.includes 子串冗余断言。
  await page.waitForFunction(() => {
    const chips = document.querySelectorAll('#assembly-summary .assembly-chip');
    if (chips.length < 10) return false;
    const byLabel = {};
    chips.forEach(chip => {
      // PR #227 审计修复（gemini）：.trim() 防止 HTML 格式微小变化（空白/换行）破坏严格相等断言
      const label = (chip.querySelector('.assembly-chip-label')?.textContent || '').trim();
      const value = (chip.querySelector('.assembly-chip-value')?.textContent || '').trim();
      byLabel[label] = value;
    });
    return (byLabel['模型'] || '').endsWith('· 预设')
      && (byLabel['身份'] || '').endsWith('· 显式')
      && (byLabel['温度'] || '') === '0.8 · 请求'
      && (byLabel['最大 tokens'] || '') === '2048 · 预设';
  }, null, { timeout: 5_000 });
  await page.unroute('**/v1/chat/preview');

  // #115 Phase 2h: 旧数据场景 — 6 个 *_revision_unavailable 诊断应让对应 chip 显示 · unavailable
  await page.route('**/v1/chat/preview', async route => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        effective: { character_id: 'legacy-char' },
        total_estimated_tokens: 1,
        segments: [],
        diagnostics: [
          { kind: 'character_revision_unavailable', message: '角色卡未升级' },
          { kind: 'persona_revision_unavailable', message: 'Persona 未升级' },
          { kind: 'preset_revision_unavailable', message: 'Preset 未升级' },
          { kind: 'lorebook_revision_unavailable', message: 'Worldbook 未升级' },
          { kind: 'state_revision_unavailable', message: 'State 未升级' },
          { kind: 'memory_revision_unavailable', message: 'Memory 未升级' },
        ],
      }),
    });
  });
  await Promise.all([
    page.waitForResponse(response => response.url().endsWith('/v1/chat/preview') && response.request().method() === 'POST'),
    page.locator('#btn-refresh-assembly').click(),
  ]);
  await page.waitForFunction(() => {
    // CodeRabbit nitpick 修复：通过 DOM 选择器按 chip label 区分 persona 和 preset
    // chip（二者 value 都是 '未启用 · unavailable'，text.includes 无法区分）。
    const chips = document.querySelectorAll('#assembly-summary .assembly-chip');
    if (chips.length < 6) return false;
    const byLabel = {};
    chips.forEach(chip => {
      // PR #227 审计修复（gemini）：.trim() 防止 HTML 格式微小变化（空白/换行）破坏严格相等断言
      const label = (chip.querySelector('.assembly-chip-label')?.textContent || '').trim();
      const value = (chip.querySelector('.assembly-chip-value')?.textContent || '').trim();
      byLabel[label] = value;
    });
    return (byLabel['角色'] || '').includes('legacy-char · unavailable')
      && (byLabel['身份'] || '').includes('unavailable')   // persona
      && (byLabel['预设'] || '').includes('unavailable')    // preset
      && (byLabel['世界书'] || '').includes('unavailable')
      && (byLabel['状态'] || '').includes('unavailable')
      && (byLabel['记忆'] || '').includes('unavailable');
  }, null, { timeout: 5_000 });
  await page.unroute('**/v1/chat/preview');

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
  //
  // 限流策略（CodeRabbit 审计 + CI 诊断双重确认）：
  // engine 全局 tower_governor 限流 10 req/s, burst 20 per IP。smoke 前面已发大量请求
  // (import/3 轮 chat SSE/history/rollback/regen)，token bucket 已耗尽。若直接 PUT 会 429；
  // PUT 重试会消耗更多 token，导致后续 GET loadLorebook 也 429（CI run 29463696091 实测）。
  // 正确做法：先等待 bucket 完全恢复（burst 20 / 10 req/s = 2s 即可恢复满），再发 PUT +
  // 后续 GET。保留 PUT 429 重试作为兜底，应对 CI 环境时序抖动。
  await page.waitForTimeout(3_000);
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
    const body = JSON.stringify(payload);
    for (let attempt = 0; attempt < 5; attempt++) {
      const r = await fetch('/v1/characters/smoke-xss/lorebook', {
        method: 'PUT',
        headers: { 'Content-Type': 'application/json' },
        body,
      });
      if (r.ok) return;
      if (r.status === 429 && attempt < 4) {
        // 退避 1s 让 token bucket 恢复（10 req/s → 1s 恢复 10 tokens）
        await new Promise(resolve => setTimeout(resolve, 1000));
        continue;
      }
      throw new Error('PUT lorebook failed: ' + r.status);
    }
  });
  // selectOption 触发 change → loadLorebook()。try/catch 在超时时打印 pageErrors / DOM 状态，
  // 便于 CI 日志直接看到真实失败原因。#182 根因曾是 Dockerfile.gateway 漏 COPY lorebook-utils.js，
  // 导致 AIRPLorebookUtils undefined → renderLoreEntry 抛 ReferenceError。
  page.on('dialog', dialog => dialog.accept());
  await page.locator('#char-select').selectOption('smoke-xss');
  try {
    await page.waitForFunction(() => document.querySelector('#lore-entries .wb-lore-entry'), null, { timeout: 10_000 });
  } catch (err) {
    const diagCspViolations = await page.evaluate(() => window.__airpCspViolations);
    const diagLoreEntriesHtml = await page.locator('#lore-entries').innerHTML().catch(() => '<unavailable>');
    const diagLoreStatus = await page.locator('#lore-status').textContent().catch(() => '<unavailable>');
    const diagSelectedChar = await page.locator('#char-select').inputValue().catch(() => '<unavailable>');
    console.error('DIAG pageErrors (uncaught JS exceptions):', JSON.stringify(pageErrors));
    console.error('DIAG cspViolations:', JSON.stringify(diagCspViolations));
    console.error('DIAG lore-entries innerHTML:', diagLoreEntriesHtml);
    console.error('DIAG lore-status text:', diagLoreStatus);
    console.error('DIAG char-select value:', diagSelectedChar);
    throw err;
  }
  // S1: selective=false 时 secondary_keys input disabled
  // #lorebook-section 是 <details>，默认 closed → 内部 #lore-entries display:none。
  // 展开后才能对内部控件做可见性敏感的操作（.check()/.click()）。
  await page.locator('#lorebook-section > summary').click();
  const secDisabledBefore = await page.locator('#lore-entries .wb-lore-secondary').first().isDisabled();
  assert.equal(secDisabledBefore, true);
  // S1: 勾选 selective 后 secondary_keys input 启用
  await page.locator('#lore-entries .wb-lore-selective input').first().check();
  const secDisabledAfter = await page.locator('#lore-entries .wb-lore-secondary').first().isDisabled();
  assert.equal(secDisabledAfter, false);
  // #186 W-01: dirty 状态下取消切换角色时，select 必须恢复为内部 selectedChar。
  const selectedBeforeCancelledSwitch = await page.locator('#char-select').inputValue();
  await page.evaluate(() => {
    const select = document.querySelector('#char-select');
    const option = document.createElement('option');
    option.value = 'cancelled-switch-target';
    option.textContent = 'cancelled-switch-target';
    select.appendChild(option);
    const originalConfirm = window.confirm;
    window.confirm = () => false;
    select.value = option.value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
    window.confirm = originalConfirm;
    option.remove();
  });
  assert.equal(await page.locator('#char-select').inputValue(), selectedBeforeCancelledSwitch);

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

  // #186 W-02: 200 + non-JSON 响应不得触发 unhandled rejection，且必须清掉旧角色条目。
  const pageErrorCountBeforeMalformedLorebook = pageErrors.length;
  await page.route('**/v1/characters/smoke-xss/lorebook', route => route.fulfill({
    status: 200,
    contentType: 'text/plain',
    body: 'not-json',
  }));
  await page.locator('#btn-refresh-lorebook').click();
  await page.waitForFunction(() => document.querySelector('#lore-status')?.textContent === '加载失败: 响应格式异常');
  assert.equal(await page.locator('#lore-entries .wb-lore-entry').count(), 0);
  await page.waitForTimeout(50);
  assert.equal(pageErrors.length, pageErrorCountBeforeMalformedLorebook);
  await page.unroute('**/v1/characters/smoke-xss/lorebook');

  // 旧响应可省略 entries；缺失字段保持向后兼容并视为空世界书。
  await page.route('**/v1/characters/smoke-xss/lorebook', route => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: '{}',
  }));
  await page.locator('#btn-refresh-lorebook').click();
  await page.waitForFunction(() => document.querySelector('#lore-status')?.textContent === '已加载 0 条条目');
  await page.unroute('**/v1/characters/smoke-xss/lorebook');

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
