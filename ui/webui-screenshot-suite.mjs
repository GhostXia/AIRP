// AIRP WebUI 截图验证套件（Task #12）。
//
// 两种模式：
//   --mode local  本地模式：自行拉起 engine daemon（target/*/airp-core 或 cargo run）
//                 + `--webui-dir webui`，对 127.0.0.1:<port> 截图。
//   --mode ci     CI 模式：连接已就绪的生产拓扑，沿用 smoke-ci.sh 的环境变量约定
//                 （AIRP_SMOKE_ORIGIN / AIRP_SMOKE_ADMIN_USER / AIRP_SMOKE_ADMIN_PASSWORD /
//                  AIRP_SMOKE_RESULT_FILE / AIRP_CHROME_SPKI / AIRP_CHROME_PATH）。
//
// 职责：
//   1. 遍历 webui/screens/ 全部屏（自动读目录，不硬编码清单），1440×900 视口逐屏
//      fullPage 截图，输出 <out>/<NN>-<slug>.png。
//   2. 关键流步骤截图：onboarding 逐步、聊天发送/流式中/完成三帧、角色导入屏（13）、
//      备份屏（22）。缺 provider 时聊天帧降级为「发送后状态帧」并如实标注，不整体失败。
//   3. 每屏断言 tokens.css 全部设计令牌的 computed 精确值（期望值在运行时解析
//      tokens.css 生成，不手抄）。任一令牌不符 → 非零退出。
//   4. 收集 securitypolicyviolation 与 pageerror，任一发生 → 非零退出。
//
// 用法示例：
//   node ui/webui-screenshot-suite.mjs --mode local --out dist/screens
//   node ui/webui-screenshot-suite.mjs --mode local --out dist/screens --screens 01,02,16
//   node ui/webui-screenshot-suite.mjs --mode ci --out deploy/production/smoke-screenshots

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');

// ── 参数解析 ──────────────────────────────────────────────────────────────
function parseArgs(argv) {
  const args = { mode: null, out: null, screens: null, webuiDir: path.join(repoRoot, 'webui'), port: 8000, chromePath: null };
  for (let i = 2; i < argv.length; i += 1) {
    const flag = argv[i];
    const next = () => {
      i += 1;
      assert.ok(i < argv.length, `missing value for ${flag}`);
      return argv[i];
    };
    if (flag === '--mode') args.mode = next();
    else if (flag === '--out') args.out = next();
    else if (flag === '--screens') args.screens = next();
    else if (flag === '--webui-dir') args.webuiDir = path.resolve(next());
    else if (flag === '--port') args.port = Number(next());
    else if (flag === '--chrome-path') args.chromePath = next();
    else assert.fail(`unknown flag ${flag}`);
  }
  assert.ok(args.mode === 'local' || args.mode === 'ci', '--mode must be local or ci');
  assert.ok(args.out, '--out <dir> is required');
  return args;
}

const args = parseArgs(process.argv);
const outDir = path.resolve(args.out);
mkdirSync(path.join(outDir, 'flow'), { recursive: true });

// ── 结果收集 ──────────────────────────────────────────────────────────────
const failures = [];
const notes = [];
let tokenAssertions = 0;
const fail = (message) => failures.push(message);
const note = (message) => { notes.push(message); console.log('  note: ' + message); };

// ── tokens.css 解析：期望值单一事实源，运行时生成而非手抄 ─────────────────
function parseTokens(webuiDir) {
  const css = readFileSync(path.join(webuiDir, 'assets', 'tokens.css'), 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '');
  const rootBlock = css.match(/:root\s*\{([\s\S]*?)\}/);
  assert.ok(rootBlock, 'tokens.css must define a :root block');
  const tokens = new Map();
  for (const match of rootBlock[1].matchAll(/(--[a-z0-9-]+)\s*:\s*([^;]+);/gi)) {
    tokens.set(match[1], normalizeValue(match[2]));
  }
  assert.ok(tokens.size >= 25, `tokens.css should define ~25+ core tokens, found ${tokens.size}`);
  return tokens;
}

function normalizeValue(value) {
  return value.replace(/\s+/g, ' ').trim();
}

const expectedTokens = parseTokens(args.webuiDir);
console.log(`tokens.css parsed: ${expectedTokens.size} design tokens will be asserted per screen`);

// ── Chrome 可执行文件解析（沿用 AIRP_CHROME_PATH 约定 + 常见安装位置兜底）──
function resolveChromeExecutable() {
  const candidates = [];
  if (args.chromePath) candidates.push(args.chromePath);
  if (process.env.AIRP_CHROME_PATH) candidates.push(process.env.AIRP_CHROME_PATH);
  if (args.mode === 'ci') candidates.push('/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser');
  if (process.env.LOCALAPPDATA) candidates.push(path.join(process.env.LOCALAPPDATA, 'Google', 'Chrome', 'Application', 'chrome.exe'));
  candidates.push(
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe',
    'C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe',
    path.join(process.env.PROGRAMFILES || '', 'Google', 'Chrome', 'Application', 'chrome.exe'),
  );
  for (const candidate of candidates) {
    if (candidate && existsSync(candidate)) return candidate;
  }
  assert.fail('no Chrome/Chromium executable found; set AIRP_CHROME_PATH or --chrome-path');
}

// ── 屏清单：自动读目录，不硬编码 ──────────────────────────────────────────
function listScreens(webuiDir, filter) {
  const screensDir = path.join(webuiDir, 'screens');
  let files = readdirSync(screensDir).filter(name => name.endsWith('.html')).sort();
  if (filter) {
    const wanted = filter.split(',').map(item => item.trim()).filter(Boolean);
    files = files.filter(file => {
      const stem = file.replace(/\.html$/, '');
      const number = stem.split('-')[0];
      return wanted.some(item => item === file || item === stem || item === number || stem.endsWith(item));
    });
  }
  assert.ok(files.length > 0, 'no screens matched');
  return files;
}

// Engine 对 /v1 全局限流：10 req/s 持续、burst 20（per IP，见 daemon/mod.rs
// RATE_LIMIT_PERIOD/RATE_LIMIT_BURST）。每屏启动会触发多个 /v1 请求，
// 屏间不加节流会打出 429，导致后续屏/流程误报连接失败（local-webui-browser-smoke
// 因此用 1.5s 间隔模拟人类操作）。此处同样用 1.5s 屏间隔保持预算内。
const INTER_SCREEN_SETTLE_MS = 1500;

// ── engine 健康探测 ───────────────────────────────────────────────────────
async function waitForHealth(origin, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let lastError = 'no attempt';
  while (Date.now() < deadline) {
    try {
      const response = await fetch(origin + '/health');
      if (response.ok) {
        const body = await response.json();
        if (body.engine === 'ok') return body;
      }
      lastError = `status ${response.status}`;
    } catch (error) {
      lastError = error.message;
    }
    await sleep(500);
  }
  throw new Error(`engine /health did not become ok within ${timeoutMs}ms: ${lastError}`);
}

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

// ── local 模式：拉起 daemon ───────────────────────────────────────────────
function killProcessTree(child) {
  if (!child || child.exitCode !== null) return;
  try {
    child.killedBySuite = true;
    if (process.platform === 'win32') spawn('taskkill', ['/pid', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
    else child.kill('SIGKILL');
  } catch { /* already gone */ }
}

async function startLocalDaemon() {
  const exe = process.platform === 'win32' ? 'airp-core.exe' : 'airp-core';
  const directCandidates = [];
  if (process.env.AIRP_ENGINE_BIN) directCandidates.push(process.env.AIRP_ENGINE_BIN);
  directCandidates.push(path.join(repoRoot, 'target', 'debug', exe), path.join(repoRoot, 'target', 'release', exe));
  const direct = directCandidates.find(candidate => existsSync(candidate));
  const webuiDirFlag = args.webuiDir;
  assert.ok(existsSync(path.join(webuiDirFlag, 'index.html')), `--webui-dir ${webuiDirFlag} must contain index.html`);
  let child;
  if (direct) {
    console.log(`starting engine daemon: ${direct} daemon --port ${args.port} --webui-dir ${webuiDirFlag}`);
    child = spawn(direct, ['daemon', '--port', String(args.port), '--webui-dir', webuiDirFlag], {
      cwd: repoRoot,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
  } else {
    console.log(`no prebuilt airp-core binary found; falling back to cargo run (slow first time)`);
    child = spawn('cargo', ['run', '-p', 'airp-core', '--', 'daemon', '--port', String(args.port), '--webui-dir', webuiDirFlag], {
      cwd: repoRoot,
      stdio: ['ignore', 'pipe', 'pipe'],
      shell: process.platform === 'win32',
    });
  }
  let tail = '';
  const capture = chunk => { tail = (tail + chunk.toString()).slice(-4000); };
  child.stdout.on('data', capture);
  child.stderr.on('data', capture);
  // 套件正常结束后会 taskkill 掉 daemon（退出码非 0），不应当作早退报错。
  child.on('exit', code => { if (code !== null && code !== 0 && !child.killedBySuite) console.error(`daemon exited early (code ${code}):\n${tail}`); });
  const origin = `http://127.0.0.1:${args.port}`;
  try {
    // cargo 冷构建可能需要数分钟；预构建二进制几秒即可就绪。
    await waitForHealth(origin, direct ? 30_000 : 600_000);
  } catch (error) {
    killProcessTree(child);
    throw new Error(`${error.message}\ndaemon output tail:\n${tail}`);
  }
  return { origin, child };
}

// ── 角色/会话准备 ─────────────────────────────────────────────────────────
// 套件专用测试角色（fixture）：当 data/characters 下没有任何「卡文件可用」的角色时
// （角色目录可能只剩运行期子目录、缺 card.json → GET /v1/characters/:id 404 →
// 聊天屏 boot 失败），用最小 TavernV2 卡自建一个，保证截图验证不依赖用户私有卡。
const SUITE_CHARACTER_NAME = '截图套件测试角色';
const SUITE_CHARACTER_CARD = JSON.stringify({
  spec: 'chara_card_v2',
  spec_version: '2.0',
  data: {
    name: SUITE_CHARACTER_NAME,
    description: 'AIRP webui 截图验证套件的临时测试角色（自动生成，可安全删除）。',
    personality: '配合截图验证，回复简短。',
    first_mes: '你好，我是截图套件测试角色。',
    mes_example: '',
    scenario: '',
    creator_notes: 'generated by ui/webui-screenshot-suite.mjs',
    tags: ['screenshot-suite-fixture'],
  },
});

async function resolveCharacterSession(request, origin, mode) {
  if (mode === 'ci') {
    const apiResult = JSON.parse(readFileSync(process.env.AIRP_SMOKE_RESULT_FILE, 'utf8'));
    assert.ok(apiResult.character_id && apiResult.session_id, 'API smoke result must provide character/session');
    return { characterId: apiResult.character_id, sessionId: apiResult.session_id };
  }
  const tryCharacters = async () => {
    const response = await request.get(origin + '/v1/characters');
    assert.ok(response.ok(), `GET /v1/characters -> ${response.status()}`);
    const list = await response.json();
    const characters = Array.isArray(list) ? list : list.characters || [];
    // GET /v1/characters 返回 Vec<String>（角色 id 列表）；兼容对象形状以防演进。
    return characters.map(first => (typeof first === 'string' ? first : first.id || first.character_id || first.name)).filter(Boolean);
  };
  // 角色目录存在≠卡可用（可能缺 card.json 只剩运行期子目录）；探测卡可读性。
  const cardUsable = async characterId => {
    try { return (await request.get(origin + '/v1/characters/' + encodeURIComponent(characterId))).ok(); }
    catch { return false; }
  };
  const importSuiteFixture = async () => {
    const response = await request.post(origin + '/v1/characters/import', { data: { card_json: SUITE_CHARACTER_CARD } });
    assert.ok(response.ok(), `POST /v1/characters/import -> ${response.status()}`);
    const result = await response.json();
    const imported = result?.character_id || result?.id;
    assert.ok(imported, '角色导入响应缺少 character_id');
    return String(imported);
  };
  const trySession = async characterId => {
    const sessionResponse = await request.post(origin + '/v1/sessions/' + encodeURIComponent(characterId));
    assert.ok(sessionResponse.ok(), `POST /v1/sessions/${characterId} -> ${sessionResponse.status()}`);
    const value = await sessionResponse.json().catch(() => null);
    const sessionId = typeof value === 'string' ? value : value?.session_id || value?.id;
    assert.ok(sessionId, 'POST /v1/sessions 响应缺少 session id');
    return sessionId;
  };
  try {
    const ids = await tryCharacters();
    // 优先用户既有可用角色；其次复用套件自建的 fixture（幂等，不重复导入）；
    // 都没有才导入最小测试卡。
    let characterId = null;
    for (const id of ids) {
      if (await cardUsable(id)) { characterId = id; break; }
    }
    if (!characterId) {
      const fixture = ids.find(id => id === SUITE_CHARACTER_NAME || id.startsWith(SUITE_CHARACTER_NAME));
      if (fixture && (await cardUsable(fixture))) characterId = fixture;
    }
    if (!characterId) {
      characterId = await importSuiteFixture();
      note(`未找到卡文件可用的既有角色，已导入套件测试角色 ${characterId}（fixture，可安全删除）`);
    }
    // 先复用已有会话（避免每次跑套件都新建会话目录），没有再创建。
    let sessionId = null;
    try {
      const listResponse = await request.get(origin + '/v1/sessions/' + encodeURIComponent(characterId));
      if (listResponse.ok()) {
        const listIds = await listResponse.json();
        if (Array.isArray(listIds) && listIds.length > 0) sessionId = String(listIds[0]);
      }
    } catch { /* fall through to create */ }
    if (!sessionId) sessionId = await trySession(characterId);
    if (sessionId) return { characterId, sessionId };
  } catch (error) {
    note(`local 模式角色/会话解析失败：${error.message?.split('\n')[0] || error}`);
  }
  note('local 模式未能解析角色/会话：屏截图继续（无 character 参数），聊天流步骤降级');
  return { characterId: null, sessionId: null };
}

// ── 页面就绪等待（复用 advanced-pages-smoke 的思路：boot 完成锚点）────────
async function waitForScreenReady(page, file) {
  // 桩页（body[data-target]）会 location.replace 到真实功能页；等待跳转完成。
  await page.waitForFunction(() => document.readyState !== 'loading', null, { timeout: 15_000 });
  const isStub = await page.evaluate(() => Boolean(document.body?.dataset?.target));
  if (isStub) {
    await page.waitForFunction(() => !document.body?.dataset?.target, null, { timeout: 15_000 });
    await page.waitForLoadState('domcontentloaded');
  }
  await page.waitForFunction(() => document.readyState === 'complete', null, { timeout: 15_000 });
  const engineStatus = page.locator('#engine-status');
  if (await engineStatus.count() > 0) {
    await page.waitForFunction(() => {
      const classes = document.querySelector('#engine-status')?.classList;
      return classes && (classes.contains('ok') || classes.contains('danger') || classes.contains('error') || classes.contains('warn') || classes.contains('neutral'));
    }, null, { timeout: 15_000 }).catch(() => fail(`${file}: #engine-status boot 未在 15s 内定型`));
  }
  const view = page.locator('#view');
  if (await view.count() > 0) {
    await page.waitForFunction(() => document.querySelector('#view')?.children.length > 0, null, { timeout: 15_000 })
      .catch(() => note(`${file}: #view 未在 15s 内渲染子节点（可能为空态）`));
  }
  await sleep(300);
}

// ── 令牌断言：每屏对 documentElement computed 值做精确比对 ────────────────
async function assertTokens(page, label) {
  const names = [...expectedTokens.keys()];
  const computed = await page.evaluate(tokenNames => {
    const style = getComputedStyle(document.documentElement);
    return tokenNames.map(name => style.getPropertyValue(name));
  }, names);
  for (let i = 0; i < names.length; i += 1) {
    tokenAssertions += 1;
    const actual = (computed[i] || '').replace(/\s+/g, ' ').trim();
    const expected = expectedTokens.get(names[i]);
    if (actual !== expected) fail(`${label}: 令牌 ${names[i]} 期望 "${expected}" 实际 "${actual}"`);
  }
}

async function collectCspViolations(page, label) {
  const violations = await page.evaluate(() => window.__airpCspViolations || []);
  for (const violation of violations) fail(`${label}: CSP violation directive=${violation.directive} blocked=${violation.blocked}`);
  return violations.length;
}

async function shoot(page, target) {
  const file = path.join(outDir, target);
  mkdirSync(path.dirname(file), { recursive: true });
  const buffer = await page.screenshot({ fullPage: true });
  writeFileSync(file, buffer);
  if (!existsSync(file) || readFileSync(file).length === 0) fail(`截图文件生成失败: ${target}`);
  console.log(`  shot ${target}`);
}

// 页内直点 onboarding 按钮（比 Playwright 指针模拟更稳：直接校验启用态并触发
// 真实 click 事件）。返回 false 表示超时内未能点击成功。
async function clickOnboardingButton(page, buttonText, timeoutMs = 12_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const state = await page.evaluate(name => {
      const candidate = [...document.querySelectorAll('#onboarding-card button')].find(item => item.textContent.includes(name));
      if (!candidate) return { present: false };
      if (candidate.disabled) return { present: true, disabled: true };
      candidate.click();
      return { present: true, disabled: false, clicked: true };
    }, buttonText);
    if (state.clicked) return true;
    await sleep(400);
  }
  return false;
}

// ── onboarding 逐步截图（尽力推进；点击失败即停并标注）────────────────────
async function captureOnboardingFlow(page, onboardingCharacterId) {
  const steps = [
    // 第 1 步的「下一步 →」在 Engine 健康检查通过后才启用（onboarding.js renderHealth），
    // 页面不会自动推进——必须点击；clickOnboardingButton 会轮询启用态。
    { heading: '检查 AIRP Engine', button: '下一步 →' },
    { heading: '配置 LLM Provider', button: '保存并下一步 →' },
    { heading: '验证模型连接', button: '下一步 →' },
    { heading: '导入或选择角色', button: '下一步 →', selectCharacter: true },
    { heading: '选择人设与预设', button: '下一步 →' },
    // 末步按钮是「发送首轮消息」，会触发真实 LLM 流式请求（本地无 key 必错，
    // CI 有 key 会消耗真实配额）：只截帧不点击，属有意设计而非降级。
    { heading: '完成首轮对话', button: null },
  ];
  for (let index = 0; index < steps.length; index += 1) {
    const step = steps[index];
    try {
      await page.waitForFunction(expected => document.querySelector('#onboarding-card .wizard-head h1')?.textContent?.includes(expected), step.heading, { timeout: 12_000 });
    } catch {
      const current = await page.evaluate(() => document.querySelector('#onboarding-card .wizard-head h1')?.textContent || '(无标题)');
      note(`onboarding 流程止步于第 ${index + 1} 步之前（未到达「${step.heading}」，当前标题：${current}）——多为本地未配置 provider 或限流所致，属预期降级`);
      return;
    }
    await shoot(page, path.join('flow', `onboarding-${index + 1}.png`));
    // 第 4 步的「下一步」依赖已选角色；尽力选中当前角色（按钮名即角色 id）。
    if (step.selectCharacter && onboardingCharacterId) {
      try {
        const choice = page.getByRole('button', { name: onboardingCharacterId, exact: true });
        if (await choice.count()) await choice.click({ timeout: 5_000 });
      } catch { note(`onboarding 第 ${index + 1} 步未能选中角色 ${onboardingCharacterId}（非致命）`); }
    }
    if (!step.button) continue;
    // 步骤按钮在异步检查完成后才启用（如第 1 步的 Engine 健康检查）；
    // clickOnboardingButton 内部轮询启用态并在页内直接触发 click。
    const clicked = await clickOnboardingButton(page, step.button);
    if (!clicked) {
      note(`onboarding 第 ${index + 1} 步「${step.button}」超时未能点击（本地无 provider 时属预期降级），流程截图到此为止`);
      return;
    }
  }
}

// ── 聊天三帧：发送后 / 流式中 / 完成。无 provider 降级为发送后状态帧 ──────
async function captureChatFlow(page, origin, request, characterId, sessionId, providerConfigured) {
  if (!characterId || !sessionId) {
    note('无角色/会话：跳过聊天流截图');
    return;
  }
  const suffix = providerConfigured ? '' : '（无 provider：帧为发送后的错误/等待态）';
  // 会话若因上次中断写入被 TurnCommit 锁在 recovering 态，输入框会一直禁用；
  // 模拟真实用户处置：接受确认弹窗并点「尝试恢复会话」（recoverSession 走
  // /v1/chat/session-recover，只隔离标记不删数据）。
  page.on('dialog', dialog => dialog.accept().catch(() => {}));
  const recoverIfLocked = async () => {
    const recover = page.locator('#session-recover');
    if (await recover.count() > 0) {
      note('会话处于 recovering 锁定态：点击「尝试恢复会话」后继续');
      await recover.click();
      return true;
    }
    return false;
  };
  const waitInputEnabled = () => page.waitForFunction(() => document.querySelector('#message-input')?.disabled === false, null, { timeout: 20_000 })
    .then(() => true).catch(() => false);
  // 聊天屏 boot 会连发多个 /v1 请求（version/health/settings/sessions/history/…）；
  // 前面 44 屏 + onboarding 流可能已把限流预算打低。启动前充分回补预算，
  // 并给 3 次重试（每次递增等待），避免 429 导致 boot 失败、输入框不启用。
  await sleep(5000);
  let inputEnabled = false;
  for (let attempt = 1; attempt <= 3 && !inputEnabled; attempt += 1) {
    await page.goto(`${origin}/screens/02-chat-space.html?character=${encodeURIComponent(characterId)}&session=${encodeURIComponent(sessionId)}`, { waitUntil: 'domcontentloaded' });
    inputEnabled = await waitInputEnabled();
    if (!inputEnabled) {
      // 锁定态恢复：boot 完成后才出现 #session-recover，给它一点渲染时间。
      await sleep(1000);
      if (await recoverIfLocked()) inputEnabled = await waitInputEnabled();
    }
    if (!inputEnabled && attempt < 3) {
      const waitMs = attempt * 5000;
      note(`聊天空间第 ${attempt} 次启动输入框未启用（可能受 /v1 限流影响），等待 ${waitMs}ms 后重载重试`);
      await sleep(waitMs);
    }
  }
  if (!inputEnabled) {
    const diagnosis = await page.evaluate(() => ({
      connection: document.querySelector('#engine-status')?.textContent?.trim() || '(无)',
      sessionStatus: document.querySelector('#session-operation-status')?.textContent?.trim() || '(无)',
      lastLog: document.querySelector('#event-log .log-item')?.textContent?.trim() || '(无)',
    })).catch(() => null);
    if (diagnosis) note(`聊天屏诊断：连接=${diagnosis.connection}｜会话状态=${diagnosis.sessionStatus}｜最近日志=${diagnosis.lastLog}`);
    await shoot(page, path.join('flow', 'chat-1-input-unavailable.png'));
    note('聊天输入框 3 次重试后仍未启用（引擎连接异常、限流未回补或会话锁定未恢复），仅保留状态帧' + suffix);
    return;
  }
  await page.locator('#message-input').fill('screenshot suite frame ' + Date.now());
  await page.locator('#send-message').click();
  await sleep(400);
  await shoot(page, path.join('flow', 'chat-1-sent.png'));
  // 流式中：发送按钮变为 stop 态（chat-space.js 约定）。
  const streaming = await page.waitForFunction(() => document.querySelector('#send-message')?.classList.contains('stop'), null, { timeout: 8_000 })
    .then(() => true).catch(() => false);
  await shoot(page, path.join('flow', 'chat-2-streaming.png'));
  if (!streaming) note('未观察到流式 stop 态' + suffix);
  await page.waitForFunction(() => !document.querySelector('#send-message')?.classList.contains('stop'), null, { timeout: 25_000 }).catch(() => {});
  await sleep(500);
  await shoot(page, path.join('flow', providerConfigured ? 'chat-3-done.png' : 'chat-3-after-send-no-provider.png'));
  if (!providerConfigured) note('本地无真实 provider key：chat-3 为发送后状态帧（预期为错误态），不代表功能失败');
}

// ── 主流程 ────────────────────────────────────────────────────────────────
const chromeExecutable = resolveChromeExecutable();
let daemonChild = null;
let origin;
let launchOptions = { headless: true, executablePath: chromeExecutable };
let contextOptions = { viewport: { width: 1440, height: 900 } };
let providerConfigured = true;

if (args.mode === 'local') {
  const daemon = await startLocalDaemon();
  origin = daemon.origin;
  daemonChild = daemon.child;
  providerConfigured = (await (await fetch(origin + '/health')).json()).provider_configured === true;
  if (!providerConfigured) note('engine /health provider_configured=false：聊天流将以降级帧呈现');
} else {
  for (const name of ['AIRP_SMOKE_ORIGIN', 'AIRP_SMOKE_ADMIN_USER', 'AIRP_SMOKE_ADMIN_PASSWORD', 'AIRP_SMOKE_RESULT_FILE', 'AIRP_CHROME_SPKI']) {
    assert.ok(process.env[name], `${name} is required in ci mode`);
  }
  origin = process.env.AIRP_SMOKE_ORIGIN;
  launchOptions = { ...launchOptions, args: [`--ignore-certificate-errors-spki-list=${process.env.AIRP_CHROME_SPKI}`] };
  contextOptions = { ...contextOptions, httpCredentials: { username: process.env.AIRP_SMOKE_ADMIN_USER, password: process.env.AIRP_SMOKE_ADMIN_PASSWORD } };
  const healthResponse = await fetch(origin + '/health').catch(() => null);
  providerConfigured = healthResponse ? (await healthResponse.json().catch(() => ({}))).provider_configured !== false : true;
}

const browser = await chromium.launch(launchOptions);
try {
  const context = await browser.newContext(contextOptions);
  const page = await context.newPage();
  const pageErrors = [];
  page.on('pageerror', error => pageErrors.push(error.message));
  await page.addInitScript(() => {
    window.__airpCspViolations = [];
    document.addEventListener('securitypolicyviolation', event => window.__airpCspViolations.push({ directive: event.effectiveDirective, blocked: event.blockedURI }));
  });

  const request = context.request;
  const { characterId, sessionId } = await resolveCharacterSession(request, origin, args.mode);

  // 1) 44 屏逐屏截图
  const screens = listScreens(args.webuiDir, args.screens);
  console.log(`capturing ${screens.length} screens from ${args.webuiDir}`);
  for (const file of screens) {
    const label = file.replace(/\.html$/, '');
    const url = new URL(origin + '/screens/' + file);
    if (characterId) url.searchParams.set('character', characterId);
    if (sessionId) url.searchParams.set('session', sessionId);
    try {
      await page.goto(url.href, { waitUntil: 'domcontentloaded' });
      await waitForScreenReady(page, file);
      await assertTokens(page, label);
      await collectCspViolations(page, label);
      await shoot(page, label + '.png');
    } catch (error) {
      fail(`${file}: ${error.message?.split('\n')[0] || error}`);
    }
    // 屏间节流：避免击穿 Engine 的 /v1 限流预算（见 INTER_SCREEN_SETTLE_MS 注释）。
    await sleep(INTER_SCREEN_SETTLE_MS);
  }

  // 2) 关键流步骤截图（先让限流预算回满）
  await sleep(3000);
  try {
    await page.goto(origin + '/screens/16-onboarding.html', { waitUntil: 'domcontentloaded' });
    await captureOnboardingFlow(page, characterId);
    await collectCspViolations(page, 'flow/onboarding');
  } catch (error) {
    note('onboarding 流截图异常（非致命）: ' + (error.message?.split('\n')[0] || error));
  }

  try {
    await captureChatFlow(page, origin, request, characterId, sessionId, providerConfigured);
    await collectCspViolations(page, 'flow/chat');
  } catch (error) {
    note('聊天流截图异常（非致命）: ' + (error.message?.split('\n')[0] || error));
  }
  await sleep(2000);

  // 角色导入屏（13）与备份屏（22）：13 是重定向桩页，落到真实功能页后再截一帧。
  try {
    const importUrl = new URL(origin + '/screens/13-import-card.html');
    if (characterId) importUrl.searchParams.set('character', characterId);
    await page.goto(importUrl.href, { waitUntil: 'domcontentloaded' });
    await waitForScreenReady(page, '13-import-card.html');
    await shoot(page, path.join('flow', 'import-13.png'));
    const backupUrl = new URL(origin + '/screens/22-backup-restore.html');
    if (characterId) backupUrl.searchParams.set('character', characterId);
    await page.goto(backupUrl.href, { waitUntil: 'domcontentloaded' });
    await waitForScreenReady(page, '22-backup-restore.html');
    await shoot(page, path.join('flow', 'backup-22.png'));
    await collectCspViolations(page, 'flow/import+backup');
  } catch (error) {
    fail('角色导入/备份关键屏截图失败: ' + (error.message?.split('\n')[0] || error));
  }

  if (pageErrors.length > 0) fail(`pageerror 共 ${pageErrors.length} 个: ${pageErrors.slice(0, 5).join(' | ')}`);
  await context.close();
} finally {
  await browser.close();
  if (daemonChild) killProcessTree(daemonChild);
}

// ── 汇总 ──────────────────────────────────────────────────────────────────
const manifest = {
  mode: args.mode,
  origin,
  generatedAt: new Date().toISOString(),
  tokenCount: expectedTokens.size,
  tokenAssertions,
  failures: failures.length,
  notes,
};
writeFileSync(path.join(outDir, 'manifest.json'), JSON.stringify(manifest, null, 2));
console.log(`token assertions: ${tokenAssertions} (${expectedTokens.size} tokens × screens)`);
if (notes.length > 0) console.log(`notes: ${notes.length}`);
if (failures.length > 0) {
  console.error(`FAILED (${failures.length}):`);
  for (const message of failures) console.error('  - ' + message);
  process.exit(1);
}
console.log('webui screenshot suite passed');
