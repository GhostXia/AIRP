// #126 D-PR2: Worldbook WebUI 管理迁移纯函数测试
// 运行：node --test webui/tests/lorebook.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { parseSecondaryKeys, buildLoreEntryDefault, collectAdvisoryFields, sanitizeLoreEntries } = require('../lorebook-utils.js');

// ── parseSecondaryKeys ─────────────────────────────────────────────────────

test('parseSecondaryKeys: 基本逗号拆分 + trim', () => {
  assert.deepEqual(parseSecondaryKeys('writer, roleplay , dragon'), ['writer', 'roleplay', 'dragon']);
});

test('parseSecondaryKeys: 移除空 token', () => {
  assert.deepEqual(parseSecondaryKeys('writer, , roleplay,'), ['writer', 'roleplay']);
});

test('parseSecondaryKeys: 重复 token 只保留首次出现', () => {
  assert.deepEqual(parseSecondaryKeys('writer, roleplay, writer'), ['writer', 'roleplay']);
});

test('parseSecondaryKeys: 前导/尾随空格不写入 key', () => {
  assert.deepEqual(parseSecondaryKeys('  writer  , roleplay '), ['writer', 'roleplay']);
});

test('parseSecondaryKeys: 空字符串 → 空数组', () => {
  assert.deepEqual(parseSecondaryKeys(''), []);
});

test('parseSecondaryKeys: 仅空白 → 空数组', () => {
  assert.deepEqual(parseSecondaryKeys('   ,  , '), []);
});

test('parseSecondaryKeys: 非字符串输入 → 空数组', () => {
  assert.deepEqual(parseSecondaryKeys(null), []);
  assert.deepEqual(parseSecondaryKeys(undefined), []);
  assert.deepEqual(parseSecondaryKeys(123), []);
});

test('parseSecondaryKeys: CR#3 支持全角逗号', () => {
  // 中文输入法下常输入全角逗号，应与 ASCII 逗号等价处理
  assert.deepEqual(parseSecondaryKeys('writer， roleplay， dragon'), ['writer', 'roleplay', 'dragon']);
});

test('parseSecondaryKeys: CR#3 混合 ASCII 与全角逗号', () => {
  assert.deepEqual(parseSecondaryKeys('writer, roleplay，dragon'), ['writer', 'roleplay', 'dragon']);
});

test('parseSecondaryKeys: CR#3 全角逗号 + 重复 token 去重', () => {
  assert.deepEqual(parseSecondaryKeys('writer， roleplay， writer'), ['writer', 'roleplay']);
});

// ── buildLoreEntryDefault ──────────────────────────────────────────────────

test('buildLoreEntryDefault: 默认 selective=false', () => {
  const entry = buildLoreEntryDefault();
  assert.equal(entry.selective, false);
});

test('buildLoreEntryDefault: 默认 secondary_keys 为空数组', () => {
  const entry = buildLoreEntryDefault();
  assert.deepEqual(entry.secondary_keys, []);
});

test('buildLoreEntryDefault: 含全部 v4 canonical 字段', () => {
  const entry = buildLoreEntryDefault();
  assert.ok('keys' in entry);
  assert.ok('content' in entry);
  assert.ok('enabled' in entry);
  assert.ok('priority' in entry);
  assert.ok('constant' in entry);
  assert.ok('comment' in entry);
  assert.ok('selective' in entry);
  assert.ok('secondary_keys' in entry);
});

test('buildLoreEntryDefault: 每次返回独立对象', () => {
  const a = buildLoreEntryDefault();
  const b = buildLoreEntryDefault();
  a.keys.push('mutated');
  assert.deepEqual(b.keys, []);
});

// ── collectAdvisoryFields ──────────────────────────────────────────────────

test('collectAdvisoryFields: top-level case_sensitive 显示', () => {
  const entry = { case_sensitive: true };
  const fields = collectAdvisoryFields(entry);
  assert.equal(fields.length, 1);
  assert.equal(fields[0].label, 'case_sensitive');
  assert.equal(fields[0].value, 'true');
});

test('collectAdvisoryFields: extensions advisory 字段显示', () => {
  const entry = { extensions: { position: 'before_char', depth: 4, probability: 50 } };
  const fields = collectAdvisoryFields(entry);
  const labels = fields.map(f => f.label);
  assert.ok(labels.includes('position'));
  assert.ok(labels.includes('depth'));
  assert.ok(labels.includes('probability'));
});

test('collectAdvisoryFields: extensions.selective 被跳过（v4 已提升为 canonical）', () => {
  const entry = { extensions: { selective: true, position: 'after_char' } };
  const fields = collectAdvisoryFields(entry);
  const labels = fields.map(f => f.label);
  assert.ok(!labels.includes('selective'));
  assert.ok(labels.includes('position'));
});

test('collectAdvisoryFields: 两条读取路径分开（case_sensitive 不假设在 extensions）', () => {
  // case_sensitive 同时出现在 top-level 和 extensions：top-level 是 canonical，
  // extensions 中的同名 key 用源限定标签 `case_sensitive (extensions)` 展示（CR-nitpick）。
  const entry = { case_sensitive: false, extensions: { case_sensitive: true } };
  const fields = collectAdvisoryFields(entry);
  // top-level 用 'case_sensitive'，extensions 用 'case_sensitive (extensions)'
  const topLevel = fields.find(f => f.label === 'case_sensitive');
  const extLevel = fields.find(f => f.label === 'case_sensitive (extensions)');
  assert.ok(topLevel, 'top-level case_sensitive 应保留原标签');
  assert.ok(extLevel, 'extensions case_sensitive 应用源限定标签');
  assert.equal(topLevel.value, 'false');
  assert.equal(extLevel.value, 'true');
});

test('collectAdvisoryFields: 仅 extensions 有 case_sensitive 时不加源限定后缀', () => {
  // 只有 extensions 中有 case_sensitive，没有 top-level 时，标签保持原样
  const entry = { extensions: { case_sensitive: true } };
  const fields = collectAdvisoryFields(entry);
  assert.equal(fields.length, 1);
  assert.equal(fields[0].label, 'case_sensitive');
  assert.equal(fields[0].value, 'true');
});

test('collectAdvisoryFields: 无 advisory 字段 → 空数组', () => {
  assert.deepEqual(collectAdvisoryFields({}), []);
  assert.deepEqual(collectAdvisoryFields({ keys: [], content: '' }), []);
  assert.deepEqual(collectAdvisoryFields(null), []);
  assert.deepEqual(collectAdvisoryFields(undefined), []);
});

test('collectAdvisoryFields: 对象类型的 extension 值序列化为 JSON', () => {
  const entry = { extensions: { recursion: { depth: 5 } } };
  const fields = collectAdvisoryFields(entry);
  assert.equal(fields[0].label, 'recursion');
  assert.equal(fields[0].value, JSON.stringify({ depth: 5 }));
});

// ── 集成场景：selective + secondary_keys 编辑 → 保存 payload ────────────────

test('集成: selective=true + secondary_keys 编辑 → 保存 payload 含两字段', () => {
  // 模拟用户编辑 selective checkbox + secondary_keys input 后的 entry 状态
  const entry = buildLoreEntryDefault();
  entry.selective = true;
  entry.secondary_keys = parseSecondaryKeys('writer, roleplay, writer');
  assert.equal(entry.selective, true);
  assert.deepEqual(entry.secondary_keys, ['writer', 'roleplay']);
  // 保存 payload = { entries: [entry] }，两字段都在
  const payload = { entries: [entry] };
  assert.ok('selective' in payload.entries[0]);
  assert.ok('secondary_keys' in payload.entries[0]);
  assert.equal(payload.entries[0].selective, true);
  assert.deepEqual(payload.entries[0].secondary_keys, ['writer', 'roleplay']);
});

test('集成: top-level case_sensitive 与 extensions advisory 都显示且无丢失', () => {
  // 模拟导入 ST card 后的 entry：top-level case_sensitive + extensions advisory
  const entry = {
    keys: ['dragon'],
    content: 'dragons are cool',
    selective: true,
    secondary_keys: ['scaled'],
    case_sensitive: true,
    extensions: { position: 'before_char', depth: 4, probability: 80, selective: false },
  };
  const fields = collectAdvisoryFields(entry);
  const labels = fields.map(f => f.label);
  // selective 不在 advisory（已提升为 canonical）
  assert.ok(!labels.includes('selective'));
  // case_sensitive 从 top-level 读
  assert.ok(labels.includes('case_sensitive'));
  // extensions 的 position/depth/probability 都在
  assert.ok(labels.includes('position'));
  assert.ok(labels.includes('depth'));
  assert.ok(labels.includes('probability'));
  // 原 entry 的 extensions 不被修改（只读展示，不丢失数据）
  assert.equal(entry.extensions.position, 'before_char');
  assert.equal(entry.extensions.depth, 4);
  assert.equal(entry.extensions.probability, 80);
  assert.equal(entry.extensions.selective, false); // 仍在 extensions（旧数据），但 UI 不展示
});

// ── S3: round-trip 保存测试 ─────────────────────────────────────────────────

test('S3 round-trip: 保存 payload 序列化/反序列化后 extensions 与 case_sensitive 无损', () => {
  // 模拟 PUT 后服务端原样存储、再 GET 回来的场景。
  // advisory 字段（case_sensitive/extensions）不应在保存过程中丢失或被改写。
  const original = {
    keys: ['dragon'],
    content: 'dragons are cool',
    enabled: true,
    priority: 10,
    constant: false,
    comment: null,
    selective: true,
    secondary_keys: ['scaled'],
    case_sensitive: true,
    extensions: { position: 'before_char', depth: 4, probability: 80, selective: false },
  };
  // 模拟保存：JSON 序列化 → 反序列化（模拟 PUT 后再 GET）
  const payload = { entries: [original] };
  const serialized = JSON.stringify(payload);
  const roundTripped = JSON.parse(serialized);
  const after = roundTripped.entries[0];
  // v4 canonical 字段保留
  assert.equal(after.selective, true);
  assert.deepEqual(after.secondary_keys, ['scaled']);
  // advisory 字段保留（未被 UI 修改）
  assert.equal(after.case_sensitive, true);
  assert.equal(after.extensions.position, 'before_char');
  assert.equal(after.extensions.depth, 4);
  assert.equal(after.extensions.probability, 80);
  assert.equal(after.extensions.selective, false); // 旧 ST 数据原样保留
  // collectAdvisoryFields 在 round-trip 后仍能正确展示
  const fields = collectAdvisoryFields(after);
  const labels = fields.map(f => f.label);
  assert.ok(labels.includes('case_sensitive'));
  assert.ok(labels.includes('position'));
  assert.ok(labels.includes('depth'));
  assert.ok(labels.includes('probability'));
  assert.ok(!labels.includes('selective')); // 仍被跳过
});

// ── #186 W-03: sanitizeLoreEntries ─────────────────────────────────────────

test('sanitizeLoreEntries: 正常数组 → 全部 valid，无 skipped', () => {
  const entries = [
    { keys: ['a'], content: 'alpha' },
    { keys: ['b'], content: 'beta' },
  ];
  const result = sanitizeLoreEntries(entries);
  assert.equal(result.valid.length, 2);
  assert.deepEqual(result.skipped, []);
});

test('sanitizeLoreEntries: 空数组 → 空 valid，空 skipped', () => {
  const result = sanitizeLoreEntries([]);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.skipped, []);
});

test('sanitizeLoreEntries: null 输入 → 空 valid，空 skipped（不抛）', () => {
  const result = sanitizeLoreEntries(null);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.skipped, []);
});

test('sanitizeLoreEntries: undefined 输入 → 空 valid，空 skipped（不抛）', () => {
  const result = sanitizeLoreEntries(undefined);
  assert.deepEqual(result.valid, []);
  assert.deepEqual(result.skipped, []);
});

test('sanitizeLoreEntries: 非数组输入（数字/字符串/对象）→ 空 valid，空 skipped', () => {
  for (const bad of [42, 'foo', {}]) {
    const result = sanitizeLoreEntries(bad);
    assert.deepEqual(result.valid, []);
    assert.deepEqual(result.skipped, []);
  }
});

test('sanitizeLoreEntries: 数组含 null → 跳过并报告索引', () => {
  const entries = [{ keys: ['a'] }, null, { keys: ['c'] }];
  const result = sanitizeLoreEntries(entries);
  assert.equal(result.valid.length, 2);
  assert.equal(result.valid[0].keys[0], 'a');
  assert.equal(result.valid[1].keys[0], 'c');
  assert.equal(result.skipped.length, 1);
  assert.equal(result.skipped[0].index, 1);
  assert.match(result.skipped[0].reason, /不是对象/);
});

test('sanitizeLoreEntries: 数组含 undefined → 跳过并报告', () => {
  const entries = [undefined, { keys: ['b'] }];
  const result = sanitizeLoreEntries(entries);
  assert.equal(result.valid.length, 1);
  assert.equal(result.skipped[0].index, 0);
  assert.match(result.skipped[0].reason, /不是对象/);
});

test('sanitizeLoreEntries: 数组含原始类型（数字/字符串/布尔）→ 跳过', () => {
  const entries = [42, 'foo', true, { keys: ['ok'] }];
  const result = sanitizeLoreEntries(entries);
  assert.equal(result.valid.length, 1);
  assert.equal(result.valid[0].keys[0], 'ok');
  assert.equal(result.skipped.length, 3);
  assert.deepEqual(result.skipped.map(s => s.index), [0, 1, 2]);
});

test('sanitizeLoreEntries: 数组元素为数组 → 跳过并标记为 "数组而非对象"', () => {
  // 数组 typeof === 'object'，但显然不是合法 entry；必须显式拒绝以免污染渲染。
  const entries = [['not', 'an', 'entry'], { keys: ['ok'] }];
  const result = sanitizeLoreEntries(entries);
  assert.equal(result.valid.length, 1);
  assert.equal(result.skipped.length, 1);
  assert.equal(result.skipped[0].index, 0);
  assert.match(result.skipped[0].reason, /数组而非对象/);
});

test('sanitizeLoreEntries: 混合正常 + 坏条目 → 只保留正常，按原索引报告坏条目', () => {
  // 真实持久化脏数据场景：好条目 + null + 数组 + 字符串 + 又一个好条目
  const entries = [
    { keys: ['good1'], content: 'one' },
    null,
    ['bad', 'array'],
    'string',
    { keys: ['good2'], content: 'two' },
  ];
  const result = sanitizeLoreEntries(entries);
  assert.equal(result.valid.length, 2);
  assert.equal(result.valid[0].keys[0], 'good1');
  assert.equal(result.valid[1].keys[0], 'good2');
  assert.equal(result.skipped.length, 3);
  assert.deepEqual(result.skipped.map(s => s.index), [1, 2, 3]);
  // 每个跳过项都有可读原因
  for (const s of result.skipped) {
    assert.ok(typeof s.reason === 'string' && s.reason.length > 0);
  }
});

test('sanitizeLoreEntries: 返回 valid 数组是新数组，不引用输入', () => {
  const entries = [{ keys: ['a'] }, { keys: ['b'] }];
  const result = sanitizeLoreEntries(entries);
  // valid 是新数组，但 entry 引用保留（UI 仍按引用编辑 entry）
  assert.notEqual(result.valid, entries);
  assert.equal(result.valid.length, entries.length);
});

test('sanitizeLoreEntries: 空对象 {} 视为合法 entry（保守不崩）', () => {
  // 空对象形状虽然缺 keys/content，但 renderLoreEntry 已用 || 兜底；
  // 形状校验只拒绝"非对象"，不强制字段 schema，避免过度收紧。
  const entries = [{}, { keys: ['ok'] }];
  const result = sanitizeLoreEntries(entries);
  assert.equal(result.valid.length, 2);
  assert.deepEqual(result.skipped, []);
});
