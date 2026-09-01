import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import test from 'node:test';
import { terminateDetachedProcessGroup } from './process-group-cleanup.mjs';

function harness(onSignal) {
  let now = 0;
  return {
    now: () => now,
    sleep: async ms => { now += ms; },
    pollMs: 10,
    signalProcessGroup: (_pid, signal) => onSignal(signal),
  };
}

test('reports a process group that already exited normally', async () => {
  const outcome = await terminateDetachedProcessGroup(100, harness(() => {
    throw Object.assign(new Error('gone'), { code: 'ESRCH' });
  }));

  assert.equal(outcome, 'already-exited');
});

test('still terminates a live group after its wrapper has already exited', async () => {
  let alive = true;
  const signals = [];
  const outcome = await terminateDetachedProcessGroup(123, harness(signal => {
    signals.push(signal);
    if (signal === 0 && !alive) throw Object.assign(new Error('gone'), { code: 'ESRCH' });
    if (signal === 'SIGTERM') alive = false;
  }));

  assert.equal(outcome, 'terminated');
  assert.ok(signals.includes('SIGTERM'));
});

test('escalates a surviving group from TERM to KILL within bounded waits', async () => {
  let alive = true;
  const signals = [];
  const outcome = await terminateDetachedProcessGroup(456, {
    ...harness(signal => {
      signals.push(signal);
      if (signal === 0 && !alive) throw Object.assign(new Error('gone'), { code: 'ESRCH' });
      if (signal === 'SIGKILL') alive = false;
    }),
    termTimeoutMs: 20,
    killTimeoutMs: 20,
  });

  assert.equal(outcome, 'forced');
  assert.ok(signals.includes('SIGTERM'));
  assert.ok(signals.includes('SIGKILL'));
});

test('forces the full stable group after supervised TERM completion', async () => {
  let alive = true;
  let terminated = false;
  const signals = [];
  const outcome = await terminateDetachedProcessGroup(654, {
    ...harness(signal => {
      signals.push(signal);
      if (signal === 0 && !alive) throw Object.assign(new Error('gone'), { code: 'ESRCH' });
      if (signal === 'SIGTERM') terminated = true;
      if (signal === 'SIGKILL') alive = false;
    }),
    hasTerminated: () => terminated,
  });

  assert.equal(outcome, 'terminated-tree-forced');
  assert.ok(signals.includes('SIGTERM'));
  assert.ok(signals.includes('SIGKILL'));
});

test('fails with distinct TERM and KILL timeout evidence when the group survives', async () => {
  await assert.rejects(
    terminateDetachedProcessGroup(789, {
      ...harness(() => {}),
      termTimeoutMs: 20,
      killTimeoutMs: 30,
    }),
    /survived SIGTERM \(20 ms\) and SIGKILL \(30 ms\)/,
  );
});

test('terminates a real Unix group after its leader exits', {
  skip: process.platform === 'win32',
}, async t => {
  const wrapper = spawn('/bin/sh', ['-c', 'sleep 30 & exit 0'], {
    detached: true,
    stdio: 'ignore',
  });
  const pid = wrapper.pid;
  assert.ok(pid);
  let groupCleanupCompleted = false;
  t.after(() => {
    if (groupCleanupCompleted) return;
    try { process.kill(-pid, 'SIGKILL'); } catch (error) {
      if (error?.code !== 'ESRCH') throw error;
    }
  });
  await once(wrapper, 'exit');
  assert.equal(wrapper.exitCode, 0);

  const outcome = await terminateDetachedProcessGroup(pid, {
    termTimeoutMs: 1_000,
    killTimeoutMs: 1_000,
  });
  groupCleanupCompleted = true;
  assert.equal(outcome, 'terminated');
  assert.throws(
    () => process.kill(-pid, 0),
    error => error?.code === 'ESRCH',
  );
});

test('force-clears a real Unix group with a TERM-resistant descendant', {
  skip: process.platform === 'win32',
}, async t => {
  const leader = spawn('/bin/sh', ['-c',
    "trap '' TERM; /bin/sh -c \"trap '' TERM; exec sleep 30\" & child=$!; echo ready; wait $child",
  ], {
    detached: true,
    stdio: ['ignore', 'pipe', 'ignore'],
  });
  const pid = leader.pid;
  assert.ok(pid);
  let groupCleanupCompleted = false;
  t.after(() => {
    if (groupCleanupCompleted) return;
    try { process.kill(-pid, 'SIGKILL'); } catch (error) {
      if (error?.code !== 'ESRCH') throw error;
    }
  });
  let readyTimeout;
  let ready;
  try {
    ready = await Promise.race([
      once(leader.stdout, 'data').then(([chunk]) => chunk.toString().trim()),
      once(leader, 'error').then(([error]) => Promise.reject(error)),
      new Promise((_, reject) => {
        readyTimeout = setTimeout(
          () => reject(new Error('TERM-resistant fixture did not become ready within 1000 ms')),
          1_000,
        );
      }),
    ]);
  } finally {
    clearTimeout(readyTimeout);
  }
  assert.equal(ready, 'ready');

  const outcome = await terminateDetachedProcessGroup(pid, {
    termTimeoutMs: 100,
    killTimeoutMs: 1_000,
  });
  groupCleanupCompleted = true;
  assert.equal(outcome, 'forced');
  assert.throws(
    () => process.kill(-pid, 0),
    error => error?.code === 'ESRCH',
  );
});
