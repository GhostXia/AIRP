import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readdir } from 'node:fs/promises';
import vm from 'node:vm';
import { sanitizeDomSnapshot } from '../../tools/agent-exploration/dom-privacy.mjs';

const rolePage = await readFile(new URL('../screens/01-role-list.html', import.meta.url), 'utf8');
const chatPage = await readFile(new URL('../screens/02-chat-space.html', import.meta.url), 'utf8');
const onboardingPage = await readFile(new URL('../screens/16-onboarding.html', import.meta.url), 'utf8');
const entryPage = await readFile(new URL('../index.html', import.meta.url), 'utf8');
const entryScript = await readFile(new URL('../assets/entry.js', import.meta.url), 'utf8');
const onboardingScript = await readFile(new URL('../assets/onboarding.js', import.meta.url), 'utf8');
const productionBrowserSmoke = await readFile(new URL('../../ui/production-browser-smoke.mjs', import.meta.url), 'utf8');
const productionAdvancedPagesSmoke = await readFile(new URL('../../ui/production-browser-advanced-pages-smoke.mjs', import.meta.url), 'utf8');
const productionSmokeCi = await readFile(new URL('../../deploy/production/smoke-ci.sh', import.meta.url), 'utf8');
const chatScript = await readFile(new URL('../assets/chat-space.js', import.meta.url), 'utf8');
const dialogueGenScript = await readFile(new URL('../assets/dialogue-gen.js', import.meta.url), 'utf8');
const dialogueFlowScript = await readFile(new URL('../assets/dialogue-flow.js', import.meta.url), 'utf8');
const agentHarnessScript = await readFile(new URL('../assets/agent-test-harness.js', import.meta.url), 'utf8');
const engineRouter = await readFile(new URL('../../engine/src/daemon/mod.rs', import.meta.url), 'utf8');

test('runtime entry redirects through an external CSP-compatible script', () => {
  assert.match(entryPage, /assets\/entry\.js/);
  assert.doesNotMatch(entryPage, /<script(?![^>]*src=)[^>]*>/i);
  assert.match(entryScript, /airp_onboarded/);
  assert.match(entryScript, /16-onboarding\.html/);
  // #303: Engine data_root 为权威源，localStorage 仅作离线后备
  assert.match(entryScript, /fetch\('health'\)/);
  assert.match(entryScript, /h\.onboarded/);
});

test('first-run onboarding uses a dedicated real-backend runtime', () => {
  assert.match(onboardingPage, /id="onboarding-steps"/);
  assert.match(onboardingPage, /id="onboarding-card"/);
  assert.match(onboardingPage, /assets\/api-client\.js/);
  assert.match(onboardingPage, /assets\/onboarding\.js/);
  assert.doesNotMatch(onboardingPage, /assets\/console-runtime\.js/);
});

test('first-run onboarding blocks blind resend after an uncertain commit', () => {
  assert.match(onboardingScript, /\['partially_committed', 'unknown'\]\.includes\(error\.commitState\)/);
  assert.match(onboardingScript, /sessionStorage\.setItem\(firstChatSessionKey, state\.sessionId\)/);
  assert.match(onboardingScript, /sessionStorage\.setItem\(firstChatUncertainKey, JSON\.stringify\(uncertainFirstChat\)\)/);
  assert.ok(
    onboardingScript.indexOf('sessionStorage.setItem(firstChatUncertainKey') < onboardingScript.indexOf("await client.stream('/v1/chat/completions'"),
    'the reload safeguard must be persisted before streaming begins',
  );
  assert.match(onboardingScript, /if \(uncertainFirstChat && state\.sessionId\)/);
  assert.match(onboardingScript, /message\.control\.disabled = true/);
  assert.match(onboardingScript, /打开对话历史确认/);
  assert.match(chatScript, /sessionStorage\.removeItem\('airp_onboarding_commit_uncertain'\)/);
});

test('production onboarding smoke waits for each rendered wizard step', () => {
  assert.match(productionBrowserSmoke, /waitForOnboardingStep/);
  assert.match(productionBrowserSmoke, /#onboarding-card \.wizard-head h1/);
  for (const heading of ['检查 AIRP Engine', '配置 LLM Provider', '验证模型连接', '导入或选择角色', '选择人设与预设', '完成首轮对话']) {
    assert.match(productionBrowserSmoke, new RegExp("waitForOnboardingStep\\(page, '" + heading + "'"));
  }
  assert.match(productionBrowserSmoke, /getByLabel\('给角色的第一句话'\)\.waitFor\(\{ state: 'visible' \}\)/);
});

test('both production restarts wait for the chat path before browser smoke', () => {
  const restartBlocks = productionSmokeCi.split('$compose restart engine gateway >/dev/null');
  assert.equal(restartBlocks.length, 3, 'the production smoke must retain two restart branches');
  for (const restartBlock of restartBlocks.slice(1)) {
    const probes = restartBlock.match(/WAIT_FOR_ENGINE_READY_CHAT_PROBE=1 wait_for_engine_ready/g) || [];
    assert.equal(probes.length, 1, 'each restart branch must wait for one chat probe');
  }
  assert.match(productionSmokeCi, /could not create disposable session/);
  assert.match(productionSmokeCi, /verify-readiness-sse\.mjs/);
});

test('production smoke covers every advanced WebUI page and a visible Engine failure', () => {
  for (const file of ['34-relationship-graph.html', '35-plot-arc.html', '36-image-gen.html', '37-character-templates.html', '38-style-learn.html', '39-dialogue-gen.html', '40-worldbook-graph.html', '41-timeline-export.html', '42-card-diff.html', '43-provider-management.html', '44-plugin-tools.html']) {
    assert.match(productionAdvancedPagesSmoke, new RegExp("file: '" + file + "'"));
  }
  assert.match(productionAdvancedPagesSmoke, /route\('\*\*\/health', route => route\.abort\('failed'\)\)/);
  assert.match(productionAdvancedPagesSmoke, /for \(const advancedPage of pages\)/);
  assert.match(productionAdvancedPagesSmoke, /classes\?\.contains\('danger'\) \|\| classes\?\.contains\('error'\)/);
});

for (const [name, html] of [['role list', rolePage], ['chat space', chatPage]]) {
  test(name + ' has no inline style or script blocked by the Engine CSP', () => {
    assert.doesNotMatch(html, /\sstyle\s*=/i);
    assert.doesNotMatch(html, /<style(?:\s|>)/i);
    assert.doesNotMatch(html, /<script(?![^>]*src=)[^>]*>/i);
  });

  test(name + ' does not include sample-only navigation chrome', () => {
    assert.doesNotMatch(html, /assets\/app\.js/);
    assert.doesNotMatch(html, /data-sample-chrome/);
  });
}

test('role list exposes the live import and data targets', () => {
  for (const id of ['engine-status', 'character-file', 'character-grid', 'stat-characters']) {
    assert.match(rolePage, new RegExp('id="' + id + '"'));
  }
  assert.match(rolePage, /class="pane-main role-main"/);
  assert.match(rolePage, /class="stat-grid"/);
  assert.match(rolePage, /class="char-grid" id="character-grid"/);
  assert.match(rolePage, /assets\/role-list\.js/);
});

test('chat space exposes session, history and streaming controls', () => {
  for (const id of ['session-list', 'message-flow', 'message-input', 'send-message', 'continue-message', 'regen-message', 'refresh-history']) {
    assert.match(chatPage, new RegExp('id="' + id + '"'));
  }
  assert.match(chatPage, /assets\/chat-space\.js/);
});

test('dialogue example card reads use the registered Engine character route', () => {
  assert.match(engineRouter, /"\/v1\/characters\/:character_id"[\s\S]*?get\(get_character_card\)/);
  assert.match(dialogueFlowScript, /'\/v1\/characters\/' \+ encodeURIComponent\(characterId\)/);
  assert.doesNotMatch(dialogueFlowScript, /\/v1\/characters\/['"]?\s*\+[^;\n]+\/card/);
  assert.match(dialogueGenScript, /dialogueFlow\.load\(characterId\)/);
});

test('agent DOM snapshots redact descendant content while retaining interface labels', () => {
  function element(tagName, { id = '', className = '', text = '', ariaLabel = null, parentElement = null } = {}) {
    return {
      tagName,
      id,
      className,
      textContent: text,
      childNodes: text ? [{ nodeType: 3, textContent: text }] : [],
      children: [],
      parentElement,
      disabled: false,
      getAttribute(name) {
        if (name === 'aria-label') return ariaLabel;
        return null;
      },
      getBoundingClientRect() { return { width: 10, height: 10 }; },
      matches() { return false; },
    };
  }

  function runHarness(html, nodes) {
    const document = {
      documentElement: html,
      body: nodes[0],
      createTreeWalker() {
        let index = 0;
        return {
          currentNode: nodes[0],
          nextNode() {
            index += 1;
            return nodes[index] || null;
          },
        };
      },
    };
    const window = {
      VITE_AIRP_AGENT_TEST: '1',
      location: { origin: 'https://example.test', pathname: '/', search: '' },
      addEventListener() {},
    };
    vm.runInNewContext(agentHarnessScript, {
      window,
      globalThis: window,
      document,
      NodeFilter: { SHOW_ELEMENT: 1 },
      console: { info() {} },
    });
    return window.__AIRP_AGENT_TEST__.getDomSnapshot();
  }

  const html = element('HTML');
  const body = element('BODY', { parentElement: html });
  const message = element('DIV', { className: 'message', parentElement: body });
  const secret = element('SPAN', {
    text: 'private user message',
    ariaLabel: 'private message preview',
    parentElement: message,
  });
  const rootSecret = element('SPAN', { text: 'root secret', parentElement: body });
  const label = element('BUTTON', {
    id: 'save-settings',
    text: 'Save',
    ariaLabel: 'Save settings',
    parentElement: body,
  });
  const snapshot = runHarness(html, [body, message, secret, label]);
  const secretLeaf = snapshot.find(element => element.text === 'private user message');
  assert.equal(secretLeaf.sensitive, true);

  const sanitized = sanitizeDomSnapshot(snapshot);
  assert.equal(sanitized.find(element => element.id === 'save-settings').text, 'Save');
  assert.equal(sanitized.find(element => element.id === 'save-settings').ariaLabel, 'Save settings');
  assert.equal(sanitized.find(element => element.tag === 'span').text, '[REDACTED]');
  assert.equal(sanitized.find(element => element.tag === 'span').ariaLabel, '[REDACTED]');

  const messageRoot = element('HTML', { className: 'message-root' });
  const rootBody = element('BODY', { parentElement: messageRoot });
  rootSecret.parentElement = rootBody;
  const rootSnapshot = runHarness(messageRoot, [rootBody, rootSecret]);
  assert.equal(rootSnapshot.find(element => element.text === 'root secret').sensitive, true);
});

test('every shipped screen is compatible with the Engine CSP', async () => {
  const directory = new URL('../screens/', import.meta.url);
  const files = (await readdir(directory)).filter(name => name.endsWith('.html'));
  assert.equal(files.length, 44);
  for (const file of files) {
    const html = await readFile(new URL(file, directory), 'utf8');
    assert.doesNotMatch(html, /\sstyle\s*=/i, file + ' contains an inline style');
    assert.doesNotMatch(html, /<style(?:\s|>)/i, file + ' contains an inline style block');
    assert.doesNotMatch(html, /<script(?![^>]*src=)[^>]*>/i, file + ' contains an inline script');
  }
});

test('operational console pages load the shared real-backend runtime', async () => {
  // CodeRabbit #12：移除 '18-group-chat.html'——该屏在 Phase 3.1 重写为专用页面
  // （用 group-chat.js，非 console-runtime.js）。cherry-pick 时误并入第二个
  // console shell 文档让旧测试假性通过；修复后该屏是专用页面，不再适用此契约。
  for (const file of ['03-workbench.html', '04-world-book.html', '05-presets.html', '06-user-persona.html', '07-agent-runs.html', '08-settings.html', '17-memory-state.html', '19-branch-tree.html', '20-assembly-preview.html', '21-usage-quota.html', '22-backup-restore.html', '23-diagnostics.html', '24-plugins.html', '25-notes-connections.html', '32-style-review.html']) {
    const html = await readFile(new URL('../screens/' + file, import.meta.url), 'utf8');
    assert.match(html, /assets\/api-client\.js/);
    assert.match(html, /assets\/console-runtime\.js/);
    assert.match(html, /id="engine-status" role="status"/);
    assert.doesNotMatch(html, /assets\/app\.js/);
  }
});

test('console-runtime implements #304 new UI components', async () => {
  const rt = await readFile(new URL('../assets/console-runtime.js', import.meta.url), 'utf8');
  // NL enhance zone with disabled button
  assert.match(rt, /nl-zone/, 'missing NL zone');
  assert.match(rt, /nl-planned-tag/, 'missing NL planned tag');
  assert.match(rt, /nlGenBtn.*disabled = true/, 'NL generate button must be disabled');
  // JSON advanced fold
  assert.match(rt, /json-advanced/, 'missing JSON advanced fold');
  assert.match(rt, /ja-bar/, 'missing JA bar');
  // Worldbook switch component
  assert.match(rt, /switch on.*switch/, 'missing .switch toggle in worldbook');
  // Model pill neutral (not false ok)
  assert.match(rt, /status-pill neutral/, 'model pill must be neutral');
  assert.doesNotMatch(rt, /status-pill ok.*已拉取/, 'model pill must not show false ok');
  // Combobox class on fallback input
  assert.match(rt, /combobox/, 'missing combobox class');
  // 05 presets must NOT contain model management
  assert.doesNotMatch(rt, /renderPresets[\s\S]*?Provider 模型/, 'presets page must not render model card');
});

// N-K 修复：PR #314 Phase 1 WebUI 关键修复点契约测试
test('PR #314 B1/B2/B3/N-D fixes are present in console-runtime and chat-space', async () => {
  const rt = await readFile(new URL('../assets/console-runtime.js', import.meta.url), 'utf8');
  const cs = await readFile(new URL('../assets/chat-space.js', import.meta.url), 'utf8');
  // B1: AnalysisFileEntry 对象解构（不能把对象当字符串）
  assert.match(rt, /files\.forEach\(entry =>/, 'B1: must iterate entries as objects');
  assert.match(rt, /entry\.filename/, 'B1: must extract filename from entry object');
  assert.doesNotMatch(rt, /files\.forEach\(filename =>/, 'B1: must not treat entry as string filename');
  // B2: CharacterRole 对齐 Engine（primary/npc，不能是 main/supporting/narrator）
  assert.match(rt, /value: 'primary'/, 'B2: must offer primary role');
  assert.match(rt, /value: 'npc'/, 'B2: must offer npc role');
  assert.doesNotMatch(rt, /value: 'main'/, 'B2: must not offer invalid main role');
  assert.doesNotMatch(rt, /value: 'supporting'/, 'B2: must not offer invalid supporting role');
  assert.doesNotMatch(rt, /value: 'narrator'/, 'B2: must not offer invalid narrator role');
  // B3: 导出不传 limit（走 legacy 全量分支）
  assert.match(cs, /\/v1\/chat\/history/, 'B3: export must call history endpoint');
  assert.doesNotMatch(cs, /limit:\s*9999/, 'B3: must not hardcode limit:9999');
  // N-D: 场景元信息竞态防护 metaToken
  assert.match(rt, /metaToken/, 'N-D: must use metaToken for race condition protection');
});

// N-K 修复：PR #314 relationship-graph 关键修复点契约测试
test('PR #314 N-F/N-G/N-H/N-I/N-M fixes are present in relationship-graph', async () => {
  const rg = await readFile(new URL('../assets/relationship-graph.js', import.meta.url), 'utf8');
  const rgHtml = await readFile(new URL('../screens/34-relationship-graph.html', import.meta.url), 'utf8');
  const rgCss = await readFile(new URL('../assets/relationship-graph.css', import.meta.url), 'utf8');
  // N-F: intensity:0 不能被 || 0.5 覆盖
  assert.match(rg, /val\.intensity != null/, 'N-F: must use explicit null check for intensity');
  assert.doesNotMatch(rg, /val\.intensity\s*\|\|\s*0\.5/, 'N-F: must not use || for intensity default');
  // N-G: rAF 空转防护
  assert.match(rg, /ensureAnim/, 'N-G: must have ensureAnim to wake sleeping rAF');
  assert.match(rg, /animFrame = null/, 'N-G: must nullify animFrame when idle');
  // N-H: 角色切换后重建导航
  assert.match(rg, /renderChrome\(\)/, 'N-H: must call renderChrome on character change');
  assert.match(rg, /nav\.replaceChildren\(\)/, 'N-H: renderChrome must clear old nav before rebuild');
  // N-I: a11y 替代表格
  assert.match(rgHtml, /sr-only/, 'N-I: must have sr-only table for a11y');
  assert.match(rgHtml, /graph-a11y-body/, 'N-I: must have a11y table body');
  assert.match(rgCss, /\.sr-only/, 'N-I: must define sr-only CSS class');
  // N-M: 节点边框不用 --text-inverse
  assert.match(rg, /primaryStrong/, 'N-M: must use primaryStrong for node border');
  assert.doesNotMatch(rg, /strokeStyle = COLORS\.inverse/, 'N-M: must not use --text-inverse for stroke');
});

// N-K 修复：PR #314 N-E bearer 跨源防护契约测试
test('PR #314 N-E bearer cross-origin protection is present in api-client', async () => {
  const ac = await readFile(new URL('../assets/api-client.js', import.meta.url), 'utf8');
  assert.match(ac, /shouldSendBearer/, 'N-E: must have shouldSendBearer guard');
  assert.match(ac, /trustedOrigins/, 'N-E: must support trustedOrigins whitelist');
  assert.match(ac, /headers\(.*,\s*base\)/, 'N-E: headers must receive target base for origin check');
});
