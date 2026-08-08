import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const roleScript = await readFile(new URL('../assets/role-list.js', import.meta.url), 'utf8');
const chatScript = await readFile(new URL('../assets/chat-space.js', import.meta.url), 'utf8');
const consoleScript = await readFile(new URL('../assets/console-runtime.js', import.meta.url), 'utf8');
const onboardingScript = await readFile(new URL('../assets/onboarding.js', import.meta.url), 'utf8');
const chatPage = await readFile(new URL('../screens/02-chat-space.html', import.meta.url), 'utf8');
const chatStyle = await readFile(new URL('../assets/chat-space.css', import.meta.url), 'utf8');
const bootScript = await readFile(new URL('../assets/widgets/boot.js', import.meta.url), 'utf8');

// ── #394 O1: observable Session Coordinator ───────────────────────────────

test('chat space observes coordinator state and blocks conflicting mutations', () => {
  assert.match(chatScript, /client\.request\('POST', '\/v1\/chat\/session-state'/);
  assert.match(chatScript, /sessionMutationBlocked\(\)/);
  assert.match(chatScript, /coordinatorPhase !== 'idle'/);
  assert.match(chatScript, /error\.status === 409.*session_busy/);
  assert.match(chatScript, /session_recovery_required.*refreshCoordinatorState/);
  assert.match(chatScript, /error\.status === 404[\s\S]*coordinatorStateSupported = false/);
});

test('chat stop preserves the Engine cancellation result and only aborts as fallback', () => {
  assert.match(chatScript, /client\.request\('POST', '\/v1\/chat\/cancel'/);
  assert.match(chatScript, /generation_id: generationId/);
  assert.match(chatScript, /abortLocalStream = false;[\s\S]*finally \{\s*if \(abortLocalStream\) controller\.abort\(\);/);
  assert.match(chatScript, /generation_committing[\s\S]*正在提交，无法取消/);
});

test('coordinator status reuses the authoritative sample tag tokens', () => {
  assert.match(chatPage, /id="session-operation-status"/);
  assert.match(chatPage, /class="tag tag-warning mono session-operation-status"/);
  assert.match(chatStyle, /\.session-operation-status \{ margin-right: var\(--space-2\); \}/);
  assert.doesNotMatch(chatStyle, /session-operation-status[^}]*#[0-9a-f]{3,8}/i);
});

// ── BUG-2 mitigation: user-side recovery for fail-closed sessions ──────────

test('chat space offers a confirmed recovery action for recovering sessions', () => {
  // Button appears only while the coordinator reports the fail-closed phase.
  assert.match(chatScript, /coordinatorPhase !== 'recovering'/);
  assert.match(chatScript, /id = 'session-recover'|id: 'session-recover'|\.id = 'session-recover'/);
  assert.match(chatScript, /尝试恢复会话/);
  // Shared confirmation UI before the mutating call, then the new endpoint.
  assert.match(chatScript, /await AIRPConfirm\.confirm\(/);
  assert.match(chatScript, /client\.request\('POST', '\/v1\/chat\/session-recover'/);
  assert.match(chatScript, /character_id: characterId, session_id: sessionId/);
  // Success refreshes the coordinator state; failure stays actionable.
  assert.match(chatScript, /session\.recover[\s\S]*refreshCoordinatorState/);
  assert.match(chatScript, /session\.recover\.error/);
});

test('diagnostics screen exposes the session recovery entry', () => {
  assert.match(consoleScript, /client\.request\('POST', '\/v1\/chat\/session-recover'/);
  assert.match(consoleScript, /会话恢复（写入中断锁死）/);
  assert.match(consoleScript, /尝试恢复会话/);
  assert.match(consoleScript, /phase === 'recovering'/);
});

test('chat space styles the recovery button with design tokens only', () => {
  assert.match(chatStyle, /\.session-recover-btn \{/);
  assert.doesNotMatch(chatStyle, /session-recover-btn[^}]*#[0-9a-f]{3,8}/i);
});

// ── B11: Delete character ──────────────────────────────────────────────────

test('role list wires DELETE /v1/characters/:id with confirmation', () => {
  assert.match(roleScript, /deleteCharacter/);
  assert.match(roleScript, /client\.request\('DELETE', '\/v1\/characters\/' \+ encodeURIComponent\(id\)\)/);
  assert.match(roleScript, /await AIRPConfirm\.confirm\(/);
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
  assert.match(chatScript, /await AIRPConfirm\.confirm\(/);
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
  // DELETE persona（无 body、无额外路径）。
  assert.match(consoleScript, /client\.request\('DELETE', '\/v1\/users\/' \+ encodeURIComponent\(state\.userId\) \+ '\/personas\/' \+ encodeURIComponent\(active\)\)\)/);
  assert.match(consoleScript, /解绑 Persona/);
  // daemon `unbind_persona_endpoint` 用 axum::extract::Query 读取 character_id（必填），
  // DELETE 不解析 JSON body——必须以 query string 传递，否则 400 BadRequest。
  assert.match(consoleScript, /\/bindings\?character_id=' \+ encodeURIComponent\(binding\.control\.value\)\)/);
  // 反向断言：DELETE bindings 不能再带 JSON body（POST bind 仍然带 body，需区分 method）。
  assert.doesNotMatch(consoleScript, /client\.request\('DELETE', '\/v1\/users\/' \+ encodeURIComponent\(state\.userId\) \+ '\/personas\/' \+ encodeURIComponent\(active\) \+ '\/bindings', \{/);
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

// ── #485 W4/W5/W6: widget boot hardening ───────────────────────────────────

test('widget boot degrades instead of failing hard (W4/W6)', () => {
  // W6：catalog 拉取带 5s 超时，engine 挂起时 boot 不悬挂。
  assert.match(bootScript, /AbortSignal\.timeout\(5000\)/);
  // W4：applySlotPlan 异常不阻断 mountSlots（slot 保持空占位）。
  assert.match(bootScript, /applySlotPlan\(plan, 'replace'\)[\s\S]*catch/);
  assert.match(bootScript, /slot 计划应用失败，继续以空计划挂载/);
});

test('widget intent handler returns an observable promise via api-client (W5)', () => {
  assert.match(bootScript, /client\.request\('POST', '\/v1\/widget-intents', envelope\)/);
  assert.match(bootScript, /return traceIntent\(/);
  assert.match(bootScript, /return resp\.json\(\)\.catch\(\(\) => null\)/, 'fallback must preserve optional engine result fields');
  assert.match(bootScript, /typeof AIRPDesktopSession === 'undefined'/);
  assert.match(bootScript, /renewDesktopSession\(\{ base: engineBase\(\) \}\)/);
  assert.match(bootScript, /notifyAuthFailure\(\{ source: 'widget-intent' \}\)/);
});
