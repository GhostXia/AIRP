import test from 'node:test';
import assert from 'node:assert/strict';
import { deleteSessionWithRetry } from './session-cleanup.mjs';

test('cleanup retries a 409 after the session becomes idle', async () => {
  const deletes = [];
  const states = [];
  const result = await deleteSessionWithRetry({
    timeoutMs: 1000,
    deleteAttempt: async () => {
      const response = deletes.length === 0
        ? { status: 409, ok: false }
        : { status: 200, ok: true };
      deletes.push(response.status);
      return response;
    },
    stateAttempt: async () => {
      states.push('idle');
      return { status: 200, ok: true, data: { phase: 'idle' } };
    },
    sleep: async () => { throw new Error('idle cleanup should retry without a busy wait'); },
  });
  assert.equal(result.ok, true);
  assert.deepEqual(deletes, [409, 200]);
  assert.deepEqual(states, ['idle']);
});

test('cleanup waits in bounded slices while generation is busy', async () => {
  const delays = [];
  let attempts = 0;
  const result = await deleteSessionWithRetry({
    timeoutMs: 1000,
    deleteAttempt: async () => {
      attempts++;
      return attempts < 2 ? { status: 409, ok: false } : { status: 200, ok: true };
    },
    stateAttempt: async () => ({ status: 200, ok: true, data: { phase: 'committing' } }),
    sleep: async (ms) => delays.push(ms),
  });
  assert.equal(result.ok, true);
  assert.equal(attempts, 2);
  assert.equal(delays.length, 1);
  assert.ok(delays[0] >= 1 && delays[0] <= 200);
});
