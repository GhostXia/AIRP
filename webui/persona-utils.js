(function (root, factory) {
  const api = factory();
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
  if (root) root.AIRPPersonaUtils = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  function describeEffectiveHint(selectedPersonaId, effectivePersona) {
    if (selectedPersonaId !== '') {
      return '已选择：' + selectedPersonaId + '（explicit）';
    }
    if (!effectivePersona || !effectivePersona.persona) return '—';
    const name = effectivePersona.persona.name || 'User';
    switch (effectivePersona.source) {
      case 'session_binding': return '生效：' + name + '（来自会话绑定）';
      case 'character_binding': return '生效：' + name + '（来自角色绑定）';
      case 'default': return '生效：' + name + '（默认）';
      default: return '—';
    }
  }

  function buildBindAction(state, scope) {
    const { selectedPersonaId, selectedChar, selectedSess, effectivePersona } = state;
    if (!selectedChar || selectedPersonaId === '' || !effectivePersona) return null;
    if (scope === 'session' && !selectedSess) return null;
    const owner = scope === 'character'
      ? (effectivePersona.bindings && effectivePersona.bindings.character_persona_id)
      : (effectivePersona.bindings && effectivePersona.bindings.session_persona_id);
    if (!owner) {
      return { kind: 'bind', personaId: selectedPersonaId, label: scope === 'character' ? '绑定到角色' : '绑定到会话' };
    }
    if (owner === selectedPersonaId) {
      return { kind: 'unbind', personaId: selectedPersonaId, label: scope === 'character' ? '解绑角色' : '解绑会话' };
    }
    return { kind: 'unbind', personaId: owner, label: '先解绑 ' + owner };
  }

  function buildPersonaPayload(userId, selectedPersonaId) {
    const payload = { user_id: userId };
    if (selectedPersonaId) payload.persona_id = selectedPersonaId;
    return payload;
  }

  return { describeEffectiveHint, buildBindAction, buildPersonaPayload };
});
