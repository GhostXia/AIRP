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
import { chatCompletion, getModel, FALLBACK_MODE, getBuiltinSmokeScript } from './llm-client.mjs';
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
    // ctx 合并 fixtures：任务模块通过 mod.FIXTURES 提供解析好的 fixture JSON，
    // Agent 脚本通过 ctx.fixtures 直接取用，不需要读 runner 文件系统。
    const ctx = { page, harness, context, origin: ORIGIN, fixtures: mod.FIXTURES || {} };
    const scriptPath = await generateAndRunScript(mod, ctx, taskDir);
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

async function generateAndRunScript(mod, ctx, taskDir) {
  // B9 方案 3：fallback 模式下（OPENAI_API_KEY 未配），不调 LLM、不重试，
  // 直接用 llm-client 内置的 minimal smoke 脚本。该脚本只 navigate + snapshot，
  // 不做业务断言，目的是验证 topology + harness + runner + reporter 全链路可达。
  // fallback 模式下所有任务集跑同一份脚本（任务差异由 reporter 的 task.name 体现）。
  if (FALLBACK_MODE) {
    const scriptContent = getBuiltinSmokeScript();
    const scriptPath = join(taskDir, 'agent-script.mjs');
    await writeFile(scriptPath, scriptContent);
    const exitCode = await runTempScript(scriptPath, ctx);
    if (exitCode === 0) return scriptPath;
    throw new Error('fallback smoke script failed (exit code ' + exitCode + '); topology/harness is broken, not WebUI business logic');
  }

  // 1. 构造 prompt（DOM 快照脱敏后再注入）
  const domSnapshot = await ctx.harness.getDomSnapshot().catch(() => []);
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
      const exitCode = await runTempScript(scriptPath, ctx);
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
  // 仅在任务模块声明了 FIXTURES 时，告诉 Agent fixture JSON 已在 ctx.fixtures 中，
  // 直接用即可，不要读文件。无 FIXTURES 的任务不附加此说明。
  const fixtureNote = mod.FIXTURES
    ? '\n\nFixtures: ctx.fixtures.characterCard is the parsed character card JSON. Use it directly in the POST /v1/characters/import body as { character_id, card_json }. Do NOT read files.'
    : '';
  return {
    system: `You are an AIRP WebUI exploratory test generator. Output ONLY a single JavaScript code block (no prose) that exports an async function:
export async function run(ctx) { /* ctx = { page, harness, origin, fixtures } */ }

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
${fixtureNote}

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
