import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { reextractCharacterResources } = require('../assets/workbench-reextract.js');

function setup(overrides = {}) {
  const requests = [];
  const statuses = [];
  const button = { disabled: false };
  return {
    requests,
    statuses,
    button,
    options: {
      characterId: 'alice/bob',
      button,
      confirm: () => true,
      request: async (...args) => {
        requests.push(args);
        return { status: 'ok' };
      },
      setStatus: (...args) => statuses.push(args),
      errorMessage: error => error.message,
      ...overrides,
    },
  };
}

test('cancelled reextract sends no request', async () => {
  let prompt = '';
  const fixture = setup({
    confirm: text => {
      prompt = text;
      return false;
    },
  });

  const outcome = await reextractCharacterResources(fixture.options);

  assert.equal(outcome.status, 'cancelled');
  assert.match(prompt, /问候语/);
  assert.match(prompt, /世界书/);
  assert.match(prompt, /覆盖/);
  assert.deepEqual(fixture.requests, []);
  assert.equal(fixture.button.disabled, false);
  assert.deepEqual(fixture.statuses, [['已取消重新提取']]);
});

test('confirmed reextract disables the button and reports success', async () => {
  const fixture = setup({
    request: async (...args) => {
      assert.equal(fixture.button.disabled, true);
      fixture.requests.push(args);
      return { status: 'ok' };
    },
  });

  const outcome = await reextractCharacterResources(fixture.options);

  assert.equal(outcome.status, 'completed');
  assert.deepEqual(fixture.requests, [
    ['POST', '/v1/characters/alice%2Fbob/reextract'],
  ]);
  assert.equal(fixture.button.disabled, false);
  assert.deepEqual(fixture.statuses.at(-1), ['附属资源重新提取完成']);
});

test('reextract distinguishes a definite failure from an unknown commit state', async () => {
  const failed = setup({
    request: async () => { throw new Error('server rejected request'); },
  });
  const failedOutcome = await reextractCharacterResources(failed.options);
  assert.equal(failedOutcome.status, 'failed');
  assert.deepEqual(failed.statuses.at(-1), ['重新提取失败：server rejected request', true]);
  assert.equal(failed.button.disabled, false);

  const unknown = setup({
    request: async () => { throw new TypeError('network disconnected'); },
  });
  const unknownOutcome = await reextractCharacterResources(unknown.options);
  assert.equal(unknownOutcome.status, 'unknown');
  assert.match(unknown.statuses.at(-1)[0], /提交状态未知/);
  assert.equal(unknown.statuses.at(-1)[1], true);
  assert.equal(unknown.button.disabled, false);
});
