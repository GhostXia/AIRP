import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CANCEL_CLEANUP_GRACE_MS,
  clampTimeout,
  readResponseBodyBounded,
  remainingMs,
  responseSucceeded,
} from './bounded-http.mjs';

function timerHarness() {
  const timers = [];
  const cleared = [];
  return {
    timers,
    cleared,
    setTimer: (fn, ms) => {
      const id = { fn, ms };
      timers.push(id);
      return id;
    },
    clearTimer: (id) => cleared.push(id),
  };
}

function assertAllTimersCleared(timers) {
  assert.equal(timers.cleared.length, timers.timers.length, 'all timers are cleared');
  assert.deepEqual(new Set(timers.cleared), new Set(timers.timers));
}

test('deadline helpers clamp requests and sleeps to the remaining budget', () => {
  let now = 40;
  assert.equal(remainingMs(100, () => now), 60);
  assert.equal(clampTimeout(100, 250, () => now), 60);
  assert.equal(clampTimeout(100, 25, () => now), 25);
  now = 100;
  assert.equal(remainingMs(100, () => now), 0);
  assert.equal(clampTimeout(100, 25, () => now), 0);
});

test('bounded reader drains a complete response, decodes split UTF-8, and releases the lock', async () => {
  const encoder = new TextEncoder();
  const bytes = encoder.encode('前缀✓');
  const split = bytes.length - 1;
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(bytes.slice(0, split));
      controller.enqueue(bytes.slice(split));
      controller.close();
    },
  });
  const response = new Response(stream, { status: 200 });
  const timers = timerHarness();
  const result = await readResponseBodyBounded(response, {
    deadline: 100,
    now: () => 0,
    ...timers,
  });

  assert.equal(result.complete, true);
  assert.equal(result.text, '前缀✓');
  assert.equal(result.bytes, bytes.byteLength);
  assert.equal(response.body.locked, false);
  assertAllTimersCleared(timers);
  assert.equal(responseSucceeded(response, result), true);
});

test('a 2xx response with a hanging body fails closed at the deadline and releases the lock', async () => {
  const stream = new ReadableStream({ start() {} });
  const response = new Response(stream, { status: 200 });
  const timers = timerHarness();
  let timeout;
  const pending = readResponseBodyBounded(response, {
    deadline: 50,
    now: () => 0,
    ...timers,
    setTimer: (fn, ms) => {
      const id = timers.setTimer(fn, ms);
      timeout = id;
      return id;
    },
  });
  assert.equal(typeof timeout?.fn, 'function');
  timeout.fn();
  const result = await pending;

  assert.equal(result.complete, false);
  assert.equal(result.timedOut, true);
  assert.equal(responseSucceeded(response, result), false);
  assert.equal(response.status, 200, 'original status is retained for diagnostics');
  assert.equal(response.body.locked, false);
  assertAllTimersCleared(timers);
});

test('oversized bodies fail closed, cancel, and release the lock', async () => {
  const stream = new ReadableStream({
    start(controller) {
      controller.enqueue(new Uint8Array([1, 2, 3, 4, 5]));
    },
  });
  const response = new Response(stream, { status: 200 });
  const timers = timerHarness();
  const result = await readResponseBodyBounded(response, {
    deadline: 100,
    now: () => 0,
    maxBytes: 4,
    ...timers,
  });

  assert.equal(result.complete, false);
  assert.equal(result.tooLarge, true);
  assert.equal(result.bytes, 5);
  assert.match(result.error, /4 byte limit/);
  assert.equal(response.body.locked, false);
  assertAllTimersCleared(timers);
});

test('reader transport rejection fails closed and releases the lock', async () => {
  const stream = new ReadableStream({
    pull(controller) {
      controller.error(new Error('synthetic body failure'));
    },
  });
  const response = new Response(stream, { status: 200 });
  const timers = timerHarness();
  const result = await readResponseBodyBounded(response, {
    deadline: 100,
    now: () => 0,
    ...timers,
  });

  assert.equal(result.complete, false);
  assert.equal(result.transportError, true);
  assert.match(result.error, /synthetic body failure/);
  assert.equal(response.body.locked, false);
  assertAllTimersCleared(timers);
});

test('a response without a reader fails closed without falling back to unbounded text()', async () => {
  let textCalled = false;
  const result = await readResponseBodyBounded({
    status: 200,
    ok: true,
    body: {},
    text: async () => {
      textCalled = true;
      return 'must not be consumed';
    },
  }, { deadline: 100, now: () => 0 });

  assert.equal(result.complete, false);
  assert.equal(result.unsupported, true);
  assert.match(result.error, /reader unavailable/);
  assert.equal(textCalled, false);
});

test('a real 204 null-body response is a complete empty body', async () => {
  const response = new Response(null, { status: 204 });
  const result = await readResponseBodyBounded(response, {
    deadline: 100,
    now: () => 0,
  });

  assert.equal(response.body, null);
  assert.equal(result.complete, true);
  assert.equal(result.text, '');
  assert.equal(result.bytes, 0);
  assert.equal(result.unsupported, false);
  assert.equal(result.lockReleased, true);
  assert.equal(responseSucceeded(response, result), true);
});

test('a never-resolving cancel is bounded by cleanup grace and still releases the lock', async () => {
  const body = {
    locked: false,
    getReader() {
      body.locked = true;
      return {
        read: () => new Promise(() => {}),
        cancel: () => new Promise(() => {}),
        releaseLock: () => { body.locked = false; },
      };
    },
  };
  const response = { body };
  let timer;
  let readyResolve;
  const ready = new Promise(resolve => { readyResolve = resolve; });
  const cleared = [];
  const pending = readResponseBodyBounded(response, {
    deadline: 0,
    now: () => 0,
    setTimer: (fn, ms) => {
      timer = { fn, ms };
      readyResolve();
      return timer;
    },
    clearTimer: (id) => cleared.push(id),
  });
  await ready;
  assert.equal(timer.ms, CANCEL_CLEANUP_GRACE_MS);
  timer.fn();
  const result = await pending;

  assert.equal(result.complete, false);
  assert.equal(result.timedOut, true);
  assert.equal(result.cleanupIncomplete, true);
  assert.match(result.cleanupError, /cleanup grace/);
  assert.equal(result.lockReleased, true);
  assert.equal(body.locked, false);
  assert.deepEqual(cleared, [timer]);
});
