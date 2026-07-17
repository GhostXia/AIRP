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

// ── #115 Phase 2h: 6 个 revision 全部渲染 + unavailable 联动 ──────────────

test('buildAssemblyViewModel renders all 6 revisions when present', () => {
  const view = buildAssemblyViewModel({
    effective: {
      character_id: 'alice', character_revision: 3,
      persona_id: 'writer', persona_revision: 7,
      preset_id: 'balanced', preset_revision: 2,
      lorebook_revision: 5,
      state_revision: 42,
      memory_revision: 9,
      provider: 'openai_compatible', model: 'test-model',
    },
    total_estimated_tokens: 0,
    segments: [],
    diagnostics: [],
  });

  assert.equal(view.chips[0].value, 'alice · r3');        // 角色
  assert.equal(view.chips[1].value, 'writer · r7');       // 身份
  assert.equal(view.chips[2].value, 'balanced · r2');     // 预设
  assert.equal(view.chips[3].value, '世界书 · r5');        // 新增：世界书
  assert.equal(view.chips[4].value, '状态 · r42');         // 新增：状态
  assert.equal(view.chips[5].value, '记忆 · r9');          // 新增：记忆
  assert.equal(view.chips[6].value, 'test-model');         // 模型
  assert.equal(view.chips[7].value, 'openai_compatible'); // 服务
});

test('buildAssemblyViewModel marks chips as unavailable when revision diagnostic present', () => {
  const view = buildAssemblyViewModel({
    effective: {
      character_id: 'alice', // 但无 character_revision
      // 其他 revision 都缺失
    },
    total_estimated_tokens: 0,
    segments: [],
    diagnostics: [
      { kind: 'character_revision_unavailable', message: '角色卡未升级' },
      { kind: 'preset_revision_unavailable', message: 'Preset 未升级' },
      { kind: 'lorebook_revision_unavailable', message: 'Worldbook 未升级' },
      { kind: 'state_revision_unavailable', message: 'State 未升级' },
      { kind: 'memory_revision_unavailable', message: 'Memory 未升级' },
      // persona_revision_unavailable 故意缺，验证无诊断时不显示 unavailable
    ],
  });

  assert.equal(view.chips[0].value, 'alice · unavailable');     // 角色 + character_revision_unavailable
  assert.equal(view.chips[1].value, '未启用');                   // 身份，无诊断 → fallback
  assert.equal(view.chips[2].value, '未启用 · unavailable');    // 预设 + preset_revision_unavailable
  assert.equal(view.chips[3].value, '世界书 · unavailable');    // 世界书 + lorebook_revision_unavailable
  assert.equal(view.chips[4].value, '状态 · unavailable');      // 状态 + state_revision_unavailable
  assert.equal(view.chips[5].value, '记忆 · unavailable');      // 记忆 + memory_revision_unavailable
});

test('buildAssemblyViewModel hostile identifiers stay inert when unavailable', () => {
  const hostile = '<img src=x onerror="globalThis.pwned=1">';
  const view = buildAssemblyViewModel({
    effective: { character_id: hostile },
    segments: [],
    diagnostics: [{ kind: 'character_revision_unavailable', message: hostile }],
  });
  assert.equal(view.chips[0].value, hostile + ' · unavailable');
  assert.equal(globalThis.pwned, undefined);
});
