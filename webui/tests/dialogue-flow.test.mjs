import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFile } from 'node:fs/promises';

const require = createRequire(import.meta.url);
const { create, mesExample } = require('../assets/dialogue-flow.js');
const page = await readFile(new URL('../screens/39-dialogue-gen.html', import.meta.url), 'utf8');
const runtime = await readFile(new URL('../assets/dialogue-gen.js', import.meta.url), 'utf8');

test('mesExample reads v2 and legacy card shapes', () => {
  assert.equal(mesExample({ data: { mes_example: 'v2' } }), 'v2');
  assert.equal(mesExample({ mes_example: 'v1' }), 'v1');
  assert.equal(mesExample({ data: {} }), '');
});

test('dialogue UI delegates generation and confirmed writes to the tested flow', () => {
  assert.ok(
    page.indexOf('assets/dialogue-flow.js') < page.indexOf('assets/dialogue-gen.js'),
    'dialogue flow must load before the page runtime',
  );
  assert.match(runtime, /dialogueFlow\.generate\(characterId, body\)/);
  assert.match(runtime, /dialogueFlow\.writePreviewAndReload\(/);
  assert.match(runtime, /dry-run 预览只授权写回生成它的角色/);
  assert.match(runtime, /只有 dry-run 结果能成为后续一次性写入凭证/);
  assert.match(runtime, /lastGenerated = '';\s*written = true;/);
  assert.doesNotMatch(runtime, /client\.request\('POST', '\/v1\/characters\/' \+ encodeURIComponent\(characterId\) \+ '\/dialogue-examples'/);
});

test('backup copy keeps an explicit handler and field trace', () => {
  assert.match(
    runtime,
    /旧值已备份[\s\S]*engine\/src\/daemon\/handlers\/dialogue_gen\.rs[\s\S]*mes_example\.bak/,
  );
});

test('load -> dry-run -> write -> reload preserves the exact approved preview', async () => {
  const previewText = '<START>\n{{user}}: Hello\n{{char}}: Hi';
  const calls = [];
  const responses = [
    { data: { mes_example: 'old value' } },
    { written: false, character_id: 'alice', mes_example: previewText, turns_generated: 1, previous_mes_example: null },
    { written: true, character_id: 'alice', mes_example: previewText, turns_generated: 1, previous_mes_example: 'old value' },
    { data: { mes_example: previewText } },
  ];
  const client = {
    async request(method, path, body) {
      calls.push({ method, path, body });
      return responses.shift();
    },
  };
  const flow = create(client);

  const initial = await flow.load('alice');
  const preview = await flow.generate('alice', { turns: 1, dry_run: true, append: false });
  const saved = await flow.writePreviewAndReload('alice', preview, false);

  assert.equal(initial.mesExample, 'old value');
  assert.equal(saved.response.written, true);
  assert.equal(saved.current.mesExample, previewText);
  assert.deepEqual(calls, [
    { method: 'GET', path: '/v1/characters/alice', body: undefined },
    { method: 'POST', path: '/v1/characters/alice/dialogue-examples', body: { turns: 1, dry_run: true, append: false } },
    {
      method: 'POST',
      path: '/v1/characters/alice/dialogue-examples',
      body: { dry_run: false, append: false, mes_example_override: previewText },
    },
    { method: 'GET', path: '/v1/characters/alice', body: undefined },
  ]);
});

test('write refuses missing preview without issuing a request', async () => {
  let calls = 0;
  const flow = create({ request: async () => { calls += 1; } });
  await assert.rejects(
    flow.writePreviewAndReload('alice', { mes_example: '' }, false),
    /无可写入的预览内容/,
  );
  assert.equal(calls, 0);
});

test('write succeeds even when the post-write reload fails', async () => {
  // Regression: append-mode retry after a reload failure duplicated mes_example.
  // writePreviewAndReload must treat the write (POST) as authoritative and
  // only attempt the reload (GET) as best-effort verification. If the reload
  // fails, the caller must still see the write as committed so it does not
  // retry and append the same content a second time.
  const calls = [];
  const writeResponse = {
    written: true,
    character_id: 'alice',
    mes_example: '<START>\n{{user}}: Hi',
    turns_generated: 1,
    previous_mes_example: null,
  };
  const client = {
    async request(method, path, body) {
      calls.push({ method, path, body });
      if (method === 'POST') return writeResponse;
      // Simulate a transient reload failure (network glitch after write commit)
      throw new Error('reload failed');
    },
  };
  const flow = create(client);

  const result = await flow.writePreviewAndReload('alice', { mes_example: '<START>\n{{user}}: Hi' }, true);

  assert.equal(result.response.written, true);
  assert.equal(result.current, null);
  assert.deepEqual(calls, [
    { method: 'POST', path: '/v1/characters/alice/dialogue-examples', body: { dry_run: false, append: true, mes_example_override: '<START>\n{{user}}: Hi' } },
    { method: 'GET', path: '/v1/characters/alice', body: undefined },
  ]);
});
