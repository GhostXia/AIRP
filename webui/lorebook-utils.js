// #126 D-PR2: Worldbook WebUI 管理迁移纯函数
// 运行：node --test webui/tests/lorebook.test.mjs
(function (root, factory) {
  const api = factory();
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  if (root) root.AIRPLorebookUtils = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  // secondary_keys 输入框文本 → 规范化数组。
  // 按逗号拆分，每个 token 去除首尾空白并移除空 token；保留首次出现的输入顺序，
  // 重复 token 只保留第一次。例：'writer, roleplay, writer' → ['writer', 'roleplay']。
  function parseSecondaryKeys(input) {
    if (typeof input !== 'string') return [];
    const seen = new Set();
    const result = [];
    for (const token of input.split(',')) {
      const trimmed = token.trim();
      if (trimmed && !seen.has(trimmed)) {
        seen.add(trimmed);
        result.push(trimmed);
      }
    }
    return result;
  }

  // 新建 lorebook entry 的默认值。v4 起 selective/secondary_keys 为 canonical 字段。
  function buildLoreEntryDefault() {
    return {
      keys: [],
      content: '',
      enabled: true,
      priority: 10,
      constant: false,
      comment: null,
      selective: false,
      secondary_keys: [],
    };
  }

  // 收集 entry 的 advisory 字段供只读展示。
  // 两条读取路径必须分开：top-level case_sensitive 不在 extensions 中读取；
  // extensions 中的 selective 已在 v4 提升为 canonical，跳过。
  // 返回 [{ label, value }]，value 为字符串展示形式。
  function collectAdvisoryFields(entry) {
    if (!entry || typeof entry !== 'object') return [];
    const fields = [];
    // top-level case_sensitive（advisory，不影响运行时 trigger 模式）
    if (entry.case_sensitive !== undefined && entry.case_sensitive !== null) {
      fields.push({ label: 'case_sensitive', value: String(entry.case_sensitive) });
    }
    // extensions advisory 字段（position/depth/probability/recursion 及未知字段）
    const ext = entry.extensions;
    if (ext && typeof ext === 'object') {
      for (const [key, value] of Object.entries(ext)) {
        // selective 已提升为 canonical，不再在 extensions 展示
        if (key === 'selective') continue;
        const display = typeof value === 'object' ? JSON.stringify(value) : String(value);
        fields.push({ label: key, value: display });
      }
    }
    return fields;
  }

  return { parseSecondaryKeys, buildLoreEntryDefault, collectAdvisoryFields };
});
