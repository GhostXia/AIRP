import test from 'node:test';
import assert from 'node:assert/strict';
import { clampTimeout, readResponseBodyBounded, remainingMs } from './bounded-http.mjs';

test('deadline helpers clamp requests and sleeps to the remaining budget', () => {
  let now = 40;
  assert.equal(remainingMs(100, () => now), 60);
  assert.equal(clampTimeout(100, 250, () => now), 60);
  assert.equal(clampTimeout(100, 25, () => now), 25);
  now = 100;
  assert.equal(remainingMs(100, () => now), 0);
  assert.equal(clampTimeout(100, 25, () => now), 0);
});
test('bounded reader drains a complete response without leaking timer state', async () => {
  const timers = [];
  const cleared = [];
  const response = {
    body: new ReadableStream({
      start(controller) {
        controller.enqueue(new TextEncoder().encode('bad '));
        controller.enqueue(new TextEncoder().encode('gateway'));
        controller.close();
      },
    }),
  };
  const result = await readResponseBodyBounded(response, {
    deadline: 100,
    now: () => 0,
    setTimer: (fn, ms) => {
      const id = { fn, ms };
      timers.push(id);
      return id;
    },
    clearTimer: (id) => cleared.push(id),
  });

  assert.deepEqual(result, { text: 'bad gateway', timedOut: false });
  assert.equal(timers.length, 3, 'one bounded timer per reader.read()');
  assert.deepEqual(cleared, timers, 'all bounded timers are cleared');
});

test('bounded reader cancels a pending response at the deadline', async () => {
  let timeout;
  let cancelled = false;
  let aborted = false;
  const response = {
    body: {
      getReader() {
        return {
          read: () => new Promise(() => {}),
          cancel: () => {
            cancelled = true;
            return Promise.resolve();
          },
        };
      },
    },
  };
  const pending = readResponseBodyBounded(response, {
    deadline: 50,
    now: () => 0,
    setTimer: (fn, ms) => {
      assert.equal(ms, 50);
      timeout = fn;
      return 1;
    },
    clearTimer: () => {},
    onTimeout: () => { aborted = true; },
  });
  assert.equal(typeof timeout, 'function');
  timeout();
  const result = await pending;

  assert.deepEqual(result, { text: '', timedOut: true });
  assert.equal(cancelled, true);
  assert.equal(aborted, true);
});
