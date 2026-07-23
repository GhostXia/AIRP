# Agent 浏览器探索测试层 实施计划

> 状态：MVP 实施计划
>
> 来源 issue：[#273 test: 引入 Agent 驱动的浏览器探索测试层](https://github.com/GhostXia/AIRP/issues/273)
>
> 基线日期：2026-07-23，`main` 分支 @ `03ffaf6`
>
> 审计模型（撰写本计划的 agent）：GLM-5.2

## 目标

在现有确定性 Playwright Smoke 之上，为 `webui/` 增加一层 Agent 驱动的浏览器探索测试：Agent 根据用户级任务自主操作 WebUI、组合功能、观察状态，输出可复现的缺陷候选报告。MVP 不阻塞 PR；后续按 PR diff 自动选择受影响任务集运行（issue #273 阶段 2）。

## 架构

```text
PR opened/updated (Edit/Branch/Swipe/Memory 等标签或 diff 命中)
  ↓
CI: .github/workflows/agent-browser-exploration.yml
  ↓
Bootstrap: 复用 deploy/production/smoke-ci.sh 的 Mock Provider + TLS + 临时数据拓扑
  ↓
Runner: tools/agent-exploration/runner.mjs
  ├─ 读 PR diff → classify → 选任务集
  ├─ 对每个任务:
  │   ├─ 构造 prompt（任务描述 + harness DOM 快照）
  │   ├─ 调 Agent (LLM) 生成临时 Playwright 脚本（方案 A）
  │   │   预留 action protocol 接口供方案 B 后续接入
  │   ├─ 隔离环境执行临时脚本
  │   ├─ 失败回传日志让 Agent 修正（最多 N 轮）
  │   ├─ 捕获: 截图 / Playwright Trace / 控制台错误 / 失败网络请求
  │   └─ 写 per-task 报告
  └─ 汇总: Markdown + JSON 报告 + artifacts bundle
  ↓
上传 artifacts, 在 PR 发结构化评论
  ↓
崩溃 / 数据损坏 / 安全 → 人工确认（阶段 2）
可用性问题 → 仅记录, 不阻塞
```

## 技术栈

- `playwright-core`（复用现有 smoke 栈，不引入新浏览器自动化框架）
- `node:test` + `node:assert`（与 `webui/tests/` 一致，无构建）
- Engine HTTP/SSE API（`/v1/chat/*`, `/v1/characters/*`, `/v1/memory/*`）
- Mock Provider（`deploy/production/mock-provider.js`）
- Agent LLM：通过 `OPENAI_API_KEY` + `OPENAI_BASE_URL` 环境变量调用（与 engine 的 provider 解耦，runner 自带 client）

## 范围与非目标

**MVP 范围**：
- webui harness（vanilla JS，多屏导航模型）
- Runner（方案 A：生成临时 Playwright 脚本，预留方案 B action protocol 接口）
- 4 个任务集：onboarding+首聊+刷新 / Regen+Swipe+刷新 / Edit+Branch+切换+刷新 / Memory
- PR-diff 分类器（阶段 2 自动触发）
- Artifacts + Markdown/JSON 报告
- CI workflow（非阻塞）

**非目标**（issue #273 明确排除 + 本计划额外排除）：
- 桌面 `ui/` (Tauri/Vue) harness 升级：与桌面 UI 路线一起开发，本计划不动 `ui/src/agent-test.ts`。**实施时必须在 issue #273 评论中显式写明此推迟决定**。
- 真实用户替代 / RP 内容质量评价 / 市场验证
- 替代现有单元测试、集成测试和确定性 Smoke
- 在真实用户数据上操作

## 文件结构

```text
webui/
├── assets/
│   └── agent-test-harness.js      # 新增：webui 侧 harness，挂 window.__AIRP_AGENT_TEST__
├── screens/
│   └── 16-onboarding.html         # 修改：条件注入 harness（仅 dev/test）
│   └── 02-chat-space.html         # 修改：条件注入 harness
│   └── 01-role-list.html          # 修改：条件注入 harness
│   └── 17-memory-state.html       # 修改：条件注入 harness
│   └── 19-branch-tree.html        # 修改：条件注入 harness
│   └── 14-message-swipe.html      # 修改：条件注入 harness
├── tests/
│   └── agent-harness.test.mjs     # 新增：harness 静态门禁（CSP + API 形状）
└── index.html                     # 不动

tools/agent-exploration/            # 新增目录
├── package.json                   # 新增：playwright-core + node:test 依赖声明
├── runner.mjs                     # 主入口：读 diff → 选任务 → 跑 → 出报告
├── harness-client.mjs             # Playwright 侧调用 window.__AIRP_AGENT_TEST__ 的 helper
├── llm-client.mjs                 # OpenAI 兼容 LLM 客户端（与 engine provider 解耦）
├── action-protocol.mjs            # 方案 B 预留：{action,target} ↔ DOM 摘要协议接口
├── classifier.mjs                 # PR diff → 任务集映射
├── reporter.mjs                   # Markdown + JSON 报告生成
├── tasks/
│   ├── onboarding-firstchat-refresh.mjs
│   ├── regen-swipe-refresh.mjs
│   ├── edit-branch-switch-refresh.mjs
│   └── memory-roundtrip.mjs
└── fixtures/
    ├── character-card.json        # 合成角色卡 fixture（不复制第三方资产）
    └── expected-shapes.md         # 各任务预期状态形状参考

.github/workflows/
└── agent-browser-exploration.yml  # 新增：阶段 2 PR 自动触发

docs/
└── AGENT-BROWSER-EXPLORATION-PLAN.md  # 本文件
```

---

## Task 1: webui 侧 Agent Test Harness 核心

**Files:**
- Create: `webui/assets/agent-test-harness.js`
- Test: `webui/tests/agent-harness.test.mjs`

**说明**：现有 `ui/src/agent-test.ts` 是 Blueprint/intent 模型，不适用 webui 的 vanilla JS + 多屏导航模型。本任务新建 webui 专属 harness，原 `ui/` harness 保持不动。

- [ ] **Step 1: 写 harness 静态门禁测试（先红）**

写入 `webui/tests/agent-harness.test.mjs`：

```javascript
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const harnessScript = await readFile(new URL('../assets/agent-test-harness.js', import.meta.url), 'utf8');

test('harness script is CSP-compatible: no inline handlers, no eval', () => {
  assert.doesNotMatch(harnessScript, /\beval\s*\(/);
  assert.doesNotMatch(harnessScript, /new\s+Function\s*\(/);
  assert.doesNotMatch(harnessScript, /document\.write\s*\(/);
});

test('harness exposes window.__AIRP_AGENT_TEST__ v2 with required methods', () => {
  assert.match(harnessScript, /window\.__AIRP_AGENT_TEST__\s*=/);
  assert.match(harnessScript, /version:\s*2/);
  for (const method of [
    'navigate', 'getCurrentScreen', 'fillInput', 'clickButton',
    'getVisibleText', 'getDomSnapshot', 'getConsoleErrors',
    'getFailedRequests', 'getApiSnapshot', 'waitFor', 'screenshot'
  ]) {
    assert.match(harnessScript, new RegExp(`${method}\\s*\\(`));
  }
});

test('harness activation gate matches existing ui/ convention', () => {
  assert.match(harnessScript, /airp_agent_test=1/);
  assert.match(harnessScript, /AIRP_AGENT_TEST/);
  assert.match(harnessScript, /VITE_AIRP_AGENT_TEST/);
});

test('harness gate defaults to off in production build', () => {
  // 默认关闭逻辑必须存在，生产构建未带 flag 时不暴露
  assert.match(harnessScript, /shouldInstallAgentTestHarness/);
});
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
node --test webui/tests/agent-harness.test.mjs
```
Expected: FAIL（`agent-test-harness.js` 不存在）

- [ ] **Step 3: 实现 harness**

写入 `webui/assets/agent-test-harness.js`：

```javascript
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
```

- [ ] **Step 4: 跑测试确认通过**

```powershell
node --test webui/tests/agent-harness.test.mjs
```
Expected: PASS（4 tests）

- [ ] **Step 5: Commit**

```powershell
git add webui/assets/agent-test-harness.js webui/tests/agent-harness.test.mjs
git commit -m "feat(webui): add agent test harness v2 for browser exploration (#273)"
```

---

## Task 2: 在关键 screen 条件注入 harness

**Files:**
- Modify: `webui/screens/16-onboarding.html`
- Modify: `webui/screens/01-role-list.html`
- Modify: `webui/screens/02-chat-space.html`
- Modify: `webui/screens/17-memory-state.html`
- Modify: `webui/screens/19-branch-tree.html`
- Modify: `webui/screens/14-message-swipe.html`
- Test: `webui/tests/agent-harness.test.mjs`（增量）

**说明**：webui CSP 禁止内联脚本，必须用外部 `<script src>`。每个 screen 在 `</body>` 前条件加载 harness（`async` + `onerror` 静默吞，让用户可删文件不破坏构建）。

- [ ] **Step 1: 增量测试 — 所有目标 screen 都引用 harness**

在 `webui/tests/agent-harness.test.mjs` 末尾追加（`readFile` 已在文件顶部导入，不重复 import）：

```javascript
const screens = [
  '16-onboarding.html', '01-role-list.html', '02-chat-space.html',
  '17-memory-state.html', '19-branch-tree.html', '14-message-swipe.html',
];

for (const screen of screens) {
  test(screen + ' conditionally loads agent-test-harness.js with silent fail', async () => {
    const html = await readFile(new URL('../screens/' + screen, import.meta.url), 'utf8');
    // async + onerror 吞错，让删文件不破坏构建
    assert.match(html, /<script\s+src="[^"]*assets\/agent-test-harness\.js"\s+async\s+onerror="[^"]*"[^>]*><\/script>/);
    // 不得放在 <head>（避免阻塞首屏）
    const headEnd = html.indexOf('</head>');
    const scriptPos = html.indexOf('agent-test-harness.js');
    assert.ok(scriptPos > headEnd, screen + ': harness script must be after </head>');
  });
}
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
node --test webui/tests/agent-harness.test.mjs
```
Expected: 6 个新 test FAIL

- [ ] **Step 3: 在 6 个 screen 注入 harness script**

对每个 screen，在 `</body>` 之前插入（位置：紧贴现有最后一个 `<script>` 之后、`</body>` 之前）：

```html
    <script src="../assets/agent-test-harness.js" async onerror="/* dev/test-only harness; missing file is OK */"></script>
```

注意：`onerror` 是**内联事件处理器**，但 webui CSP 已存在内联事件处理器禁用规则。需检查 `webui/index.html` 与各 screen 的 CSP `script-src`，若禁内联事件则改用：

```html
    <script src="../assets/agent-test-harness.js" async></script>
```

并依赖 harness 内部的 `shouldInstallAgentTestHarness()` 守卫（文件缺失时浏览器自然报 404，不阻塞页面；harness 内 flag 默认 off，生产用户即使 404 也无影响）。

**注意（async 加载竞态）**：`<script async>` 在页面 `load` 事件后才安装 `window.__AIRP_AGENT_TEST__`。`HarnessClient.navigate()` 不能在调用 `window.__AIRP_AGENT_TEST__.navigate(...)` 后立即返回——必须在 `page.goto()` 后用 bounded `isReady()` 轮询等待 harness 安装完成（见 Task 3 `HarnessClient.navigate` 实现）。否则下一个 helper 调用会观察到 `undefined` 的 `window.__AIRP_AGENT_TEST__`。

实施时先读 `webui/index.html` CSP meta 与 engine `daemon/mod.rs` 的 CSP header，确认内联 `onerror` 是否允许；如不允许，采用纯 `<script async>` 方案并更新测试正则为：

```javascript
assert.match(html, /<script\s+src="[^"]*assets\/agent-test-harness\.js"\s+async><\/script>/);
```

- [ ] **Step 4: 跑测试确认通过**

```powershell
node --test webui/tests/agent-harness.test.mjs
```
Expected: PASS（10 tests）

- [ ] **Step 5: 手动 sanity check — 启动本地 engine + webui，浏览器访问 `?airp_agent_test=1`**

```powershell
# 终端 1：启动 engine（前台有头，按 AGENTS.md D 盘工具链）
$env:RUSTUP_HOME = "D:\.rustup"; $env:CARGO_HOME = "D:\.cargo"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
cargo run -p airp-core -- daemon --host 127.0.0.1 --port 8765 --webui-dir webui

# 终端 2：访问并验证
# 浏览器打开 http://127.0.0.1:8765/?airp_agent_test=1
# DevTools Console 应看到 "[AIRP] webui agent test harness v2 enabled"
# window.__AIRP_AGENT_TEST__ 应存在且 .version === 2
```

- [ ] **Step 6: Commit**

```powershell
git add webui/screens/*.html webui/tests/agent-harness.test.mjs
git commit -m "feat(webui): inject agent test harness into 6 key screens (#273)"
```

---

## Task 3: Agent 探索 Runner 核心（方案 A + 方案 B 接口预留）

**Files:**
- Create: `tools/agent-exploration/package.json`
- Create: `tools/agent-exploration/llm-client.mjs`
- Create: `tools/agent-exploration/harness-client.mjs`
- Create: `tools/agent-exploration/action-protocol.mjs`
- Create: `tools/agent-exploration/reporter.mjs`
- Create: `tools/agent-exploration/runner.mjs`

- [ ] **Step 1: 写 package.json**

写入 `tools/agent-exploration/package.json`：

```json
{
  "name": "airp-agent-exploration",
  "version": "0.1.0",
  "private": true,
  "type": "module",
  "description": "Agent-driven browser exploratory testing for AIRP WebUI (#273)",
  "scripts": {
    "test": "node --test classifier.test.mjs",
    "run": "node runner.mjs"
  },
  "dependencies": {
    "playwright-core": "^1.40.0"
  }
}
```

- [ ] **Step 2: 写 LLM client（OpenAI 兼容）**

写入 `tools/agent-exploration/llm-client.mjs`：

```javascript
// OpenAI 兼容 LLM 客户端。与 engine provider 解耦：runner 自带 client。
// 环境变量：OPENAI_BASE_URL, OPENAI_API_KEY, OPENAI_MODEL（默认 gpt-4o-mini）

const BASE_URL = process.env.OPENAI_BASE_URL || 'https://api.openai.com/v1';
const API_KEY = process.env.OPENAI_API_KEY;
const MODEL = process.env.OPENAI_MODEL || 'gpt-4o-mini';

if (!API_KEY) {
  console.error('[llm-client] OPENAI_API_KEY is required for agent exploration');
  process.exit(2);
}

export async function chatCompletion(messages, { maxTokens = 2048, temperature = 0.2, timeoutMs = 60000 } = {}) {
  // Bounded deadline: 防止 stalled provider 把 task/workflow 拖到 30 分钟超时
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  let res;
  try {
    res = await fetch(BASE_URL + '/chat/completions', {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'Authorization': 'Bearer ' + API_KEY,
      },
      body: JSON.stringify({ model: MODEL, messages, max_tokens: maxTokens, temperature }),
      signal: controller.signal,
    });
  } catch (err) {
    if (err && err.name === 'AbortError') {
      throw new Error('LLM request timed out after ' + timeoutMs + 'ms (provider stalled?)');
    }
    throw err;
  } finally {
    clearTimeout(timer);
  }
  if (!res.ok) {
    const text = await res.text();
    throw new Error(`LLM ${res.status}: ${text}`);
  }
  const json = await res.json();
  return json.choices?.[0]?.message?.content || '';
}

export function getModel() { return MODEL; }
```

- [ ] **Step 3: 写 harness client（Playwright 侧调用页面内 harness）**

写入 `tools/agent-exploration/harness-client.mjs`：

```javascript
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
```

- [ ] **Step 4: 写 action protocol（方案 B 预留接口）**

写入 `tools/agent-exploration/action-protocol.mjs`：

```javascript
// 方案 B 预留：Agent 每次只发一个 {action, target}，执行器返回 DOM 摘要 + 控制台错误 + 截图。
// MVP 不实现执行器，只定义协议契约，供后续 #273 阶段 3 接入。

export const ACTION_PROTOCOL_VERSION = 1;

export const SUPPORTED_ACTIONS = [
  'navigate',     // { action: 'navigate', target: '16-onboarding.html', params?: {} }
  'click',        // { action: 'click', target: '按钮文本' | '#selector' }
  'fill',         // { action: 'fill', target: '#message-input', value: '文本' }
  'wait',         // { action: 'wait', target: predicateId, timeoutMs?: 5000 }
  'snapshot',     // { action: 'snapshot' } → 返回 DOM 快照 + console + requests
  'screenshot',   // { action: 'screenshot' }
];

export function validateAction(action) {
  if (!action || typeof action !== 'object') return { ok: false, error: 'action must be object' };
  if (!SUPPORTED_ACTIONS.includes(action.action)) return { ok: false, error: 'unsupported action: ' + action.action };
  if (!action.target && action.action !== 'snapshot' && action.action !== 'screenshot') {
    return { ok: false, error: 'target required for ' + action.action };
  }
  return { ok: true };
}

// 预留执行器接口；MVP 抛 NotImplemented
export async function executeAction(harnessClient, action) {
  throw new Error('action-protocol executor not implemented in MVP (Plan A only); see #273 阶段 3');
}
```

- [ ] **Step 5: 写 reporter（Markdown + JSON）**

写入 `tools/agent-exploration/reporter.mjs`：

```javascript
import { writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';

export async function writeReport(reportDir, run) {
  await mkdir(reportDir, { recursive: true });

  // JSON 报告
  const jsonPath = join(reportDir, 'report.json');
  await writeFile(jsonPath, JSON.stringify(run, null, 2));

  // Markdown 报告
  const md = renderMarkdown(run);
  const mdPath = join(reportDir, 'report.md');
  await writeFile(mdPath, md);

  return { jsonPath, mdPath };
}

function renderMarkdown(run) {
  const lines = [];
  lines.push('# Agent Browser Exploration Report');
  lines.push('');
  lines.push('**Run ID:** ' + run.runId);
  lines.push('**Trigger:** ' + run.trigger);
  lines.push('**PR:** ' + (run.prNumber || 'N/A'));
  lines.push('**Started:** ' + run.startedAt);
  lines.push('**Duration:** ' + ((run.endedAt ? Date.parse(run.endedAt) : Date.now()) - Date.parse(run.startedAt)) + 'ms');
  lines.push('**LLM:** ' + run.llmModel);
  lines.push('');
  lines.push('## Summary');
  lines.push('- Total tasks: ' + run.tasks.length);
  lines.push('- Passed: ' + run.tasks.filter(t => t.result === 'Passed').length);
  lines.push('- Failed: ' + run.tasks.filter(t => t.result === 'Failed').length);
  lines.push('- Flaky: ' + run.tasks.filter(t => t.result === 'Flaky').length);
  lines.push('');
  for (const task of run.tasks) {
    lines.push('## Task: ' + task.name);
    lines.push('');
    lines.push('**Result:** ' + task.result);
    lines.push('');
    lines.push('### Description');
    lines.push(task.description);
    lines.push('');
    if (task.reproduction && task.reproduction.length) {
      lines.push('### Reproduction');
      for (let i = 0; i < task.reproduction.length; i++) lines.push((i + 1) + '. ' + task.reproduction[i]);
      lines.push('');
    }
    if (task.expected) { lines.push('### Expected'); lines.push(task.expected); lines.push(''); }
    if (task.actual) { lines.push('### Actual'); lines.push(task.actual); lines.push(''); }
    if (task.evidence) {
      lines.push('### Evidence');
      for (const [k, v] of Object.entries(task.evidence)) lines.push('- **' + k + ':** ' + v);
      lines.push('');
    }
    if (task.consoleErrors && task.consoleErrors.length) {
      lines.push('### Console Errors');
      for (const e of task.consoleErrors.slice(0, 10)) lines.push('- ' + JSON.stringify(e));
      lines.push('');
    }
    if (task.failedRequests && task.failedRequests.length) {
      lines.push('### Failed Requests');
      for (const r of task.failedRequests.slice(0, 10)) lines.push('- ' + JSON.stringify(r));
      lines.push('');
    }
    if (task.suspectedArea) { lines.push('### Suspected Area'); lines.push(task.suspectedArea); lines.push(''); }
    if (task.reproducibility) { lines.push('### Reproducibility'); lines.push(task.reproducibility); lines.push(''); }
  }
  return lines.join('\n');
}
```

- [ ] **Step 6: 写 runner 主入口**

写入 `tools/agent-exploration/runner.mjs`：

```javascript
// Agent 探索 runner 主入口
//
// 用法:
//   node runner.mjs --origin http://127.0.0.1:8765 --task onboarding-firstchat-refresh
//   node runner.mjs --pr 295 --report-dir artifacts/agent-exploration
//
// 环境变量:
//   OPENAI_BASE_URL, OPENAI_API_KEY, OPENAI_MODEL  — LLM
//   AIRP_CHROME_PATH                                — playwright-core Chrome
//   AIRP_AUTH_USER, AIRP_AUTH_PASSWORD              — production topology basic auth

import { chromium } from 'playwright-core';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { chatCompletion, getModel } from './llm-client.mjs';
import { HarnessClient } from './harness-client.mjs';
import { writeReport } from './reporter.mjs';
import { classifyPrDiff, DIFF_TASK_MAP } from './classifier.mjs';

const args = parseArgs(process.argv.slice(2));
const ORIGIN = args.origin || process.env.AIRP_SMOKE_ORIGIN || 'http://127.0.0.1:8765';
const CHROME = args['chrome-path'] || process.env.AIRP_CHROME_PATH;
const REPORT_DIR = args['report-dir'] || 'artifacts/agent-exploration';
const MAX_STEPS = Number(args['max-steps'] || 30);
const MAX_TOKENS = Number(args['max-tokens'] || 8000);
const MAX_REVISIONS = Number(args['max-revisions'] || 2);

if (!CHROME) {
  console.error('AIRP_CHROME_PATH or --chrome-path is required');
  process.exit(2);
}

// 任务集选择
let taskNames;
if (args.task) {
  taskNames = [args.task];
} else if (args.pr) {
  // 优先从 --diff-file 读 (workflow 用单独 step 取 diff, runner 不持有 GITHUB_TOKEN)
  let diff;
  if (args['diff-file']) {
    diff = await readFile(args['diff-file'], 'utf8');
  } else {
    diff = await fetchPrDiff(args.pr);
  }
  taskNames = classifyPrDiff(diff);
} else {
  // 默认跑全部 4 个任务集
  taskNames = Object.keys(DIFF_TASK_MAP);
  taskNames = [...new Set(taskNames)];
}

console.log('[runner] origin=' + ORIGIN);
console.log('[runner] tasks=' + JSON.stringify(taskNames));
console.log('[runner] llm=' + getModel());

const taskModules = {
  'onboarding-firstchat-refresh': './tasks/onboarding-firstchat-refresh.mjs',
  'regen-swipe-refresh': './tasks/regen-swipe-refresh.mjs',
  'edit-branch-switch-refresh': './tasks/edit-branch-switch-refresh.mjs',
  'memory-roundtrip': './tasks/memory-roundtrip.mjs',
};

const run = {
  runId: 'run-' + Date.now(),
  trigger: args.pr ? 'pr-' + args.pr : 'manual',
  prNumber: args.pr || null,
  startedAt: new Date().toISOString(),
  llmModel: getModel(),
  tasks: [],
};

const browser = await chromium.launch({ headless: true, executablePath: CHROME });
try {
  for (const name of taskNames) {
    const mod = await import(taskModules[name]);
    const taskResult = await runTask(browser, mod, name);
    run.tasks.push(taskResult);
  }
} finally {
  await browser.close();
}

run.endedAt = new Date().toISOString();
const { jsonPath, mdPath } = await writeReport(resolve(REPORT_DIR), run);
console.log('[runner] report: ' + mdPath);

// 阶段 2: 任何 task Failed 即 exit 1（让 workflow step 失败，触发 if: failure() 占位评论步骤）。
// workflow job 级 continue-on-error: true 仍然 non-blocking（不会阻塞 PR 合并），
// 但 exit 1 让 CI 红 + 触发 workflow 中的 failure 步骤，确保失败信号不会因
// PR 评论 step 自身失败（report 未生成 / gh 不可用）而完全消失。
if (run.tasks.some(t => t.result === 'Failed')) {
  const failed = run.tasks.filter(t => t.result === 'Failed');
  console.log('[runner] ' + failed.length + ' task(s) failed: ' + failed.map(t => t.name).join(', '));
  console.log('[runner] report: ' + mdPath);
  process.exit(1);
}

async function runTask(browser, mod, name) {
  const context = await browser.newContext({
    httpCredentials: process.env.AIRP_AUTH_USER ? { username: process.env.AIRP_AUTH_USER, password: process.env.AIRP_AUTH_PASSWORD } : undefined,
  });
  await context.tracing.start({ screenshots: true, snapshots: true, sources: true });

  const taskDir = join(resolve(REPORT_DIR), name);
  await mkdir(taskDir, { recursive: true });

  // B3 修复：result 提前初始化，保证 page.goto/waitForReady 失败时 catch/finally
  // 仍能访问 result，避免异常冒泡出 runTask 导致外层 for 循环整批跳过。
  const result = {
    name,
    description: mod.DESCRIPTION,
    result: 'Passed',
    reproduction: [],
    expected: mod.EXPECTED,
    actual: null,
    evidence: {},
    consoleErrors: [],
    failedRequests: [],
    suspectedArea: null,
    reproducibility: null,
  };

  let tracingStopped = false;
  let page = null;
  let harness = null;
  try {
    page = await context.newPage();
    // 传 origin 给 HarnessClient，让 navigate() 用 page.goto() 而不是 in-page href
    harness = new HarnessClient(page, ORIGIN);
    // 关键：page 创建后停留在 about:blank, harness 未安装。必须先 goto 一个会加载
    // harness 的 screen 并等待 async <script> 把 window.__AIRP_AGENT_TEST__ 装好,
    // 否则 generateAndRunScript() 里第一次 harness.getDomSnapshot() 会 evaluate 到 undefined。
    // 用 role-list 作为初始 screen (它是 home 页, 所有任务都可以从这里导航)。
    // B3: page.goto + waitForReady 移入 try 块——origin 不可达 / harness 未装好等
    // 失败不应冒泡到外层 for 循环导致剩余任务整批跳过。
    await page.goto(ORIGIN + '/screens/01-role-list.html?airp_agent_test=1', { waitUntil: 'load' });
    await harness.waitForReady();

    // 让 Agent 生成临时 Playwright 脚本（方案 A）
    const scriptPath = await generateAndRunScript(mod, page, harness, taskDir, context);
    result.evidence.script = scriptPath;

    // 收集 harness 状态
    result.consoleErrors = await harness.getConsoleErrors();
    result.failedRequests = await harness.getFailedRequests();

    // 截图
    const screenshotPath = join(taskDir, 'final.png');
    await harness.screenshot(screenshotPath);
    result.evidence.screenshot = screenshotPath;

    // Trace
    const tracePath = join(taskDir, 'trace.zip');
    await context.tracing.stop({ path: tracePath });
    result.evidence.trace = tracePath;
    tracingStopped = true;

    // 任务模块自检
    const checkResult = await mod.check(harness, result);
    if (!checkResult.ok) {
      result.result = 'Failed';
      result.actual = checkResult.actual;
      result.suspectedArea = checkResult.suspectedArea;
    }
  } catch (err) {
    result.result = 'Failed';
    result.actual = String(err && err.stack || err);
    if (harness) {
      try { result.consoleErrors = await harness.getConsoleErrors(); } catch {}
      try { result.failedRequests = await harness.getFailedRequests(); } catch {}
    }
    if (!tracingStopped) {
      try {
        const tracePath = join(taskDir, 'trace.zip');
        await context.tracing.stop({ path: tracePath });
        result.evidence.trace = tracePath;
        tracingStopped = true;
      } catch {}
    }
  } finally {
    // B3 修复：tracing 未停或停失败时，先强制 stop 再关 context，避免
    // context.close() 因 tracing 仍活跃而抛错跳过 finally 后续逻辑。
    // try/catch 包裹 stop 保证即使第二次 stop 也安全（Playwright 对已停的 tracing
    // 调 stop 会抛错，try/catch 吞掉即可）。
    if (!tracingStopped) {
      try { await context.tracing.stop(); } catch {}
    }
    try { await context.close(); } catch {}
  }

  return result;
}

async function generateAndRunScript(mod, page, harness, taskDir, context) {
  // 1. 构造 prompt（DOM 快照脱敏后再注入）
  const domSnapshot = await harness.getDomSnapshot().catch(() => []);
  const sanitized = sanitizeDomSnapshot(domSnapshot);
  const prompt = buildPrompt(mod, sanitized);

  let lastError = null;
  // ES module strict mode 要求显式声明；否则首次 lastScriptContent = scriptContent 抛 ReferenceError
  let lastScriptContent = '';
  for (let revision = 0; revision <= MAX_REVISIONS; revision++) {
    const messages = revision === 0
      ? [{ role: 'system', content: prompt.system }, { role: 'user', content: prompt.user }]
      : [
          { role: 'system', content: prompt.system },
          { role: 'user', content: prompt.user },
          { role: 'assistant', content: lastScriptContent },
          { role: 'user', content: 'Previous script failed with:\n' + lastError + '\n\nRevise and output a complete corrected script.' },
        ];

    const content = await chatCompletion(messages, { maxTokens: MAX_TOKENS, temperature: 0.2 });
    const scriptContent = extractCodeBlock(content);
    lastScriptContent = scriptContent;

    const scriptPath = join(taskDir, 'agent-script.mjs');
    await writeFile(scriptPath, scriptContent);

    // 2. 执行临时脚本
    try {
      const exitCode = await runTempScript(scriptPath, { page, harness, context, origin: ORIGIN });
      if (exitCode === 0) return scriptPath;
      lastError = 'script exit code: ' + exitCode;
    } catch (err) {
      lastError = String(err && err.stack || err);
    }
  }
  throw new Error('agent script failed after ' + (MAX_REVISIONS + 1) + ' revisions; last error:\n' + lastError);
}

// 脱敏 DOM 快照：message/memory/history 类元素的内容可能含用户数据，
// 不应原样发送给外部 LLM（OPENAI_BASE_URL 可指向外部服务，--origin 也可被操作者改到真实实例）
function sanitizeDomSnapshot(snapshot) {
  const messageLike = /message|msg|chat|memory|history|conversation|reply|content/i;
  return snapshot.map(el => {
    const scope = (el.id || '') + ' ' + (el.classes || []).join(' ') + ' ' + (el.role || '');
    if (el.text && messageLike.test(scope)) {
      return { ...el, text: '[REDACTED]' };
    }
    return el;
  });
}

function buildPrompt(mod, domSnapshot) {
  return {
    system: `You are an AIRP WebUI exploratory test generator. Output ONLY a single JavaScript code block (no prose) that exports an async function:
export async function run(ctx) { /* ctx = { page, harness, origin } */ }

Rules:
- Use only playwright-core page API and ctx.harness (window.__AIRP_AGENT_TEST__ wrapper).
- Each step must have a wait/poll, not a fixed sleep longer than 2s.
- On assertion failure, throw with a clear message starting with "ASSERT: ".
- Max ${MAX_STEPS} steps.
- Navigate to the task's first screen explicitly: await ctx.harness.navigate('screen.html', params).
- Do not call ctx.page.evaluate with closures over Node variables; pass primitive args only.
- Do not read or write files; the runner handles artifacts.`,
    user: `Task: ${mod.DESCRIPTION}

Task contract:
- Expected: ${mod.EXPECTED}
- Key API endpoints available (same-origin):
  - POST /v1/chat/completions (SSE) — send {character_id, session_id, message}
  - POST /v1/chat/history — {character_id, session_id, limit?}
  - POST /v1/chat/regen — {character_id, session_id?}
  - POST /v1/chat/swipe — {character_id, session_id?, message_id, index}
  - PUT  /v1/chat/message — {character_id, session_id?, message_id, content} (user msg only)
  - POST /v1/chat/branch/switch — {character_id, session_id?, target_leaf_id}
  - GET  /v1/memory/resident?character_id=...&session_id=...
  - PUT  /v1/memory/resident — {character_id, session_id?, user_id?, content}
  - GET  /v1/characters
  - POST /v1/characters/import — {character_id, card_json} or {character_id, card_path}
  - POST /v1/sessions/:character_id — create session

Initial DOM snapshot (truncated, current page may differ; call harness.navigate first):
${JSON.stringify(domSnapshot).slice(0, 4000)}

Output the script now. Only the code block, no explanation.`,
  };
}

function extractCodeBlock(content) {
  const m = content.match(/```(?:javascript|js)?\n([\s\S]*?)```/);
  return m ? m[1] : content;
}

async function runTempScript(scriptPath, ctx) {
  // LLM 生成的脚本是不可信代码。Prompt 里的"不要读写文件"不是安全边界：
  // 脚本能访问 process.env、fs、network、process.exit。
  //
  // MVP（方案 A）安全策略——多层防御，但承认非完美隔离：
  // 1. **Secret scrub**: 调用前清空 process.env 中匹配 SECRET_PATTERNS 的 key,
  //    避免 OPENAI_API_KEY 等被生成的脚本 exfiltrate。finally 恢复。
  // 2. **process.exit override**: 临时把 process.exit 替换为 throw, 防止脚本
  //    偷偷 exit(0) 中断 runner 后清场。finally 恢复。
  // 3. **GITHUB_TOKEN 不进 runner env**: workflow 用单独 step 取 PR diff 写到
  //    文件, runner 从 --diff-file 读, 不直接持有 repo write token。runner env
  //    只剩 OPENAI_API_KEY (agent 自己的低价值 key)。
  //
  // 真正的文件系统/网络/进程隔离要等方案 B action-protocol 把执行迁到受限
  // child process / container (见 Task 3 Step 4)。方案 A 接受"脚本理论上仍能
  // fs.readFile 本机文件"的风险, 因为: (a) CI runner 是临时 VM; (b) 无持久
  // secret 在磁盘; (c) workflow 已拆分 GITHUB_TOKEN; (d) Plan B 是已规划的
  // 收敛路径。此风险接受点须在 issue #273 评论中显式记录。
  const SECRET_PATTERNS = [/OPENAI_API_KEY/i, /GITHUB_TOKEN/i, /API_KEY/i, /SECRET/i, /PASSWORD/i, /TOKEN/i, /_KEY$/i];
  const savedSecrets = {};
  for (const key of Object.keys(process.env)) {
    if (SECRET_PATTERNS.some(p => p.test(key))) {
      savedSecrets[key] = process.env[key];
      delete process.env[key];
    }
  }
  const savedExit = process.exit;
  process.exit = (code) => {
    throw new Error('agent script attempted process.exit(' + code + '); blocked by runner sandbox');
  };
  try {
    const mod = await import('file://' + scriptPath + '?t=' + Date.now());
    if (typeof mod.run !== 'function') throw new Error('agent script must export async function run(ctx)');
    await mod.run(ctx);
    return 0;
  } finally {
    process.exit = savedExit;
    for (const [key, value] of Object.entries(savedSecrets)) {
      process.env[key] = value;
    }
  }
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a.startsWith('--')) {
      const key = a.slice(2);
      const val = argv[i + 1] && !argv[i + 1].startsWith('--') ? argv[++i] : 'true';
      out[key] = val;
    }
  }
  return out;
}

async function fetchPrDiff(prNumber) {
  // 简单实现：调 GitHub API 取 diff
  const token = process.env.GITHUB_TOKEN;
  const res = await fetch('https://api.github.com/repos/GhostXia/AIRP/pulls/' + prNumber, {
    headers: {
      'Accept': 'application/vnd.github.v3.diff',
      'Authorization': token ? 'Bearer ' + token : undefined,
      'User-Agent': 'airp-agent-exploration',
    },
  });
  if (!res.ok) throw new Error('fetchPrDiff ' + res.status + ': ' + await res.text());
  return await res.text();
}
```

- [ ] **Step 7: Commit**

```powershell
git add tools/agent-exploration/package.json tools/agent-exploration/llm-client.mjs tools/agent-exploration/harness-client.mjs tools/agent-exploration/action-protocol.mjs tools/agent-exploration/reporter.mjs tools/agent-exploration/runner.mjs
git commit -m "feat(tools): add agent exploration runner core (#273)"
```

---

## Task 4: PR-diff 分类器（阶段 2 自动触发）

**Files:**
- Create: `tools/agent-exploration/classifier.mjs`
- Test: `tools/agent-exploration/classifier.test.mjs`

- [ ] **Step 1: 写分类器测试（先红）**

写入 `tools/agent-exploration/classifier.test.mjs`：

```javascript
import test from 'node:test';
import assert from 'node:assert/strict';
import { classifyPrDiff, DIFF_TASK_MAP } from './classifier.mjs';

test('Edit message PR maps to edit-branch task', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/engine/src/daemon/handlers/chat.rs\n+pub async fn edit_message';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('edit-branch-switch-refresh'));
});

test('Swipe PR maps to regen-swipe task', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/engine/src/daemon/handlers/chat.rs\n+async fn swipe_chat';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('regen-swipe-refresh'));
});

test('Memory PR maps to memory-roundtrip task', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/memory.rs b/engine/src/daemon/handlers/memory.rs\n+pub async fn update_resident_memory';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('memory-roundtrip'));
});

test('Onboarding PR maps to onboarding-firstchat-refresh task', () => {
  const diff = 'diff --git a/webui/assets/onboarding.js b/webui/assets/onboarding.js\n+onboardingSteps';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('onboarding-firstchat-refresh'));
});

test('Unrelated PR returns empty task set', () => {
  const diff = 'diff --git a/README.md b/README.md\n+documentation change';
  const tasks = classifyPrDiff(diff);
  assert.deepEqual(tasks, []);
});

test('Multi-area PR deduplicates tasks', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/...\n+swipe\n+edit_message';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.length >= 2);
  assert.equal(new Set(tasks).size, tasks.length);
});

// B1 regression: path-only (no keyword) must NOT trigger a task set.
// 改 chat.rs 但内容与 swipe/edit/memory 无关时，不应启动 LLM+Chrome 探索。
test('Path-only diff without matching keywords returns empty', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/engine/src/daemon/handlers/chat.rs\n+pub fn unrelated_refactor() {}';
  const tasks = classifyPrDiff(diff);
  assert.deepEqual(tasks, [], 'path-only hit must not trigger; got ' + JSON.stringify(tasks));
});

// B1 regression: keyword-only (no path) must NOT trigger a task set.
// 防止 README/docs 提到 swipe/onboarding 等关键字但未改对应代码时误触发。
test('Keyword-only diff without matching paths returns empty', () => {
  const diff = 'diff --git a/README.md b/README.md\n+Documentation about onboarding flow and swipe behavior';
  const tasks = classifyPrDiff(diff);
  assert.deepEqual(tasks, [], 'keyword-only hit must not trigger; got ' + JSON.stringify(tasks));
});

// B1 regression: 任意路径单行无关改动，覆盖所有任务集的 paths。
test('Any single path change without keywords returns empty', () => {
  const cases = [
    'diff --git a/webui/assets/onboarding.js b/webui/assets/onboarding.js\n+export const unrelated = 1;',
    'diff --git a/webui/screens/14-message-swipe.html b/webui/screens/14-message-swipe.html\n+<div>layout tweak</div>',
    'diff --git a/engine/src/chat_store.rs b/engine/src/chat_store.rs\n+fn internal_helper() {}',
    'diff --git a/engine/src/memory/store.rs b/engine/src/memory/store.rs\n+fn internal_helper() {}',
  ];
  for (const diff of cases) {
    const tasks = classifyPrDiff(diff);
    assert.deepEqual(tasks, [], 'path-only should not trigger for diff: ' + diff + '; got ' + JSON.stringify(tasks));
  }
});
```

- [ ] **Step 2: 跑测试确认失败**

```powershell
cd tools\agent-exploration
node --test classifier.test.mjs
```
Expected: FAIL（`classifier.mjs` 不存在）

- [ ] **Step 3: 实现分类器**

写入 `tools/agent-exploration/classifier.mjs`：

```javascript
// PR diff → 任务集映射
// 命中规则：文件路径模式 AND 内容关键字同时命中才触发任务集；只看 +/- 行。
// 单独路径命中（如 chat.rs 改一行无关代码）不触发，避免高频文件引发不可控成本。

export const DIFF_TASK_MAP = {
  'onboarding-firstchat-refresh': {
    paths: [/webui\/assets\/onboarding\.js/, /webui\/screens\/16-onboarding\.html/, /engine\/src\/daemon\/handlers\/onboarding/],
    keywords: [/onboarding/i, /first.?chat/i, /first_mes/i],
  },
  'regen-swipe-refresh': {
    paths: [/engine\/src\/daemon\/handlers\/chat\.rs/, /webui\/assets\/chat-space\.js/, /webui\/screens\/14-message-swipe\.html/],
    keywords: [/regen/i, /swipe/i, /smooth.?stream/i, /candidate/i],
  },
  'edit-branch-switch-refresh': {
    paths: [/engine\/src\/chat_store\.rs/, /engine\/src\/daemon\/handlers\/chat\.rs/, /webui\/screens\/19-branch-tree\.html/],
    // 注意：原 /\bedit\b.*message/i 的尾部 \b 不会匹配 "edit_message"（_ 是 word char，无边界），
    // 导致纯路径命中下 keyword 永远失效。改用 \bedit.*message/i 让 "edit_message"、"edit message"
    // 等都能匹配，同时保留起始 \b 避免误匹配 "credit_message" 等无关词。
    keywords: [/\bedit.*message/i, /branch/i, /switch_branch/i, /active_leaf/i, /rollback/i],
  },
  'memory-roundtrip': {
    paths: [/engine\/src\/daemon\/handlers\/memory\.rs/, /engine\/src\/memory/, /webui\/screens\/17-memory-state\.html/],
    keywords: [/resident.?memory/i, /user.?model/i, /memory.?extract/i],
  },
};

export function classifyPrDiff(diff) {
  if (!diff || typeof diff !== 'string') return [];
  // 提取 diff 中变更的文件路径
  const pathMatch = diff.match(/^diff --git a\/(\S+) b\/\S+$/gm) || [];
  const paths = pathMatch.map(l => l.replace(/^diff --git a\//, '').replace(/ b\/\S+$/, ''));

  // 提取 +/- 行内容
  const changedLines = diff.split('\n').filter(l => l.startsWith('+') || l.startsWith('-')).join('\n');

  const hits = new Set();
  for (const [taskName, rule] of Object.entries(DIFF_TASK_MAP)) {
    const pathHit = rule.paths.some(p => paths.some(pp => p.test(pp)));
    const keywordHit = rule.keywords.some(k => k.test(changedLines));
    // path AND keyword：两者必须同时命中。
    // 单独 path 命中（例如改 chat.rs 但内容与 swipe/edit/memory 无关）不触发，
    // 否则 engine 最高频文件每次改动都会启动 2+ 任务集的 LLM+Chrome 探索，CI 成本不可控。
    if (pathHit && keywordHit) hits.add(taskName);
  }
  return [...hits];
}
```

- [ ] **Step 4: 跑测试确认通过**

```powershell
cd tools\agent-exploration
node --test classifier.test.mjs
```
Expected: PASS（9 tests，含 3 个 B1 regression: path-only / keyword-only / 多路径不触发）

- [ ] **Step 5: Commit**

```powershell
git add tools/agent-exploration/classifier.mjs tools/agent-exploration/classifier.test.mjs
git commit -m "feat(tools): add PR-diff to task-set classifier (#273)"
```

---

## Task 5: 任务集 1 — onboarding + 首聊 + 刷新恢复

**Files:**
- Create: `tools/agent-exploration/tasks/onboarding-firstchat-refresh.mjs`
- Create: `tools/agent-exploration/fixtures/character-card.json`

**说明**：此任务是模板，后续 3 个任务遵循同一模块结构（`DESCRIPTION` / `EXPECTED` / `check` 导出 + Agent 生成脚本）。本任务给完整代码，后续任务给差异部分。

- [ ] **Step 1: 写合成角色卡 fixture**

写入 `tools/agent-exploration/fixtures/character-card.json`（合成数据，不复制任何第三方角色卡）：

```json
{
  "spec": "chara_card_v2",
  "spec_version": "2.0",
  "data": {
    "name": "AIRP-Test-Fixture-Aria",
    "description": "Synthetic roleplay test fixture for agent exploration. Aria is a calm librarian who answers questions about books.",
    "personality": "calm, precise, helpful",
    "scenario": "A visitor enters the library and asks about a book.",
    "first_mes": "Welcome to the library. How may I help you find a book today?",
    "mes_example": "",
    "creator_notes": "Synthetic fixture for AIRP agent browser exploration (#273). Not derived from any third-party character.",
    "system_prompt": "You are Aria, a calm librarian. Keep replies under 60 words.",
    "post_history_instructions": "",
    "alternate_greetings": [],
    "character_book": null,
    "tags": ["agent-exploration", "synthetic"],
    "creator": "airp-agent-exploration",
    "character_version": "1",
    "extensions": {}
  }
}
```

- [ ] **Step 2: 写任务模块**

写入 `tools/agent-exploration/tasks/onboarding-firstchat-refresh.mjs`：

```javascript
// 任务集 1: onboarding + 首聊 + 刷新恢复
// 参考 issue #273 MVP 必备项 1

import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const __dirname = dirname(fileURLToPath(import.meta.url));
const cardPath = join(__dirname, '..', 'fixtures', 'character-card.json');

export const DESCRIPTION = `从空数据目录完成 onboarding 流程；导入合成角色卡 fixture；选择模型并完成首次聊天；
发送一轮用户消息并等待 assistant 流式回复完成；刷新页面后确认聊天历史一致。

步骤提示：
1. await ctx.harness.navigate('16-onboarding.html') — onboarding 入口（空数据目录会自动跳到这）
2. 完成每一步的"下一步"按钮点击（6 步），跳过/选择角色/配置 provider 都按默认走
3. 在"给角色的第一句话"输入框填 'Hello Aria, what books do you recommend?'
4. 点击"发送首轮消息"，等待"进入对话空间"按钮出现
5. 进入 02-chat-space.html，再发一条用户消息 'Tell me about sci-fi books.'
6. 等待 assistant 流式回复完成（send-button 不再含 stop class）
7. 调用 ctx.harness.getApiSnapshot('/v1/chat/history', 'POST', {character_id, session_id, limit: 50})
   记录 messages 数组
8. 刷新页面：await ctx.page.reload()
9. 再次调用 getApiSnapshot 取 history，对比 messages 完全一致
10. ASSERT: 两次 history 的 message_ids 顺序与内容完全相等`;

export const EXPECTED = `刷新页面后，/v1/chat/history 返回的 message_ids 顺序与 messages 内容与刷新前完全一致；
页面 DOM 上显示的对话条数也与 API 返回一致。`;

export async function check(harness, result) {
  // 二次校验：刷新前后 history 一致性由 Agent 脚本断言；此处只做兜底
  const consoleErrors = result.consoleErrors || [];
  const severeErrors = consoleErrors.filter(e => !/Deprecation|harvest|analytics/i.test(e.message || ''));
  if (severeErrors.length > 0) {
    return {
      ok: false,
      actual: severeErrors.length + ' severe console errors after refresh: ' + JSON.stringify(severeErrors.slice(0, 3)),
      suspectedArea: 'onboarding or chat-space runtime; check network/console evidence',
    };
  }
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx failed requests: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine 5xx during onboarding/firstchat; check engine logs',
    };
  }
  return { ok: true };
}

// 通过 ctx.fixtures 把解析好的角色卡 JSON 传给 Agent 脚本。
// 不要传 runner-local 路径：engine server 和 Agent 脚本都不应读 runner 文件系统。
// runner 在 runTask 里读取该 fixture JSON 并放入 ctx.fixtures.characterCard。
export const FIXTURES = { characterCard: JSON.parse(await readFile(cardPath, 'utf8')) };
```

注意：`runner.mjs` 需要把 `mod.FIXTURES` 注入到 `ctx`（不是 prompt），让 Agent 脚本通过 `ctx.fixtures.characterCard` 直接拿 JSON 并 POST 到 `/v1/characters/import` 的 `card_json` 字段。`buildPrompt` 的 prompt 只需告诉 Agent："fixture JSON 在 `ctx.fixtures.characterCard`，调 `/v1/characters/import` 时用 `card_json` 字段提交，不要读文件"。

在 `runner.mjs` 的 `runTask` 中构造 ctx 时合并 fixtures：

```javascript
const ctx = { page, harness, context, origin: ORIGIN, fixtures: mod.FIXTURES || {} };
```

并把 `generateAndRunScript` 内 `runTempScript(scriptPath, ctx)` 保持一致。`buildPrompt` 的 system prompt 已有"不要读写文件"规则，只需在 user prompt 末尾补一句：

```javascript
const fixtureNote = mod.FIXTURES
  ? '\n\nFixtures: ctx.fixtures.characterCard is the parsed character card JSON. Use it directly in the POST /v1/characters/import body as { character_id, card_json }. Do NOT read files.'
  : '';
```

- [ ] **Step 3: 手动跑一次验证**

```powershell
# 前置：engine 在 127.0.0.1:8765 跑，已配置 mock provider（或本地 provider）
# Chrome 路径设到 AIRP_CHROME_PATH
$env:AIRP_CHROME_PATH = "C:\Program Files\Google\Chrome\Application\chrome.exe"
$env:OPENAI_API_KEY = "sk-..."
$env:OPENAI_MODEL = "gpt-4o-mini"

cd tools\agent-exploration
npm install
node runner.mjs --origin http://127.0.0.1:8765 --task onboarding-firstchat-refresh --report-dir ../../artifacts/agent-exploration
```

Expected：`artifacts/agent-exploration/onboarding-firstchat-refresh/` 下有 `agent-script.mjs`、`final.png`、`trace.zip`；`report.md` 中该任务 result 为 Passed 或 Failed（首次跑允许 Failed，看 evidence 调 prompt）。

- [ ] **Step 4: Commit**

```powershell
git add tools/agent-exploration/tasks/onboarding-firstchat-refresh.mjs tools/agent-exploration/fixtures/ tools/agent-exploration/runner.mjs
git commit -m "feat(tools): add onboarding+firstchat+refresh exploration task (#273)"
```

---

## Task 6: 任务集 2 — Regen + Swipe + 刷新恢复

**Files:**
- Create: `tools/agent-exploration/tasks/regen-swipe-refresh.mjs`

**说明**：模块结构与 Task 5 一致；本任务只给 `DESCRIPTION` / `EXPECTED` / `check` 差异部分，模块外壳（`import` / `export`）复制 Task 5。

- [ ] **Step 1: 写任务模块**

写入 `tools/agent-exploration/tasks/regen-swipe-refresh.mjs`：

```javascript
// 任务集 2: Regen + Swipe + 刷新恢复
// 参考 issue #273 MVP 必备项 2

export const DESCRIPTION = `前置：已完成 onboarding 并有至少一轮 assistant 回复（可直接调 /v1/characters/import
和 /v1/chat/completions API 准备数据，不必走 onboarding UI）。

步骤提示：
1. 导航到 02-chat-space.html?character=<id>&session=<id>
2. 记录当前最后一条 assistant 消息的 message_id 和 content
3. 调 /v1/chat/regen 重新生成最后一条 assistant 消息，等待流式完成
4. 调 /v1/chat/history 确认 assistant 消息已被替换（message_id 不变或新生成；
   按 engine 当前契约：regen 替换最后一条，durable ID 行为以 engine 实现为准）
5. 对该 assistant 消息调 /v1/chat/swipe 至少 3 次，每次 index 0/1/2
   等待每次切换后 history 反映新候选
6. 切换到候选 1，发一条新用户消息 'Continue the story.'
7. 等待 assistant 流式回复完成
8. 刷新页面：await ctx.page.reload()
9. 再次调 /v1/chat/history
10. ASSERT: 当前激活候选与刷新前一致（通过 history 的 active candidate 字段或 message 内容比对）
11. ASSERT: 后续对话上下文连续（用户消息 'Continue the story.' 后跟 assistant 回复）`;

export const EXPECTED = `Swipe 切换后，刷新页面应保持当前激活候选；
后续对话上下文基于当前激活候选继续，不串扰其他候选内容。`;

export async function check(harness, result) {
  // 兜底：检查是否有 5xx 或严重 console 错误
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx during swipe/regen: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine /v1/chat/swipe or /v1/chat/regen handler',
    };
  }
  // 检查是否有 unhandledrejection
  const unhandled = (result.consoleErrors || []).filter(e => e.type === 'unhandledrejection');
  if (unhandled.length > 0) {
    return {
      ok: false,
      actual: unhandled.length + ' unhandled promise rejections: ' + JSON.stringify(unhandled.slice(0, 3)),
      suspectedArea: 'webui swipe runtime; check chat-space.js candidate switching',
    };
  }
  return { ok: true };
}
```

- [ ] **Step 2: 跑一次验证**

```powershell
cd tools\agent-exploration
node runner.mjs --origin http://127.0.0.1:8765 --task regen-swipe-refresh --report-dir ../../artifacts/agent-exploration
```

- [ ] **Step 3: Commit**

```powershell
git add tools/agent-exploration/tasks/regen-swipe-refresh.mjs
git commit -m "feat(tools): add regen+swipe+refresh exploration task (#273)"
```

---

## Task 7: 任务集 3 — Edit + Branch + 切换 + 刷新恢复

**Files:**
- Create: `tools/agent-exploration/tasks/edit-branch-switch-refresh.mjs`

- [ ] **Step 1: 写任务模块**

写入 `tools/agent-exploration/tasks/edit-branch-switch-refresh.mjs`：

```javascript
// 任务集 3: Edit + Branch + 切换 + 刷新恢复
// 参考 issue #273 MVP 必备项 3

export const DESCRIPTION = `前置：用 API 准备一个角色 + 一个有至少 2 轮对话的 session。

步骤提示：
1. 导航到 02-chat-space.html?character=<id>&session=<id>
2. 记录当前 history 的所有 message_ids 和 active_leaf（通过 /v1/chat/history）
3. 找到第一条 role=user 消息的 message_id
4. 调 PUT /v1/chat/message 编辑该用户消息 content 为 'I changed my question: what is the library policy on late returns?'
5. 由于编辑历史用户消息会触发分支语义（按 engine 当前实现：编辑 user 消息可能创建分支或原地替换；
   以 engine 实现为准），调 /v1/chat/history 确认当前 active path 状态
6. 如果 engine 支持分支（chat_store 有 branch_tree）：调 /v1/chat/branch/switch
   切换到原 active_leaf（编辑前的 leaf）
7. 在原分支继续发一条用户消息 'Thanks.', 等待 assistant 回复
8. 切换回新分支（编辑后的 leaf），发一条用户消息 'And the fines?', 等待 assistant 回复
9. 多次切换两个分支（至少 3 次来回），每次确认 history 只显示当前 active path
10. 刷新页面：await ctx.page.reload()
11. 再次调 /v1/chat/history
12. ASSERT: 当前 active_leaf 与刷新前一致
13. ASSERT: 当前 active path 的消息序列与刷新前一致
14. ASSERT: 另一分支的数据未被删除（切回另一分支验证其消息序列仍在）`;

export const EXPECTED = `编辑历史用户消息后建立的分支，与原分支共存；
多次切换后，刷新页面应保持当前 active path；
另一分支数据未被污染或删除。`;

export async function check(harness, result) {
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx during branch ops: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine chat_store branch_tree or /v1/chat/branch/switch handler',
    };
  }
  const errors = (result.consoleErrors || []).filter(e => e.type === 'error' && !/favicon|networkerror/i.test(e.message || ''));
  if (errors.length > 5) {
    return {
      ok: false,
      actual: errors.length + ' console errors during branch switching: ' + JSON.stringify(errors.slice(0, 3)),
      suspectedArea: 'webui branch-tree rendering or chat-space active path rendering',
    };
  }
  return { ok: true };
}
```

- [ ] **Step 2: 跑一次验证**

```powershell
cd tools\agent-exploration
node runner.mjs --origin http://127.0.0.1:8765 --task edit-branch-switch-refresh --report-dir ../../artifacts/agent-exploration
```

- [ ] **Step 3: Commit**

```powershell
git add tools/agent-exploration/tasks/edit-branch-switch-refresh.mjs
git commit -m "feat(tools): add edit+branch+switch+refresh exploration task (#273)"
```

---

## Task 8: 任务集 4 — Memory 任务

**Files:**
- Create: `tools/agent-exploration/tasks/memory-roundtrip.mjs`

- [ ] **Step 1: 写任务模块**

写入 `tools/agent-exploration/tasks/memory-roundtrip.mjs`：

```javascript
// 任务集 4: Memory 任务
// 参考 issue #273 任务集 4（用户选择包含在 MVP 中）

export const DESCRIPTION = `前置：用 API 准备角色 + session，并完成至少 1 轮对话。

步骤提示：
1. 导航到 02-chat-space.html?character=<id>&session=<id>
2. 发一条用户消息包含明确事实: 'My name is Agent-Tester and I live in Taipei.'
3. 等待 assistant 回复完成
4. 等待 memory 抽取（如果 engine 是异步抽取，轮询 /v1/memory/resident 至少 10 秒）
5. 导航到 17-memory-state.html?character=<id>&session=<id>
6. ASSERT: resident memory 中能找到 'Agent-Tester' 或 'Taipei' 相关条目
7. 手动编辑 resident memory: 调 PUT /v1/memory/resident 添加一条 'User prefers concise answers.'
8. 导航回 02-chat-space.html，发一条用户消息 'What do you know about me?'
9. 等待 assistant 回复
10. ASSERT: assistant 回复中应体现新写入的记忆（'concise' 或相关词）
   注: mock provider 可能不真实反映；如果用 mock provider, ASSERT 改为:
   调 /v1/chat/preview 确认 prompt 装配摘要中包含 'User prefers concise answers.'
11. 刷新 17-memory-state.html
12. ASSERT: resident memory 仍包含手动添加的条目
13. 终止 engine (如果 runner 支持) → 重启 engine → 重新访问 17-memory-state.html
14. ASSERT: resident memory 持久化（仍包含手动条目）

注: 步骤 13-14 的 engine 重启由 runner 控制；如果 runner 不支持, 改为只测刷新恢复。`;

export const EXPECTED = `Memory 抽取能捕获对话中的事实；
手动修改 resident memory 后，后续对话的 prompt 装配包含新记忆；
刷新和（如可测）重启 engine 后，resident memory 持久化。`;

export async function check(harness, result) {
  const failed = (result.failedRequests || []).filter(r => r.status && r.status >= 500);
  if (failed.length > 0) {
    return {
      ok: false,
      actual: failed.length + ' 5xx during memory ops: ' + JSON.stringify(failed.slice(0, 3)),
      suspectedArea: 'engine /v1/memory/resident handler or memory extraction pipeline',
    };
  }
  // PUT 4xx 是合理的（body 超限等），不算 task 失败
  return { ok: true };
}
```

- [ ] **Step 2: 跑一次验证**

```powershell
cd tools\agent-exploration
node runner.mjs --origin http://127.0.0.1:8765 --task memory-roundtrip --report-dir ../../artifacts/agent-exploration
```

- [ ] **Step 3: Commit**

```powershell
git add tools/agent-exploration/tasks/memory-roundtrip.mjs
git commit -m "feat(tools): add memory roundtrip exploration task (#273)"
```

---

## Task 9: CI Workflow（阶段 2：重大功能 PR 自动运行，非阻塞）

**Files:**
- Create: `.github/workflows/agent-browser-exploration.yml`

**说明**：用户选择阶段 2（重大功能 PR 自动运行）。workflow 在 PR 打开/更新时触发，跑分类器选择任务，失败不阻塞合并（`continue-on-error`），只在 PR 发评论。

- [ ] **Step 1: 写 workflow**

写入 `.github/workflows/agent-browser-exploration.yml`：

```yaml
name: Agent Browser Exploration

# 阶段 2: 重大功能 PR 自动运行。Non-blocking (continue-on-error at job level via if + outputs).
# 触发条件: PR opened/synchronized/reopened, 且 diff 命中 Edit/Branch/Swipe/Memory/onboarding 任一区域。
on:
  pull_request:
    types: [opened, synchronize, reopened]
    paths:
      - 'engine/src/daemon/handlers/chat.rs'
      - 'engine/src/daemon/handlers/memory.rs'
      - 'engine/src/chat_store.rs'
      - 'engine/src/memory/**'
      - 'webui/assets/onboarding.js'
      - 'webui/assets/chat-space.js'
      - 'webui/screens/16-onboarding.html'
      - 'webui/screens/02-chat-space.html'
      - 'webui/screens/14-message-swipe.html'
      - 'webui/screens/17-memory-state.html'
      - 'webui/screens/19-branch-tree.html'

permissions:
  contents: read
  pull-requests: write

jobs:
  explore:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    continue-on-error: true  # MVP: non-blocking
    steps:
      - name: Checkout
        uses: actions/checkout@v4
        with:
          fetch-depth: 0

      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: '20'

      - name: Install Chrome
        id: setup-chrome
        uses: browser-actions/setup-chrome@v1
        with:
          chrome-version: stable

      - name: Install runner deps
        working-directory: tools/agent-exploration
        run: npm install

      - name: Bootstrap production smoke topology
        working-directory: deploy/production
        run: |
          # 用 bootstrap-topology.sh 只启 mock provider + TLS + 临时数据拓扑,
          # 不跑 smoke-ci.sh 自身（避免重复跑现有 smoke）
          # 脚本会启 docker compose 后台拓扑, 等待 health ready 后返回, 不阻塞
          ./bootstrap-topology.sh
          echo "TOPOLOGY_BOOTSTRAPPED=1" >> $GITHUB_ENV

      - name: Fetch PR diff
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        run: |
          # 单独 step 取 PR diff 到文件, runner 不需要 GITHUB_TOKEN (减少 secret 暴露面)
          mkdir -p artifacts/agent-exploration
          gh api repos/GhostXia/AIRP/pulls/${{ github.event.pull_request.number }} \
            -H "Accept: application/vnd.github.v3.diff" \
            > artifacts/agent-exploration/pr-diff.patch

      - name: Run agent exploration
        env:
          AIRP_SMOKE_ORIGIN: https://localhost:9443
          AIRP_SMOKE_ADMIN_USER: airp-smoke
          AIRP_SMOKE_ADMIN_PASSWORD: synthetic-smoke-password
          AIRP_CHROME_PATH: ${{ steps.setup-chrome.outputs.chrome-path }}
          AIRP_AUTH_USER: airp-smoke
          AIRP_AUTH_PASSWORD: synthetic-smoke-password
          OPENAI_API_KEY: ${{ secrets.AGENT_EXPLORATION_OPENAI_KEY }}
          OPENAI_MODEL: ${{ secrets.AGENT_EXPLORATION_MODEL || 'gpt-4o-mini' }}
          # 故意不传 GITHUB_TOKEN: runner 不需要它 (diff 已在上一步取到文件),
          # 避免生成的临时脚本 exfiltrate repo write token。PR 评论用单独 step 的 GH_TOKEN。
        working-directory: tools/agent-exploration
        run: |
          node runner.mjs \
            --origin $AIRP_SMOKE_ORIGIN \
            --pr ${{ github.event.pull_request.number }} \
            --diff-file ../../artifacts/agent-exploration/pr-diff.patch \
            --report-dir ../../artifacts/agent-exploration \
            --chrome-path $AIRP_CHROME_PATH

      - name: Teardown topology
        if: always()
        working-directory: deploy/production
        run: |
          # 用同一脚本 teardown, 保证 mock provider + 临时数据卷在 job 结束前清理
          ./bootstrap-topology.sh --teardown

      - name: Upload artifacts
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: agent-exploration-${{ github.event.pull_request.number }}
          path: artifacts/agent-exploration/
          retention-days: 14

      # B2 修复：runner 失败时 exit 1（虽然 job continue-on-error: true 不阻塞合并，
      # 但 step failure 会触发本步骤），先发占位评论确保失败信号不会因下游
      # Publish PR comment 自身失败（report.md 未生成 / gh 不可用）而完全丢失。
      - name: Publish failure placeholder comment
        if: failure()
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          PR_NUMBER: ${{ github.event.pull_request.number }}
        run: |
          # 用 --body-file - 通过 stdin 传 body，避免引号转义 / 长度限制
          gh pr comment "$PR_NUMBER" --body-file - <<'EOF'
          ## ⚠️ Agent exploration runner failed

          The exploration runner exited with a non-zero status. The actual report comment may be missing or partial.

          - See the [workflow run](${{ github.server_url }}/${{ github.repository }}/actions/runs/${{ github.run_id }}) for logs.
          - Download the `agent-exploration-${{ github.event.pull_request.number }}` artifact for any partial report/trace files.

          This is a non-blocking placeholder (workflow has `continue-on-error: true`); merge is not blocked. Investigate the failure if it persists.
          EOF

      - name: Publish PR comment
        if: always()
        env:
          GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          PR_NUMBER: ${{ github.event.pull_request.number }}
        run: |
          node tools/agent-exploration/post-pr-comment.mjs artifacts/agent-exploration/report.md
```

注意：
1. `bootstrap-topology.sh` 必须在 Task 9 Step 1 实施前从 `deploy/production/smoke-ci.sh` 抽取并落地为独立脚本：仅启 mock provider + TLS + 临时数据拓扑、等待 health ready 后返回（不阻塞、不跑 smoke），并支持 `--teardown` 子命令做幂等清理。本计划不展开其代码（避免文档过长），但 workflow 中已直接调用该脚本，缺失会导致 Step 1 失败。**B4 修复**：脚本必须以 git 可执行权限（100755）提交，否则 workflow `./bootstrap-topology.sh` 会因 Permission denied (exit 126) 失败。命令：`git update-index --chmod=+x deploy/production/bootstrap-topology.sh`。
2. `post-pr-comment.mjs` 需在 Task 9 Step 2 创建。**B5 修复**：脚本必须先用 `fs.access` 检查 report 是否存在，不存在时优雅退出（exit 0 + 警告），不要让 `readFile` 抛 ENOENT 导致 step 失败——topology bootstrap 失败时 runner 不跑，report 不会生成，failure() placeholder step 已发占位评论。

- [ ] **Step 2: 写 PR 评论发布脚本**

写入 `tools/agent-exploration/post-pr-comment.mjs`：

```javascript
// 读 report.md, 截断到 GitHub 评论长度上限 (65536), 通过 gh CLI 发评论
// 用 --body-file 模式（项目 memory: PowerShell/gh 多行特殊字符问题）

import { readFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';

const reportPath = process.argv[2];
if (!reportPath) {
  console.error('usage: post-pr-comment.mjs <report.md>');
  process.exit(2);
}

const prNumber = process.env.PR_NUMBER;
const token = process.env.GH_TOKEN;
if (!prNumber || !token) {
  console.error('PR_NUMBER and GH_TOKEN env vars required');
  process.exit(2);
}

let body = await readFile(reportPath, 'utf8');
const MAX = 65000;  // 留余量
if (body.length > MAX) {
  body = body.slice(0, MAX) + '\n\n... (report truncated; see artifacts for full report)';
}

// 用 gh pr comment --body-file 避免 PowerShell 特殊字符问题
const result = spawnSync('gh', ['pr', 'comment', prNumber, '--body-file', '-'], {
  input: body,
  env: process.env,
  encoding: 'utf8',
});

if (result.status !== 0) {
  console.error('gh pr comment failed:', result.stderr);
  process.exit(result.status || 1);
}
console.log('PR comment posted:', result.stdout.trim());
```

- [ ] **Step 3: 在 PR 评论模板里加 non-blocking 声明**

在 `reporter.mjs` 的 `renderMarkdown` 头部加一行：

```javascript
lines.push('> ⚠️ **Non-blocking** (阶段 2 MVP): 此报告不阻塞 PR 合并。崩溃/数据损坏/安全问题需人工确认；可用性问题仅记录。');
lines.push('');
```

- [ ] **Step 4: Commit**

```powershell
git add .github/workflows/agent-browser-exploration.yml tools/agent-exploration/post-pr-comment.mjs tools/agent-exploration/reporter.mjs
git commit -m "ci: add agent browser exploration workflow (#273 stage 2)"
```

---

## Task 10: 在 issue #273 评论中记录推迟项 + 验收

**Files:**
- 无本地文件改动；通过 GitHub issue comment 提交

**说明**：用户决策"桌面 ui 的 harness 和桌面 ui 一起开发"，必须在 issue #273 显式记录此推迟决定，避免未来 agent 误以为桌面 harness 是本 MVP 范围。

- [ ] **Step 1: 准备 issue 评论内容**

写入临时文件 `issue-273-status.md`（用完即删）：

```markdown
## Agent 浏览器探索测试层 — 实施计划已就位

**计划文件**：`docs/AGENT-BROWSER-EXPLORATION-PLAN.md`

### MVP 范围（webui 优先）

- ✅ webui 侧 Agent Test Harness v2（`webui/assets/agent-test-harness.js`）
- ✅ Runner（方案 A：Agent 生成临时 Playwright 脚本；方案 B action protocol 接口预留）
- ✅ 4 个任务集：onboarding+首聊+刷新 / Regen+Swipe+刷新 / Edit+Branch+切换+刷新 / Memory
- ✅ PR-diff 分类器（阶段 2 自动触发）
- ✅ Artifacts + Markdown/JSON 报告
- ✅ CI workflow（非阻塞，阶段 2）

### 显式推迟项

**桌面 `ui/` (Tauri/Vue) harness 升级不在本 MVP 范围**：现有 `ui/src/agent-test.ts`（v1, Blueprint/intent 模型）保持不动。桌面 harness 与桌面 UI 路线一起开发，按 `ui/README.md` 当前说明（"桌面开发、打包验收和性能计划当前暂停"），等桌面路线恢复后再升级到 v2 接口。

### 验收标准对齐 issue #273

- ✅ 可以在隔离数据目录中启动本地 WebUI（复用 production smoke 拓扑）
- ✅ Agent 能根据用户级任务生成浏览器操作（方案 A 生成临时 Playwright 脚本）
- ✅ 至少覆盖 onboarding、聊天和三个组合功能任务（4 个任务集）
- ✅ 每次运行有最大步骤数（默认 30）和总超时（workflow 30 分钟）
- ✅ 失败时保存截图和 Playwright Trace
- ✅ 输出实际操作步骤、预期结果和实际结果（report.md）
- ✅ 不读取真实用户数据或真实 API key（合成 fixture + 专用 LLM key）
- ✅ Agent 失败不会影响现有确定性 Smoke（独立 workflow, continue-on-error）
- ⏳ Agent 发现的问题可以被转换为固定 Playwright 回归测试（流程闭环, 不在本 MVP 自动化）
- ✅ 初期不作为阻塞式 PR 门禁（continue-on-error + PR 评论提示）

### 非目标（再次明确）

- 不替代真实用户验证
- 不评价 RP 内容质量
- 不在真实用户数据上操作

实施按计划文件 Task 1-10 顺序执行；每个 task 完成后 commit。
```

- [ ] **Step 2: 用 gh CLI 发评论（--body-file 避免 PowerShell 字符问题）**

```powershell
gh issue comment 273 --body-file issue-273-status.md
Remove-Item issue-273-status.md
```

- [ ] **Step 3: Commit 计划文件本身**

```powershell
git add docs/AGENT-BROWSER-EXPLORATION-PLAN.md
git commit -m "docs: add agent browser exploration implementation plan (#273)"
```

---

## 自检结果

### 1. Spec 覆盖度

| issue #273 要求 | 对应 Task |
|---|---|
| 第一层：保留现有 Playwright Smoke | 不动（非本计划范围） |
| 第二层：Agent 浏览器探索测试 | Task 1-3（harness + runner） |
| 任务集 1: 基础聊天 | Task 5 |
| 任务集 2: Swipe | Task 6 |
| 任务集 3: Edit/Branch | Task 7 |
| 任务集 4: Memory | Task 8 |
| 方案 A: 生成临时 Playwright 脚本 | Task 3 runner 核心 |
| 方案 B: 逐步动作 | Task 3 Step 4 action-protocol.mjs（接口预留） |
| 测试环境边界（隔离、Mock Provider、临时数据） | Task 9 workflow 复用 smoke-ci.sh 拓扑 |
| 测试报告格式（Markdown + 证据） | Task 3 Step 5 reporter.mjs |
| 门禁阶段 2: 重大功能 PR 自动运行 | Task 9 workflow（continue-on-error） |
| 验收标准（10 条） | Task 10 Step 1 逐条对齐 |
| 非目标 | 计划头部"范围与非目标"显式声明 |
| MVP 建议 | Task 1-8 是 MVP, Task 9-10 是阶段 2 触发 + 验收 |

**缺口**：
- issue #273 提到"PR 感知的任务选择"中"Portable ZIP" / "NPC / Plot" / "Persistence" 任务未在本 MVP 覆盖。这些是后续扩展项，不在 issue #273 MVP 必备项里，本计划不实现。
- issue #273 阶段 3"稳定场景转为确定性门禁"是后续阶段，不在本 MVP。

### 2. 占位符扫描

- 无 "TBD" / "TODO" / "fill in details" / "add error handling" / "similar to Task N"（除 Task 6/7/8 显式说"模块结构与 Task 5 一致"并给出差异代码，符合 DRY 原则，不算占位符）
- 所有代码步骤都给了完整可执行代码
- 唯一明确"不在本计划展开代码"的是 Task 9 Step 1 的 `bootstrap-topology.sh` 抽取——这是合理的范围切分，且明确标注了"实施时必须先做这步"

### 3. 类型一致性

- `AgentTestHarness` v2 接口在 Task 1 定义，Task 3 `HarnessClient` 调用所有方法名一致：`navigate` / `getCurrentScreen` / `fillInput` / `clickButton` / `getVisibleText` / `getDomSnapshot` / `getConsoleErrors` / `getFailedRequests` / `getApiSnapshot` / `waitFor` / `screenshot`
- `run` 对象的 `tasks[].result` 取值统一：`'Passed' | 'Failed' | 'Flaky'`
- 任务模块统一导出 `DESCRIPTION` / `EXPECTED` / `check`，Task 5 额外导出 `FIXTURES`（Task 6/7/8 不需要 fixture，但 `runner.mjs` 的 `mod.FIXTURES` 检查是 optional chaining，无类型不一致）
- `buildPrompt(mod, domSnapshot)` 在 Task 5 修改签名后，所有任务调用一致

---

## Execution Handoff

计划完成并保存至 `docs/AGENT-BROWSER-EXPLORATION-PLAN.md`。两种执行方式：

1. **Subagent-Driven（推荐）** — 每个 Task 派一个 fresh subagent 执行，task 间 review，迭代快
2. **Inline Execution** — 在当前会话按 executing-plans 批量执行，带 checkpoint

请选择执行方式，或先审阅计划再决定。
