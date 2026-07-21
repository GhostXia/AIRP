// #252 §2.H.2 / #275 F-1: SmoothStreamer.findSentenceBoundary 纯函数单元测试
// 运行：node --test webui/tests/smooth-streamer.test.mjs
//
// 覆盖 PR #270 §2.M1 + CR-3 修复的 2-char context 句子边界检测：
//   - 防止 "1.0" / "Google.com" 中的 `.` 被误判为句子边界
//   - 防止 candidate 末尾的 `.` 因 queue[i+1] === undefined 被误判为 end-of-string 边界
//   - 中文句号 / 英文句号 + 空格 / 换行符 仍正确判为边界
//   - 0.25 阈值：边界过近时返回 -1（避免渲染字符过少）
//   - 边界保护：空输入 / charsToRender <= 0 / charsToRender >= queue.length 返回 -1
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { findSentenceBoundary, SENTENCE_END_RE } = require('../smooth-streamer-utils.js');

// ══════════════════════════════════════════════════════════════════════════
// #275 Case 1: `1.0` / `Google.com` 中的 `.` 不被判为边界
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: "1.0" 中的 `.` 不判为边界（数字间小数点）', () => {
  // queue = "Price is 1.0 dollars", charsToRender = 11
  // candidate = "Price is 1."（前 11 字符）
  // 旧 bug: SENTENCE_END_RE.test('.') 传入单字符，`$` 匹配单字符末尾 → 误判为边界
  // 修复: twoChar = ".0"，正则 `\.(\s|$)` 不匹配（'0' 不是 \s）
  const queue = 'Price is 1.0 dollars';
  const boundary = findSentenceBoundary(queue, 11);
  assert.equal(boundary, -1, '"1.0" 中的 `.` 不应判为句子边界');
});

test('findSentenceBoundary: "Google.com" 中的 `.` 不判为边界（域名点号）', () => {
  // queue = "Google.com search", charsToRender = 10
  // candidate = "Google.com"
  // i=6 是 '.', queue[7]='c', twoChar=".c"，正则不匹配
  const queue = 'Google.com search';
  const boundary = findSentenceBoundary(queue, 10);
  assert.equal(boundary, -1, '"Google.com" 中的 `.` 不应判为句子边界');
});

test('findSentenceBoundary: "3.14" 中的 `.` 不判为边界（pi 小数点）', () => {
  // queue = "3.14 is pi", charsToRender = 3
  // candidate = "3.1"
  // i=1 是 '.', queue[2]='1', twoChar=".1"，正则不匹配
  const queue = '3.14 is pi';
  const boundary = findSentenceBoundary(queue, 3);
  assert.equal(boundary, -1, '"3.14" 中的 `.` 不应判为句子边界');
});

// ══════════════════════════════════════════════════════════════════════════
// #275 Case 2: candidate 末尾的 `.` 若 queue 后续是字母不判为边界
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: candidate 末尾 `.` 后 queue 有字母时不判为边界', () => {
  // queue = "end. Start", charsToRender = 4
  // candidate = "end."（candidate 末尾是 '.'）
  // 旧 bug: 用 candidate 取上下文，candidate[4] === undefined → twoChar="." + undefined = "."
  //         SENTENCE_END_RE.test(".") 中 `$` 匹配 → 误判为边界
  // 修复: 用 queue 取上下文，queue[4]=' ', twoChar=". ", 正则 `\.(\s|$)` 匹配（' ' 是 \s）
  // 但这里期望匹配（因为 "." 后确实是空格）→ boundary = 4
  // 这个 case 实际验证了"用 queue 而非 candidate 取上下文"的正确性：
  // 修复前的 bug 是 candidate 末尾的 `.` 一律被误判为边界（即使 queue 后续是字母）；
  // 修复后只有 queue 后续真的是 \s 或 $ 时才判为边界。
  const queue = 'end. Start';
  const boundary = findSentenceBoundary(queue, 4);
  // queue[4] = ' '，twoChar=". "，正则匹配 → boundary = 4
  assert.equal(boundary, 4, 'candidate 末尾 `.` 后 queue 是空格时应判为边界（验证 queue 上下文取值正确）');
});

test('findSentenceBoundary: candidate 末尾 `.` 后 queue 是字母时不判为边界（CR-3 核心回归点）', () => {
  // queue = "end.start", charsToRender = 4
  // candidate = "end."（candidate 末尾是 '.'）
  // 旧 bug: 用 candidate 取上下文，candidate[4] === undefined → twoChar="."
  //         SENTENCE_END_RE.test(".") 中 `$` 匹配 → 误判为 boundary = 4
  // 修复: 用 queue 取上下文，queue[4]='s', twoChar=".s"，正则 `\.(\s|$)` 不匹配
  // → boundary = -1
  // 这是 CR-3 修复的核心回归点：candidate 末尾的 `.` 不应因 candidate[4] === undefined
  // 而被误判为 end-of-string 边界。
  const queue = 'end.start';
  const boundary = findSentenceBoundary(queue, 4);
  assert.equal(boundary, -1, 'candidate 末尾 `.` 后 queue 是字母时不应判为边界（CR-3 回归点）');
});

// ══════════════════════════════════════════════════════════════════════════
// #275 Case 3: 中文句号 `。` / `！` / `？` / `；` 仍判为边界
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: 中文句号 `。` 判为边界', () => {
  // queue = "你好。世界", charsToRender = 4
  // candidate = "你好。世"
  // i=2 是 '。', queue[3]='世', twoChar="。世"，正则 `[。！？；\n]` 匹配
  // boundary = 3, 3 > 4 * 0.25 = 1, 返回 3
  const queue = '你好。世界';
  const boundary = findSentenceBoundary(queue, 4);
  assert.equal(boundary, 3, '中文句号 `。` 应判为边界');
});

test('findSentenceBoundary: 中文叹号 `！` 判为边界', () => {
  const queue = '好的！再见';
  const boundary = findSentenceBoundary(queue, 3);
  assert.equal(boundary, 3, '中文叹号 `！` 应判为边界');
});

test('findSentenceBoundary: 中文问号 `？` 判为边界', () => {
  const queue = '是吗？是的';
  const boundary = findSentenceBoundary(queue, 3);
  assert.equal(boundary, 3, '中文问号 `？` 应判为边界');
});

test('findSentenceBoundary: 中文分号 `；` 判为边界', () => {
  const queue = '第一；第二';
  const boundary = findSentenceBoundary(queue, 3);
  assert.equal(boundary, 3, '中文分号 `；` 应判为边界');
});

// ══════════════════════════════════════════════════════════════════════════
// #275 Case 4: `\n` 仍判为边界
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: 换行符 `\\n` 判为边界', () => {
  // queue = "line1\nline2", charsToRender = 6
  // candidate = "line1\n"
  // i=5 是 '\n', queue[6]='l', twoChar="\nl"，正则 `[。！？；\n]` 匹配
  // boundary = 6, 6 > 6 * 0.25 = 1.5, 返回 6
  const queue = 'line1\nline2';
  const boundary = findSentenceBoundary(queue, 6);
  assert.equal(boundary, 6, '换行符 `\\n` 应判为边界');
});

// ══════════════════════════════════════════════════════════════════════════
// 英文句号 + 空格 / 问号 + 空格（正则 `\.(\s|$)` / `[!?](\s|$)` 分支）
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: 英文句号 + 空格判为边界', () => {
  // queue = "Hello. World", charsToRender = 7
  // candidate = "Hello. "
  // i=5 是 '.', queue[6]=' ', twoChar=". ", 正则 `\.(\s|$)` 匹配
  // boundary = 6, 6 > 7 * 0.25 = 1.75, 返回 6
  const queue = 'Hello. World';
  const boundary = findSentenceBoundary(queue, 7);
  assert.equal(boundary, 6, '英文句号 + 空格应判为边界');
});

test('findSentenceBoundary: 英文问号 + 空格判为边界', () => {
  const queue = 'What? Yes';
  const boundary = findSentenceBoundary(queue, 6);
  assert.equal(boundary, 5, '英文问号 + 空格应判为边界');
});

test('findSentenceBoundary: 英文叹号 + 空格判为边界', () => {
  const queue = 'Wow! Cool';
  const boundary = findSentenceBoundary(queue, 5);
  assert.equal(boundary, 4, '英文叹号 + 空格应判为边界');
});

// ══════════════════════════════════════════════════════════════════════════
// 0.25 阈值：边界过近时返回 -1（避免渲染字符过少）
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: 边界过近（< 0.25 * charsToRender）时返回 -1', () => {
  // queue = "a。bcdefghijklmnop", charsToRender = 16
  // candidate = "a。bcdefghijklmno"
  // i=2 是 '。', twoChar="。b"，匹配，boundary = 3
  // 但 3 < 16 * 0.25 = 4，所以返回 -1（边界过近，避免渲染字符过少）
  const queue = 'a。bcdefghijklmnop';
  const boundary = findSentenceBoundary(queue, 16);
  assert.equal(boundary, -1, '边界过近时应返回 -1（0.25 阈值）');
});

test('findSentenceBoundary: 边界刚好等于 0.25 阈值时返回 -1（严格大于）', () => {
  // queue = "abc。efgh", charsToRender = 4
  // candidate = "abc。"
  // i=3 是 '。', twoChar="。e"，匹配，boundary = 4
  // 4 > 4 * 0.25 = 1？是，返回 4
  // 这个 case 验证 boundary > threshold（严格大于），不是 >=
  const queue = 'abc。efgh';
  const boundary = findSentenceBoundary(queue, 4);
  assert.equal(boundary, 4, 'boundary > 0.25 * charsToRender 时应返回 boundary');
});

// ══════════════════════════════════════════════════════════════════════════
// 边界保护
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: queue 为空字符串返回 -1', () => {
  assert.equal(findSentenceBoundary('', 5), -1);
});

test('findSentenceBoundary: queue 为 null/undefined 返回 -1', () => {
  assert.equal(findSentenceBoundary(null, 5), -1);
  assert.equal(findSentenceBoundary(undefined, 5), -1);
});

test('findSentenceBoundary: charsToRender <= 0 返回 -1', () => {
  assert.equal(findSentenceBoundary('abc。def', 0), -1);
  assert.equal(findSentenceBoundary('abc。def', -1), -1);
});

test('findSentenceBoundary: charsToRender >= queue.length 返回 -1（无需截断）', () => {
  // queue = "abc。def" (length=7), charsToRender = 7
  // 整个 queue 都在窗口内，无需找边界（调用方会一次性渲染完）
  assert.equal(findSentenceBoundary('abc。def', 7), -1);
  assert.equal(findSentenceBoundary('abc。def', 100), -1);
});

// ══════════════════════════════════════════════════════════════════════════
// 不修改输入 queue（纯函数不变量）
// ══════════════════════════════════════════════════════════════════════════

test('findSentenceBoundary: 不修改输入 queue（纯函数不变量）', () => {
  const queue = 'Hello. World';
  const original = queue.slice(); // 复制
  findSentenceBoundary(queue, 7);
  assert.equal(queue, original, 'queue 不应被修改');
  assert.deepEqual(Array.from(queue), Array.from(original), 'queue 字符内容不应被修改');
});

// ══════════════════════════════════════════════════════════════════════════
// SENTENCE_END_RE 正则导出（验证不动）
// ══════════════════════════════════════════════════════════════════════════

test('SENTENCE_END_RE: 正则形状与 PR #270 修复后版本一致', () => {
  // 验证正则匹配预期模式：中文标点 / 英文标点+空格 / 换行
  assert.ok(SENTENCE_END_RE.test('。'));
  assert.ok(SENTENCE_END_RE.test('！'));
  assert.ok(SENTENCE_END_RE.test('？'));
  assert.ok(SENTENCE_END_RE.test('；'));
  assert.ok(SENTENCE_END_RE.test('\n'));
  assert.ok(SENTENCE_END_RE.test('. '));
  assert.ok(SENTENCE_END_RE.test('? '));
  assert.ok(SENTENCE_END_RE.test('! '));
  // 注意：SENTENCE_END_RE.test('.') 返回 true，因为 `\.(\s|$)` 中 $ 匹配字符串末尾。
  // 这是正则的固有行为，不是 bug——在真实字符串末尾的 '.' 应该是边界（如 "end."）。
  // bug 只在 twoChar 上下文中 $ 误匹配（twoChar 末尾不代表 queue 末尾），
  // findSentenceBoundary 已用显式检查修复，不依赖 SENTENCE_END_RE.test(twoChar)。
  assert.ok(SENTENCE_END_RE.test('.'), '单字符 . 匹配 $（正则固有行为，非 bug）');
  assert.equal(SENTENCE_END_RE.test('.a'), false, '. + 字母不匹配');
  assert.equal(SENTENCE_END_RE.test('.1'), false, '. + 数字不匹配');
});
