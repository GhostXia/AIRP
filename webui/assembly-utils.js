(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.AIRPAssemblyUtils = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  const KIND_LABELS = {
    card: '角色卡',
    persona: '用户身份',
    lorebook: '世界书',
    state: '角色状态',
    preset: '预设',
    scene: '场景',
    memory: '记忆与上下文',
    history: '对话历史',
    user: '本轮消息',
  };

  function valueOrFallback(value, fallback) {
    return value === undefined || value === null || value === '' ? fallback : String(value);
  }

  function withRevision(value, revision, fallback) {
    const label = valueOrFallback(value, fallback);
    return revision === undefined || revision === null ? label : label + ' · r' + revision;
  }

  function buildAssemblyViewModel(trace) {
    if (!trace || typeof trace !== 'object') return null;
    const effective = trace.effective || {};
    const segments = Array.isArray(trace.segments) ? trace.segments : [];
    const diagnostics = Array.isArray(trace.diagnostics) ? trace.diagnostics : [];
    return {
      chips: [
        { label: '角色', value: withRevision(effective.character_id, effective.character_revision, '未选择') },
        { label: '身份', value: withRevision(effective.persona_id, effective.persona_revision, '未启用') },
        { label: '预设', value: withRevision(effective.preset_id, effective.preset_revision, '未启用') },
        { label: '模型', value: valueOrFallback(effective.model, '未配置') },
        { label: '服务', value: valueOrFallback(effective.provider, '未知') },
      ],
      metrics: segments.length + ' 项 · 约 ' + Number(trace.total_estimated_tokens || 0).toLocaleString() + ' tokens',
      segments: segments.map((segment, index) => ({
        order: index + 1,
        label: KIND_LABELS[segment.source_kind] || valueOrFallback(segment.source_kind, '未知来源'),
        identity: valueOrFallback(segment.display_name || segment.item_id || segment.source_id, ''),
        stability: segment.stable_or_volatile === 'volatile' ? '随对话变化' : '配置材料',
        stabilityClass: segment.stable_or_volatile === 'volatile' ? 'volatile' : 'stable',
        size: Number(segment.chars || 0).toLocaleString() + ' 字符 · 约 ' + Number(segment.estimated_tokens || 0).toLocaleString() + ' tokens',
        truncated: Boolean(segment.truncated),
      })),
      diagnostics: diagnostics.map(item => valueOrFallback(item && item.message, '')).filter(Boolean),
    };
  }

  return { buildAssemblyViewModel };
});
