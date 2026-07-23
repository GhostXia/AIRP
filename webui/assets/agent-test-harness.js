// AIRP WebUI Agent Test Harness v2
// Dev/test-only. Activation gates (any one):
//   ?airp_agent_test=1  |  localStorage.AIRP_AGENT_TEST=1  |  VITE_AIRP_AGENT_TEST=1
// Users who don't want this surface can delete this file; screens load it via
// a conditional <script> that fails silently when absent.
(function () {
  'use strict';

  function shouldInstallAgentTestHarness() {
    // VITE_AIRP_AGENT_TEST=1 is the Vite build-time flag for the desktop ui/ harness.
    // In webui's vanilla classic <script>, there is no import.meta.env; a build step
    // or operator may set window.VITE_AIRP_AGENT_TEST=1 before this script loads.
    if (globalThis.VITE_AIRP_AGENT_TEST === '1') return true;
    try {
      const params = new URLSearchParams(window.location.search);
      if (params.get('airp_agent_test') === '1') return true;
      return window.localStorage.getItem('AIRP_AGENT_TEST') === '1';
    } catch {
      return false;
    }
  }

  if (!shouldInstallAgentTestHarness()) return;

  const consoleErrors = [];
  const failedRequests = [];
  window.addEventListener('error', (e) => consoleErrors.push({ type: 'error', message: e.message, source: e.filename + ':' + e.lineno }));
  window.addEventListener('unhandledrejection', (e) => consoleErrors.push({ type: 'unhandledrejection', reason: String(e.reason) }));
  const origFetch = window.fetch && window.fetch.bind(window);
  if (origFetch) {
    window.fetch = async function (...args) {
      try {
        const res = await origFetch(...args);
        if (!res.ok) failedRequests.push({ url: typeof args[0] === 'string' ? args[0] : args[0].url, status: res.status });
        return res;
      } catch (err) {
        failedRequests.push({ url: String(args[0]), error: String(err) });
        throw err;
      }
    };
  }

  function clone(value) {
    if (value == null) return value;
    return typeof structuredClone === 'function' ? structuredClone(value) : JSON.parse(JSON.stringify(value));
  }

  function getOrigin() {
    // 同源默认；?engine=... 仅开发联调用，harness 不读它
    return window.location.origin;
  }

  async function apiRequest(method, path, body) {
    const res = await fetch(getOrigin() + path, {
      method,
      headers: body ? { 'Content-Type': 'application/json' } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    let json = null;
    try { json = text ? JSON.parse(text) : null; } catch {}
    return { status: res.status, ok: res.ok, json, text };
  }

  function buildDomSnapshot() {
    // 简化 a11y-like 树：可交互元素 + 可见文本节点
    const out = [];
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_ELEMENT);
    let node = walker.currentNode;
    while (node) {
      const tag = node.tagName.toLowerCase();
      const interactive = ['a', 'button', 'input', 'textarea', 'select', '[role="button"]'].some(s => s.startsWith('[') ? node.matches(s) : tag === s);
      if (interactive || (node.textContent && node.textContent.trim() && node.children.length === 0)) {
        const rect = node.getBoundingClientRect();
        if (rect.width === 0 && rect.height === 0) { node = walker.nextNode(); continue; }
        out.push({
          tag,
          id: node.id || null,
          classes: node.className && typeof node.className === 'string' ? node.className.split(/\s+/).filter(Boolean) : [],
          text: (node.textContent || '').trim().slice(0, 200),
          role: node.getAttribute('role'),
          ariaLabel: node.getAttribute('aria-label'),
          disabled: node.disabled || false,
          visible: rect.width > 0 && rect.height > 0,
        });
      }
      node = walker.nextNode();
    }
    return out;
  }

  const harness = {
    version: 2,

    navigate(screen, params) {
      const url = new URL(window.location.origin + '/screens/' + screen);
      if (params) for (const [k, v] of Object.entries(params)) url.searchParams.set(k, v);
      url.searchParams.set('airp_agent_test', '1');
      window.location.href = url.href;
    },

    getCurrentScreen() {
      const m = window.location.pathname.match(/\/screens\/([^/]+\.html)/);
      return m ? m[1] : null;
    },

    fillInput(selector, text) {
      const el = document.querySelector(selector);
      if (!el) throw new Error('fillInput: selector not found: ' + selector);
      if (el.tagName === 'INPUT' || el.tagName === 'TEXTAREA') {
        el.value = text;
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
      } else throw new Error('fillInput: not an input: ' + selector);
    },

    clickButton(selectorOrText) {
      // 先按可见文本找 button，避免非选择器文本（如"发送首轮消息"）触发 querySelector 语法错误
      const buttons = Array.from(document.querySelectorAll('button, [role="button"]'));
      let el = buttons.find(b => (b.textContent || '').trim() === selectorOrText);
      if (!el) {
        // 不是按钮文本时，作为 CSS 选择器尝试；捕获语法错误以免整段崩溃
        try { el = document.querySelector(selectorOrText); } catch (e) {
          throw new Error('clickButton: not a valid selector and no button text matched: ' + selectorOrText);
        }
      }
      if (!el) throw new Error('clickButton: not found: ' + selectorOrText);
      if (el.disabled) throw new Error('clickButton: disabled: ' + selectorOrText);
      el.click();
    },

    getVisibleText(selector = 'body') {
      const el = document.querySelector(selector);
      return el ? (el.textContent || '').trim() : '';
    },

    getDomSnapshot() {
      return buildDomSnapshot();
    },

    getConsoleErrors() {
      return clone(consoleErrors);
    },

    getFailedRequests() {
      return clone(failedRequests);
    },

    async getApiSnapshot(path, method = 'GET', body) {
      return apiRequest(method, path, body);
    },

    async waitFor(predicateId, timeoutMs = 5000) {
      // predicateId 是预定义 predicate 标识符字符串，避免 new Function（CSP unsafe-eval 禁止）
      const PREDICATES = {
        'send-button-ready': () => {
          const btn = document.querySelector('.send-button, [data-send], button[type="submit"]');
          return !!btn && !btn.classList.contains('stop') && !btn.disabled;
        },
        'no-pending-request': () => !document.querySelector('[data-pending-request="true"]'),
        'message-list-stable': () => {
          const list = document.querySelector('[data-message-list], .message-list, .chat-history');
          return !!list && list.querySelectorAll('[data-message-id]').length > 0;
        },
      };
      const predicate = PREDICATES[predicateId];
      if (typeof predicate !== 'function') throw new Error('waitFor: unknown predicate id: ' + predicateId);
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        try { if (predicate()) return true; } catch {}
        await new Promise(r => window.setTimeout(r, 100));
      }
      return false;
    },

    async screenshot() {
      // harness 不能直接截图；Playwright 侧通过 page.screenshot() 调用
      // 此处仅返回当前屏幕标识供 runner 协调
      return { screen: this.getCurrentScreen(), timestamp: Date.now() };
    },
  };

  window.__AIRP_AGENT_TEST__ = harness;
  console.info('[AIRP] webui agent test harness v2 enabled');
})();
