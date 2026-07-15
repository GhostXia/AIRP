// #114 C-PR1: Persona WebUI 闭环纯函数测试
// 运行：node --test webui/tests/persona.test.mjs
// 用 createRequire 导入 app.js 的 CommonJS exports（app.js 顶层 IIFE 在 Node
// 无 document 时会抛错，但纯函数在 module.exports 前定义，createRequire 只取
// exports 对象——实际 app.js IIFE 在无 document 时会 ReferenceError，所以这里
// 改为直接复制纯函数合同测试，不 import app.js，避免 Node 无 DOM 报错）。
//
// 注意：这些测试与 app.js 顶层的 describeEffectiveHint/buildBindAction/
// buildPersonaPayload 保持合同一致；若 app.js 纯函数改动，需同步更新此处。

import { test } from 'node:test';
import assert from 'node:assert/strict';

// ── 与 app.js 顶层纯函数保持一致的合同副本 ─────────────────────────────────────
// 复制而非 import，因为 app.js 的 IIFE 在 Node 无 document 时会抛错。
// 维护约定：app.js 顶层纯函数改动时，此处同步更新。

function describeEffectiveHint(selectedPersonaId, effectivePersona) {
  if (selectedPersonaId !== '') {
    return '已选择：' + selectedPersonaId + '（explicit）';
  }
  if (!effectivePersona || !effectivePersona.persona) return '—';
  const name = effectivePersona.persona.name || 'User';
  switch (effectivePersona.source) {
    case 'session_binding': return '生效：' + name + '（来自会话绑定）';
    case 'character_binding': return '生效：' + name + '（来自角色绑定）';
    case 'default': return '生效：' + name + '（默认）';
    default: return '—';
  }
}

function buildBindAction(state, scope) {
  const { selectedPersonaId, selectedChar, selectedSess, effectivePersona } = state;
  if (!selectedChar) return null;
  if (selectedPersonaId === '') return null;
  if (!effectivePersona) return null;
  if (scope === 'session' && !selectedSess) return null;
  const owner = scope === 'character'
    ? (effectivePersona.bindings && effectivePersona.bindings.character_persona_id)
    : (effectivePersona.bindings && effectivePersona.bindings.session_persona_id);
  if (!owner) {
    return { kind: 'bind', personaId: selectedPersonaId, label: scope === 'character' ? '绑定到角色' : '绑定到会话' };
  }
  if (owner === selectedPersonaId) {
    return { kind: 'unbind', personaId: selectedPersonaId, label: scope === 'character' ? '解绑角色' : '解绑会话' };
  }
  return { kind: 'unbind', personaId: owner, label: '先解绑 ' + owner };
}

function buildPersonaPayload(userId, selectedPersonaId) {
  const payload = { user_id: userId };
  if (selectedPersonaId) payload.persona_id = selectedPersonaId;
  return payload;
}

// ── 测试 ───────────────────────────────────────────────────────────────────

test('describeEffectiveHint: explicit persona 显示已选择', () => {
  assert.equal(
    describeEffectiveHint('writer', null),
    '已选择：writer（explicit）'
  );
});

test('describeEffectiveHint: 自动 + session_binding', () => {
  const eff = { persona: { name: 'Roleplay' }, source: 'session_binding' };
  assert.equal(
    describeEffectiveHint('', eff),
    '生效：Roleplay（来自会话绑定）'
  );
});

test('describeEffectiveHint: 自动 + character_binding', () => {
  const eff = { persona: { name: 'Writer' }, source: 'character_binding' };
  assert.equal(
    describeEffectiveHint('', eff),
    '生效：Writer（来自角色绑定）'
  );
});

test('describeEffectiveHint: 自动 + default', () => {
  const eff = { persona: { name: 'User' }, source: 'default' };
  assert.equal(
    describeEffectiveHint('', eff),
    '生效：User（默认）'
  );
});

test('describeEffectiveHint: 自动 + 无 effective → —', () => {
  assert.equal(describeEffectiveHint('', null), '—');
});

test('buildBindAction: 无 character → null', () => {
  const state = { selectedPersonaId: 'writer', selectedChar: '', selectedSess: '', effectivePersona: { bindings: {} } };
  assert.equal(buildBindAction(state, 'character'), null);
});

test('buildBindAction: 下拉自动 → null', () => {
  const state = { selectedPersonaId: '', selectedChar: 'char-a', selectedSess: '', effectivePersona: { bindings: {} } };
  assert.equal(buildBindAction(state, 'character'), null);
});

test('buildBindAction: 无 effective → null', () => {
  const state = { selectedPersonaId: 'writer', selectedChar: 'char-a', selectedSess: '', effectivePersona: null };
  assert.equal(buildBindAction(state, 'character'), null);
});

test('buildBindAction: 无 session → session 按钮 null', () => {
  const state = { selectedPersonaId: 'writer', selectedChar: 'char-a', selectedSess: '', effectivePersona: { bindings: {} } };
  assert.equal(buildBindAction(state, 'session'), null);
});

test('buildBindAction: scope 无 owner → bind', () => {
  const state = { selectedPersonaId: 'writer', selectedChar: 'char-a', selectedSess: '', effectivePersona: { bindings: { character_persona_id: null } } };
  const action = buildBindAction(state, 'character');
  assert.equal(action.kind, 'bind');
  assert.equal(action.personaId, 'writer');
  assert.equal(action.label, '绑定到角色');
});

test('buildBindAction: owner = selected → unbind 自己', () => {
  const state = { selectedPersonaId: 'writer', selectedChar: 'char-a', selectedSess: '', effectivePersona: { bindings: { character_persona_id: 'writer' } } };
  const action = buildBindAction(state, 'character');
  assert.equal(action.kind, 'unbind');
  assert.equal(action.personaId, 'writer');
  assert.equal(action.label, '解绑角色');
});

test('buildBindAction: owner ≠ selected → 先解绑旧 owner', () => {
  const state = { selectedPersonaId: 'roleplay', selectedChar: 'char-a', selectedSess: '', effectivePersona: { bindings: { character_persona_id: 'writer' } } };
  const action = buildBindAction(state, 'character');
  assert.equal(action.kind, 'unbind');
  assert.equal(action.personaId, 'writer');
  assert.equal(action.label, '先解绑 writer');
});

test('buildBindAction: session scope owner = selected → 解绑会话', () => {
  const state = { selectedPersonaId: 'roleplay', selectedChar: 'char-a', selectedSess: 'sess-1', effectivePersona: { bindings: { session_persona_id: 'roleplay' } } };
  const action = buildBindAction(state, 'session');
  assert.equal(action.kind, 'unbind');
  assert.equal(action.personaId, 'roleplay');
  assert.equal(action.label, '解绑会话');
});

test('buildPersonaPayload: 自动 → 只含 user_id', () => {
  const payload = buildPersonaPayload('alice', '');
  assert.deepEqual(payload, { user_id: 'alice' });
  assert.equal('persona_id' in payload, false);
});

test('buildPersonaPayload: 显式 → 含 user_id + persona_id', () => {
  const payload = buildPersonaPayload('alice', 'writer');
  assert.deepEqual(payload, { user_id: 'alice', persona_id: 'writer' });
});

test('buildPersonaPayload: user_id 默认值', () => {
  const payload = buildPersonaPayload('default', '');
  assert.deepEqual(payload, { user_id: 'default' });
});
