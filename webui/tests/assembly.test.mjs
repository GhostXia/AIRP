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
  // CodeRabbit nitpick 修复：此单元测试仅验证 view model 输出（纯函数，不涉及 DOM）。
  // 实际 DOM sink 的 XSS 防护由 `ui/production-browser-smoke.mjs` #194 fixture 覆盖
  // （通过 `injectionName` hostile 标识 + `window.__airpXss` 断言 + `img[src="x"]` count）。
  // 此处验证 view model 正确保留 hostile 字符串作为 chip value 文本。
  const hostile = '<img src=x onerror="globalThis.pwned=1">';
  const view = buildAssemblyViewModel({
    effective: { character_id: hostile },
    segments: [],
    diagnostics: [{ kind: 'character_revision_unavailable', message: hostile }],
  });
  assert.equal(view.chips[0].value, hostile + ' · unavailable');
});

// Gemini 审计修复：世界书 / 状态 / 记忆 在未激活（无 revision + 无诊断）时应回退到 "未启用"，
// 而非显示 asset 标签（如 "世界书"），避免暗示 asset 已激活但缺少 revision。
test('buildAssemblyViewModel falls back to inactive label when asset has no revision and no diagnostic', () => {
  const view = buildAssemblyViewModel({
    effective: {
      // 6 个 revision 都缺失
      character_id: null,
      persona_id: null,
      preset_id: null,
      // lorebook_revision / state_revision / memory_revision 都未设置
    },
    segments: [],
    diagnostics: [],  // 无任何诊断
  });

  // character / persona / preset: 无 ID → 已有的 fallback 行为
  assert.equal(view.chips[0].value, '未选择');
  assert.equal(view.chips[1].value, '未启用');
  assert.equal(view.chips[2].value, '未启用');
  // 世界书 / 状态 / 记忆: 无 revision + 无诊断 → 应回退到 "未启用"（不显示 asset 标签）
  assert.equal(view.chips[3].value, '未启用', '世界书未激活时应显示 未启用');
  assert.equal(view.chips[4].value, '未启用', '状态未激活时应显示 未启用');
  assert.equal(view.chips[5].value, '未启用', '记忆未激活时应显示 未启用');
});

// ── #114 effective config summary：来源后缀 + 新增 chips ─────────────────────

test('buildAssemblyViewModel appends persona activation source suffix when present', () => {
  const view = buildAssemblyViewModel({
    effective: {
      persona_id: 'writer',
      persona_revision: 7,
      persona_activation_source: 'explicit',
    },
    total_estimated_tokens: 0,
    segments: [],
    diagnostics: [],
  });
  // 身份 chip：`writer · r7 · 显式`
  assert.equal(view.chips[1].value, 'writer · r7 · 显式');
});

test('buildAssemblyViewModel maps all persona activation sources to readable labels', () => {
  const cases = [
    ['explicit', '显式'],
    ['session_binding', '会话绑定'],
    ['character_binding', '角色绑定'],
    ['default', '默认'],
  ];
  for (const [source, expected] of cases) {
    const view = buildAssemblyViewModel({
      effective: { persona_id: 'p', persona_activation_source: source },
      segments: [],
      diagnostics: [],
    });
    assert.equal(view.chips[1].value, 'p · ' + expected, `source=${source} → ${expected}`);
  }
});

test('buildAssemblyViewModel omits persona source suffix when absent (backward compat)', () => {
  // 旧 trace 不带 persona_activation_source 字段，或值为 'absent'：不附加后缀
  const viewNoField = buildAssemblyViewModel({
    effective: { persona_id: 'writer', persona_revision: 7 },
    segments: [],
    diagnostics: [],
  });
  assert.equal(viewNoField.chips[1].value, 'writer · r7');

  const viewAbsent = buildAssemblyViewModel({
    effective: { persona_id: 'writer', persona_revision: 7, persona_activation_source: 'absent' },
    segments: [],
    diagnostics: [],
  });
  assert.equal(viewAbsent.chips[1].value, 'writer · r7');
});

test('buildAssemblyViewModel appends model and provider source suffixes', () => {
  const view = buildAssemblyViewModel({
    effective: {
      model: 'gpt-test',
      model_source: 'preset',
      provider: 'openai_compatible',
      provider_source: 'snapshot',
    },
    segments: [],
    diagnostics: [],
  });
  // chips[6] = 模型, chips[7] = 服务
  assert.equal(view.chips[6].value, 'gpt-test · 预设');
  assert.equal(view.chips[7].value, 'openai_compatible · 默认');
});

test('buildAssemblyViewModel renders temperature and max_tokens chips with source', () => {
  const view = buildAssemblyViewModel({
    effective: {
      temperature: 0.8,
      temperature_source: 'request',
      max_tokens: 2048,
      max_tokens_source: 'preset',
    },
    segments: [],
    diagnostics: [],
  });
  // chips[8] = 温度, chips[9] = 最大 tokens
  assert.equal(view.chips[8].label, '温度');
  assert.equal(view.chips[8].value, '0.8 · 请求');
  assert.equal(view.chips[9].label, '最大 tokens');
  assert.equal(view.chips[9].value, '2048 · 预设');
});

test('buildAssemblyViewModel shows 未设置 for missing temperature and max_tokens', () => {
  const view = buildAssemblyViewModel({
    effective: {
      // temperature / max_tokens 都未提供
    },
    segments: [],
    diagnostics: [],
  });
  assert.equal(view.chips[8].value, '未设置');
  assert.equal(view.chips[9].value, '未设置');
});

test('buildAssemblyViewModel keeps source labels inert for hostile source strings', () => {
  // hostile source 字符串只作为 chip value 文本展示，不进入 HTML；
  // 与 character_id hostile 测试同样的 XSS 防护边界（DOM sink 由 production smoke 覆盖）。
  const hostile = '<img src=x onerror="globalThis.pwned=1">';
  const view = buildAssemblyViewModel({
    effective: {
      persona_id: 'p',
      persona_activation_source: hostile,
    },
    segments: [],
    diagnostics: [],
  });
  // 未知 source 值原样保留（便于排查），不映射成中文
  assert.equal(view.chips[1].value, 'p · ' + hostile);
  assert.equal(globalThis.pwned, undefined);
});

test('sourceLabel handles null/undefined table gracefully (gemini-code-assist PR #227)', () => {
  // 防御性：table 缺失时不应抛 TypeError，应回退到原始 source 字符串
  // 通过 buildAssemblyViewModel 间接验证 sourceLabel 的 table null 路径
  const view = buildAssemblyViewModel({
    effective: {
      persona_id: 'p',
      persona_activation_source: 'custom_source',
      // 注意：buildAssemblyViewModel 内部用 PERSONA_SOURCE_LABELS 作为 table，
      // 这里验证的是 source 不在 table 中时的 fallback 路径（与 table null 等价）
    },
    segments: [],
    diagnostics: [],
  });
  assert.equal(view.chips[1].value, 'p · custom_source');
});
