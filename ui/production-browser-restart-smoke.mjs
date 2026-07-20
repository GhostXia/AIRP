import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { chromium } from 'playwright-core';

const origin = process.env.AIRP_SMOKE_ORIGIN;
const username = process.env.AIRP_SMOKE_ADMIN_USER;
const password = process.env.AIRP_SMOKE_ADMIN_PASSWORD;
const browserStateFile = process.env.AIRP_SMOKE_BROWSER_STATE_FILE;
const browserResultFile = process.env.AIRP_SMOKE_BROWSER_RESULT_FILE;
const executablePath = process.env.AIRP_CHROME_PATH || '/usr/bin/google-chrome';
const chromeSpki = process.env.AIRP_CHROME_SPKI;

for (const [name, value] of Object.entries({
  origin,
  username,
  password,
  browserStateFile,
  browserResultFile,
  chromeSpki,
})) {
  assert.ok(value, `${name} is required`);
}
assert.match(chromeSpki, /^[A-Za-z0-9+/]{43}=$/, 'chromeSpki must be one SHA-256 hash');

const expected = JSON.parse(readFileSync(browserResultFile, 'utf8'));
for (const field of ['firstMessage', 'characterId', 'sessionId']) {
  assert.ok(expected[field], `${field} is required in the browser result`);
}
assert.ok(expected.firstAssistant?.messageId, 'firstAssistant.messageId is required');
assert.ok(expected.firstAssistant?.text?.trim(), 'firstAssistant.text is required');

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
    storageState: browserStateFile,
  });
  const page = await context.newPage();
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
  await page.waitForFunction(({ characterId, sessionId, firstMessage, firstAssistant }) => {
    const assistants = Array.from(document.querySelectorAll('#chat-log .msg.assistant[data-message-id]'));
    return document.querySelector('#char-select')?.value === characterId
      && document.querySelector('#sess-select')?.value === sessionId
      && document.querySelector('#chat-log')?.textContent?.includes(firstMessage)
      && assistants.some(node => node.dataset.messageId === firstAssistant.messageId
        && node.querySelector('.text')?.textContent === firstAssistant.text);
  }, expected, { timeout: 15_000 });

  assert.equal(await page.locator('#char-select').inputValue(), expected.characterId);
  assert.equal(await page.locator('#sess-select').inputValue(), expected.sessionId);
  const durableIdsBefore = await page.locator('#chat-log .msg.assistant[data-message-id]').evaluateAll(nodes =>
    nodes.map(node => node.dataset.messageId)
  );

  // engine restart 后第一个真实 SSE 流仍可能被 Caddy upstream reset（PR #251 重试记录）。
  // transient error（fetch error / stream interrupted / HTTP 5xx）允许重试一次：
  // - 重试前清掉错误消息 DOM，避免污染下一轮 transientErrors 收集
  // - 重试消息用新 timestamp 防止去重
  // - 最多 2 次尝试，全失败才断言失败
  const sendSecondTurn = async (messageText) => {
    await page.locator('[data-view="session"]').first().click();
    await page.locator('#chat-input').fill(messageText);
    const chatResponsePromise = page.waitForResponse(response =>
      response.url().endsWith('/v1/chat/completions') && response.request().method() === 'POST'
    );
    await page.locator('#btn-send').click();
    const chatResponse = await chatResponsePromise;
    assert.equal(chatResponse.status(), 200);
    await page.waitForFunction(() => document.querySelector('#btn-stop')?.hidden === true, null, { timeout: 15_000 });
    return page.locator('#chat-log .msg.assistant .text').evaluateAll(nodes =>
      nodes.map(node => node.textContent || '').filter(text => /^\[(?:fetch error|stream interrupted|HTTP )/.test(text))
    );
  };

  const clearTransientErrorMessages = async () => {
    // 删除 .msg.assistant 节点中只含 transient error 标记的（保留真实消息）
    await page.evaluate(() => {
      const nodes = document.querySelectorAll('#chat-log .msg.assistant .text');
      nodes.forEach(node => {
        if (/^\[(?:fetch error|stream interrupted|HTTP )/.test(node.textContent || '')) {
          node.closest('.msg.assistant')?.remove();
        }
      });
    });
  };

  let secondMessage = `restart continuity ${Date.now()}`;
  let transientErrors = await sendSecondTurn(secondMessage);
  if (transientErrors.length > 0) {
    console.log(`second turn transient error (will retry once): ${JSON.stringify(transientErrors)}`);
    await clearTransientErrorMessages();
    await page.waitForTimeout(1_000);
    secondMessage = `restart continuity retry ${Date.now()}`;
    transientErrors = await sendSecondTurn(secondMessage);
  }
  assert.deepEqual(transientErrors, [], 'second turn must complete without a transient chat error');

  // Reload from the durable history endpoint instead of accepting the streamed
  // DOM as proof that the second turn survived the restarted stack.
  await page.waitForTimeout(2_500);
  await page.locator('#btn-history').click();
  await page.waitForFunction(({ secondMessage, durableIdsBefore }) => {
    const log = document.querySelector('#chat-log');
    const assistants = Array.from(document.querySelectorAll('#chat-log .msg.assistant[data-message-id]'));
    return log?.textContent?.includes(secondMessage)
      && assistants.some(node => !durableIdsBefore.includes(node.dataset.messageId)
        && node.querySelector('.text')?.textContent?.trim());
  }, { secondMessage, durableIdsBefore }, { timeout: 10_000 });

  const durableIdsAfter = await page.locator('#chat-log .msg.assistant[data-message-id]').evaluateAll(nodes =>
    nodes.map(node => node.dataset.messageId)
  );
  assert.ok(
    durableIdsAfter.some(messageId => !durableIdsBefore.includes(messageId)),
    'second turn must create a durable assistant message',
  );
  assert.deepEqual(await page.evaluate(() => window.__airpCspViolations), []);
  assert.deepEqual(pageErrors, []);
  await context.close();
  console.log('production browser restart continuity smoke passed');
} finally {
  await browser.close();
}
