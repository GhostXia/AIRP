// #126 D-PR2: Worldbook WebUI 管理迁移纯函数测试
// 运行：node --test webui/tests/lorebook.test.mjs
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { parseSecondaryKeys, buildLoreEntryDefault, collectAdvisoryFields } = require('../lorebook-utils.js');

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
  // extensions 中的同名 key 仍原样展示（advisory），两者各显示一次。
  const entry = { case_sensitive: false, extensions: { case_sensitive: true } };
  const fields = collectAdvisoryFields(entry);
  const csFields = fields.filter(f => f.label === 'case_sensitive');
  assert.equal(csFields.length, 2);
  assert.equal(csFields[0].value, 'false'); // top-level
  assert.equal(csFields[1].value, 'true');  // extensions
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
