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
