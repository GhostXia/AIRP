import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { writeReport } from './reporter.mjs';

function createRun(consoleErrors) {
  return {
    runId: 'reporter-test',
    trigger: 'test',
    prNumber: null,
    startedAt: '2026-01-01T00:00:00.000Z',
    endedAt: '2026-01-01T00:00:01.000Z',
    llmModel: 'test-model',
    tasks: [{
      name: 'console-errors',
      result: 'Failed',
      description: 'Reporter truncation test',
      consoleErrors,
    }],
  };
}

test('reporter shows all console errors when the list has at most ten entries', async () => {
  const reportDir = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-reporter-'));
  try {
    const errors = Array.from({ length: 10 }, (_, index) => ({ message: `error-${index}` }));
    const { mdPath } = await writeReport(reportDir, createRun(errors));
    const markdown = await readFile(mdPath, 'utf8');

    for (const error of errors) assert.ok(markdown.includes(`- ${JSON.stringify(error)}`));
    assert.ok(!markdown.includes('仅显示前'));
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});

test('reporter indicates the total when console errors are truncated', async () => {
  const reportDir = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-reporter-'));
  try {
    const errors = Array.from({ length: 12 }, (_, index) => ({ message: `error-${index}` }));
    const { mdPath } = await writeReport(reportDir, createRun(errors));
    const markdown = await readFile(mdPath, 'utf8');

    for (const error of errors.slice(0, 10)) assert.ok(markdown.includes(`- ${JSON.stringify(error)}`));
    assert.ok(!markdown.includes(`- ${JSON.stringify(errors[10])}`));
    assert.ok(markdown.includes('_(共 12 条，仅显示前 10)_'));
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});
