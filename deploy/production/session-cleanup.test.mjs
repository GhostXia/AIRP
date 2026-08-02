import test from 'node:test';
import assert from 'node:assert/strict';
import { classifySessionPhase, deleteSessionWithRetry } from './session-cleanup.mjs';

function deterministicClock() {
  const clock = { value: 0 };
  const delays = [];
  return {
    now: () => clock.value,
    delays,
    sleep: async (ms) => {
      delays.push(ms);
      clock.value += ms;
    },
  };
}

test('cleanup retries a 409 immediately only after an explicit idle phase', async () => {
  const cases = [
    { name: 'idle', state: { status: 200, ok: true, data: { phase: 'idle' } }, phase: 'idle', expectSleep: false },
    { name: 'generating', state: { status: 200, ok: true, data: { phase: 'generating' } }, phase: 'generating', expectSleep: true },
    { name: 'committing', state: { status: 200, ok: true, data: { phase: 'committing' } }, phase: 'committing', expectSleep: true },
    { name: 'recovering', state: { status: 200, ok: true, data: { phase: 'recovering' } }, phase: 'recovering', expectSleep: true },
    { name: 'unknown', state: { status: 200, ok: true, data: { phase: 'future-phase' } }, phase: 'unknown', expectSleep: true },
    { name: 'missing', state: { status: 200, ok: true, data: {} }, phase: 'unknown', expectSleep: true },
    { name: 'probe failure', state: { status: 503, ok: false, data: null }, phase: 'probe-failure', expectSleep: true },
  ];

  for (const scenario of cases) {
    assert.equal(classifySessionPhase(scenario.state), scenario.phase, scenario.name);
    const clock = deterministicClock();
    const deletes = [];
    const states = [];
    const result = await deleteSessionWithRetry({
      timeoutMs: 1000,
      now: clock.now,
      sleep: clock.sleep,
      deleteAttempt: async (remainingMs) => {
        deletes.push(remainingMs);
        return deletes.length === 1 ? { status: 409, ok: false } : { status: 204, ok: true };
      },
      stateAttempt: async (remainingMs) => {
        states.push(remainingMs);
        return scenario.state;
      },
    });

    assert.equal(result.ok, true, scenario.name);
    assert.deepEqual(deletes, scenario.expectSleep ? [1000, 800] : [1000, 1000], scenario.name);
    assert.deepEqual(states, scenario.expectSleep ? [1000] : [1000], scenario.name);
    if (scenario.expectSleep) {
      assert.deepEqual(clock.delays, [200], scenario.name);
    } else {
      assert.deepEqual(clock.delays, [], scenario.name);
    }
  }
});
test('cleanup keeps the absolute deadline for continuous 409 responses', async () => {
  const clock = deterministicClock();
  const deletes = [];
  const states = [];
  const result = await deleteSessionWithRetry({
    timeoutMs: 450,
    now: clock.now,
    sleep: clock.sleep,
    deleteAttempt: async (remainingMs) => {
      deletes.push(remainingMs);
      return { status: 409, ok: false };
    },
    stateAttempt: async (remainingMs) => {
      states.push(remainingMs);
      return { status: 200, ok: true, data: { phase: 'committing' } };
    },
  });

  assert.equal(result.ok, false);
  assert.equal(result.status, 409);
  assert.equal(result.deadlineExceeded, true);
  assert.deepEqual(clock.delays, [200, 200, 50]);
  assert.deepEqual(deletes, [450, 250, 50]);
  assert.deepEqual(states, [450, 250, 50]);
  assert.equal(result.lastPhase, 'committing');
  assert.deepEqual(result.lastState.data, { phase: 'committing' });
});

test('cleanup reports probe diagnostics when the deadline expires during a bounded wait', async () => {
  const clock = deterministicClock();
  const result = await deleteSessionWithRetry({
    timeoutMs: 100,
    now: clock.now,
    sleep: clock.sleep,
    deleteAttempt: async () => ({ status: 409, ok: false, text: 'busy' }),
    stateAttempt: async () => ({ status: 502, ok: false, text: 'probe unavailable' }),
  });

  assert.equal(result.deadlineExceeded, true);
  assert.equal(result.text, 'busy');
  assert.equal(result.lastPhase, null);
  assert.equal(result.lastState.text, 'probe unavailable');
  assert.deepEqual(clock.delays, [100]);
});
