import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { buildAssemblyViewModel } = require('../assembly-utils.js');

test('buildAssemblyViewModel presents effective revisions and ordered materials', () => {
  const view = buildAssemblyViewModel({
    effective: {
      character_id: 'alice',
      persona_id: 'writer',
      persona_revision: 7,
      preset_id: 'balanced',
      provider: 'openai_compatible',
      model: 'test-model',
    },
    total_estimated_tokens: 15,
    segments: [
      { source_kind: 'card', chars: 40, estimated_tokens: 10, stable_or_volatile: 'stable' },
      { source_kind: 'user', chars: 20, estimated_tokens: 5, stable_or_volatile: 'volatile' },
    ],
    diagnostics: [],
  });

  assert.equal(view.chips[1].value, 'writer · r7');
  assert.equal(view.metrics, '2 项 · 约 15 tokens');
  assert.deepEqual(view.segments.map(segment => segment.label), ['角色卡', '本轮消息']);
  assert.deepEqual(view.segments.map(segment => segment.stabilityClass), ['stable', 'volatile']);
});

test('buildAssemblyViewModel keeps hostile identifiers as inert display data', () => {
  const hostile = '<img src=x onerror="globalThis.pwned=1">';
  const view = buildAssemblyViewModel({
    effective: { character_id: hostile },
    segments: [{ source_kind: 'card', source_id: hostile }],
    diagnostics: [{ message: hostile }],
  });

  assert.equal(view.chips[0].value, hostile);
  assert.equal(view.segments[0].identity, hostile);
  assert.equal(view.diagnostics[0], hostile);
  assert.equal(globalThis.pwned, undefined);
});

test('buildAssemblyViewModel rejects absent trace', () => {
  assert.equal(buildAssemblyViewModel(null), null);
});
