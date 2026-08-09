import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runTasks } from './runner.mjs';
import { writeReport } from './reporter.mjs';

function fakeBrowser() {
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
      if (source.includes('getConsoleErrors') || source.includes('getFailedRequests')) return [];
      if (source.includes('getDomSnapshot')) return [{ tag: 'body' }];
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

function makeReport(tasks) {
  const now = new Date().toISOString();
  return {
    runId: 'deterministic-test',
    trigger: 'manual',
    prNumber: null,
    startedAt: now,
    endedAt: now,
    llmModel: 'test',
    tasks,
  };
}

test('deterministic task executes module.run and writes task result to report', async () => {
  const reportDir = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-deterministic-'));
  let runCalls = 0;
  const taskModule = {
    DETERMINISTIC: true,
    DESCRIPTION: 'deterministic synthetic preview task',
    EXPECTED: 'module run is called',
    async run(ctx) {
      runCalls += 1;
      assert.equal(typeof ctx.apiCall, 'function');
    },
    async check() {
      return { ok: true };
    },
  };

  try {
    const tasks = await runTasks(
      fakeBrowser(),
      ['preview-chat-assembly'],
      { 'preview-chat-assembly': taskModule },
      { origin: 'http://synthetic.test', reportDir },
    );
    assert.equal(runCalls, 1, 'deterministic module.run(ctx) must be called exactly once');
    assert.equal(tasks[0].name, 'preview-chat-assembly');
    assert.equal(tasks[0].result, 'Passed');
    assert.equal(tasks[0].evidence.execution, 'deterministic task module');

    await writeReport(reportDir, makeReport(tasks));
    const report = JSON.parse(await readFile(join(reportDir, 'report.json'), 'utf8'));
    assert.equal(report.tasks[0].name, 'preview-chat-assembly');
    assert.equal(report.tasks[0].result, 'Passed');
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});

test('deterministic assertion failure becomes Failed task result', async () => {
  const reportDir = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-deterministic-fail-'));
  let runCalls = 0;
  const taskModule = {
    DETERMINISTIC: true,
    DESCRIPTION: 'deterministic failing task',
    EXPECTED: 'synthetic assertion fails closed',
    async run() {
      runCalls += 1;
      throw new Error('ASSERT: synthetic preview invariant failed');
    },
    async check() {
      return { ok: true };
    },
  };

  try {
    const [task] = await runTasks(
      fakeBrowser(),
      ['preview-chat-assembly'],
      { 'preview-chat-assembly': taskModule },
      { origin: 'http://synthetic.test', reportDir },
    );
    assert.equal(runCalls, 1, 'failing deterministic module.run(ctx) must be called');
    assert.equal(task.name, 'preview-chat-assembly');
    assert.equal(task.result, 'Failed');
    assert.match(task.actual, /synthetic preview invariant failed/);
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});

test('ordinary task without deterministic metadata keeps generated-script path', async () => {
  const reportDir = await mkdtemp(join(tmpdir(), 'airp-agent-exploration-ordinary-'));
  let directRunCalls = 0;
  const taskModule = {
    DESCRIPTION: 'ordinary synthetic task',
    EXPECTED: 'fallback script runs',
    async run() {
      directRunCalls += 1;
    },
    async check() {
      return { ok: true };
    },
  };

  try {
    const [task] = await runTasks(
      fakeBrowser(),
      ['ordinary-task'],
      { 'ordinary-task': taskModule },
      { origin: 'http://synthetic.test', reportDir },
    );
    assert.equal(directRunCalls, 0, 'ordinary task must not bypass generated-script path');
    assert.equal(task.result, 'Passed', task.actual || 'ordinary task failed without actual details');
    assert.match(task.evidence.script, /agent-script\.mjs$/);
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});
