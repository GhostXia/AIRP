import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { canRunInfrastructureSmoke, classifyPrTasks, hasFailedTasks, runInfrastructureSmoke } from './runner.mjs';

const runnerPath = fileURLToPath(new URL('./runner.mjs', import.meta.url));
const INFRA_DIFF = [
  'diff --git a/.github/workflows/agent-browser-exploration.yml b/.github/workflows/agent-browser-exploration.yml',
  '+      - npm test',
  'diff --git a/tools/agent-exploration/runner.mjs b/tools/agent-exploration/runner.mjs',
  '+const mode = \'infrastructure-smoke\';',
].join('\n');

test('empty PR classification is a failure with a diagnostic', () => {
  const classification = classifyPrTasks('diff --git a/README.md b/README.md\n+documentation change', 322);

  assert.deepEqual(classification.taskNames, []);
  assert.match(classification.diagnostic, /triggered workflow but no task classified/i);
});

test('non-empty PR classification remains successful', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/engine/src/daemon/handlers/chat.rs\n+async fn swipe_chat';
  const classification = classifyPrTasks(diff, 322);

  assert.deepEqual(classification.taskNames, ['regen-swipe-refresh']);
  assert.equal(classification.diagnostic, null);
});

test('allowlisted empty PR classification can opt into infrastructure smoke', () => {
  const classification = classifyPrTasks(INFRA_DIFF, 322);

  assert.deepEqual(classification.taskNames, []);
  assert.equal(canRunInfrastructureSmoke(INFRA_DIFF, classification.taskNames, 322), true);
});

test('mixed empty PR classification cannot opt into infrastructure smoke', () => {
  const mixedDiff = INFRA_DIFF + '\n' + 'diff --git a/README.md b/README.md\n+documentation change';
  const classification = classifyPrTasks(mixedDiff, 322);

  assert.deepEqual(classification.taskNames, []);
  assert.equal(canRunInfrastructureSmoke(mixedDiff, classification.taskNames, 322), false);
});

test('a real Failed task remains a non-success result', () => {
  assert.equal(hasFailedTasks([{ name: 'regen-swipe-refresh', result: 'Failed' }]), true);
  assert.equal(hasFailedTasks([{ name: 'regen-swipe-refresh', result: 'Passed' }]), false);
});

function fakeInfrastructureBrowser() {
  let currentUrl = 'about:blank';
  const page = {
    async goto(url) {
      currentUrl = url;
    },
    url() {
      return currentUrl;
    },
    async close() {},
    async screenshot() {},
    async evaluate(fn) {
      const source = String(fn);
      if (source.includes('version === 2')) return true;
      if (source.includes('waitFor')) return true;
      if (source.includes('getDomSnapshot')) return [{ tag: 'body' }];
      if (source.includes('getConsoleErrors') || source.includes('getFailedRequests')) return [];
      return [];
    },
  };
  const context = {
    tracing: {
      async start() {},
      async stop() {},
    },
    async newPage() {
      return page;
    },
    async close() {},
  };
  return {
    async newContext() {
      return context;
    },
  };
}

test('infrastructure smoke uses the fixed builtin script and reports a non-business task', async () => {
  const reportDir = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-infra-'));
  try {
    const [result] = await runInfrastructureSmoke(fakeInfrastructureBrowser(), {
      origin: 'http://synthetic.test',
      reportDir,
    });
    assert.equal(result.name, 'infrastructure-smoke');
    assert.equal(result.result, 'Passed');
    assert.match(result.evidence.script, /agent-script\.mjs$/);
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});

test('empty PR exits non-successfully, writes a diagnostic report, and skips browser setup', async () => {
  const root = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-classification-'));
  const diffPath = join(root, 'empty.patch');
  const reportDir = join(root, 'report');
  await writeFile(diffPath, 'diff --git a/README.md b/README.md\n+documentation change\n');

  const env = { ...process.env };
  delete env.AIRP_CHROME_PATH;
  try {
    const result = spawnSync(
      process.execPath,
      [runnerPath, '--pr', '322', '--diff-file', diffPath, '--report-dir', reportDir],
      { encoding: 'utf8', env },
    );

    assert.equal(result.status, 1, result.stderr || result.stdout);
    assert.match(result.stderr, /triggered workflow but no task classified/i);

    const report = JSON.parse(await readFile(join(reportDir, 'report.json'), 'utf8'));
    assert.equal(report.status, 'Failed');
    assert.deepEqual(report.tasks, []);
    assert.match(report.diagnostic, /triggered workflow but no task classified/i);

    const markdown = await readFile(join(reportDir, 'report.md'), 'utf8');
    assert.match(markdown, /## Diagnostic/);
    assert.match(markdown, /triggered workflow but no task classified/i);
    assert.doesNotMatch(result.stderr, /AIRP_CHROME_PATH or --chrome-path is required/);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test('infrastructure-smoke mode rejects an empty classification with a mixed path diff', async () => {
  const root = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-infra-reject-'));
  const diffPath = join(root, 'mixed.patch');
  const reportDir = join(root, 'report');
  const mixedDiff = INFRA_DIFF + '\n' + 'diff --git a/README.md b/README.md\n+documentation change\n';
  await writeFile(diffPath, mixedDiff);

  const env = { ...process.env };
  delete env.AIRP_CHROME_PATH;
  try {
    const result = spawnSync(
      process.execPath,
      [runnerPath, '--pr', '322', '--mode', 'infrastructure-smoke', '--diff-file', diffPath, '--report-dir', reportDir],
      { encoding: 'utf8', env },
    );
    assert.equal(result.status, 1, result.stderr || result.stdout);
    assert.match(result.stderr, /all changed paths.*infrastructure allowlist/i);
    const report = JSON.parse(await readFile(join(reportDir, 'report.json'), 'utf8'));
    assert.equal(report.status, 'Failed');
    assert.match(report.diagnostic, /infrastructure allowlist/i);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
