import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';

function fakeContext({ failTracingStart = false } = {}) {
  let pageClosed = false;
  let contextClosed = false;
  const page = {
    async goto() {},
    async close() {
      pageClosed = true;
    },
    async screenshot() {},
    async evaluate(fn) {
      const source = String(fn);
      if (source.includes('version === 2')) return true;
      if (source.includes('getDomSnapshot')) return [{ tag: 'body' }];
      if (source.includes('getConsoleErrors') || source.includes('getFailedRequests')) return [];
      return [];
    },
  };
  return {
    tracing: {
      async start() {
        if (failTracingStart) throw new Error('synthetic tracing.start failure');
      },
      async stop() {},
    },
    async newPage() {
      return page;
    },
    async close() {
      contextClosed = true;
    },
    get pageClosed() {
      return pageClosed;
    },
    get contextClosed() {
      return contextClosed;
    },
  };
}

test('newContext failure is a Failed task and does not skip the next task', async () => {
  const { runTasks } = await import(`./runner.mjs?f11-regression=${Date.now()}`);
  const reportDir = await mkdtemp(`${tmpdir()}/airp-agent-exploration-runner-`);
  let newContextCalls = 0;
  const contexts = [];
  const browser = {
    async newContext() {
      newContextCalls += 1;
      if (newContextCalls === 1) throw new Error('synthetic newContext failure');
      const context = fakeContext({ failTracingStart: true });
      contexts.push(context);
      return context;
    },
  };
  const taskModule = {
    DESCRIPTION: 'synthetic task',
    EXPECTED: 'task completes',
    async check() {
      return { ok: true };
    },
  };

  try {
    const tasks = await runTasks(
      browser,
      ['first-task', 'next-task'],
      { 'first-task': taskModule, 'next-task': taskModule },
      { origin: 'http://synthetic.test', reportDir },
    );

    assert.equal(newContextCalls, 2, 'the second task must still create a context');
    assert.deepEqual(tasks.map(task => task.name), ['first-task', 'next-task']);
    assert.equal(tasks[0].result, 'Failed');
    assert.match(tasks[0].actual, /synthetic newContext failure/);
    assert.equal(tasks[1].result, 'Failed');
    assert.match(tasks[1].actual, /synthetic tracing\.start failure/);
    assert.equal(contexts[0].pageClosed, false);
    assert.equal(contexts[0].contextClosed, true);
  } finally {
    await rm(reportDir, { recursive: true, force: true });
  }
});
