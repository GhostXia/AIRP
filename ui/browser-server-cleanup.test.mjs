import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import test from 'node:test';
import { closeOwnedBrowser } from './browser-server-cleanup.mjs';

function fakeProcess() {
  return Object.assign(new EventEmitter(), { exitCode: null, signalCode: null });
}

test('force-kills owned Chromium after graceful close times out', async () => {
  const child = fakeProcess();
  let killed = false;
  const outcome = await closeOwnedBrowser({ close: () => new Promise(() => {}) }, {
    process: () => child,
    close: async () => {},
    kill: async () => {
      killed = true;
      setTimeout(() => {
        child.signalCode = 'SIGKILL';
        child.emit('exit', null, 'SIGKILL');
      }, 20);
    },
  }, 10);
  assert.equal(outcome, 'forced');
  assert.equal(killed, true);
  assert.equal(child.signalCode, 'SIGKILL');
});

test('preserves graceful and forced Chromium cleanup failures', async () => {
  const closeError = new Error('close failed');
  const killError = new Error('kill failed');
  await assert.rejects(
    closeOwnedBrowser({ close: async () => { throw closeError; } }, {
      process: () => fakeProcess(),
      close: async () => {},
      kill: async () => { throw killError; },
    }, 10),
    error => error instanceof AggregateError
      && error.errors[0] === closeError
      && error.errors[1] === killError,
  );
});
