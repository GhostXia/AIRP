import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { classifyPrTasks } from './runner.mjs';

const runnerPath = fileURLToPath(new URL('./runner.mjs', import.meta.url));

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
