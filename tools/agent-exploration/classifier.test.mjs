import test from 'node:test';
import assert from 'node:assert/strict';
import { classifyPrDiff, DIFF_TASK_MAP } from './classifier.mjs';

test('Edit message PR maps to edit-branch task', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/engine/src/daemon/handlers/chat.rs\n+pub async fn edit_message';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('edit-branch-switch-refresh'));
});

test('Swipe PR maps to regen-swipe task', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/engine/src/daemon/handlers/chat.rs\n+async fn swipe_chat';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('regen-swipe-refresh'));
});

test('Memory PR maps to memory-roundtrip task', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/memory.rs b/engine/src/daemon/handlers/memory.rs\n+pub async fn update_resident_memory';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('memory-roundtrip'));
});

test('Onboarding PR maps to onboarding-firstchat-refresh task', () => {
  const diff = 'diff --git a/webui/assets/onboarding.js b/webui/assets/onboarding.js\n+onboardingSteps';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.includes('onboarding-firstchat-refresh'));
});

test('Unrelated PR returns empty task set', () => {
  const diff = 'diff --git a/README.md b/README.md\n+documentation change';
  const tasks = classifyPrDiff(diff);
  assert.deepEqual(tasks, []);
});

test('Multi-area PR deduplicates tasks', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/...\n+swipe\n+edit_message';
  const tasks = classifyPrDiff(diff);
  assert.ok(tasks.length >= 2);
  assert.equal(new Set(tasks).size, tasks.length);
});

// B1 regression: path-only (no keyword) must NOT trigger a task set.
// 改 chat.rs 但内容与 swipe/edit/memory 无关时，不应启动 LLM+Chrome 探索。
test('Path-only diff without matching keywords returns empty', () => {
  const diff = 'diff --git a/engine/src/daemon/handlers/chat.rs b/engine/src/daemon/handlers/chat.rs\n+pub fn unrelated_refactor() {}';
  const tasks = classifyPrDiff(diff);
  assert.deepEqual(tasks, [], 'path-only hit must not trigger; got ' + JSON.stringify(tasks));
});

// B1 regression: keyword-only (no path) must NOT trigger a task set.
// 防止 README/docs 提到 swipe/onboarding 等关键字但未改对应代码时误触发。
test('Keyword-only diff without matching paths returns empty', () => {
  const diff = 'diff --git a/README.md b/README.md\n+Documentation about onboarding flow and swipe behavior';
  const tasks = classifyPrDiff(diff);
  assert.deepEqual(tasks, [], 'keyword-only hit must not trigger; got ' + JSON.stringify(tasks));
});

// B1 regression: 任意路径单行无关改动，覆盖所有任务集的 paths。
test('Any single path change without keywords returns empty', () => {
  const cases = [
    'diff --git a/webui/assets/onboarding.js b/webui/assets/onboarding.js\n+export const unrelated = 1;',
    'diff --git a/webui/screens/14-message-swipe.html b/webui/screens/14-message-swipe.html\n+<div>layout tweak</div>',
    'diff --git a/engine/src/chat_store.rs b/engine/src/chat_store.rs\n+fn internal_helper() {}',
    'diff --git a/engine/src/memory/store.rs b/engine/src/memory/store.rs\n+fn internal_helper() {}',
  ];
  for (const diff of cases) {
    const tasks = classifyPrDiff(diff);
    assert.deepEqual(tasks, [], 'path-only should not trigger for diff: ' + diff + '; got ' + JSON.stringify(tasks));
  }
});
