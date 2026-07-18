(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  else root.AIRPAssemblyUtils = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  const KIND_LABELS = {
    card: '角色卡',
    known: '关卡已知信息',
    persona: '用户身份',
    lorebook: '世界书',
    state: '角色状态',
    preset: '预设',
    scene: '场景',
    memory: '记忆与上下文',
    history: '对话历史',
    user: '本轮消息',
  };

  // #114 effective config summary：来源标签的稳定中文映射。
  // engine 侧用 snake_case 字符串（"explicit"/"session_binding"/"request"/"preset"/"snapshot"），
  // WebUI 在此统一翻译为用户可读短语；未知值原样保留以便排查。
  const PERSONA_SOURCE_LABELS = {
    explicit: '显式',
    session_binding: '会话绑定',
    character_binding: '角色绑定',
    default: '默认',
    absent: '未启用',
  };
  const PARAM_SOURCE_LABELS = {
    request: '请求',
    preset: '预设',
    snapshot: '默认',
  };

  function valueOrFallback(value, fallback) {
    return value === undefined || value === null || value === '' ? fallback : String(value);
  }

  function sourceLabel(source, table) {
    if (source === undefined || source === null || source === '') return '';
    if (table[source]) return table[source];
    // #221 L4：未知 source 值（engine 未来新增）回退到原始字符串，
    // 并在控制台告警便于开发者发现 label table 未同步。
    if (typeof console !== 'undefined' && console.warn) {
      console.warn('unknown source label:', source);
    }
    return String(source);
  }

  // 把来源后缀附加到既有 value 上；source 为空时原样返回，保持向后兼容。
  function withSourceSuffix(value, source, table) {
    const label = sourceLabel(source, table);
    return label ? value + ' · ' + label : value;
  }

  // #115 Phase 2h：chips 从 5 项扩展到 8 项（新增 世界书 / 状态 / 记忆），
  // withRevision 联动 diagnostics：当对应 asset 有 *_revision_unavailable 诊断时
  // 显示 `· unavailable` 标识。无诊断时不显示（向后兼容旧测试）。
  function withRevision(value, revision, fallback, diagnostics, unavailableKind) {
    const label = valueOrFallback(value, fallback);
    if (revision !== undefined && revision !== null) {
      return label + ' · r' + revision;
    }
    if (diagnostics && unavailableKind && diagnostics.some(d => d && d.kind === unavailableKind)) {
      return label + ' · unavailable';
    }
    return label;
  }

  // Gemini 审计修复：世界书 / 状态 / 记忆 没有独立 ID 字段，`value` 直接传 asset 标签
  // 会导致 asset 未激活时仍显示标签（如 "世界书"）而非回退到 "未启用"。
  // withAssetRevision 仅在 asset 激活（有 revision 或 unavailable 诊断）时展示标签，
  // 未激活时回退到 fallback，语义与 character/persona/preset chip 一致。
  function withAssetRevision(assetLabel, revision, fallback, diagnostics, unavailableKind) {
    const hasDiagnostic = diagnostics && unavailableKind
      && diagnostics.some(d => d && d.kind === unavailableKind);
    const isActive = (revision !== undefined && revision !== null) || hasDiagnostic;
    return withRevision(isActive ? assetLabel : null, revision, fallback, diagnostics, unavailableKind);
  }

  // #114 effective config summary：构造温度/max_tokens chip 的 value。
  // - 有生效值 + 有来源 → `<value> · <来源>`
  // - 有生效值 + 无来源（理论不会发生，但容错） → `<value>`
  // - 无生效值 → fallback（"未设置"）
  function withParamSource(value, source, table, fallback) {
    if (value === undefined || value === null) return fallback;
    const base = String(value);
    const label = sourceLabel(source, table);
    return label ? base + ' · ' + label : base;
  }

  function buildAssemblyViewModel(trace) {
    if (!trace || typeof trace !== 'object') return null;
    const effective = trace.effective || {};
    const segments = Array.isArray(trace.segments) ? trace.segments : [];
    const diagnostics = Array.isArray(trace.diagnostics) ? trace.diagnostics : [];

    // #114：身份 chip 增加 persona_activation_source 后缀；
    // absent 时不附加（保持向后兼容：无 user_id 时 persona 本就不展示）。
    const personaBase = withRevision(effective.persona_id, effective.persona_revision, '未启用', diagnostics, 'persona_revision_unavailable');
    const personaValue = effective.persona_activation_source && effective.persona_activation_source !== 'absent'
      ? withSourceSuffix(personaBase, effective.persona_activation_source, PERSONA_SOURCE_LABELS)
      : personaBase;

    // 模型 / 服务 chip 增加来源后缀
    const modelBase = valueOrFallback(effective.model, '未配置');
    const modelValue = withSourceSuffix(modelBase, effective.model_source, PARAM_SOURCE_LABELS);
    const providerBase = valueOrFallback(effective.provider, '未知');
    const providerValue = withSourceSuffix(providerBase, effective.provider_source, PARAM_SOURCE_LABELS);

    // 温度 / 最大 tokens 新增 chip：未提供时显示 "未设置"
    const temperatureValue = withParamSource(effective.temperature, effective.temperature_source, PARAM_SOURCE_LABELS, '未设置');
    const maxTokensValue = withParamSource(effective.max_tokens, effective.max_tokens_source, PARAM_SOURCE_LABELS, '未设置');

    return {
      chips: [
        { label: '角色', value: withRevision(effective.character_id, effective.character_revision, '未选择', diagnostics, 'character_revision_unavailable') },
        { label: '身份', value: personaValue },
        { label: '预设', value: withRevision(effective.preset_id, effective.preset_revision, '未启用', diagnostics, 'preset_revision_unavailable') },
        { label: '世界书', value: withAssetRevision('世界书', effective.lorebook_revision, '未启用', diagnostics, 'lorebook_revision_unavailable') },
        { label: '状态', value: withAssetRevision('状态', effective.state_revision, '未启用', diagnostics, 'state_revision_unavailable') },
        { label: '记忆', value: withAssetRevision('记忆', effective.memory_revision, '未启用', diagnostics, 'memory_revision_unavailable') },
        { label: '模型', value: modelValue },
        { label: '服务', value: providerValue },
        { label: '温度', value: temperatureValue },
        { label: '最大 tokens', value: maxTokensValue },
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
