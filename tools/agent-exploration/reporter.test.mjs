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

test('reporter labels infrastructure smoke separately from business tasks', async () => {
  const reportDir = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-reporter-infra-'));
  try {
    const run = {
      runId: 'reporter-infra-test',
      trigger: 'pr-322',
      prNumber: 322,
      mode: 'infrastructure-smoke',
      startedAt: '2026-01-01T00:00:00.000Z',
      endedAt: '2026-01-01T00:00:01.000Z',
      llmModel: 'builtin-smoke (no LLM)',
      tasks: [{
        name: 'infrastructure-smoke',
        result: 'Passed',
        description: 'Topology smoke',
        expected: 'Smoke completes',
      }],
    };
    const { mdPath } = await writeReport(reportDir, run);
    const markdown = await readFile(mdPath, 'utf8');
    assert.match(markdown, /- Mode: infrastructure-smoke/);
    assert.match(markdown, /## Task: infrastructure-smoke/);
    assert.doesNotMatch(markdown, /regen-swipe-refresh|memory-roundtrip/);
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});
