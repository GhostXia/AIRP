// #114 C-PR1: Persona WebUI 闭环纯函数测试
// 运行：node --test webui/tests/persona.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { describeEffectiveHint, buildBindAction, buildPersonaPayload } = require('../persona-utils.js');

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
