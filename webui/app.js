// AIRP Engine Console — backend validation harness (M1)
// Zero-build native JS.  plan: docs/WEBUI-BACKEND-PLAN.md

(function () {
  'use strict';

  const { describeEffectiveHint, buildBindAction, buildPersonaPayload } = window.AIRPPersonaUtils;

  // ── DOM refs ─────────────────────────────────────────────────────────────
  const $ = (s) => document.querySelector(s);
  const $$ = (s) => document.querySelectorAll(s);
  const runtimeConfig = window.AIRP_WEBUI_CONFIG || { mode: 'development' };
  const productionMode = runtimeConfig.mode === 'production';
  const engineUrl = $('#engine-url');
  const bearerToken = $('#bearer-token');
  const btnConnect = $('#btn-connect');
  const connStatus = $('#conn-status');
  const connText = $('#conn-text');
  const accessKeyWarning = $('#access-key-warning');
  const healthInfo = $('#health-info');
  const settingsDisplay = $('#settings-display');
  const modelsDisplay = $('#models-display');
  const btnRefreshModels = $('#btn-refresh-models');
  const providerSettings = $('#provider-settings');
  const providerKind = $('#provider-kind');
  const providerEndpoint = $('#provider-endpoint');
  const providerModel = $('#provider-model');
  const providerApiKey = $('#provider-api-key');
  const providerSaveStatus = $('#provider-save-status');
  const btnSaveProvider = $('#btn-save-provider');
  const charSelect = $('#char-select');
  const sessSelect = $('#sess-select');
  const chatLog = $('#chat-log');
  const chatInput = $('#chat-input');
  const btnSend = $('#btn-send');
  const btnStop = $('#btn-stop');
  const btnHistory = $('#btn-history');
  const btnRegen = $('#btn-regen');
  const btnRollback = $('#btn-rollback');
  const historyToolbar = $('#history-toolbar');
  const btnLoadEarlier = $('#btn-load-earlier');
  const historyStatus = $('#history-status');
  const agentInput = $('#agent-input');
  const btnAgentRun = $('#btn-agent-run');
  const agentOutput = $('#agent-output');
  const agentToolCatalog = $('#agent-tool-catalog');
  const btnAgentTools = $('#btn-agent-tools');
  const eventLog = $('#event-log');
  const btnClearLog = $('#btn-clear-log');
  const btnRefreshChars = $('#btn-refresh-chars');
  const btnNewSession = $('#btn-new-session');
  const charAvatar = $('#char-avatar');
  const stateDisplay = $('#state-display');
  const stateHistoryDisplay = $('#state-history-display');
  const stateHistoryLimit = $('#state-history-limit');
  const btnRefreshState = $('#btn-refresh-state');
  const characterList = $('#character-list');
  const btnDeleteChar = $('#btn-delete-char');
  const btnDeleteSession = $('#btn-delete-session');
  const personaForm = $('#persona-form');
  const personaUserId = $('#persona-user-id');
  const personaName = $('#persona-name');
  const personaDescription = $('#persona-description');
  const personaVariables = $('#persona-variables');
  const personaStatus = $('#persona-status');
  const btnLoadPersona = $('#btn-load-persona');
  const btnSavePersona = $('#btn-save-persona');
  const personaSelect = $('#persona-select');
  const btnNewPersona = $('#btn-new-persona');
  const btnDeletePersona = $('#btn-delete-persona');
  const personaNewIdRow = $('#persona-new-id-row');
  const personaNewId = $('#persona-new-id');
  const btnCreatePersona = $('#btn-create-persona');
  const btnCancelCreatePersona = $('#btn-cancel-create-persona');
  const btnBindCharacter = $('#btn-bind-character');
  const btnBindSession = $('#btn-bind-session');
  const personaEffectiveHint = $('#persona-effective-hint');
  const presetSelect = $('#preset-select');
  const presetImportFile = $('#preset-import-file');
  const presetImportId = $('#preset-import-id');
  const btnImportPreset = $('#btn-import-preset');
  const presetStatus = $('#preset-status');

  // ── state ────────────────────────────────────────────────────────────────
  let base = productionMode ? window.location.origin : engineUrl.value.replace(/\/+$/, '');
  let bearer = '';
  let selectedChar = '';
  let selectedSess = '';
  let personaRevision = 0;
  let activePersona = { name: 'User', description: '', variables: {} };
  let selectedPersonaId = 'default';
  let creatingPersona = false;
  // #114 C-PR1：缓存最近一次 effective 端点结果，供按钮决策与 hint 展示。
  // null = 尚未查询或查询失败；effectivePersona.source ∈
  // 'session_binding' | 'character_binding' | 'default'。
  let effectivePersona = null;
  let effectivePersonaRequestId = 0;
  let abortController = null;   // for chat SSE
  let agentAbort = null;        // for agent run SSE — 二次点击先 abort 前一个，防事件交错竞态（issue #43/#44 D）
  const HISTORY_PAGE_SIZE = 50;
  const countFormatter = new Intl.NumberFormat();
  let historyRequestSeq = 0;
  let historyState = { scope: '', oldestId: '', hasMore: false, total: 0, loading: false };
  let selectedMessageId = '';
  const messageNodes = new Map();

  // ── event log ────────────────────────────────────────────────────────────
  function logEvent(method, path, status, ms, detail) {
    const el = document.createElement('div');
    el.className = 'event';
    const now = new Date().toLocaleTimeString();
    const code = Number(status);
    const cls = code >= 200 && code < 300 ? 'ok' : code >= 500 || code === 0 ? 'err' : 'unknown';
    appendInline(el, 'span', 'ts', now);
    el.append(' ');
    appendInline(el, 'span', 'status ' + cls, String(status));
    el.append(' ');
    appendInline(el, 'span', 'method', String(method));
    el.append(' ');
    appendInline(el, 'span', 'path', String(path));
    el.append(' ' + ms + 'ms');
    if (detail) {
      el.appendChild(document.createElement('br'));
      appendInline(el, 'span', 'detail', String(detail));
    }
    eventLog.prepend(el);
    if (eventLog.children.length > 200) eventLog.removeChild(eventLog.lastChild);
  }

  function appendInline(parent, tag, className, text) {
    const node = document.createElement(tag);
    node.className = className;
    node.textContent = text;
    parent.appendChild(node);
    return node;
  }

  // ── HTTP helpers ─────────────────────────────────────────────────────────
  function headers(extra) {
    const h = { 'Content-Type': 'application/json', ...extra };
    if (bearer) h['Authorization'] = 'Bearer ' + bearer;
    return h;
  }

  async function api(method, path, body) {
    const url = base + path;
    const t0 = performance.now();
    try {
      const opts = { method, headers: headers() };
      if (body !== undefined) opts.body = JSON.stringify(body);
      const res = await fetch(url, opts);
      const ms = Math.round(performance.now() - t0);
      const text = await res.text();
      logEvent(method, path, res.status, ms);
      let data;
      try { data = JSON.parse(text); } catch { data = text; }
      return { ok: res.ok, status: res.status, data, text };
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      logEvent(method, path, 0, ms, e.message);
      return { ok: false, status: 0, data: null, text: e.message };
    }
  }

  // ── connection ───────────────────────────────────────────────────────────
  // #67 #9 fix: formatError 用白名单展开已知字段，其余字段折叠为 raw JSON 显示。
  // 避免 engine 错误模型扩展（如 request_id / hint / suggestion）时 webui 自动丢失。
  function formatError(data, text) {
    if (data && typeof data === 'object' && data.error) {
      const err = data.error;
      const KNOWN_FIELDS = ['code', 'message', 'upstream_status', 'upstream_body', 'detail'];
      const lines = [err.code || 'error'];
      if (err.message) lines.push(err.message);
      if (err.upstream_status) lines.push('upstream_status=' + err.upstream_status);
      if (err.upstream_body) lines.push('upstream_body=' + err.upstream_body);
      if (err.detail) lines.push('detail=' + err.detail);
      // 折叠未知字段为 raw JSON（排除已知字段和已展开的字段）
      const extras = {};
      let hasExtra = false;
      for (const k of Object.keys(err)) {
        if (!KNOWN_FIELDS.includes(k)) {
          extras[k] = err[k];
          hasExtra = true;
        }
      }
      if (hasExtra) lines.push('extras=' + JSON.stringify(extras));
      return lines.join('\n');
    }
    if (typeof data === 'string' && data) return data;
    if (text) return text;
    return JSON.stringify(data);
  }

  async function connect() {
    // #68 #5: 任何路径进入 connect() 都取消 pending auto-connect，避免
    // keydown Enter / btn-click 后 300ms 又被 setTimeout 触发一次（重复请求）。
    cancelAutoConnect();
    base = productionMode ? window.location.origin : engineUrl.value.replace(/\/+$/, '');
    bearer = productionMode ? '' : (bearerToken.value || '');
    // W-01: 持久化到 sessionStorage（关 tab 即清，缩短泄漏 token 的存活窗口）
    // 注意：sessionStorage 不缓解 XSS——同 tab 任意脚本仍可读。选它而非 localStorage
    // 只是为了让「tab 关闭后 token 失效」，降低意外跨会话复用的风险。
    if (!productionMode) {
      try {
        sessionStorage.setItem('airp_engine_url', base);
        sessionStorage.setItem('airp_bearer', bearer);
      } catch {}
    }
    connStatus.className = 'status-dot dot-unknown';
    connText.textContent = '连接中…';
    const r = await api('GET', '/version');
    if (r.ok) {
      // WEBUI-BACKEND-PLAN §4.2: /health 就绪探针，区分"engine 起了"与"能跑对话"
      const h = await api('GET', '/health');
      let detail = (r.data?.version || r.text).slice(0, 40);
      if (h.ok && h.data) {
        if (!h.data.provider_configured) detail += '  ⚠ provider 未配置';
        if (!h.data.data_root_writable) detail += '  ⚠ data_root 不可写';
      }
      connStatus.className = 'status-dot dot-ok';
      connText.textContent = '已连接  ' + detail;
      refreshAll();
    } else {
      connStatus.className = 'status-dot dot-err';
      connText.textContent = '连接失败: ' + formatError(r.data, r.text);
    }
  }

  btnConnect.addEventListener('click', connect);
  engineUrl.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });
  bearerToken.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });

  // #68 #5 fix: auto-connect 竞态保护。用户在 300ms 延迟内编辑 URL/bearer
  // 会触发 connect 读半截值，发请求到不存在的主机污染 event log。
  // 改成：用户输入时取消 pending auto-connect，由 Enter/click 显式触发。
  let pendingAutoConnect = null;
  function scheduleAutoConnect() {
    if (pendingAutoConnect) clearTimeout(pendingAutoConnect);
    pendingAutoConnect = setTimeout(() => {
      pendingAutoConnect = null;
      connect();
    }, 300);
  }
  function cancelAutoConnect() {
    if (pendingAutoConnect) {
      clearTimeout(pendingAutoConnect);
      pendingAutoConnect = null;
    }
  }
  engineUrl.addEventListener('input', cancelAutoConnect);
  bearerToken.addEventListener('input', cancelAutoConnect);

  // ── refresh all left-panel data ──────────────────────────────────────────
  async function refreshAll() {
    await Promise.all([
      refreshHealth(),
      refreshSettings(),
      refreshModels(),
      refreshChars(),
      refreshAgentTools(),
      refreshPersonaList().then(refreshPersona),
      refreshPresets(),
    ]);
    // 初次连接后自动加载当前角色的 chat history（PLAN §9 P1 "交互收口"）。
    // refreshChars 内部已设置 selectedChar；此处 await 完成后即可拉 history。
    if (selectedChar) loadHistory();
  }

  async function refreshHealth() {
    const r = await api('GET', '/version');
    if (r.ok) healthInfo.textContent = 'version: ' + (r.data?.version || r.text);
    else healthInfo.textContent = 'err: ' + formatError(r.data, r.text);
  }

  async function refreshAgentTools() {
    if (!agentToolCatalog) return;
    agentToolCatalog.textContent = '加载中…';
    const response = await api('GET', '/v1/agent/tools');
    if (!response.ok || !Array.isArray(response.data)) {
      agentToolCatalog.textContent = '工具目录加载失败：' + formatError(response.data, response.text);
      return;
    }
    agentToolCatalog.replaceChildren();
    for (const tool of response.data) {
      const row = document.createElement('div');
      row.className = 'agent-tool-option';

      const allow = document.createElement('input');
      allow.type = 'checkbox';
      allow.className = 'agent-tool-allow';
      allow.dataset.toolName = String(tool.name || '');
      allow.setAttribute('aria-label', '允许工具 ' + String(tool.name || ''));
      row.appendChild(allow);

      const details = document.createElement('span');
      details.className = 'agent-tool-details';
      appendInline(details, 'strong', '', String(tool.name || 'unnamed'));
      appendInline(details, 'small', '', String(tool.description || ''));
      row.appendChild(details);

      const effect = document.createElement('span');
      effect.className = 'agent-tool-effect effect-' + String(tool.side_effect || 'unknown');
      effect.textContent = String(tool.side_effect || 'unknown');
      row.appendChild(effect);

      if (tool.side_effect === 'destructive') {
        const confirmLabel = document.createElement('label');
        confirmLabel.className = 'agent-tool-confirm-label';
        const confirmBox = document.createElement('input');
        confirmBox.type = 'checkbox';
        confirmBox.className = 'agent-tool-confirm';
        confirmBox.dataset.toolName = String(tool.name || '');
        confirmBox.addEventListener('change', () => {
          if (confirmBox.checked) allow.checked = true;
        });
        confirmLabel.append(confirmBox, '确认');
        row.appendChild(confirmLabel);
      }
      agentToolCatalog.appendChild(row);
    }
  }

  if (btnAgentTools) btnAgentTools.addEventListener('click', refreshAgentTools);

  function commaSeparatedValues(selector) {
    return (document.querySelector(selector)?.value || '')
      .split(',')
      .map(value => value.trim())
      .filter(Boolean);
  }

  function checkedToolNames(selector) {
    return Array.from(document.querySelectorAll(selector + ':checked'))
      .map(input => input.dataset.toolName)
      .filter(Boolean);
  }

  function uniqueValues(values) {
    return Array.from(new Set(values));
  }

  async function refreshSettings() {
    const r = await api('GET', '/v1/settings');
    if (r.ok) {
      const s = { ...r.data };
      if (s.api_key) s.api_key = maskSecret(s.api_key);
      if (s.access_api_key) s.access_api_key = maskSecret(s.access_api_key);
      settingsDisplay.textContent = JSON.stringify(s, null, 2);
      if (providerKind && s.provider) providerKind.value = s.provider;
      if (providerEndpoint) providerEndpoint.value = s.endpoint || '';
      if (providerModel) providerModel.value = s.model || '';
      if (accessKeyWarning) {
        const unprotected = s.access_api_key_set === false;
        accessKeyWarning.hidden = !unprotected;
        accessKeyWarning.textContent = unprotected
          ? '未设置 Bearer；仅限受信的本地开发环境。'
          : '';
      }
    } else {
      settingsDisplay.textContent = 'err: ' + formatError(r.data, r.text);
      if (accessKeyWarning) accessKeyWarning.hidden = true;
    }
  }

  async function refreshModels() {
    if (!modelsDisplay) return;
    modelsDisplay.textContent = 'loading...';
    const r = await api('GET', '/v1/models');
    if (r.ok) {
      const models = Array.isArray(r.data?.data) ? r.data.data : null;
      modelsDisplay.textContent = models
        ? models.map(m => m.id || JSON.stringify(m)).join('\n')
        : JSON.stringify(r.data, null, 2);
    } else {
      modelsDisplay.textContent = 'err:\n' + formatError(r.data, r.text);
    }
  }

  async function saveProviderSettings(event) {
    event.preventDefault();
    const endpoint = providerEndpoint.value.trim();
    const model = providerModel.value.trim();
    if (!endpoint || !model) {
      providerSaveStatus.textContent = 'Endpoint 和 Model 必填';
      return;
    }
    providerSaveStatus.textContent = '应用中…';
    if (btnSaveProvider) btnSaveProvider.disabled = true;
    try {
      const payload = { provider: providerKind.value, endpoint, model };
      const key = providerApiKey.value.trim();
      if (key) payload.api_key = key;
      const saved = await api('POST', '/v1/settings', payload);
      providerApiKey.value = '';
      if (!saved.ok) {
        providerSaveStatus.textContent = '保存失败: ' + formatError(saved.data, saved.text);
        return;
      }
      const validation = await api('GET', '/v1/models');
      providerSaveStatus.textContent = validation.ok
        ? '已应用；真实 /v1/models provider 请求通过'
        : '已应用；provider 验证失败: ' + formatError(validation.data, validation.text);
      await Promise.all([refreshSettings(), refreshModels()]);
    } finally {
      if (btnSaveProvider) btnSaveProvider.disabled = false;
    }
  }

  if (providerSettings) providerSettings.addEventListener('submit', saveProviderSettings);

  function maskSecret(value) {
    const s = String(value);
    if (s.length <= 8) return '•'.repeat(Math.max(4, s.length));
    return s.slice(0, 4) + '…' + s.slice(-4);
  }

  async function refreshChars() {
    const r = await api('GET', '/v1/characters');
    if (r.ok) {
      const ids = Array.isArray(r.data) ? r.data : [];
      replaceOptions(charSelect, ids);
      if (ids.length && !ids.includes(selectedChar)) { selectedChar = ids[0]; selectedSess = ''; }
      if (selectedChar && ids.includes(selectedChar)) charSelect.value = selectedChar;
      await renderCharacterCards(ids);
      await refreshSessions();
      refreshAvatar();
      refreshStateAll();
    }
  }

  function replaceOptions(select, ids, labelFn) {
    select.textContent = '';
    ids.forEach(id => {
      const value = String(id);
      const option = document.createElement('option');
      option.value = value;
      option.textContent = labelFn ? labelFn(value) : value;
      // A4 fix: option 文本可能被 <select> 宽度截断（如完整 UUID），加 title
      // 让用户 hover 看完整值。零成本改善可读性。
      option.title = value;
      select.appendChild(option);
    });
  }

  function rememberWorkspace() {
    try {
      const userId = personaUserId ? (personaUserId.value.trim() || 'default') : 'default';
      const presetId = presetSelect ? (presetSelect.value || '') : '';
      localStorage.setItem('airp_user_id', userId);
      localStorage.setItem('airp_preset_id', presetId);
      localStorage.setItem('airp_character_id', selectedChar || '');
      localStorage.setItem('airp_session_id', selectedSess || '');
      // #114 C-PR1：「自动」= 空字符串，需与"未设置"区分。明确存空字符串表示
      // 用户主动选了「自动」；未设置（null）由启动恢复用 getItem(...) === null 判定。
      localStorage.setItem('airp_persona_id', selectedPersonaId);
    } catch {}
  }

  function parsePersonaVariables() {
    let value;
    try { value = JSON.parse(personaVariables.value || '{}'); }
    catch { throw new Error('变量必须是有效 JSON'); }
    if (!value || Array.isArray(value) || typeof value !== 'object') {
      throw new Error('变量必须是 JSON object');
    }
    for (const [key, item] of Object.entries(value)) {
      if (typeof item !== 'string') throw new Error('变量 ' + key + ' 必须是字符串');
    }
    return value;
  }

  async function refreshPersonaList() {
    if (!personaSelect) return;
    const userId = personaUserId.value.trim() || 'default';
    const r = await api('GET', '/v1/users/' + encodeURIComponent(userId) + '/personas');
    if (!r.ok) {
      personaStatus.textContent = 'Persona 列表加载失败: ' + formatError(r.data, r.text);
      personaSelect.textContent = '';
      return;
    }
    const ids = Array.isArray(r.data) ? r.data.map(String) : [];
    personaSelect.textContent = '';
    // #114 C-PR1：顶部插入「自动」option，value="" 表示跟随绑定/默认。
    const autoOption = document.createElement('option');
    autoOption.value = '';
    autoOption.textContent = '自动（跟随绑定/默认）';
    personaSelect.appendChild(autoOption);
    ids.forEach(id => {
      const option = document.createElement('option');
      option.value = id;
      option.textContent = id;
      personaSelect.appendChild(option);
    });
    // selectedPersonaId === '' = 「自动」；必须是 ids 中的合法 id 或空串才保留。
    if (selectedPersonaId === '' || ids.includes(selectedPersonaId)) {
      personaSelect.value = selectedPersonaId;
    } else if (ids.includes('default')) {
      selectedPersonaId = 'default';
      personaSelect.value = 'default';
    }
    updateDeleteButtonState();
  }

  function updateDeleteButtonState() {
    const auto = selectedPersonaId === '';
    if (btnDeletePersona) {
      btnDeletePersona.disabled = auto || selectedPersonaId.toLowerCase() === 'default';
    }
    if (btnSavePersona) btnSavePersona.disabled = auto;
  }

  async function refreshPersona() {
    if (!personaUserId) return;
    if (creatingPersona) return;
    // #114 C-PR1：「自动」选中时表单填 effective persona（只读），不直接读某个 pid。
    if (selectedPersonaId === '') {
      await refreshEffectivePersona();
      return;
    }
    const userId = personaUserId.value.trim() || 'default';
    const pid = selectedPersonaId || 'default';
    personaStatus.textContent = '加载中…';
    const r = await api('GET', '/v1/users/' + encodeURIComponent(userId) + '/personas/' + encodeURIComponent(pid));
    if (!r.ok) {
      personaStatus.textContent = '加载失败: ' + formatError(r.data, r.text);
      return;
    }
    const persona = r.data || {};
    personaRevision = Number(persona.revision) || 0;
    activePersona = {
      name: persona.name || 'User',
      description: persona.description || '',
      variables: persona.variables && typeof persona.variables === 'object' ? persona.variables : {},
    };
    personaName.value = activePersona.name;
    personaDescription.value = activePersona.description;
    personaVariables.value = JSON.stringify(activePersona.variables, null, 2);
    personaName.readOnly = false;
    personaDescription.readOnly = false;
    personaVariables.readOnly = false;
    personaStatus.textContent = pid + ' · revision ' + personaRevision + (persona.updated_at ? ' · ' + persona.updated_at : '');
    rememberWorkspace();
  }

  // ── #114 C-PR1：Effective Persona 查询 + 绑定按钮 ─────────────────────────
  //
  // 查询当前角色/会话下生效的 Persona（binding→default 两层，explicit 层由下拉
  // 本地判定）。结果驱动 hint 展示与绑定/解绑按钮状态。纯函数 describeEffectiveSource
  // 与 buildBindAction 已提取到模块顶层，供 Node 测试。

  async function refreshEffectivePersona() {
    if (!personaUserId) return;
    const requestId = ++effectivePersonaRequestId;
    if (!selectedChar) {
      effectivePersona = null;
      if (personaEffectiveHint) personaEffectiveHint.textContent = '请先选择角色';
      updateBindingButtons();
      return;
    }
    const userId = personaUserId.value.trim() || 'default';
    const characterId = selectedChar;
    const sessionId = selectedSess;
    effectivePersona = null;
    updateBindingButtons();
    if (personaEffectiveHint) personaEffectiveHint.textContent = '生效查询中…';
    let url = '/v1/users/' + encodeURIComponent(userId) + '/persona/effective?character_id=' + encodeURIComponent(selectedChar);
    if (selectedSess) url += '&session_id=' + encodeURIComponent(selectedSess);
    const r = await api('GET', url);
    if (requestId !== effectivePersonaRequestId
      || characterId !== selectedChar
      || sessionId !== selectedSess
      || userId !== (personaUserId.value.trim() || 'default')) return;
    if (!r.ok) {
      effectivePersona = null;
      if (r.status === 404) {
        await refreshPersonaList();
        if (requestId !== effectivePersonaRequestId
          || characterId !== selectedChar
          || sessionId !== selectedSess
          || userId !== (personaUserId.value.trim() || 'default')) return;
      }
      if (personaEffectiveHint) personaEffectiveHint.textContent = '生效查询失败: ' + formatError(r.data, r.text);
      updateBindingButtons();
      return;
    }
    effectivePersona = r.data || null;
    // 「自动」选中时把 effective persona 填入表单（只读）。
    if (selectedPersonaId === '' && effectivePersona && effectivePersona.persona) {
      const p = effectivePersona.persona;
      personaRevision = Number(p.revision) || 0;
      activePersona = {
        name: p.name || 'User',
        description: p.description || '',
        variables: p.variables && typeof p.variables === 'object' ? p.variables : {},
      };
      personaName.value = activePersona.name;
      personaDescription.value = activePersona.description;
      personaVariables.value = JSON.stringify(activePersona.variables, null, 2);
      personaName.readOnly = true;
      personaDescription.readOnly = true;
      personaVariables.readOnly = true;
      personaStatus.textContent = '生效: ' + (p.id || 'default') + ' · revision ' + personaRevision + '（只读，如需编辑请先选择具体 Persona）';
    }
    if (personaEffectiveHint) {
      personaEffectiveHint.textContent = describeEffectiveHint(selectedPersonaId, effectivePersona);
    }
    updateBindingButtons();
    updateDeleteButtonState();
  }

  function updateBindingButtons() {
    const state = {
      selectedPersonaId,
      selectedChar,
      selectedSess,
      effectivePersona,
    };
    const charAction = buildBindAction(state, 'character');
    const sessAction = buildBindAction(state, 'session');
    if (btnBindCharacter) {
      btnBindCharacter.disabled = !charAction;
      btnBindCharacter.textContent = charAction ? charAction.label : '绑定到角色';
    }
    if (btnBindSession) {
      btnBindSession.disabled = !sessAction;
      btnBindSession.textContent = sessAction ? sessAction.label : '绑定到会话';
    }
  }

  async function performBind(scope) {
    if (!personaUserId) return;
    const state = {
      selectedPersonaId,
      selectedChar,
      selectedSess,
      effectivePersona,
    };
    const action = buildBindAction(state, scope);
    if (!action) return;
    const userId = personaUserId.value.trim() || 'default';
    const personaId = action.personaId;
    if (action.kind === 'bind') {
      const body = { character_id: selectedChar };
      if (scope === 'session' && selectedSess) body.session_id = selectedSess;
      const r = await api('POST', '/v1/users/' + encodeURIComponent(userId) + '/personas/' + encodeURIComponent(personaId) + '/bindings', body);
      if (!r.ok) {
        if (personaEffectiveHint) personaEffectiveHint.textContent = '绑定失败: ' + formatError(r.data, r.text);
        return;
      }
    } else {
      // unbind：DELETE owner 的该 scope 绑定
      let url = '/v1/users/' + encodeURIComponent(userId) + '/personas/' + encodeURIComponent(personaId) + '/bindings?character_id=' + encodeURIComponent(selectedChar);
      if (scope === 'session' && selectedSess) url += '&session_id=' + encodeURIComponent(selectedSess);
      const r = await api('DELETE', url);
      if (!r.ok) {
        if (personaEffectiveHint) personaEffectiveHint.textContent = '解绑失败: ' + formatError(r.data, r.text);
        return;
      }
    }
    await refreshEffectivePersona();
  }

  if (btnBindCharacter) btnBindCharacter.addEventListener('click', () => performBind('character'));
  if (btnBindSession) btnBindSession.addEventListener('click', () => performBind('session'));

  async function savePersona(event) {
    event.preventDefault();
    if (creatingPersona) { await createPersona(); return; }
    if (selectedPersonaId === '') {
      personaStatus.textContent = '自动模式只读；请选择具体 Persona 后再保存';
      updateDeleteButtonState();
      return;
    }
    const userId = personaUserId.value.trim() || 'default';
    const pid = selectedPersonaId || 'default';
    let variables;
    try { variables = parsePersonaVariables(); }
    catch (error) { personaStatus.textContent = error.message; return; }
    personaStatus.textContent = '保存中…';
    if (btnSavePersona) btnSavePersona.disabled = true;
    try {
      const payload = {
        expected_revision: personaRevision,
        name: personaName.value.trim() || 'User',
        description: personaDescription.value.trim(),
        variables,
      };
      const r = await api('PUT', '/v1/users/' + encodeURIComponent(userId) + '/personas/' + encodeURIComponent(pid), payload);
      if (!r.ok) {
        personaStatus.textContent = '保存失败: ' + formatError(r.data, r.text);
        return;
      }
      await refreshPersona();
      personaStatus.textContent = '已保存 · ' + personaStatus.textContent;
    } finally {
      updateDeleteButtonState();
    }
  }

  function enterCreateMode() {
    creatingPersona = true;
    if (personaNewIdRow) personaNewIdRow.hidden = false;
    if (btnCreatePersona) btnCreatePersona.hidden = false;
    if (btnCancelCreatePersona) btnCancelCreatePersona.hidden = false;
    if (btnSavePersona) btnSavePersona.hidden = true;
    if (btnLoadPersona) btnLoadPersona.hidden = true;
    if (personaSelect) personaSelect.disabled = true;
    if (btnNewPersona) btnNewPersona.disabled = true;
    if (btnDeletePersona) btnDeletePersona.disabled = true;
    personaNewId.value = '';
    personaName.value = '';
    personaDescription.value = '';
    personaVariables.value = '{}';
    personaStatus.textContent = '填写新 Persona ID 和字段后点创建';
    if (personaNewId) personaNewId.focus();
  }

  function exitCreateMode() {
    creatingPersona = false;
    if (personaNewIdRow) personaNewIdRow.hidden = true;
    if (btnCreatePersona) btnCreatePersona.hidden = true;
    if (btnCancelCreatePersona) btnCancelCreatePersona.hidden = true;
    if (btnSavePersona) btnSavePersona.hidden = false;
    if (btnLoadPersona) btnLoadPersona.hidden = false;
    if (personaSelect) personaSelect.disabled = false;
    if (btnNewPersona) btnNewPersona.disabled = false;
    updateDeleteButtonState();
  }

  async function createPersona() {
    const userId = personaUserId.value.trim() || 'default';
    const pid = personaNewId.value.trim();
    if (!pid) { personaStatus.textContent = '请填写 Persona ID'; return; }
    let variables;
    try { variables = parsePersonaVariables(); }
    catch (error) { personaStatus.textContent = error.message; return; }
    personaStatus.textContent = '创建中…';
    if (btnCreatePersona) btnCreatePersona.disabled = true;
    try {
      const payload = {
        persona_id: pid,
        name: personaName.value.trim() || 'User',
        description: personaDescription.value.trim(),
        variables,
      };
      const r = await api('POST', '/v1/users/' + encodeURIComponent(userId) + '/personas', payload);
      if (!r.ok) {
        personaStatus.textContent = '创建失败: ' + formatError(r.data, r.text);
        return;
      }
      selectedPersonaId = pid;
      exitCreateMode();
      await refreshPersonaList();
      await refreshPersona();
      personaStatus.textContent = '已创建 · ' + personaStatus.textContent;
    } finally {
      if (btnCreatePersona) btnCreatePersona.disabled = false;
    }
  }

  async function deletePersona() {
    if (selectedPersonaId === '') {
      personaStatus.textContent = '自动模式不能删除 Persona';
      updateDeleteButtonState();
      return;
    }
    const userId = personaUserId.value.trim() || 'default';
    const pid = selectedPersonaId || 'default';
    if (pid.toLowerCase() === 'default') return;
    personaStatus.textContent = '删除中…';
    if (btnDeletePersona) btnDeletePersona.disabled = true;
    try {
      const r = await api('DELETE', '/v1/users/' + encodeURIComponent(userId) + '/personas/' + encodeURIComponent(pid));
      if (!r.ok) {
        personaStatus.textContent = '删除失败: ' + formatError(r.data, r.text);
        return;
      }
      selectedPersonaId = 'default';
      await refreshPersonaList();
      await refreshPersona();
      personaStatus.textContent = '已删除 · ' + personaStatus.textContent;
    } finally {
      updateDeleteButtonState();
    }
  }

  async function refreshPresets() {
    if (!presetSelect) return;
    const previous = presetSelect.value;
    const r = await api('GET', '/v1/presets');
    if (!r.ok) {
      presetStatus.textContent = '加载失败: ' + formatError(r.data, r.text);
      return;
    }
    const ids = Array.isArray(r.data) ? r.data.map(String) : [];
    presetSelect.textContent = '';
    const none = document.createElement('option');
    none.value = '';
    none.textContent = '不使用预设';
    presetSelect.appendChild(none);
    ids.forEach(id => {
      const option = document.createElement('option');
      option.value = id;
      option.textContent = id;
      presetSelect.appendChild(option);
    });
    const stored = (() => { try { return localStorage.getItem('airp_preset_id') || ''; } catch { return ''; } })();
    presetSelect.value = ids.includes(previous) ? previous : (ids.includes(stored) ? stored : '');
    presetStatus.textContent = ids.length + ' 个可用预设';
    rememberWorkspace();
  }

  async function importPreset() {
    const file = presetImportFile.files && presetImportFile.files[0];
    const presetId = presetImportId.value.trim();
    if (!file || !presetId) {
      presetStatus.textContent = '请选择 JSON 文件并填写 Preset ID';
      return;
    }
    presetStatus.textContent = '导入中…';
    if (btnImportPreset) btnImportPreset.disabled = true;
    try {
      let presetJson;
      try { presetJson = await file.text(); }
      catch (error) { presetStatus.textContent = '读取失败: ' + error.message; return; }
      const r = await api('POST', '/v1/presets/import', { preset_id: presetId, preset_json: presetJson });
      if (!r.ok) {
        presetStatus.textContent = '导入失败: ' + formatError(r.data, r.text);
        return;
      }
      await refreshPresets();
      presetSelect.value = presetId;
      presetImportFile.value = '';
      presetImportId.value = '';
      rememberWorkspace();
      presetStatus.textContent = '已导入 ' + presetId + ' · prompts ' + (r.data?.prompts_count ?? 0);
    } finally {
      if (btnImportPreset) btnImportPreset.disabled = false;
    }
  }

  if (personaForm) personaForm.addEventListener('submit', savePersona);
  if (btnLoadPersona) btnLoadPersona.addEventListener('click', async () => { await refreshPersonaList(); await refreshPersona(); });
  if (personaSelect) personaSelect.addEventListener('change', () => {
    // #114 C-PR1：保留空串 = 「自动」；不再用 || 'default' 抹掉。
    selectedPersonaId = personaSelect.value;
    updateDeleteButtonState();
    if (selectedPersonaId === '') {
      // 「自动」：表单只读填 effective persona，由 refreshEffectivePersona 驱动。
      refreshEffectivePersona();
    } else {
      refreshPersona();
    }
    rememberWorkspace();
    updateBindingButtons();
  });
  if (btnNewPersona) btnNewPersona.addEventListener('click', enterCreateMode);
  if (btnDeletePersona) btnDeletePersona.addEventListener('click', deletePersona);
  if (btnCreatePersona) btnCreatePersona.addEventListener('click', createPersona);
  if (btnCancelCreatePersona) btnCancelCreatePersona.addEventListener('click', async () => {
    exitCreateMode();
    await refreshPersona();
  });
  if (presetSelect) presetSelect.addEventListener('change', rememberWorkspace);
  if (btnImportPreset) btnImportPreset.addEventListener('click', importPreset);

  // The V2 character page is a view over the same selected character state as
  // the accessibility-friendly select in the sidebar.  Keeping one source of
  // truth avoids duplicating the existing session, avatar, and state flows.
  async function renderCharacterCards(ids) {
    if (!characterList) return;
    characterList.textContent = '';
    if (!ids.length) {
      appendInline(characterList, 'p', 'empty-state', '没有可用角色。请先导入角色卡。');
      return;
    }
    const summaries = await Promise.all(ids.map(async id => {
      const characterId = String(id);
      const r = await api('GET', '/v1/characters/' + encodeURIComponent(characterId));
      const card = r.ok && r.data && typeof r.data === 'object' ? (r.data.data || r.data) : {};
      return {
        id: characterId,
        name: typeof card.name === 'string' && card.name.trim() ? card.name : characterId,
        description: typeof card.description === 'string' && card.description.trim()
          ? card.description
          : '选择后加载会话、聊天记录与状态。',
      };
    }));
    summaries.forEach(summary => {
      const characterId = summary.id;
      const card = document.createElement('button');
      card.type = 'button';
      card.className = 'character-card' + (characterId === selectedChar ? ' selected' : '');
      card.setAttribute('aria-pressed', characterId === selectedChar ? 'true' : 'false');
      appendInline(card, 'span', 'character-card-name', summary.name);
      appendInline(card, 'span', 'character-card-copy', summary.description);
      appendInline(card, 'span', 'character-card-meta', '打开对话空间 →');
      card.addEventListener('click', () => {
        if (charSelect.value !== characterId) {
          charSelect.value = characterId;
          charSelect.dispatchEvent(new Event('change'));
        }
        showView('session');
      });
      characterList.appendChild(card);
    });
  }

  function showView(name) {
    const view = document.getElementById('view-' + name);
    if (!view) return;
    document.querySelectorAll('.app-view').forEach(item => {
      const active = item === view;
      item.hidden = !active;
      item.classList.toggle('active', active);
    });
    document.querySelectorAll('.page-nav-item').forEach(item => {
      item.classList.toggle('active', item.dataset.view === name);
    });
    if (window.location.hash !== '#' + name) window.location.hash = name;
  }

  function showViewFromHash() {
    const view = window.location.hash.slice(1);
    showView(view === 'session' ? 'session' : 'characters');
  }

  document.querySelectorAll('[data-view]').forEach(control => {
    control.addEventListener('click', (event) => {
      event.preventDefault();
      showView(control.dataset.view);
    });
  });

  window.addEventListener('hashchange', showViewFromHash);
  showViewFromHash();

  // summary 内的 button 点击不触发 details toggle
  document.querySelectorAll('summary > button').forEach(b => {
    b.addEventListener('click', e => e.stopPropagation());
  });
  btnRefreshChars.addEventListener('click', refreshChars);
  if (btnRefreshModels) btnRefreshModels.addEventListener('click', refreshModels);

  async function refreshSessions() {
    const cid = charSelect.value;
    if (!cid) return;
    selectedChar = cid;
    const r = await api('GET', '/v1/sessions/' + encodeURIComponent(cid));
    if (r.ok) {
      const ids = Array.isArray(r.data) ? r.data.map(s => s.session_id || s) : [];
      // W-08 fix: 不再截断到前 12 字符——UUID 前 12 位区分度低，用户分不清多个 session。
      // 与 charSelect 保持一致，显示完整 ID；select 元素宽度由 CSS 控制。
      replaceOptions(sessSelect, ids);
      if (!ids.includes(selectedSess)) selectedSess = ids[0] || '';
      if (selectedSess) sessSelect.value = selectedSess;
    }
  }

  // POST /v1/sessions 返回引擎 `Json<SessionId>` 序列化的纯字符串（如
  // "550e8400-…"）；历史/兼容路径可能回 `{session_id}` 或 `{uuid}` 对象。
  // 抽出来供 btnNewSession / doSend 复用，消除上一轮 review 留下的两处复制。
  function extractSessionId(r) {
    const raw = r && r.data;
    if (!raw) return '';
    if (typeof raw === 'string') return raw;
    if (typeof raw === 'object') return raw.session_id || raw.uuid || String(raw);
    return String(raw);
  }

  // 终止在飞 chat/agent stream。Kimi-K2.7-Code 已修 chat 这条；
  // 同源 race 同样存在于 agent run（issue #43/#44 二次点击 abort 路径的姊妹），
  // 用户切 session/character 时不 abort 同样会让 agent event 写回已清空视图。
  function abortInFlightStreams() {
    if (abortController) {
      abortController.abort();
      abortController = null;
    }
    if (agentAbort) {
      agentAbort.abort();
      agentAbort = null;
    }
  }

  // 切换 session / character / 新建 session 时统一：终止在飞 stream + 清空视图。
  function clearChatView() {
    abortInFlightStreams();
    resetHistoryView();
  }

  function currentHistoryScope() {
    return selectedChar + '::' + (selectedSess || 'legacy');
  }

  function resetHistoryView() {
    historyRequestSeq++;
    chatLog.textContent = '';
    messageNodes.clear();
    selectedMessageId = '';
    historyState = { scope: currentHistoryScope(), oldestId: '', hasMore: false, total: 0, loading: false };
    historyToolbar.hidden = true;
    historyStatus.textContent = '';
    btnLoadEarlier.disabled = false;
  }

  // ── avatar: fetch as blob with bearer, render via object URL ─────────────
  // <img src> 无法附 Authorization 头，故 fetch blob → createObjectURL。
  // 切换角色时 revoke 旧 URL 防泄漏。
  let avatarUrl = '';
  async function refreshAvatar() {
    if (avatarUrl) { URL.revokeObjectURL(avatarUrl); avatarUrl = ''; }
    const cid = selectedChar || charSelect.value;
    if (!cid || !charAvatar) { if (charAvatar) charAvatar.hidden = true; return; }
    try {
      const res = await fetch(base + '/v1/characters/' + encodeURIComponent(cid) + '/avatar', {
        headers: bearer ? { Authorization: 'Bearer ' + bearer } : {},
      });
      logEvent('GET', '/v1/characters/:id/avatar', res.status, 0);
      if (!res.ok) { charAvatar.hidden = true; return; }
      const blob = await res.blob();
      if (!blob.size || !blob.type.startsWith('image/')) { charAvatar.hidden = true; return; }
      avatarUrl = URL.createObjectURL(blob);
      charAvatar.src = avatarUrl;
      charAvatar.hidden = false;
    } catch (e) {
      charAvatar.hidden = true;
    }
  }

  // ── state: live.json + state history ─────────────────────────────────────
  // 404 = 角色尚未有 state；与空对象 {} 区分开显示（PLAN §2.1 L47）。
  async function refreshState() {
    if (!stateDisplay) return;
    const cid = selectedChar || charSelect.value;
    if (!cid) { stateDisplay.textContent = '—'; return; }
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(cid) + '/state');
    if (r.status === 404) {
      stateDisplay.textContent = '(404 — 该角色尚无 live.json)';
      return;
    }
    if (r.ok) {
      // try pretty-print; if not JSON, show raw
      try { stateDisplay.textContent = JSON.stringify(typeof r.data === 'string' ? JSON.parse(r.data) : r.data, null, 2); }
      catch { stateDisplay.textContent = String(r.data || r.text); }
    } else {
      stateDisplay.textContent = 'err: ' + formatError(r.data, r.text);
    }
  }

  async function refreshStateHistory() {
    if (!stateHistoryDisplay) return;
    const cid = selectedChar || charSelect.value;
    if (!cid) { stateHistoryDisplay.textContent = '—'; return; }
    const limit = Math.max(1, Math.min(1000, parseInt(stateHistoryLimit.value, 10) || 20));
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(cid) + '/state/history?limit=' + limit);
    if (r.status === 404) {
      stateHistoryDisplay.textContent = '(404 — 尚无 state history)';
      return;
    }
    if (r.ok) {
      const arr = Array.isArray(r.data) ? r.data : [];
      stateHistoryDisplay.textContent = arr.length
        ? arr.map((e, i) => '[' + i + '] ' + JSON.stringify(e)).join('\n')
        : '(empty array)';
    } else {
      stateHistoryDisplay.textContent = 'err: ' + formatError(r.data, r.text);
    }
  }

  function refreshStateAll() {
    refreshState();
    refreshStateHistory();
  }

  if (btnRefreshState) btnRefreshState.addEventListener('click', refreshStateAll);
  if (stateHistoryLimit) stateHistoryLimit.addEventListener('change', refreshStateHistory);

  charSelect.addEventListener('change', async () => {
    // CR#2: 切角色前若有未保存 lorebook 修改则提示
    if (!confirmDiscardLoreIfDirty('切换角色')) return;
    selectedChar = charSelect.value;
    selectedSess = '';
    // 切角色时清零 loreDirty（旧角色的修改已被丢弃或保存）
    setLoreDirty(false);
    renderCharacterCards(Array.from(charSelect.options, option => option.value));
    clearChatView();
    await refreshSessions();
    refreshAvatar();
    refreshStateAll();
    // 自动加载 history：切角色后立即拉取该角色已有 chat history，
    // 避免用户每次都需手点 History 按钮（PLAN §9 P1 "交互收口"）。
    loadHistory();
    // #114 C-PR1：切角色后刷新 effective persona + 绑定按钮。
    refreshEffectivePersona();
    // #126 D-PR2：切角色后加载主面板 lorebook-section。
    loadLorebook();
    rememberWorkspace();
  });
  sessSelect.addEventListener('change', () => {
    selectedSess = sessSelect.value;
    clearChatView();
    loadHistory();
    // #114 C-PR1：切会话后刷新 effective persona + 绑定按钮。
    refreshEffectivePersona();
    rememberWorkspace();
  });

  btnNewSession.addEventListener('click', async () => {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/sessions/' + encodeURIComponent(selectedChar));
    if (r.ok) {
      // 新建后自动选中该 session，省用户再手动点
      const newId = extractSessionId(r);
      if (newId) selectedSess = newId;
      clearChatView();
      await refreshSessions();
      rememberWorkspace();
    }
  });

  if (btnDeleteSession) btnDeleteSession.addEventListener('click', async () => {
    if (!selectedChar || !selectedSess) return;
    const doomed = selectedSess;
    if (!window.confirm('删除当前会话 ' + doomed + '？此操作不可撤销。')) return;
    const r = await api('DELETE', '/v1/sessions/' + encodeURIComponent(selectedChar) + '/' + encodeURIComponent(doomed));
    if (!r.ok) {
      appendMsg('assistant', '[session delete failed] ' + formatError(r.data, r.text), false, new Date());
      return;
    }
    selectedSess = '';
    clearChatView();
    await refreshSessions();
    rememberWorkspace();
    if (selectedSess) loadHistory();
  });

  // ── chat: send & stream ─────────────────────────────────────────────────
  // 极简 markdown 渲染（PLAN §4.1.1 "支持 streaming delta 和 markdown"）。
  // 零构建约束 → 不引入第三方库；手写覆盖 code fence / inline code / 标题 /
  // 粗体 / 斜体 / 段落换行，足够 chat 场景。安全策略：先 escapeHtml 全转义，
  // 再用 private-use Unicode 占位符抽出 code fence（防止内部 \n 被 <br> 替换），
  // 最后应用其它行内/块级转换。所有用户内容已被转义，不会注入 HTML。
  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function renderMarkdown(text) {
    if (text == null) return '';
    // 行级分块：先把 fenced code blocks 抽出（用占位符），再按行 split，
    // 块级（pre/h1-h3）独立包裹，避免非法 HTML 嵌套（<p><pre>、<p><h1>）。
    const codeBlocks = [];
    // F16/F17 fix: 用带随机 nonce 的 private-use 占位符，避免用户输入同形序列被误替换。
    const phNonce = Math.random().toString(36).slice(2, 10);
    const phPrefix = '\uF8FFCB' + phNonce + '_';
    const phSuffix = '\uF8FF';
    const phRegex = new RegExp(phPrefix + '(\\d+)' + phSuffix, 'g');
    const placeholder = (i) => '\n' + phPrefix + i + phSuffix + '\n';
    let s = escapeHtml(text);
    // F18 fix: 统一换行符，避免 CRLF 导致空行识别失败。
    s = s.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
    // 1. fenced code blocks ```lang\n...\n``` 抽到占位行
    s = s.replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => {
      // lang 暂未用于高亮（避免引第三方）；保留以便未来加 highlight.js
      const i = codeBlocks.push('<pre class="md-code"><code>' + code.replace(/\n$/, '') + '</code></pre>') - 1;
      return placeholder(i);
    });
    // 2. 行内 markdown：inline code / bold / italic 在非占位行上。
    //    占位行使用随机 nonce，用户输入的同形序列不会被后续恢复步骤误匹配。
    s = s.replace(/`([^`\n]+)`/g, '<code class="md-code-inline">$1</code>');
    s = s.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
    s = s.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
    // 3. 标题转块级（不被 <p> 包裹）。Fenced placeholder 行不含 #。
    s = s.replace(/^### (.+)$/gm, '<h3 class="md-h">$1</h3>');
    s = s.replace(/^## (.+)$/gm, '<h2 class="md-h">$1</h2>');
    s = s.replace(/^# (.+)$/gm, '<h1 class="md-h">$1</h1>');
    // 4. 块级切分：按行扫描，块级元素/占位行独占一段，blank line 切 paragraph
    const lines = s.split('\n');
    const out = [];
    let para = [];
    const flushPara = () => {
      if (para.length) {
        out.push('<p>' + para.join('<br>') + '</p>');
        para = [];
      }
    };
    const phLineRegex = new RegExp('^' + phPrefix + '\\d+' + phSuffix + '$');
    for (const line of lines) {
      const isBlock = /^(<h[1-3] )/.test(line) || phLineRegex.test(line);
      if (line === '') { flushPara(); continue; }
      if (isBlock) { flushPara(); out.push(line); }
      else { para.push(line); }
    }
    flushPara();
    // 5. 恢复 code blocks (顺序与 codeBlocks.push 一致)
    return out.join('\n').replace(phRegex, (_, i) => codeBlocks[+i]);
  }

  // appendMsg: 流式中用 textContent（保 cursor 动画 + 性能），完成后切 innerHTML 跑 markdown。
  // W-06 fix: 加可选 ts 参数（Date 或省略）。流式新消息传 new Date() 显示 HH:MM:SS；
  // loadHistory 不传（engine 的 chat_log.jsonl 不存消息时间戳，避免用加载时刻误导用户）。
  function appendMsg(role, text, isStreaming, ts, messageId, options) {
    const opts = options || {};
    let div = messageId ? messageNodes.get(messageId) : null;
    const isNew = !div;
    if (!div) div = document.createElement('div');
    const safeRole = role === 'user' ? 'user' : 'assistant';
    div.className = 'msg ' + safeRole;
    if (messageId === selectedMessageId) div.classList.add('selected');
    div.textContent = '';
    if (messageId) {
      div.dataset.messageId = messageId;
      div.tabIndex = 0;
      div.setAttribute('role', 'button');
      div.setAttribute('aria-label', '选择此消息作为回滚位置');
      const select = () => selectRollbackMessage(messageId);
      div.onclick = select;
      div.onkeydown = e => {
        if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); select(); }
      };
      messageNodes.set(messageId, div);
    }
    if (ts instanceof Date) {
      const hh = String(ts.getHours()).padStart(2, '0');
      const mm = String(ts.getMinutes()).padStart(2, '0');
      const ss = String(ts.getSeconds()).padStart(2, '0');
      appendInline(div, 'span', 'ts', hh + ':' + mm + ':' + ss);
      div.append(' ');
    }
    appendInline(div, 'span', 'role', role);
    const textNode = appendInline(div, 'span', 'text' + (isStreaming ? ' streaming' : ''), '');
    if (isStreaming) {
      textNode.textContent = text;
    } else {
      textNode.innerHTML = renderMarkdown(text);
    }
    if (isNew) {
      if (opts.prepend) chatLog.prepend(div);
      else chatLog.appendChild(div);
    }
    if (opts.scroll !== false) chatLog.scrollTop = chatLog.scrollHeight;
    return textNode;
  }

  function selectRollbackMessage(messageId) {
    if (selectedMessageId) messageNodes.get(selectedMessageId)?.classList.remove('selected');
    selectedMessageId = messageId;
    messageNodes.get(messageId)?.classList.add('selected');
  }

  async function doSend() {
    const text = chatInput.value.trim();
    if (!text || !selectedChar) return;
    chatInput.value = '';
    appendMsg('user', text, false, new Date());

    // create session if none
    if (!selectedSess) {
      const r = await api('POST', '/v1/sessions/' + encodeURIComponent(selectedChar));
      if (!r.ok) { appendMsg('assistant', '[session create failed]', false, new Date()); return; }
      const newId = extractSessionId(r);
      if (!newId) { appendMsg('assistant', '[session create: empty id]', false, new Date()); return; }
      selectedSess = newId;
      await refreshSessions();
    }

    // abort prior stream
    if (abortController) abortController.abort();
    // 用局部引用 ac：finally 清理时只清「仍是当前实例」的情况，避免旧请求 finally
    // 在新请求已开始后误清新请求的 abortController 与 btnStop 可见性（异步竞态）。
    const ac = new AbortController();
    abortController = ac;
    if (btnStop) btnStop.hidden = false;
    const url = base + '/v1/chat/completions';
    const t0 = performance.now();
    let msgEl = null;

    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify(buildChatPayload(text)),
        signal: ac.signal,
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', '/v1/chat/completions', res.status, Math.round(performance.now() - t0), errBody);
        appendMsg('assistant', '[HTTP ' + res.status + '] ' + errBody, false, new Date());
        return;
      }
      msgEl = appendMsg('assistant', '', true, new Date());
      let acc = '';
      const seq = await streamSse(res, (chunk, seq) => {
        // A2: 只把 body_chunk 渲染到正文。think_chunk（心理独白，应折叠）和
        // action_options（选项数组，content 是对象不是 string）都不应混入 body。
        if (chunk.type === 'body_chunk' && chunk.text) {
          acc += chunk.text;
          msgEl.textContent = acc;
          if (seq % 5 === 0) {
            logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'chunk#' + seq);
          }
        }
      });
      msgEl.classList.remove('streaming');
      // 流式期间用 textContent（保 cursor 动画 + 性能），完成后切 markdown innerHTML。
      if (acc) msgEl.innerHTML = renderMarkdown(acc);
      logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'done/' + seq + 'chunks');
    } catch (e) {
      if (msgEl) {
        msgEl.classList.remove('streaming');
        // 异常时若已有部分内容，仍切 markdown；否则保留 raw 错误文本
        if (acc) msgEl.innerHTML = renderMarkdown(acc);
      }
      if (e.name === 'AbortError') {
        logEvent('SSE', '/v1/chat/completions', 0, Math.round(performance.now() - t0), 'aborted');
        return;
      }
      if (e.kind === 'stream_interrupt') {
        logEvent('SSE', '/v1/chat/completions', 0, Math.round(performance.now() - t0), 'stream interrupted: ' + e.message);
        appendMsg('assistant', '[stream interrupted: engine disconnected] ' + e.message, false, new Date());
        return;
      }
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), e.message);
      appendMsg('assistant', '[fetch error] ' + e.message, false, new Date());
    } finally {
      // 只在全局仍是当前实例时清理，避免被旧请求 finally 误清新请求状态（race）
      if (abortController === ac) {
        abortController = null;
        if (btnStop) btnStop.hidden = true;
      }
    }
  }

  function buildChatPayload(text) {
    // #114 C-PR1：始终传 user_id；selectedPersonaId 非空才传 persona_id（explicit），
    // 空串（「自动」）不传 persona_id，让 engine pipeline 按 binding→default 解析。
    // user_profile 改为非权威 override（空 name/空 variables），避免 WebUI 缓存的
    // 表单值覆盖 engine 刚解析出的 Persona；未来显式 override 需独立 UI 与合同。
    const userId = personaUserId.value.trim() || 'default';
    return buildSessionPayload({
      ...buildPersonaPayload(userId, selectedPersonaId),
      user_profile: { name: '', variables: {} },
      preset_id: presetSelect.value || undefined,
      message: text,
    });
  }

  function buildSessionPayload(extra) {
    const payload = { character_id: selectedChar, ...(extra || {}) };
    if (selectedSess) payload.session_id = selectedSess;
    return payload;
  }

  async function streamSse(res, onChunk) {
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = '';
    let seq = 0;
    let sawDone = false;
    while (!sawDone) {
      let done, value;
      try {
        ({ done, value } = await reader.read());
      } catch (e) {
        // 主动 abort（用户取消 / timeout）保持原语义，向上抛 AbortError。
        if (e && e.name === 'AbortError') throw e;
        // 网络中途断开（reader.read 抛 TypeError: network error 等）转 typed error，
        // 让调用方区分「engine 断连」vs「主动取消」vs「其他 fetch error」（issue #47）。
        const err = new Error(e && e.message ? e.message : 'stream interrupted');
        err.kind = 'stream_interrupt';
        throw err;
      }
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      const lines = buf.split('\n');
      buf = lines.pop() || '';
      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        const data = line.slice(6).trim();
        if (data === '[DONE]') {
          sawDone = true;
          break;
        }
        try {
          const chunk = JSON.parse(data);
          seq++;
          onChunk(chunk, seq);
        } catch {}
      }
    }
    return seq;
  }

  btnSend.addEventListener('click', doSend);
  chatInput.addEventListener('keydown', e => { if (e.ctrlKey && e.key === 'Enter') doSend(); });

  // ── history / regen / rollback (P1: destructive confirm) ────────────────
  // loadHistory 抽出来供 charSelect/sessSelect 切换时自动复用，避免用户每次手点。
  function updateHistoryToolbar() {
    const loaded = messageNodes.size;
    historyToolbar.hidden = historyState.total === 0;
    historyStatus.textContent = countFormatter.format(loaded) + ' / ' + countFormatter.format(historyState.total) + ' 条消息';
    btnLoadEarlier.hidden = !historyState.hasMore;
    btnLoadEarlier.disabled = historyState.loading;
    btnLoadEarlier.textContent = historyState.loading ? '加载中…' : '加载更早';
  }

  async function loadHistory(options) {
    if (!selectedChar) return;
    const opts = options || {};
    const scope = currentHistoryScope();
    if (historyState.scope !== scope || opts.reset) resetHistoryView();
    const before = opts.before || '';
    if (historyState.loading) return;
    historyState.loading = true;
    updateHistoryToolbar();
    const requestSeq = ++historyRequestSeq;
    const payload = buildSessionPayload({ limit: HISTORY_PAGE_SIZE });
    if (before) payload.before = before;
    const r = await api('POST', '/v1/chat/history', payload);
    if (requestSeq !== historyRequestSeq || scope !== currentHistoryScope()) return;
    historyState.loading = false;
    if (r.ok) {
      const data = r.data && typeof r.data === 'object' ? r.data : {};
      const msgs = data.messages || r.data || [];
      // #73 方案 B：消息级时间戳（与 messages 一一对应）。旧会话可能无 ts → null。
      const tss = Array.isArray(data.message_timestamps) ? data.message_timestamps : [];
      const ids = Array.isArray(data.message_ids) ? data.message_ids : [];
      // A-3：长度不匹配是 engine bug，显式 warn 暴露而非静默降级
      if (tss.length !== msgs.length || ids.length !== msgs.length) {
        console.warn('engine bug: history parallel arrays must have equal lengths');
      }
      const previousHeight = chatLog.scrollHeight;
      // 持久化窗口到达后移除已完成的 optimistic 节点，避免刷新产生重复消息。
      if (!before && !abortController) {
        chatLog.querySelectorAll('.msg:not([data-message-id])').forEach(node => node.remove());
      }
      // prepend 时逆序插入，最终 DOM 仍保持服务端的时间正序。
      const indexes = before ? Array.from(msgs.keys()).reverse() : Array.from(msgs.keys());
      indexes.forEach(i => {
        const m = msgs[i];
        const tsRaw = tss[i];
        const ts = tsRaw ? new Date(tsRaw) : null;
        appendMsg(m.role || 'assistant', m.text || m.content || '', false, ts, ids[i], {
          prepend: Boolean(before),
          scroll: false,
        });
      });
      historyState.oldestId = data.oldest_id || historyState.oldestId;
      historyState.hasMore = Boolean(data.has_more);
      historyState.total = Number(data.total) || msgs.length;
      if (before) chatLog.scrollTop += chatLog.scrollHeight - previousHeight;
      else if (opts.scroll !== false) chatLog.scrollTop = chatLog.scrollHeight;
      updateHistoryToolbar();
    } else if (r.status === 404) {
      // #68 #8：404 = 角色无 history（engine #67 #4 已改 NotFound），视为无内容而非错误，
      // 静默清空 chatLog 避免用户无操作却见 [history err 404]。仅渲染 session info（如有）。
      resetHistoryView();
    } else if (r.status !== 0) {
      // 0 = 网络层失败（已 logEvent），其它状态码显式提示
      appendMsg('assistant', '[history err ' + r.status + '] ' + formatError(r.data, r.text), false, new Date());
      updateHistoryToolbar();
    }
  }

  btnHistory.addEventListener('click', () => loadHistory({ scroll: false }));
  btnLoadEarlier.addEventListener('click', () => {
    if (historyState.hasMore && historyState.oldestId) loadHistory({ before: historyState.oldestId, scroll: false });
  });

  btnRegen.addEventListener('click', async () => {
    if (!selectedChar) return;
    if (!window.confirm('Regenerate 会重写/删除最后一条 assistant 消息，不可撤销。继续？')) return;
    const r = await api('POST', '/v1/chat/regen', buildSessionPayload());
    if (r.ok) loadHistory({ reset: true });
  });

  btnRollback.addEventListener('click', async () => {
    if (!selectedChar) return;
    if (!selectedMessageId) {
      window.alert('请先在对话中点选要保留的最后一条消息。');
      return;
    }
    if (!window.confirm('确认回滚到选中的消息？其后的消息将被删除且不可撤销。')) return;
    const r = await api('POST', '/v1/chat/rollback', buildSessionPayload({ message_id: selectedMessageId }));
    if (r.ok) loadHistory({ reset: true });
  });

  // ── agent run (P1: classified event log + collapsible raw JSON) ─────────
  const agentStepCounter = $('#agent-step-counter');
  const agentMaxSteps = $('#agent-max-steps');
  const btnAgentClear = $('#btn-agent-clear');

  const AGENT_TYPE_LABEL = {
    plan: 'PLAN',
    tool_call: 'TOOL_CALL',
    tool_result: 'TOOL_RESULT',
    delta: 'DELTA',
    done: 'DONE',
  };
  // 上限 20 仅为防御性 UX cap；引擎本身无此限制（u32::MAX）。
  const AGENT_MAX_STEPS_CAP = 20;
  // stop_reason snake_case → 人类可读标签
  const STOP_REASON_LABEL = {
    converged: 'converged',
    step_cap: 'step cap reached',
    token_budget: 'token budget exhausted',
    wall_clock: 'wall clock timeout',
    cancelled: 'cancelled',
    upstream_error: 'upstream error',
  };
  const AGENT_TYPE_CLASS = {
    plan: 'ev-plan',
    tool_call: 'ev-tool',
    tool_result: 'ev-result',
    delta: 'ev-delta',
    done: 'ev-done',
  };

  function summarizeAgentEvent(chunk) {
    // 返回 {label, summary, isDone}；summary 是人类可读的一行
    // PlanAction 是 #[serde(rename_all = "snake_case")]，故 JSON 里是
    //   {"action":"generate"} / {"action":"finish"}
    //   {"action":{"call_tool":{"tool","params"}}}
    const t = chunk.type;
    if (t === 'plan') {
      const action = chunk.action;
      if (action && typeof action === 'object' && action.call_tool) {
        return { label: 'PLAN', summary: 'step ' + chunk.step + ' → call ' + action.call_tool.tool };
      }
      if (action === 'generate') return { label: 'PLAN', summary: 'step ' + chunk.step + ' → generate' };
      if (action === 'finish') return { label: 'PLAN', summary: 'step ' + chunk.step + ' → finish' };
      return { label: 'PLAN', summary: 'step ' + chunk.step };
    }
    if (t === 'tool_call') return { label: 'TOOL_CALL', summary: 'step ' + chunk.step + ' · ' + chunk.tool };
    if (t === 'tool_result') return { label: 'TOOL_RESULT', summary: 'step ' + chunk.step + ' · ' + chunk.tool };
    if (t === 'delta') return { label: 'DELTA', summary: 'step ' + chunk.step + ' · ' + (chunk.chunk || '').slice(0, 60) };
    if (t === 'done') return { label: 'DONE', summary: chunk.stop_reason + ' · steps=' + chunk.steps_taken + ' · tokens~' + chunk.tokens_estimated, isDone: true };
    return { label: (t || 'EVENT').toUpperCase(), summary: '' };
  }

  // agent output DOM 上限：长跑累积可膨胀，封顶防回流压力（issue F）
  const AGENT_OUTPUT_MAX_ROWS = 500;

  function appendAgentEvent(chunk) {
    const info = summarizeAgentEvent(chunk);
    const row = document.createElement('div');
    const cls = AGENT_TYPE_CLASS[chunk.type] || '';
    row.className = cls ? 'agent-ev ' + cls : 'agent-ev';
    appendInline(row, 'span', 'ev-label', info.label);
    row.append(' ');
    appendInline(row, 'span', 'ev-summary', info.summary || '');
    // 折叠 raw JSON，summary 带事件类型提示方便长流扫读（issue O）
    const details = document.createElement('details');
    details.className = 'ev-raw';
    const summary = document.createElement('summary');
    summary.textContent = 'raw (' + info.label.toLowerCase() + ')';
    details.appendChild(summary);
    const pre = document.createElement('pre');
    pre.className = 'mono';
    pre.textContent = JSON.stringify(chunk, null, 2);
    details.appendChild(pre);
    row.appendChild(details);
    agentOutput.appendChild(row);
    // DOM 上限：超则删最早行
    while (agentOutput.children.length > AGENT_OUTPUT_MAX_ROWS) {
      agentOutput.removeChild(agentOutput.firstChild);
    }
    agentOutput.scrollTop = agentOutput.scrollHeight;
    return info;
  }

  // agent run 客户端超时（30s；agent loop 比单轮 chat 慢，给宽点）
  const AGENT_RUN_TIMEOUT_MS = 30000;

  btnAgentRun.addEventListener('click', async () => {
    const input = agentInput.value.trim();
    if (!input || !selectedChar) return;
    // 二次点击先 abort 前一个 run，防 SSE 事件交错竞态（与 chat send 路径对齐）
    if (agentAbort) agentAbort.abort();
    agentAbort = new AbortController();
    const timeoutTimer = setTimeout(() => agentAbort.abort(), AGENT_RUN_TIMEOUT_MS);
    agentOutput.innerHTML = '';
    agentStepCounter.textContent = 'running…';
    const path = '/v1/agent/run';
    const t0 = performance.now();
    let stepCount = 0;
    let lastDone = null;
    try {
      const maxSteps = Math.max(1, Math.min(AGENT_MAX_STEPS_CAP, parseInt(agentMaxSteps.value, 10) || 3));
      const res = await fetch(base + path, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify((() => {
          const payload = { ...buildChatPayload(input), max_steps: maxSteps };
          const enabled = $('#agent-tools-enabled')?.checked === true;
          if (enabled) {
            if (!bearer) {
              throw new Error('启用 Agent 工具必须先配置 daemon Bearer');
            }
            payload.capabilities = ['call:tool'];
            const allowlist = uniqueValues([
              ...commaSeparatedValues('#agent-tool-allowlist'),
              ...checkedToolNames('.agent-tool-allow'),
            ]);
            if (allowlist.length) payload.allowed_tools = allowlist;
            const confirmed = uniqueValues([
              ...commaSeparatedValues('#agent-tool-confirm'),
              ...checkedToolNames('.agent-tool-confirm'),
            ]);
            if (confirmed.length) {
              if (!confirm('确认允许本次运行执行破坏性工具：' + confirmed.join(', ') + '？')) {
                throw new Error('已取消破坏性工具确认');
              }
              payload.confirm_tools = confirmed;
            }
          }
          return payload;
        })()),
        signal: agentAbort.signal,
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', path, res.status, Math.round(performance.now() - t0), errBody);
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[HTTP ' + res.status + '] ' + errBody;
        agentOutput.appendChild(row);
        agentStepCounter.textContent = 'http ' + res.status;
        return;
      }
      const seq = await streamSse(res, (chunk, seq) => {
        // 防畸形 chunk：SSE 解析已 try/catch JSON.parse，但未知 type 走 fallback
        if (!chunk || typeof chunk !== 'object') {
          logEvent('SSE', path, 200, Math.round(performance.now() - t0), '#' + seq + ' invalid chunk');
          return;
        }
        const info = appendAgentEvent(chunk);
        if (chunk.type === 'plan') {
          stepCount = chunk.step;
          agentStepCounter.textContent = 'step ' + stepCount + ' · ' + seq + ' events · ' + Math.round(performance.now() - t0) + 'ms';
        }
        if (info.isDone) lastDone = chunk;
        logEvent('SSE', path, 200, Math.round(performance.now() - t0), '#' + seq + ' ' + info.label + (info.summary ? ' ' + info.summary : ''));
      });
      const ms = Math.round(performance.now() - t0);
      logEvent('SSE', path, 200, ms, 'done/' + seq + 'events');
      agentStepCounter.textContent = lastDone
        ? STOP_REASON_LABEL[lastDone.stop_reason] + ' · ' + lastDone.steps_taken + ' steps · ' + ms + 'ms'
        : (stepCount ? 'step ' + stepCount + ' · ' : '') + seq + ' events · ' + ms + 'ms';
    } catch (e) {
      if (e.name === 'AbortError') {
        logEvent('SSE', path, 0, Math.round(performance.now() - t0), 'aborted/timeout');
        agentStepCounter.textContent = stepCount ? 'aborted at step ' + stepCount : 'aborted';
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[aborted]';
        agentOutput.appendChild(row);
      } else if (e.kind === 'stream_interrupt') {
        logEvent('SSE', path, 0, Math.round(performance.now() - t0), 'stream interrupted: ' + e.message);
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[stream interrupted: engine disconnected] ' + e.message;
        agentOutput.appendChild(row);
        agentStepCounter.textContent = stepCount ? 'interrupted at step ' + stepCount : 'stream interrupted';
      } else {
        logEvent('POST', path, 0, Math.round(performance.now() - t0), e.message);
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[fetch error] ' + e.message;
        agentOutput.appendChild(row);
        agentStepCounter.textContent = 'fetch error';
      }
    } finally {
      clearTimeout(timeoutTimer);
      agentAbort = null;
    }
  });

  if (btnAgentClear) btnAgentClear.addEventListener('click', () => {
    agentOutput.textContent = '—';
    agentStepCounter.textContent = '';
  });

  // ── clear log ────────────────────────────────────────────────────────────
  btnClearLog.addEventListener('click', () => { eventLog.innerHTML = ''; });

  // ── M3: import via multipart/base64 (NEVER card_path) ───────────────────
  // 审计裁定：Web 永不走 card_path（RR-001）。浏览器读文件 → base64 →
  // card_png_base64 或 card_json。engine 侧 AIRP_ALLOW_LOCAL_PATH 未设时
  // card_path 被拒，此为纵深防御；WebUI 自身亦不发 card_path。
  const importFile = $('#import-file');
  const btnImport = $('#btn-import');
  const importResult = $('#import-result');

  btnImport.addEventListener('click', async () => {
    const file = importFile.files[0];
    if (!file) { importResult.textContent = '请先选文件'; return; }
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    const isPng = bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4E && bytes[3] === 0x47;
    // C3 fix: 客户端 size gate。engine import 路由 body limit = 10MB
    // (daemon/mod.rs:200 DefaultBodyLimit::max(10 * 1024 * 1024))。
    // 按路径动态计算最终 body 长度，零误差拦截：
    //   PNG 路径: body = {"card_png_base64":"<b64>"}，外壳 22 字节 + base64 膨胀 4/3
    //   JSON 路径: body = {"card_json":"<text>"}，外壳 16 字节，无 base64 膨胀
    // A2 fix: 改为动态精确计算，替代之前的 7.5MB 固定近似阈值（原阈值比安全值大 18 字节）。
    const ENGINE_BODY_LIMIT = 10 * 1024 * 1024; // 10 MB
    const PNG_WRAPPER = 22; // {"card_png_base64":""}
    const JSON_WRAPPER = 16; // {"card_json":""}
    const b64Len = isPng ? Math.ceil(file.size / 3) * 4 : 0;
    const wrapperLen = isPng ? PNG_WRAPPER : JSON_WRAPPER;
    const estBodyLen = (isPng ? b64Len : file.size) + wrapperLen;
    if (estBodyLen > ENGINE_BODY_LIMIT) {
      const limitMB = (ENGINE_BODY_LIMIT / 1024 / 1024).toFixed(1);
      importResult.textContent = '✗ 文件过大（' + (file.size / 1024 / 1024).toFixed(1) + 'MB' + (isPng ? ' PNG → base64 ' + (b64Len / 1024 / 1024).toFixed(1) + 'MB' : '') + '），上限 ' + limitMB + 'MB body';
      return;
    }
    importResult.textContent = '上传中…';
    let body;
    if (isPng) {
      const b64 = await readFileAsBase64(file);
      body = { card_png_base64: b64 };
    } else {
      // treat as JSON text
      const text = new TextDecoder().decode(bytes);
      body = { card_json: text };
    }
    const r = await api('POST', '/v1/characters/import', body);
    if (r.ok) {
      importResult.textContent = '✓ 导入成功: ' + (r.data?.character_id || '?');
      refreshChars();
    } else {
      importResult.textContent = '✗ ' + (r.status || 'err') + ': ' + formatError(r.data, r.text);
    }
  });

  function readFileAsBase64(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const value = String(reader.result || '');
        resolve(value.includes(',') ? value.slice(value.indexOf(',') + 1) : value);
      };
      reader.onerror = () => reject(reader.error || new Error('file read failed'));
      reader.readAsDataURL(file);
    });
  }

  // ── M2: concurrent stream test ───────────────────────────────────────────
  // 启动两条并发 chat.send，验证 id-keyed chat state 不串扰（PR #6 修的 race）。
  const btnConcurrent = $('#btn-concurrent');
  const concurrentStatus = $('#concurrent-status');

  btnConcurrent.addEventListener('click', async () => {
    if (!selectedChar) { concurrentStatus.textContent = '请先选角色'; return; }
    concurrentStatus.textContent = '启动两条并发流…';
    const t0 = performance.now();
    const promises = [
      doSendText('并发流 A: 你好'),
      doSendText('并发流 B: 再见'),
    ];
    const results = await Promise.all(promises);
    const ms = Math.round(performance.now() - t0);
    const ok = results.every(r => r.ok);
    logEvent('CONCURRENT', '/v1/chat/completions ×2', ok ? 200 : 500, ms, ok ? '两条流均完成' : JSON.stringify(results));
    // #68 #9：补真断言——读 engine 端 /v1/chat/history 验证持久化顺序与串扰保护，
    // 不只看 results.every。回归 PR #6 的 id-keyed race 修复（原测试只校 ok flag，
    // 断言被悄悄废掉）。失败时显式列出 mismatch 供诊断。
    let assertion = '';
    if (ok) {
      try {
        // MVP §3.2：补传 session_id，避免读回该角色所有会话的消息（旁路 session 隔离）。
        const h = await api('POST', '/v1/chat/history', buildSessionPayload());
        if (h.ok) {
          const data = h.data && typeof h.data === 'object' ? h.data : {};
          const msgs = Array.isArray(data.messages) ? data.messages : [];
          const expectN = 4;
          if (msgs.length !== expectN) {
            assertion = '✗ history msgs=' + msgs.length + ' 期望 ' + expectN;
          } else {
            // 审计 CR2：engine chat_completion 先写 user 后写 assistant，并发两请求
            // 写入顺序非确定——可能产 [u,u,a,a]（两 user 先写两 assistant 后写）也合法。
            // 不硬校交替，只校"2 user + 2 assistant + 内容匹配 A/B"。
            const roles = msgs.map(m => m.role || 'assistant');
            const userCount = roles.filter(r => r === 'user').length;
            const asstCount = roles.filter(r => r === 'assistant').length;
            const roleOk = userCount === 2 && asstCount === 2;
            const userMsgs = msgs.filter(m => (m.role || 'assistant') === 'user');
            const hasA = userMsgs.some(m => (m.text || m.content || '').includes('并发流 A'));
            const hasB = userMsgs.some(m => (m.text || m.content || '').includes('并发流 B'));
            const contentOk = hasA && hasB;
            if (!roleOk) assertion = '✗ role 计数错: user=' + userCount + ' assistant=' + asstCount + ' 期望各 2';
            else if (!contentOk) assertion = '✗ user 内容不匹配 A/B';
            else assertion = '✓ history 含 2 user + 2 assistant，内容匹配 A/B，无串扰';
          }
        } else {
          assertion = '⚠ history 读取失败 ' + h.status + '（无法校持久化）';
        }
      } catch (e) { assertion = '⚠ history 读取异常: ' + e.message; }
    }
    concurrentStatus.textContent = (ok ? '✓' : '✗') + ' 两条流完成 (' + ms + 'ms)。' + (assertion || '检查 chat log 顺序：u-A → a-A → u-B → a-B 应基本交替，无串扰。');
  });

  // 抽出 doSend 的纯逻辑供并发复用（不发 user DOM、不读 input）
  async function doSendText(text) {
    if (!selectedChar) return { ok: false, status: 0, error: 'no character' };
    appendMsg('user', text, false, new Date());
    const url = base + '/v1/chat/completions';
    const t0 = performance.now();
    let msgEl = null;
    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify(buildChatPayload(text)),
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', '/v1/chat/completions', res.status, Math.round(performance.now() - t0), errBody);
        return { ok: false, status: res.status, error: errBody };
      }
      msgEl = appendMsg('assistant', '', true, new Date());
      let acc = '';
      const seq = await streamSse(res, (chunk) => {
        // A2: 同 doSend，只渲染 body_chunk；think_chunk / action_options 不混入 body。
        if (chunk.type === 'body_chunk' && chunk.text) {
          acc += chunk.text;
          msgEl.textContent = acc;
        }
      });
      msgEl.classList.remove('streaming');
      if (acc) msgEl.innerHTML = renderMarkdown(acc);
      return { ok: true, status: 200, chunks: seq };
    } catch (e) {
      if (msgEl) {
        msgEl.classList.remove('streaming');
        if (acc) msgEl.innerHTML = renderMarkdown(acc);
      }
      const interrupted = e.kind === 'stream_interrupt';
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), (interrupted ? 'stream interrupted: ' : '') + e.message);
      return { ok: false, status: 0, error: e.message, kind: interrupted ? 'stream_interrupt' : undefined };
    }
  }

  // ── P1: one-click diagnostics ────────────────────────────────────────────
  // 依次跑 version/settings/models，输出可复制的诊断摘要。
  // 不发真实 chat/agent run（避免消耗 provider quota）；只验证后端可达性。
  const btnDiag = $('#btn-diag');
  const btnDiagCopy = $('#btn-diag-copy');
  const diagOutput = $('#diag-output');
  let lastDiagText = '';

  // 诊断专用：带 timeout 的 api 包装。engine 卡死时 fail-fast 而非永悬
  // （诊断的本职就是探 engine 卡死，自己不能跟着卡）。
  // 用 AbortController 真切断 fetch，而非 Promise.race 留 fetch 悬跑。
  async function diagApi(method, path, timeoutMs = 5000) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    const t0 = performance.now();
    try {
      const url = base + path;
      const res = await fetch(url, { method, headers: headers(), signal: ctrl.signal });
      const text = await res.text();
      const ms = Math.round(performance.now() - t0);
      logEvent(method, path, res.status, ms);
      let data;
      try { data = JSON.parse(text); } catch { data = text; }
      return { ok: res.ok, status: res.status, data, text, ms };
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      const aborted = e.name === 'AbortError';
      const msg = aborted ? 'timeout after ' + timeoutMs + 'ms' : e.message;
      logEvent(method, path, 0, ms, msg);
      return { ok: false, status: 0, data: null, text: msg, ms };
    } finally {
      clearTimeout(timer);
    }
  }

  async function runDiagnostics() {
    if (!base) { diagOutput.textContent = '请先连接 engine'; return; }
    diagOutput.textContent = '诊断中…';
    const lines = [];
    lines.push('=== AIRP Engine Diagnostics ===');
    lines.push('time: ' + new Date().toISOString());
    lines.push('engine_url: ' + (productionMode ? '(same-origin gateway)' : base));
    lines.push('bearer: ' + (productionMode
      ? '(gateway-managed; unavailable to browser)'
      : (bearer ? '(set, len=' + bearer.length + ')' : '(empty — engine 无鉴权或未配 bearer)')));
    lines.push('');

    // 1. version
    {
      const r = await diagApi('GET', '/version');
      lines.push('[1] GET /version  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) lines.push('    name=' + (r.data?.name || '?') + ' version=' + (r.data?.version || r.text || '?'));
      else lines.push('    err: ' + formatError(r.data, r.text));
    }
    // 1b. health
    {
      const r = await diagApi('GET', '/health');
      lines.push('[1b] GET /health  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) {
        const h = r.data || {};
        lines.push('    engine=' + (h.engine || '?') + ' provider_configured=' + (h.provider_configured ? 'true' : 'false') + ' data_root_writable=' + (h.data_root_writable ? 'true' : 'false'));
      } else {
        lines.push('    err: ' + formatError(r.data, r.text));
      }
    }
    // 2. settings
    {
      const r = await diagApi('GET', '/v1/settings');
      lines.push('[2] GET /v1/settings  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) {
        const s = r.data || {};
        const hasApiKey = s.api_key_set === true;
        const hasAccessKey = s.access_api_key_set === true;
        lines.push('    endpoint=' + (s.endpoint || '(unset)'));
        lines.push('    model=' + (s.model || '(unset)'));
        lines.push('    api_key=' + (hasApiKey ? '(set)' : '(MISSING — provider 调用会失败)'));
        lines.push('    access_api_key=' + (hasAccessKey ? '(set — 需 bearer)' : '(empty — 无鉴权)'));
      } else {
        lines.push('    err: ' + formatError(r.data, r.text));
      }
    }
    // 3. models (provider smoke)
    {
      const r = await diagApi('GET', '/v1/models');
      lines.push('[3] GET /v1/models  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) {
        const models = Array.isArray(r.data?.data) ? r.data.data.map(m => m.id) : null;
        lines.push('    models: ' + (models ? models.length + ' 个 → ' + models.slice(0, 5).join(', ') + (models.length > 5 ? ' …' : '') : JSON.stringify(r.data).slice(0, 80)));
      } else {
        lines.push('    err: ' + formatError(r.data, r.text));
      }
    }
    // 4. characters
    {
      const r = await diagApi('GET', '/v1/characters');
      lines.push('[4] GET /v1/characters  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) lines.push('    count=' + (Array.isArray(r.data) ? r.data.length : 0));
      else lines.push('    err: ' + formatError(r.data, r.text));
    }
    lines.push('');
    lines.push('=== End ===');
    lastDiagText = lines.join('\n');
    diagOutput.textContent = lastDiagText;
  }

  if (btnDiag) btnDiag.addEventListener('click', runDiagnostics);
  if (btnDiagCopy) btnDiagCopy.addEventListener('click', async () => {
    if (!lastDiagText) { diagOutput.textContent = '先点「一键诊断」'; return; }
    try {
      await navigator.clipboard.writeText(lastDiagText);
      diagOutput.textContent = lastDiagText + '\n\n[已复制到剪贴板]';
    } catch {
      diagOutput.textContent = lastDiagText + '\n\n[剪贴板不可用，请手动选中复制]';
    }
  });

  // ── Workbench（角色卡编辑，PR F）─────────────────────────────────────────
  // 用户需求：导入角色卡后，点击「工作台」进入编辑视图，可改角色卡，
  // 然后在右侧 session 区新建对话。工作台是 overlay 面板，不挡 chat。
  // #126 D-PR2：lorebook 编辑器已迁移到主面板 lorebook-section，workbench 只保留角色卡 + 拆解。
  const workbenchPanel = $('#workbench-panel');
  const wbCharName = $('#wb-char-name');
  const wbCardFields = $('#wb-card-fields');
  const wbCardStatus = $('#wb-card-status');
  const btnWorkbench = $('#btn-workbench');
  const btnReextract = $('#btn-reextract');
  const btnWbClose = $('#btn-wb-close');
  const btnWbSaveCard = $('#btn-wb-save-card');
  const wbDirtyDot = $('#wb-dirty-dot');

  // ── Lorebook 主面板 section（#126 D-PR2：从 workbench 迁移到 character-scoped 主面板）─
  const loreEntries = $('#lore-entries');
  const loreStatus = $('#lore-status');
  const btnLoreAdd = $('#btn-lore-add');
  const btnLoreSave = $('#btn-lore-save');
  const btnLoreRefresh = $('#btn-refresh-lorebook');

  // 当前工作台缓存的角色卡数据
  let wbCardData = null;
  let wbDirty = false;

  // lorebook 主面板缓存（character-scoped，随角色切换自动加载）
  let loreData = null;
  // CR#2: stale-response 防护 — 每次发起 loadLorebook 自增，response 回来时若已过期则丢弃
  let loreRequestId = 0;
  // CR#2: lorebook 未保存修改跟踪，切角色/刷新时提示
  let loreDirty = false;

  // 角色卡可编辑字段（TavernCardV2 data 层）
  const CARD_FIELDS = [
    { key: 'name', label: '名称', type: 'input' },
    { key: 'description', label: '描述', type: 'textarea', tall: true },
    { key: 'personality', label: '性格', type: 'textarea' },
    { key: 'scenario', label: '场景', type: 'textarea' },
    { key: 'first_mes', label: '开场白', type: 'textarea', tall: true },
    { key: 'system_prompt', label: '系统提示词', type: 'textarea', tall: true },
    { key: 'mes_example', label: '对话示例', type: 'textarea', tall: true },
  ];

  function openWorkbench() {
    if (!selectedChar) {
      alert('请先选择一个角色');
      return;
    }
    workbenchPanel.hidden = false;
    wbCharName.textContent = selectedChar;
    wbCardStatus.textContent = '加载中…';
    setWbDirty(false);  // 切角色打开时清 dirty，避免上次残留
    loadWorkbenchCard();
  }

  function setWbDirty(dirty) {
    wbDirty = dirty;
    if (wbDirtyDot) wbDirtyDot.hidden = !dirty;
  }

  function closeWorkbench() {
    if (wbDirty) {
      if (!confirm('工作台有未保存修改，关闭后修改将丢失。确定关闭？')) return;
    }
    workbenchPanel.hidden = true;
    wbCardData = null;
    setWbDirty(false);
  }

  async function loadWorkbenchCard() {
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar));
    if (r.ok) {
      wbCardData = r.data;
      renderCardFields();
      wbCardStatus.textContent = '已加载（修改后点保存写回）';
    } else {
      wbCardStatus.textContent = '加载失败: ' + formatError(r.data, r.text);
      wbCardFields.innerHTML = '';
    }
  }

  function renderCardFields() {
    if (!wbCardData) return;
    const data = wbCardData.data || wbCardData;
    wbCardFields.innerHTML = '';

    // name 单独一行
    const nameField = CARD_FIELDS.find(f => f.key === 'name');
    if (nameField) wbCardFields.appendChild(makeCardField(nameField, data));

    // 提示词类字段分组
    const promptFields = CARD_FIELDS.filter(f => f.key !== 'name');
    if (promptFields.length) {
      const group = document.createElement('div');
      group.className = 'wb-group';
      const title = document.createElement('div');
      title.className = 'wb-group-title';
      title.textContent = '提示词与背景';
      group.appendChild(title);
      for (const f of promptFields) group.appendChild(makeCardField(f, data));
      wbCardFields.appendChild(group);
    }
  }

  function makeCardField(f, data) {
    const wrap = document.createElement('div');
    wrap.className = 'wb-field';
    const lbl = document.createElement('label');
    lbl.textContent = f.label;
    const el = document.createElement(f.type === 'textarea' ? 'textarea' : 'input');
    if (f.type === 'textarea' && f.tall) el.classList.add('wb-tall');
    el.value = data[f.key] || '';
    el.dataset.field = f.key;
    el.addEventListener('input', () => setWbDirty(true));
    wrap.appendChild(lbl);
    wrap.appendChild(el);
    return wrap;
  }

  async function saveWorkbenchCard() {
    if (!wbCardData || !selectedChar) return;
    const data = wbCardData.data || wbCardData;
    // A-01 修复：保存前深拷贝表单原始值。保存失败时恢复 wbCardData，
    // 让用户能「回到上次保存的版本」而不是带着失败的 mutation。
    const snapshot = JSON.parse(JSON.stringify(wbCardData));
    // 收集表单值
    for (const f of CARD_FIELDS) {
      const el = wbCardFields.querySelector('[data-field="' + f.key + '"]');
      if (el) data[f.key] = el.value;
    }
    wbCardStatus.textContent = '保存中…';
    btnWbSaveCard.disabled = true;
    const r = await api('PUT', '/v1/characters/' + encodeURIComponent(selectedChar), wbCardData);
    btnWbSaveCard.disabled = false;
    if (r.ok) {
      wbCardStatus.textContent = '已保存 ✓';
      setWbDirty(false);
      refreshAvatar();
    } else {
      // 保存失败：恢复 wbCardData 到保存前状态，并把表单字段也回滚
      wbCardData = snapshot;
      const data2 = wbCardData.data || wbCardData;
      for (const f of CARD_FIELDS) {
        const el = wbCardFields.querySelector('[data-field="' + f.key + '"]');
        if (el) el.value = data2[f.key] || '';
      }
      wbCardStatus.textContent = '保存失败: ' + formatError(r.data, r.text);
    }
  }

  async function loadLorebook() {
    // PR#182 CI 诊断（临时，根因定位后随 smoke DIAG 一并移除）：记录 loadLorebook 各分支，
    // 配合 production-browser-smoke.mjs 的 page.on('console') 捕获，区分「未调用 / early-return /
    // fetch 挂起 / guard 静默丢弃 / 正常返回」五种情况。
    console.warn('[lore-diag] loadLorebook entry', { selectedChar, loreRequestId });
    if (!selectedChar) {
      console.warn('[lore-diag] loadLorebook early-return: selectedChar empty');
      return;
    }
    // CR#2: 自增 requestId 并在 response 回来时校验，避免快速切角色时旧响应覆盖新数据
    const requestId = ++loreRequestId;
    const characterId = selectedChar;
    console.warn('[lore-diag] loadLorebook before api', { requestId, loreRequestId, characterId });
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar) + '/lorebook');
    console.warn('[lore-diag] loadLorebook api resolved', { requestId, loreRequestId, characterId, selectedChar, status: r.status, ok: r.ok });
    // 过期响应直接丢弃：切角色或重新发起 load 后，旧 response 不再适用
    if (requestId !== loreRequestId || characterId !== selectedChar) {
      console.warn('[lore-diag] loadLorebook guard silent-return', { requestId, loreRequestId, characterId, selectedChar });
      return;
    }
    if (r.status === 404) {
      loreData = { entries: [] };
      renderLoreEntries();
      loreStatus.textContent = '该角色尚无世界书（可新建条目后保存）';
    } else if (r.ok) {
      loreData = r.data;
      if (!loreData.entries) loreData.entries = [];
      renderLoreEntries();
      loreStatus.textContent = '已加载 ' + loreData.entries.length + ' 条条目';
    } else {
      loreStatus.textContent = '加载失败: ' + formatError(r.data, r.text);
    }
  }

  // CR#2: lorebook 修改时标记 dirty；保存成功或主动 reload 后清零
  function setLoreDirty(dirty) {
    loreDirty = dirty;
  }

  // CR#2: 切角色/手动刷新前若 loreDirty 则提示用户确认丢弃未保存修改
  function confirmDiscardLoreIfDirty(action) {
    if (!loreDirty) return true;
    return confirm('世界书有未保存修改，' + action + '后修改将丢失。确定继续？');
  }

  function renderLoreEntries() {
    if (!loreData) return;
    loreEntries.innerHTML = '';
    loreData.entries.forEach((entry, i) => {
      loreEntries.appendChild(renderLoreEntry(entry, i));
    });
  }

  function renderLoreEntry(entry, index) {
    const div = document.createElement('div');
    div.className = 'wb-lore-entry collapsed';
    div.dataset.index = String(index);

    const head = document.createElement('div');
    head.className = 'wb-lore-head';

    // index + 展开切换
    const toggle = document.createElement('button');
    toggle.className = 'wb-lore-toggle';
    toggle.textContent = '▸';
    toggle.title = '展开/折叠';
    // S5: aria-expanded 同步折叠状态，屏幕阅读器可感知
    toggle.setAttribute('aria-expanded', 'false');
    toggle.setAttribute('aria-label', '展开/折叠条目 #' + (index + 1));
    toggle.addEventListener('click', () => {
      div.classList.toggle('collapsed');
      const collapsed = div.classList.contains('collapsed');
      toggle.textContent = collapsed ? '▸' : '▾';
      toggle.setAttribute('aria-expanded', String(!collapsed));
    });
    head.appendChild(toggle);

    const lbl = document.createElement('span');
    lbl.className = 'wb-lore-index';
    lbl.textContent = '条目 #' + (index + 1);
    head.appendChild(lbl);

    // keys 行内编辑
    const keysInput = document.createElement('input');
    keysInput.className = 'wb-lore-keys';
    keysInput.type = 'text';
    keysInput.value = (entry.keys || []).join(', ');
    keysInput.placeholder = '关键词（逗号分隔）';
    keysInput.addEventListener('input', (e) => {
      entry.keys = e.target.value.split(',').map(s => s.trim()).filter(Boolean);
      setLoreDirty(true);
    });
    head.appendChild(keysInput);

    // priority
    const priInput = document.createElement('input');
    priInput.className = 'wb-lore-priority';
    priInput.type = 'number';
    priInput.min = '0';
    priInput.step = '1';
    priInput.value = entry.priority ?? 10;
    priInput.title = '优先级';
    priInput.addEventListener('input', (e) => {
      const v = parseInt(e.target.value, 10);
      entry.priority = isNaN(v) ? 10 : v;
      setLoreDirty(true);
    });
    head.appendChild(priInput);

    // enabled
    const enLbl = document.createElement('label');
    enLbl.className = 'wb-lore-enabled';
    const enCb = document.createElement('input');
    enCb.type = 'checkbox';
    enCb.checked = entry.enabled !== false;
    enCb.addEventListener('change', () => {
      entry.enabled = enCb.checked;
      setLoreDirty(true);
    });
    enLbl.appendChild(enCb);
    enLbl.appendChild(document.createTextNode('启用'));
    head.appendChild(enLbl);

    // constant（v2 语义：常驻注入，不依赖关键词命中）
    const constLbl = document.createElement('label');
    constLbl.className = 'wb-lore-constant';
    const constCb = document.createElement('input');
    constCb.type = 'checkbox';
    constCb.checked = entry.constant === true;
    constCb.title = '常驻注入：启用后无论关键词是否命中都会注入';
    constCb.addEventListener('change', () => {
      entry.constant = constCb.checked;
      setLoreDirty(true);
    });
    constLbl.appendChild(constCb);
    constLbl.appendChild(document.createTextNode('常驻'));
    head.appendChild(constLbl);

    // #126 D-PR2: selective（v4 运行时字段）— primary 命中后还需 secondary_keys 任一命中
    const selLbl = document.createElement('label');
    selLbl.className = 'wb-lore-selective';
    const selCb = document.createElement('input');
    selCb.type = 'checkbox';
    selCb.checked = entry.selective === true;
    selCb.title = '选择性：启用后 primary key 命中还需 secondary key 任一命中才注入';
    selCb.addEventListener('change', () => {
      entry.selective = selCb.checked;
      secInput.disabled = !selCb.checked;
      setLoreDirty(true);
    });
    selLbl.appendChild(selCb);
    selLbl.appendChild(document.createTextNode('选择性'));
    head.appendChild(selLbl);

    // #126 D-PR2: secondary_keys（v4 运行时字段，逗号分隔；selective=false 时 disabled）
    const secInput = document.createElement('input');
    secInput.className = 'wb-lore-secondary';
    secInput.type = 'text';
    secInput.value = (entry.secondary_keys || []).join(', ');
    secInput.placeholder = 'secondary keys（逗号分隔）';
    secInput.disabled = !selCb.checked;
    secInput.title = 'selective=true 时，primary 命中后还需任一 secondary key 命中才注入';
    secInput.addEventListener('input', (e) => {
      entry.secondary_keys = AIRPLorebookUtils.parseSecondaryKeys(e.target.value);
      setLoreDirty(true);
    });
    head.appendChild(secInput);

    // delete
    const del = document.createElement('button');
    del.className = 'wb-lore-del';
    del.textContent = '✕';
    del.title = '删除此条目';
    del.setAttribute('aria-label', '删除条目 #' + (index + 1));
    // delete：只移除该条目 DOM + 数据，不全量重渲染（A-02 修复）
    // 全量重渲染会丢失其他条目的展开/折叠状态与未保存的 input 值。
    del.addEventListener('click', () => {
      // A-03 修复：删除前按对象身份重新定位 entry，避免重编序号后闭包 index 过时
      // 导致连续删除时删错条目。
      const currentIndex = loreData.entries.indexOf(entry);
      if (currentIndex >= 0) loreData.entries.splice(currentIndex, 1);
      div.remove();
      // 重编后续条目的序号显示（dataset.index + lbl 文本），保持视觉一致
      loreEntries.querySelectorAll('.wb-lore-entry').forEach((e, i) => {
        e.dataset.index = String(i);
        const lbl = e.querySelector('.wb-lore-index');
        if (lbl) lbl.textContent = '条目 #' + (i + 1);
      });
      setLoreDirty(true);
    });
    head.appendChild(del);

    div.appendChild(head);

    // body
    const body = document.createElement('div');
    body.className = 'wb-lore-body';

    const contentTa = document.createElement('textarea');
    contentTa.placeholder = '注入内容';
    contentTa.value = entry.content || '';
    contentTa.addEventListener('input', (e) => {
      entry.content = e.target.value;
      setLoreDirty(true);
    });
    body.appendChild(contentTa);

    const cmtInput = document.createElement('input');
    cmtInput.className = 'wb-lore-comment';
    cmtInput.type = 'text';
    cmtInput.placeholder = '注释（可选）';
    cmtInput.value = entry.comment || '';
    cmtInput.addEventListener('input', (e) => {
      entry.comment = e.target.value || null;
      setLoreDirty(true);
    });
    body.appendChild(cmtInput);

    // #126 D-PR2: advisory 字段只读展示区（case_sensitive 从 top-level 读，
    // 其余从 extensions 读；selective 已提升为 canonical，跳过）
    const advisory = AIRPLorebookUtils.collectAdvisoryFields(entry);
    if (advisory.length > 0) {
      const advWrap = document.createElement('div');
      advWrap.className = 'wb-lore-advisory';
      const advTitle = document.createElement('div');
      advTitle.className = 'wb-lore-advisory-title';
      advTitle.textContent = 'advisory（不影响运行时）';
      advWrap.appendChild(advTitle);
      for (const f of advisory) {
        const row = document.createElement('div');
        row.className = 'wb-lore-advisory-row';
        const lblEl = document.createElement('span');
        lblEl.className = 'wb-lore-advisory-label';
        lblEl.textContent = f.label + ':';
        const valEl = document.createElement('span');
        valEl.className = 'wb-lore-advisory-value';
        valEl.textContent = f.value;
        row.appendChild(lblEl);
        row.appendChild(valEl);
        advWrap.appendChild(row);
      }
      body.appendChild(advWrap);
    }

    div.appendChild(body);
    return div;
  }

  function addLoreEntry() {
    if (!loreData) loreData = { entries: [] };
    const newEntry = AIRPLorebookUtils.buildLoreEntryDefault();
    loreData.entries.push(newEntry);
    // S6: 仅 append 新条目 DOM，不全量重绘，避免已有条目展开状态丢失
    const newIndex = loreData.entries.length - 1;
    const newDiv = renderLoreEntry(newEntry, newIndex);
    newDiv.classList.remove('collapsed');
    const newToggle = newDiv.querySelector('.wb-lore-toggle');
    if (newToggle) {
      newToggle.textContent = '▾';
      newToggle.setAttribute('aria-expanded', 'true');
    }
    loreEntries.appendChild(newDiv);
    newDiv.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    setLoreDirty(true);
  }

  async function saveLorebook() {
    if (!loreData || !selectedChar) return;
    // CodeRabbit 阻塞修复：记录发起 save 时的 characterId，response 回来后若已切角色则
    // 不清 dirty、不覆盖 loreStatus（避免旧 save 清掉新角色的未保存修改状态）。
    const savedChar = selectedChar;
    loreStatus.textContent = '保存中…';
    btnLoreSave.disabled = true;
    const r = await api('PUT', '/v1/characters/' + encodeURIComponent(selectedChar) + '/lorebook', loreData);
    btnLoreSave.disabled = false;
    // 若 save 期间已切角色，丢弃此 response 的副作用（不清 dirty、不覆盖 status）
    if (savedChar !== selectedChar) return;
    if (r.ok) {
      loreStatus.textContent = '已保存 ✓（' + (r.data?.entries_count ?? '?') + ' 条）';
      setLoreDirty(false);
    } else {
      loreStatus.textContent = '保存失败: ' + formatError(r.data, r.text);
    }
  }

  // ── Decompose tab：拆解 / analysis 文件列表 / enhance / apply ──────────────
  // A4 修复：所有 analysis MD 内容（来自 LLM 或用户编辑，属 untrusted data）
  // 一律用 textContent 渲染，绝不用 innerHTML 注入。原始内容显示在 <pre>（只读），
  // 增强后内容显示在 <textarea>（可编辑，apply 时写盘）。
  const wbDecomposeStatus = $('#wb-decompose-status');
  const wbAnalysisFiles = $('#wb-analysis-files');
  const wbAnalysisViewer = $('#wb-analysis-viewer');
  const wbAnalysisFilename = $('#wb-analysis-filename');
  const wbAnalysisOriginal = $('#wb-analysis-original');
  const wbAnalysisEnhanced = $('#wb-analysis-enhanced');
  const btnWbDecompose = $('#btn-wb-decompose');
  const btnWbListAnalysis = $('#btn-wb-list-analysis');
  const btnWbAnalysisEnhance = $('#btn-wb-analysis-enhance');
  const btnWbAnalysisApply = $('#btn-wb-analysis-apply');
  const btnWbAnalysisClose = $('#btn-wb-analysis-close');
  const wbDecomposeForce = $('#wb-decompose-force');
  // L4 修复（issue #92）：预设拆解按钮 + 预设选择器
  const btnWbDecomposePreset = $('#btn-wb-decompose-preset');
  const wbPresetSelect = $('#wb-preset-select');
  let wbAnalysisCurrentFilename = null;

  async function decomposeCharacter() {
    if (!selectedChar) return;
    const force = wbDecomposeForce && wbDecomposeForce.checked;
    wbDecomposeStatus.textContent = '拆解中…';
    btnWbDecompose.disabled = true;
    const path = '/v1/characters/' + encodeURIComponent(selectedChar) + '/decompose' + (force ? '?force=true' : '');
    const r = await api('POST', path);
    btnWbDecompose.disabled = false;
    if (r.ok) {
      const data = r.data || {};
      wbDecomposeStatus.textContent = '已拆解 ✓（' + (data.files_written?.length ?? '?') + ' 个文件'
        + (data.lorebook_decomposed ? '，含世界书' : '') + '）';
      loadAnalysisFileList();
    } else {
      wbDecomposeStatus.textContent = '拆解失败: ' + formatError(r.data, r.text);
    }
  }

  async function loadAnalysisFileList() {
    if (!selectedChar) return;
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis');
    wbAnalysisFiles.innerHTML = '';
    if (!r.ok) {
      wbDecomposeStatus.textContent = '加载文件列表失败: ' + formatError(r.data, r.text);
      return;
    }
    const files = (r.data && r.data.files) || [];
    if (!files.length) {
      const empty = document.createElement('div');
      empty.className = 'hint';
      empty.textContent = '尚无 analysis 文件（点"拆解角色卡"生成）';
      wbAnalysisFiles.appendChild(empty);
      return;
    }
    wbDecomposeStatus.textContent = '已加载 ' + files.length + ' 个文件';
    for (const f of files) {
      const row = document.createElement('div');
      row.className = 'wb-analysis-file';
      const name = document.createElement('span');
      name.className = 'wb-af-name';
      // A4 修复：filename 来自服务端但仍是 untrusted，用 textContent
      name.textContent = f.filename;
      const size = document.createElement('span');
      size.className = 'wb-af-size';
      size.textContent = f.size + ' B';
      row.appendChild(name);
      row.appendChild(size);
      row.addEventListener('click', () => loadAnalysisFile(f.filename));
      wbAnalysisFiles.appendChild(row);
    }
  }

  async function loadAnalysisFile(filename) {
    if (!selectedChar) return;
    // filename 已通过服务端白名单校验（[a-z0-9_/.-]+\.md），直接拼接到 URL 路径。
    // 不用 encodeURIComponent——它会编码 / 为 %2F，axum /*filename 通配符不解码。
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis/' + filename);
    if (!r.ok) {
      wbDecomposeStatus.textContent = '读文件失败: ' + formatError(r.data, r.text);
      return;
    }
    wbAnalysisCurrentFilename = filename;
    // A4 修复：用 textContent 渲染 untrusted MD 内容，绝不用 innerHTML
    wbAnalysisFilename.textContent = filename;
    wbAnalysisOriginal.textContent = r.data.content || '';
    wbAnalysisEnhanced.value = r.data.content || '';
    btnWbAnalysisApply.hidden = true;
    wbAnalysisViewer.hidden = false;
    wbDecomposeStatus.textContent = '已加载 ' + filename;
  }

  async function enhanceAnalysis() {
    if (!selectedChar || !wbAnalysisCurrentFilename) return;
    wbDecomposeStatus.textContent = '生成 diff 预览中…';
    btnWbAnalysisEnhance.disabled = true;
    const body = { action: 'enhance' };
    const r = await api('POST',
      '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis/' + wbAnalysisCurrentFilename,
      body);
    btnWbAnalysisEnhance.disabled = false;
    if (!r.ok) {
      wbDecomposeStatus.textContent = 'enhance 失败: ' + formatError(r.data, r.text);
      return;
    }
    const data = r.data || {};
    // A4 修复：enhanced_md 来自 LLM/占位，仍是 untrusted，用 .value 设置（textarea）
    wbAnalysisEnhanced.value = data.enhanced_md || '';
    btnWbAnalysisApply.hidden = false;
    wbDecomposeStatus.textContent = data.has_changes
      ? '已生成 diff 预览（有变化，确认后点 Apply 写盘）'
      : '已生成 diff 预览（无变化）';
  }

  async function applyEnhanced() {
    if (!selectedChar || !wbAnalysisCurrentFilename) return;
    const enhanced_md = wbAnalysisEnhanced.value;
    if (!confirm('确认把 enhanced_md 写入 ' + wbAnalysisCurrentFilename + ' ？此操作覆盖原文件。')) return;
    wbDecomposeStatus.textContent = '写盘中…';
    btnWbAnalysisApply.disabled = true;
    const body = { action: 'apply', enhanced_md };
    const r = await api('POST',
      '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis/' + wbAnalysisCurrentFilename,
      body);
    btnWbAnalysisApply.disabled = false;
    if (r.ok) {
      wbDecomposeStatus.textContent = '已写盘 ✓';
      // 重新加载原文件内容
      wbAnalysisOriginal.textContent = enhanced_md;
      btnWbAnalysisApply.hidden = true;
    } else {
      wbDecomposeStatus.textContent = 'apply 失败: ' + formatError(r.data, r.text);
    }
  }

  function closeAnalysisViewer() {
    wbAnalysisViewer.hidden = true;
    wbAnalysisCurrentFilename = null;
  }

  // L4 修复（issue #92）：预设拆解 — 复用 decomposeCharacter 的 force 选项 + 状态栏
  async function decomposePreset() {
    const presetId = wbPresetSelect.value;
    if (!presetId) {
      wbDecomposeStatus.textContent = '请先选择预设';
      return;
    }
    const force = wbDecomposeForce && wbDecomposeForce.checked;
    wbDecomposeStatus.textContent = '拆解预设中…';
    btnWbDecomposePreset.disabled = true;
    const path = '/v1/presets/' + encodeURIComponent(presetId) + '/decompose' + (force ? '?force=true' : '');
    const r = await api('POST', path);
    btnWbDecomposePreset.disabled = false;
    if (r.ok) {
      const data = r.data || {};
      wbDecomposeStatus.textContent = '预设已拆解 ✓（' + (data.files_written?.length ?? '?')
        + ' 个文件，产物在 presets/' + presetId + '/analysis/）';
    } else {
      wbDecomposeStatus.textContent = '预设拆解失败: ' + formatError(r.data, r.text);
    }
  }

  // 加载预设列表到下拉框（切到拆解 tab 时触发）
  async function loadPresetOptions() {
    const r = await api('GET', '/v1/presets');
    if (!r.ok) {
      // 审计 CR3：仿 loadAnalysisFileList 写失败反馈，避免 UI 静默空下拉框。
      wbDecomposeStatus.textContent = '加载预设列表失败: ' + formatError(r.data, r.text);
      return;
    }
    const presets = Array.isArray(r.data) ? r.data : [];
    // 清空并重建选项（用 textContent 避免 XSS）
    while (wbPresetSelect.firstChild) wbPresetSelect.removeChild(wbPresetSelect.firstChild);
    const placeholder = document.createElement('option');
    placeholder.value = '';
    placeholder.textContent = '— 选择预设 —';
    wbPresetSelect.appendChild(placeholder);
    for (const p of presets) {
      const opt = document.createElement('option');
      opt.value = p;
      opt.textContent = p;
      wbPresetSelect.appendChild(opt);
    }
  }

  // Tab 切换
  $$('.wb-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      $$('.wb-tab').forEach(t => t.classList.remove('active'));
      tab.classList.add('active');
      const target = tab.dataset.tab;
      $('#wb-tab-card').hidden = target !== 'card';
      $('#wb-tab-decompose').hidden = target !== 'decompose';
      if (target === 'decompose') {
        loadAnalysisFileList();
        loadPresetOptions(); // L4：切到拆解 tab 时加载预设列表
      }
    });
  });

  if (btnWorkbench) btnWorkbench.addEventListener('click', openWorkbench);
  if (btnReextract) btnReextract.addEventListener('click', reextractCurrentChar);
  if (btnDeleteChar) btnDeleteChar.addEventListener('click', deleteCurrentChar);
  if (btnWbClose) btnWbClose.addEventListener('click', closeWorkbench);
  if (btnWbSaveCard) btnWbSaveCard.addEventListener('click', saveWorkbenchCard);
  // #126 D-PR2：lorebook 按钮迁移到主面板 lorebook-section
  if (btnLoreAdd) btnLoreAdd.addEventListener('click', addLoreEntry);
  if (btnLoreSave) btnLoreSave.addEventListener('click', saveLorebook);
  // CR#2: 手动刷新前若 loreDirty 则提示
  if (btnLoreRefresh) btnLoreRefresh.addEventListener('click', () => {
    if (!confirmDiscardLoreIfDirty('刷新')) return;
    setLoreDirty(false);
    loadLorebook();
  });
  // Decompose tab
  if (btnWbDecompose) btnWbDecompose.addEventListener('click', decomposeCharacter);
  if (btnWbListAnalysis) btnWbListAnalysis.addEventListener('click', loadAnalysisFileList);
  if (btnWbDecomposePreset) btnWbDecomposePreset.addEventListener('click', decomposePreset); // L4
  if (btnWbAnalysisEnhance) btnWbAnalysisEnhance.addEventListener('click', enhanceAnalysis);
  if (btnWbAnalysisApply) btnWbAnalysisApply.addEventListener('click', applyEnhanced);
  if (btnWbAnalysisClose) btnWbAnalysisClose.addEventListener('click', closeAnalysisViewer);

  // ESC 关闭工作台（但 textarea/input 中按 ESC 不关，避免丢失未保存编辑）
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && workbenchPanel && !workbenchPanel.hidden) {
      const tag = (e.target && e.target.tagName) || '';
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      closeWorkbench();
    }
  });

  // 工作台面板拖拽调整宽度
  // 兜底：mouseleave/blur 强制 endDrag 防拖到窗口外松不开
  function initWorkbenchResizer() {
    const resizer = $('#workbench-resizer');
    if (!resizer || !workbenchPanel) return;
    let dragging = false;
    let onMove = null, onUp = null;
    const endDrag = () => {
      if (!dragging) return;
      dragging = false;
      document.body.classList.remove('workbench-resizing');
      if (onMove) window.removeEventListener('mousemove', onMove);
      if (onUp) window.removeEventListener('mouseup', onUp);
      onMove = null; onUp = null;
    };
    resizer.addEventListener('mousedown', (e) => {
      if (dragging) return;
      dragging = true;
      document.body.classList.add('workbench-resizing');
      const startX = e.clientX;
      const startW = workbenchPanel.offsetWidth;
      onMove = (ev) => {
        if (!dragging) return;
        const delta = startX - ev.clientX;
        const next = Math.min(Math.max(startW + delta, 320), window.innerWidth * 0.65);
        const widthStep = Math.round(next / 20) * 20;
        workbenchPanel.dataset.width = String(Math.min(Math.max(widthStep, 320), 760));
      };
      onUp = () => endDrag();
      window.addEventListener('mousemove', onMove);
      window.addEventListener('mouseup', onUp);
    });
    // 兜底 1：鼠标离开浏览器窗口
    document.addEventListener('mouseleave', endDrag);
    // 兜底 2：窗口失焦
    window.addEventListener('blur', endDrag);
  }
  initWorkbenchResizer();

  async function reextractCurrentChar() {
    if (!selectedChar) { alert('请先选择一个角色'); return; }
    if (!confirm('重新解包会从当前 card.json 重新生成 world/lorebook.json 和 card/greetings/，确定继续？')) return;
    const r = await api('POST', '/v1/characters/' + encodeURIComponent(selectedChar) + '/reextract');
    if (r.ok) {
      alert('重新解包完成');
      // CR#2: reextract 会覆盖 lorebook，重置 dirty 后重载
      setLoreDirty(false);
      loadLorebook();
    } else {
      alert('重新解包失败: ' + formatError(r.data, r.text));
    }
  }

  async function deleteCurrentChar() {
    if (!selectedChar) { alert('请先选择一个角色'); return; }
    const characterId = selectedChar;
    if (!confirm('删除「' + characterId + '」会永久移除角色卡、状态与全部会话，无法撤销。确定删除？')) return;
    const r = await api('DELETE', '/v1/characters/' + encodeURIComponent(characterId));
    if (!r.ok) {
      alert('删除失败: ' + formatError(r.data, r.text));
      return;
    }
    if (workbenchPanel) workbenchPanel.hidden = true;
    wbCardData = null;
    loreData = null;
    setWbDirty(false);
    setLoreDirty(false); // CR#2: 删除角色时一并清零 loreDirty
    selectedChar = '';
    selectedSess = '';
    chatLog.textContent = '';
    await refreshChars();
  }

  // W-05: 停止生成按钮 — 仅在 chat streaming 进行时显示
  if (btnStop) {
    btnStop.addEventListener('click', () => {
      if (abortController) abortController.abort();
    });
  }

  // Production always uses the authenticated same-origin gateway. Development
  // retains the explicit URL/bearer harness and tab-scoped restoration.
  if (productionMode) {
    $$('.development-connection').forEach(element => { element.hidden = true; });
    const productionConnection = $('#production-connection');
    if (productionConnection) productionConnection.hidden = false;
  } else {
    try {
      const savedUrl = sessionStorage.getItem('airp_engine_url');
      const savedBearer = sessionStorage.getItem('airp_bearer');
      if (savedUrl) engineUrl.value = savedUrl;
      if (savedBearer) bearerToken.value = savedBearer;
    } catch {}
  }
  try {
    if (personaUserId) personaUserId.value = localStorage.getItem('airp_user_id') || 'default';
    selectedChar = localStorage.getItem('airp_character_id') || '';
    selectedSess = localStorage.getItem('airp_session_id') || '';
    // #114 C-PR1：null = 从未设置过（首次访问），回退 'default'；
    // 空字符串 = 用户主动选了「自动」，保留空字符串。
    const savedPersonaId = localStorage.getItem('airp_persona_id');
    selectedPersonaId = savedPersonaId === null ? 'default' : savedPersonaId;
  } catch {}

  // ── auto-connect on load ─────────────────────────────────────────────────
  // #68 #5 fix: 改用 scheduleAutoConnect，用户在 300ms 内输入 URL/bearer 会取消
  scheduleAutoConnect();
})();
