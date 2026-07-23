// 通过 page.evaluate 调用页面内 window.__AIRP_AGENT_TEST__ 的 helper

export class HarnessClient {
  constructor(page, origin) {
    this.page = page;
    this.origin = origin;
  }

  async isReady() {
    return await this.page.evaluate(() => !!(window.__AIRP_AGENT_TEST__ && window.__AIRP_AGENT_TEST__.version === 2));
  }

  // bounded wait for async-loaded harness to install window.__AIRP_AGENT_TEST__
  async waitForReady(timeoutMs = 10000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (await this.isReady()) return;
      await new Promise(r => setTimeout(r, 100));
    }
    throw new Error('HarnessClient: harness not ready after ' + timeoutMs + 'ms (async <script> not installed?)');
  }

  // navigate uses page.goto() (waits for load) then bounded waitForReady()
  // instead of in-page window.location.href, so the next helper call cannot
  // race the async harness script load.
  async navigate(screen, params) {
    const url = new URL(this.origin + '/screens/' + screen);
    if (params) for (const [k, v] of Object.entries(params)) url.searchParams.set(k, v);
    url.searchParams.set('airp_agent_test', '1');
    await this.page.goto(url.href, { waitUntil: 'load' });
    await this.waitForReady();
  }

  async getCurrentScreen() {
    return await this.page.evaluate(() => window.__AIRP_AGENT_TEST__.getCurrentScreen());
  }

  async fillInput(selector, text) {
    return await this.page.evaluate(([s, t]) => window.__AIRP_AGENT_TEST__.fillInput(s, t), [selector, text]);
  }

  async clickButton(selectorOrText) {
    return await this.page.evaluate((s) => window.__AIRP_AGENT_TEST__.clickButton(s), selectorOrText);
  }

  async getVisibleText(selector) {
    return await this.page.evaluate((s) => window.__AIRP_AGENT_TEST__.getVisibleText(s), selector);
  }

  async getDomSnapshot() {
    return await this.page.evaluate(() => window.__AIRP_AGENT_TEST__.getDomSnapshot());
  }

  async getConsoleErrors() {
    return await this.page.evaluate(() => window.__AIRP_AGENT_TEST__.getConsoleErrors());
  }

  async getFailedRequests() {
    return await this.page.evaluate(() => window.__AIRP_AGENT_TEST__.getFailedRequests());
  }

  async getApiSnapshot(path, method = 'GET', body) {
    return await this.page.evaluate(([p, m, b]) => window.__AIRP_AGENT_TEST__.getApiSnapshot(p, m, b), [path, method, body]);
  }

  // predicateId 是预定义标识符字符串，由 harness 内 PREDICATES 注册表解析；
  // 不再用 new Function(predicateSrc)（webui CSP 禁止 unsafe-eval）。
  async waitFor(predicateId, timeoutMs = 5000) {
    return await this.page.evaluate(([id, t]) => window.__AIRP_AGENT_TEST__.waitFor(id, t), [predicateId, timeoutMs]);
  }

  async screenshot(path) {
    return await this.page.screenshot({ path, fullPage: true });
  }

  async saveTrace(context, path) {
    return await context.tracing.stop({ path });
  }
}
