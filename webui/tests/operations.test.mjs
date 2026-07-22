import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const roleScript = await readFile(new URL('../assets/role-list.js', import.meta.url), 'utf8');
const chatScript = await readFile(new URL('../assets/chat-space.js', import.meta.url), 'utf8');
const consoleScript = await readFile(new URL('../assets/console-runtime.js', import.meta.url), 'utf8');
const onboardingScript = await readFile(new URL('../assets/onboarding.js', import.meta.url), 'utf8');
const chatPage = await readFile(new URL('../screens/02-chat-space.html', import.meta.url), 'utf8');

// ── B11: Delete character ──────────────────────────────────────────────────

test('role list wires DELETE /v1/characters/:id with confirmation', () => {
  assert.match(roleScript, /deleteCharacter/);
  assert.match(roleScript, /client\.request\('DELETE', '\/v1\/characters\/' \+ encodeURIComponent\(id\)\)/);
  assert.match(roleScript, /window\.confirm\(/);
  assert.match(roleScript, /此操作不可撤销/);
});

test('role list renders a per-card delete button as sibling control', () => {
  assert.match(roleScript, /cc-delete/);
  assert.match(roleScript, /cc-open/);
  assert.match(roleScript, /aria-label.*删除角色/);
  assert.doesNotMatch(roleScript, /role', 'button'/);
});

// ── B12: Delete session ────────────────────────────────────────────────────

test('chat space wires DELETE /v1/sessions/:char/:session with confirmation', () => {
  assert.match(chatScript, /deleteSession/);
  assert.match(chatScript, /client\.request\('DELETE', '\/v1\/sessions\/' \+ encodeURIComponent\(characterId\) \+ '\/' \+ encodeURIComponent\(id\)\)/);
  assert.match(chatScript, /全部消息将不可恢复/);
});

test('chat space renders a per-session delete button', () => {
  assert.match(chatScript, /session-delete/);
  assert.match(chatScript, /aria-label.*删除会话/);
});

// ── B4: Chat history search ────────────────────────────────────────────────

test('chat space wires POST /v1/chat/search', () => {
  assert.match(chatScript, /searchHistory/);
  assert.match(chatScript, /client\.request\('POST', '\/v1\/chat\/search'/);
  assert.match(chatScript, /character_id.*session_id.*query.*limit/);
});

test('chat space HTML exposes search input and button', () => {
  assert.match(chatPage, /id="search-input"/);
  assert.match(chatPage, /id="search-button"/);
  assert.match(chatPage, /type="search"/);
});

test('chat space search handles empty results gracefully', () => {
  assert.match(chatScript, /chat\.search\.empty/);
  assert.match(chatScript, /无匹配结果/);
});

// ── B13/B14: Persona delete and unbind ─────────────────────────────────────

test('console persona page wires DELETE persona and DELETE bindings', () => {
  assert.match(consoleScript, /删除 Persona/);
  assert.match(consoleScript, /client\.request\('DELETE', '\/v1\/users\/' \+ encodeURIComponent\(state\.userId\) \+ '\/personas\/' \+ encodeURIComponent\(active\)\)/);
  assert.match(consoleScript, /解绑 Persona/);
  assert.match(consoleScript, /client\.request\('DELETE', '\/v1\/users\/' \+ encodeURIComponent\(state\.userId\) \+ '\/personas\/' \+ encodeURIComponent\(active\) \+ '\/bindings'/);
});

test('console persona delete guards the default persona', () => {
  assert.match(consoleScript, /不能删除 default Persona/);
  assert.match(consoleScript, /active === 'default'/);
});

// ── B9/B10: State history and schema ───────────────────────────────────────

test('console memory page fetches state history and schema', () => {
  assert.match(consoleScript, /\/v1\/characters\/' \+ encodeURIComponent\(state\.characterId\) \+ '\/state\/history/);
  assert.match(consoleScript, /\/v1\/characters\/' \+ encodeURIComponent\(state\.characterId\) \+ '\/state\/schema/);
  assert.match(consoleScript, /状态变更历史/);
  assert.match(consoleScript, /状态 JSON Schema/);
});

// ── #295 §2: field helper select/type defense ──────────────────────────────

test('field helper does not set type on select elements (console-runtime)', () => {
  assert.match(consoleScript, /options\.type && !options\.select/);
  assert.doesNotMatch(consoleScript, /if \(options && options\.type\) control\.type/);
});

test('field helper does not set type on select elements (onboarding)', () => {
  assert.match(onboardingScript, /options\.type && !options\.select/);
  assert.doesNotMatch(onboardingScript, /if \(options && options\.type\) control\.type/);
});
