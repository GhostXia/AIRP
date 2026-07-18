import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { computeHistoryToolbarState } = require('../history-utils.js');

// CodeRabbit 建议：显式 en-US locale，避免 CI 在非西方 locale（如 ar-EG）下
// Intl.NumberFormat() 产出阿拉伯数字导致断言失败。
const fmt = new Intl.NumberFormat('en-US');

test('computeHistoryToolbarState shows loading UI when loading=true', () => {
  const ui = computeHistoryToolbarState(
    { total: 10, hasMore: true, loading: true },
    5,
    fmt
  );
  assert.equal(ui.loadEarlierDisabled, true);
  assert.equal(ui.loadEarlierText, '加载中…');
  assert.equal(ui.loadEarlierHidden, false);
  assert.equal(ui.toolbarHidden, false);
  assert.equal(ui.statusText, '5 / 10 条消息');
});

test('computeHistoryToolbarState recovers button after loading=false (#148 core)', () => {
  // #148 验收：网络失败后 loading 必须为 false，按钮恢复可用且文字不再是"加载中"
  const ui = computeHistoryToolbarState(
    { total: 10, hasMore: true, loading: false },
    5,
    fmt
  );
  assert.equal(ui.loadEarlierDisabled, false);
  assert.equal(ui.loadEarlierText, '加载更早');
  assert.equal(ui.loadEarlierHidden, false);
});

test('computeHistoryToolbarState hides toolbar when total=0', () => {
  const ui = computeHistoryToolbarState(
    { total: 0, hasMore: false, loading: false },
    0,
    fmt
  );
  assert.equal(ui.toolbarHidden, true);
  assert.equal(ui.loadEarlierHidden, true);
  assert.equal(ui.statusText, '0 / 0 条消息');
});

test('computeHistoryToolbarState hides loadEarlier when hasMore=false', () => {
  const ui = computeHistoryToolbarState(
    { total: 50, hasMore: false, loading: false },
    50,
    fmt
  );
  assert.equal(ui.loadEarlierHidden, true);
  assert.equal(ui.loadEarlierDisabled, false);
  assert.equal(ui.loadEarlierText, '加载更早');
});

test('computeHistoryToolbarState handles null/undefined historyState gracefully', () => {
  const ui = computeHistoryToolbarState(null, 0, fmt);
  assert.equal(ui.toolbarHidden, true);
  assert.equal(ui.loadEarlierHidden, true);
  assert.equal(ui.loadEarlierDisabled, false);
  assert.equal(ui.loadEarlierText, '加载更早');
});

test('computeHistoryToolbarState handles null/undefined countFormatter gracefully (gemini-code-assist)', () => {
  // 防御性：纯函数不应因 countFormatter 缺失而抛 TypeError
  const ui = computeHistoryToolbarState(
    { total: 10, hasMore: true, loading: false },
    5,
    null
  );
  assert.equal(ui.statusText, '5 / 10 条消息');
  assert.equal(ui.loadEarlierDisabled, false);
});

test('computeHistoryToolbarState handles countFormatter without format method', () => {
  const ui = computeHistoryToolbarState(
    { total: 100, hasMore: false, loading: false },
    50,
    {}
  );
  assert.equal(ui.statusText, '50 / 100 条消息');
});

test('computeHistoryToolbarState formats counts with provided formatter', () => {
  const calls = [];
  const stubFmt = { format: (n) => { calls.push(n); return '[' + n + ']'; } };
  const ui = computeHistoryToolbarState(
    { total: 100, hasMore: true, loading: false },
    42,
    stubFmt
  );
  assert.deepEqual(calls, [42, 100]);
  assert.equal(ui.statusText, '[42] / [100] 条消息');
});

test('computeHistoryToolbarState simulates full network-failure recovery lifecycle', () => {
  // #148 完整生命周期：idle → loading → network failure (status=0) → recovered
  const state = { total: 10, hasMore: true, loading: false };
  const loaded = 5;

  // 1. idle (before click)
  let ui = computeHistoryToolbarState(state, loaded, fmt);
  assert.equal(ui.loadEarlierText, '加载更早');
  assert.equal(ui.loadEarlierDisabled, false);

  // 2. loading (user clicked, request in flight)
  state.loading = true;
  ui = computeHistoryToolbarState(state, loaded, fmt);
  assert.equal(ui.loadEarlierText, '加载中…');
  assert.equal(ui.loadEarlierDisabled, true);

  // 3. network failure (r.status === 0) → app.js sets loading=false + updateHistoryToolbar()
  state.loading = false;
  ui = computeHistoryToolbarState(state, loaded, fmt);
  assert.equal(ui.loadEarlierText, '加载更早');
  assert.equal(ui.loadEarlierDisabled, false);

  // 4. user can retry (button enabled)
  assert.equal(ui.loadEarlierHidden, false);
});
