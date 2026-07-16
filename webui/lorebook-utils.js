// #126 D-PR2: Worldbook WebUI 管理迁移纯函数
// 运行：node --test webui/tests/lorebook.test.mjs
(function (root, factory) {
  const api = factory();
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  if (root) root.AIRPLorebookUtils = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  // secondary_keys 输入框文本 → 规范化数组。
  // 按逗号拆分（ASCII ',' 与全角 '，' 均支持），每个 token 去除首尾空白并移除空 token；
  // 保留首次出现的输入顺序，重复 token 只保留第一次。
  // 例：'writer, roleplay， writer' → ['writer', 'roleplay']
  function parseSecondaryKeys(input) {
    if (typeof input !== 'string') return [];
    // CR#3: 同时按 ASCII 逗号与全角逗号拆分，中文输入法下也易输入
    const tokens = input.split(/[,，]/);
    const seen = new Set();
    const result = [];
    for (const token of tokens) {
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
  // CR-nitpick: 当 top-level 与 extensions 同时存在 case_sensitive 时，
  // extensions 来源的标签加 ` (extensions)` 后缀以区分来源，避免用户看到两条同名行无法分辨。
  function collectAdvisoryFields(entry) {
    if (!entry || typeof entry !== 'object') return [];
    const fields = [];
    const hasTopLevelCaseSensitive = entry.case_sensitive !== undefined && entry.case_sensitive !== null;
    // top-level case_sensitive（advisory，不影响运行时 trigger 模式）
    if (hasTopLevelCaseSensitive) {
      fields.push({ label: 'case_sensitive', value: String(entry.case_sensitive) });
    }
    // extensions advisory 字段（position/depth/probability/recursion 及未知字段）
    const ext = entry.extensions;
    if (ext && typeof ext === 'object') {
      for (const [key, value] of Object.entries(ext)) {
        // selective 已提升为 canonical，不再在 extensions 展示
        if (key === 'selective') continue;
        const display = typeof value === 'object' ? JSON.stringify(value) : String(value);
        // CR-nitpick: 若 top-level 已有 case_sensitive，extensions 中的同名 key 用源限定标签
        const label = (key === 'case_sensitive' && hasTopLevelCaseSensitive)
          ? 'case_sensitive (extensions)'
          : key;
        fields.push({ label, value: display });
      }
    }
    return fields;
  }

  return { parseSecondaryKeys, buildLoreEntryDefault, collectAdvisoryFields };
});
