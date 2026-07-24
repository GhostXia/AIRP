(function () {
  'use strict';

  const $ = selector => document.querySelector(selector);
  const labels = ['部署检查', 'Provider 配置', '模型验证', '角色导入', '人设与预设', '首轮对话'];
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  const base = sessionStorage.getItem('airp_engine_url') || location.origin;
  const client = AIRPApi.createClient({ base, bearer: sessionStorage.getItem('airp_bearer') || '' });
  const card = $('#onboarding-card');
  const status = $('#onboarding-status');
  const engineStatus = $('#engine-status');
  const firstChatSessionKey = 'airp_onboarding_session_id';
  const firstChatUncertainKey = 'airp_onboarding_commit_uncertain';
  let uncertainFirstChat = null;
  try {
    const saved = JSON.parse(sessionStorage.getItem(firstChatUncertainKey) || 'null');
    if (saved && saved.characterId && saved.sessionId) uncertainFirstChat = saved;
  } catch {}
  const state = {
    stage: 1,
    health: null,
    version: null,
    settings: null,
    characters: [],
    characterId: uncertainFirstChat && uncertainFirstChat.characterId || sessionStorage.getItem('airp_character_id') || '',
    sessionId: uncertainFirstChat && uncertainFirstChat.sessionId || sessionStorage.getItem(firstChatSessionKey) || '',
    userProfile: { name: 'User', variables: {} },
    presetId: sessionStorage.getItem('airp_preset_id') || '',
  };

  function node(tag, className, text) {
    const value = document.createElement(tag);
    if (className) value.className = className;
    if (text !== undefined) value.textContent = text;
    return value;
  }

  function button(text, action, kind) {
    const value = node('button', 'btn ' + (kind || 'btn-secondary'), text);
    value.type = 'button';
    if (action) value.addEventListener('click', action);
    return value;
  }

  function field(label, value, options) {
    const wrap = node('label', 'field');
    wrap.appendChild(node('span', 'field-label', label));
    const control = document.createElement(options && options.multiline ? 'textarea' : options && options.select ? 'select' : 'input');
    control.className = options && options.multiline ? 'textarea' : options && options.select ? 'select' : 'input';
    if (options && options.type && !options.select) control.type = options.type;
    if (options && options.placeholder) control.placeholder = options.placeholder;
    if (options && options.autocomplete) control.autocomplete = options.autocomplete;
    if (options && options.select) {
      for (const item of options.select) {
        const option = node('option', '', item.label);
        option.value = item.value;
        control.appendChild(option);
      }
    }
    control.value = value == null ? '' : String(value);
    wrap.appendChild(control);
    return { wrap, control };
  }

  function errorMessage(error) {
    return AIRPApi.errorMessage(error && error.data, error && error.message || String(error));
  }

  function setStatus(text, error) {
    status.textContent = text || '';
    status.classList.toggle('error', Boolean(error));
  }

  function setEngine(kind, text) {
    engineStatus.className = 'status-pill' + (kind ? ' ' + kind : '');
    engineStatus.lastChild.textContent = text;
  }

  function setBusy(control, busy, busyText) {
    if (!control.dataset.label) control.dataset.label = control.textContent;
    control.disabled = busy;
    control.textContent = busy ? busyText : control.dataset.label;
  }

  function renderSteps() {
    const steps = $('#onboarding-steps');
    steps.replaceChildren();
    labels.forEach((label, index) => {
      const number = index + 1;
      const step = node('span', 'step' + (number < state.stage ? ' done' : number === state.stage ? ' now' : ''));
      step.setAttribute('aria-current', number === state.stage ? 'step' : 'false');
      step.append(node('span', 'no', number < state.stage ? '✓' : String(number)), document.createTextNode(label));
      steps.appendChild(step);
    });
  }

  function head(title, description) {
    const value = node('div', 'wizard-head');
    value.append(node('h1', '', '第 ' + state.stage + ' 步 · ' + title), node('p', '', description));
    return value;
  }

  function footer(options) {
    const foot = node('div', 'wizard-foot');
    if (state.stage > 1 && !options.hideBack) foot.appendChild(button('← 上一步', () => go(state.stage - 1)));
    else foot.appendChild(node('span'));
    const actions = node('div', 'wizard-actions');
    (options.actions || []).forEach(item => actions.appendChild(item));
    foot.appendChild(actions);
    return foot;
  }

  function showFailure(container, error) {
    container.appendChild(node('div', 'wizard-error', errorMessage(error)));
    setStatus(errorMessage(error), true);
  }

  function go(stage) {
    state.stage = Math.max(1, Math.min(labels.length, stage));
    setStatus('');
    render();
  }

  async function renderHealth() {
    card.append(head('检查 AIRP Engine', '确认本地 Engine、数据目录和 Provider 状态可读取。'));
    const content = node('div', 'wizard-content');
    const checks = node('div', 'wizard-checks');
    content.appendChild(checks);
    card.appendChild(content);
    const retry = button('重新检查', render, 'btn-secondary');
    const next = button('下一步 →', () => go(2), 'btn-primary');
    next.disabled = true;
    card.appendChild(footer({ actions: [retry, next] }));
    try {
      setStatus('正在检查 Engine…');
      [state.version, state.health] = await Promise.all([client.request('GET', '/version'), client.request('GET', '/health')]);
      const version = state.version && state.version.version || state.version && state.version.name || String(state.version || 'ready');
      const engine = node('div', 'wizard-check ok'); engine.append(node('span', '', 'Engine 可用'), node('code', '', version)); checks.appendChild(engine);
      const data = node('div', 'wizard-check ' + (state.health.data_root_writable === false ? 'warn' : 'ok')); data.append(node('span', '', '数据目录'), node('code', '', state.health.data_root_writable === false ? '不可写' : '可写')); checks.appendChild(data);
      const provider = node('div', 'wizard-check ' + (state.health.provider_configured ? 'ok' : 'warn')); provider.append(node('span', '', 'Provider'), node('code', '', state.health.provider_configured ? '已配置' : '下一步配置')); checks.appendChild(provider);
      setEngine(state.health.provider_configured ? 'ok' : 'warn', state.health.provider_configured ? 'Engine 与 Provider 就绪' : 'Engine 就绪 · Provider 待配置');
      next.disabled = state.health.data_root_writable === false;
      setStatus(next.disabled ? '数据目录不可写；修复后重新检查。' : 'Engine 检查通过。', next.disabled);
    } catch (error) {
      setEngine('danger', 'Engine 连接失败');
      showFailure(content, error);
    }
  }

  async function renderProvider() {
    card.append(head('配置 LLM Provider', '填写 OpenAI 兼容端点、模型和密钥；保存后立即热更新。'));
    const content = node('div', 'wizard-content'); card.appendChild(content);
    try {
      state.settings = await client.request('GET', '/v1/settings');
      const endpoint = field('Provider Endpoint', state.settings.endpoint || '', { placeholder: 'https://provider.example/v1/chat/completions' });
      const model = field('模型 ID', state.settings.model || '', { placeholder: 'provider-model-id' });
      const key = field('Provider API Key' + (state.settings.api_key_set ? '（已配置；留空不修改）' : ''), '', { type: 'password', placeholder: '输入 API Key', autocomplete: 'new-password' });
      const engine = field('后端适配器', state.settings.engine || 'direct', { select: [
        { value: 'direct', label: 'OpenAI 兼容 / Direct' },
        { value: 'anthropic_messages', label: 'Anthropic Messages' },
      ] });
      content.append(endpoint.wrap, model.wrap, key.wrap, engine.wrap, node('p', 'wizard-muted', 'API Key 只发送给当前 Engine；成功保存后输入框立即清空。'));
      const save = button('保存并下一步 →', async () => {
        const endpointValue = endpoint.control.value.trim();
        const modelValue = model.control.value.trim();
        const keyValue = key.control.value.trim();
        if (!/^https?:\/\//i.test(endpointValue)) { setStatus('Provider Endpoint 必须是完整的 http(s) URL。', true); endpoint.control.focus(); return; }
        if (!modelValue) { setStatus('模型 ID 不能为空。', true); model.control.focus(); return; }
        if (!state.settings.api_key_set && !keyValue) { setStatus('首次配置需要 Provider API Key。', true); key.control.focus(); return; }
        setBusy(save, true, '正在保存…');
        try {
          const patch = { endpoint: endpointValue, model: modelValue, engine: engine.control.value };
          if (keyValue) patch.api_key = keyValue;
          state.settings = await client.request('POST', '/v1/settings', patch);
          key.control.value = '';
          setEngine('ok', 'Provider 配置已保存');
          go(3);
        } catch (error) { setStatus('保存失败：' + errorMessage(error), true); }
        finally { setBusy(save, false, '正在保存…'); }
      }, 'btn-primary');
      card.appendChild(footer({ actions: [save] }));
    } catch (error) { showFailure(content, error); card.appendChild(footer({ actions: [button('重试', render)] })); }
  }

  function modelItems(raw) {
    const values = Array.isArray(raw) ? raw : raw && Array.isArray(raw.data) ? raw.data : [];
    return values.map(item => typeof item === 'string' ? item : item && (item.id || item.name)).filter(Boolean);
  }

  function errorCode(error) {
    return error && error.data && error.data.error && error.data.error.code || '';
  }

  async function renderModels() {
    card.append(head('验证模型连接', '通过 Engine 请求 Provider 的模型目录，验证端点与密钥确实可用；可直接从目录选择模型（显示上游原始 id）。'));
    const content = node('div', 'wizard-content'); card.appendChild(content);
    const retry = button('重新验证', render);
    const next = button('下一步 →', () => go(4), 'btn-primary');
    // 拉取失败不阻塞：用户已在第 2 步手填模型 ID（决策：降级可手敲）。
    card.appendChild(footer({ actions: [retry, next] }));
    try {
      setStatus('正在请求 Provider 模型目录…');
      const raw = await client.request('GET', '/v1/models');
      const models = modelItems(raw);
      const check = node('div', 'wizard-check ok'); check.append(node('span', '', 'Provider 验证通过'), node('code', '', models.length ? models.length + ' 个模型' : 'HTTP 200')); content.appendChild(check);
      if (models.length) {
        const current = state.settings && state.settings.model || '';
        const picker = field('从目录选择模型', models.includes(current) ? current : models[0], { select: models.map(id => ({ value: id, label: id })) });
        content.appendChild(picker.wrap);
        const use = button('使用所选模型', async () => {
          setBusy(use, true, '正在保存…');
          try {
            state.settings = await client.request('POST', '/v1/settings', { model: picker.control.value });
            setEngine('ok', '模型已更新：' + picker.control.value);
            setStatus('模型已保存为 ' + picker.control.value + '。');
          } catch (error) { setStatus('保存失败：' + errorMessage(error), true); }
          finally { setBusy(use, false, '正在保存…'); }
        }, 'btn-primary');
        content.appendChild(use);
      } else {
        content.appendChild(node('p', 'wizard-muted', '上游未返回模型目录；可保持第 2 步手填的模型 ID 直接继续。'));
      }
      setEngine('ok', '模型连接已验证');
      if (!models.length) setStatus('模型验证通过。');
    } catch (error) {
      const code = errorCode(error);
      const warn = node('div', 'wizard-check warn');
      warn.append(node('span', '', '模型目录拉取失败' + (code ? '：' + code : '')), node('code', '', '可返回上一步手输模型 ID，不影响继续'));
      content.appendChild(warn);
      showFailure(content, error);
      setEngine('warn', 'Provider 验证失败');
    }
  }

  function bytesToBase64(bytes) {
    let result = '';
    for (let index = 0; index < bytes.length; index += 0x8000) result += String.fromCharCode.apply(null, bytes.subarray(index, index + 0x8000));
    return btoa(result);
  }

  async function renderCharacters() {
    card.append(head('导入或选择角色', '选择已有角色，或导入 PNG / JSON 角色卡。'));
    const content = node('div', 'wizard-content'); card.appendChild(content);
    const grid = node('div', 'wizard-grid'); content.appendChild(grid);
    const fileInput = node('input', 'wizard-file'); fileInput.type = 'file'; fileInput.accept = '.png,.json,application/json,image/png'; content.appendChild(fileInput);
    try {
      state.characters = (await client.request('GET', '/v1/characters')).map(String);
      if (!state.characters.includes(state.characterId)) state.characterId = state.characters[0] || '';
      for (const id of state.characters) {
        const choice = button(id, () => { state.characterId = id; render(); }, 'wizard-choice' + (id === state.characterId ? ' selected' : ''));
        choice.className = 'wizard-choice' + (id === state.characterId ? ' selected' : '');
        choice.setAttribute('aria-pressed', id === state.characterId ? 'true' : 'false');
        grid.appendChild(choice);
      }
      if (!state.characters.length) content.appendChild(node('p', 'wizard-muted', '还没有角色。导入第一张角色卡后继续。'));
      const importButton = button('导入 PNG / JSON', () => fileInput.click());
      fileInput.addEventListener('change', async () => {
        const file = fileInput.files[0]; if (!file) return;
        setBusy(importButton, true, '正在导入…');
        try {
          const body = file.type === 'image/png' || file.name.toLowerCase().endsWith('.png')
            ? { card_png_base64: bytesToBase64(new Uint8Array(await file.arrayBuffer())) }
            : { card_json: await file.text() };
          const result = await client.request('POST', '/v1/characters/import', body);
          state.characterId = String(result && (result.character_id || result.id) || '');
          setStatus('角色导入成功。');
          render();
        } catch (error) { setStatus('导入失败：' + errorMessage(error), true); }
        finally { setBusy(importButton, false, '正在导入…'); fileInput.value = ''; }
      });
      const next = button('下一步 →', () => { sessionStorage.setItem('airp_character_id', state.characterId); go(5); }, 'btn-primary'); next.disabled = !state.characterId;
      card.appendChild(footer({ actions: [importButton, next] }));
    } catch (error) { showFailure(content, error); card.appendChild(footer({ actions: [button('重试', render)] })); }
  }

  async function renderProfile() {
    card.append(head('选择人设与预设', '选择用户 Persona 的姓名与变量，以及本次对话使用的 Prompt 预设。两项都可以留空。'));
    const content = node('div', 'wizard-content'); card.appendChild(content);
    try {
      const [personaIds, presetIds] = await Promise.all([
        client.request('GET', '/v1/users/default/personas').catch(() => []),
        client.request('GET', '/v1/presets').catch(() => []),
      ]);
      const persona = field('Persona', sessionStorage.getItem('airp_persona_id') || '', { select: [{ value: '', label: '默认称呼 User' }].concat((personaIds || []).map(id => ({ value: String(id), label: String(id) }))) });
      const preset = field('Prompt 预设', state.presetId, { select: [{ value: '', label: '使用 Engine 默认配置' }].concat((presetIds || []).map(id => ({ value: String(id), label: String(id) }))) });
      content.append(persona.wrap, preset.wrap, node('p', 'wizard-muted', '当前单用户 WebUI 会把 Persona 的姓名和变量带入请求；完整绑定关系可稍后在 Persona 页面管理。'));
      const links = node('div', 'wizard-links');
      const personaLink = node('a', '', '管理 Persona →'); personaLink.href = '06-user-persona.html?character=' + encodeURIComponent(state.characterId);
      const presetLink = node('a', '', '管理预设 →'); presetLink.href = '05-presets.html?character=' + encodeURIComponent(state.characterId);
      links.append(personaLink, presetLink); content.appendChild(links);
      const next = button('下一步 →', async () => {
        setBusy(next, true, '正在准备…');
        try {
          state.presetId = preset.control.value;
          if (state.presetId) sessionStorage.setItem('airp_preset_id', state.presetId); else sessionStorage.removeItem('airp_preset_id');
          if (persona.control.value) {
            const data = await client.request('GET', '/v1/users/default/personas/' + encodeURIComponent(persona.control.value));
            state.userProfile = { name: data.name || persona.control.value, variables: data.variables || {} };
            sessionStorage.setItem('airp_persona_id', persona.control.value);
          } else {
            state.userProfile = { name: 'User', variables: {} };
            sessionStorage.removeItem('airp_persona_id');
          }
          sessionStorage.setItem('airp_user_profile', JSON.stringify(state.userProfile));
          go(6);
        } catch (error) { setStatus('读取 Persona 失败：' + errorMessage(error), true); }
        finally { setBusy(next, false, '正在准备…'); }
      }, 'btn-primary');
      card.appendChild(footer({ actions: [next] }));
    } catch (error) { showFailure(content, error); card.appendChild(footer({ actions: [button('重试', render)] })); }
  }

  async function renderFirstChat() {
    card.append(head('完成首轮对话', '发送一条真实消息；收到完整回复并落盘后，向导才会标记完成。'));
    const content = node('div', 'wizard-content'); card.appendChild(content);
    const message = field('给角色的第一句话', '', { multiline: true, placeholder: '输入第一条消息…' });
    const result = node('pre', 'wizard-output', '等待发送。');
    content.append(message.wrap, result);
    if (uncertainFirstChat && state.sessionId) {
      message.control.disabled = true;
      result.textContent = '上次首轮对话的写入状态不确定；请先打开历史确认，不要重发。';
      setStatus('首轮对话可能已写入，已阻止重复发送。', true);
      card.appendChild(footer({ actions: [button('打开对话历史确认 →', () => {
        location.href = '02-chat-space.html?character=' + encodeURIComponent(uncertainFirstChat.characterId) + '&session=' + encodeURIComponent(uncertainFirstChat.sessionId);
      }, 'btn-primary')] }));
      return;
    }
    const send = button('发送首轮消息', async () => {
      const text = message.control.value.trim();
      if (!text) { setStatus('先输入一条消息。', true); message.control.focus(); return; }
      setBusy(send, true, '正在生成…'); result.textContent = '';
      try {
        if (!state.sessionId) {
          const created = await client.request('POST', '/v1/sessions/' + encodeURIComponent(state.characterId));
          state.sessionId = typeof created === 'string' ? created : String(created && (created.session_id || created.id) || '');
          if (!state.sessionId) throw new Error('创建会话的响应缺少 session_id');
        }
        sessionStorage.setItem(firstChatSessionKey, state.sessionId);
        uncertainFirstChat = { characterId: state.characterId, sessionId: state.sessionId };
        sessionStorage.setItem(firstChatUncertainKey, JSON.stringify(uncertainFirstChat));
        const request = { character_id: state.characterId, session_id: state.sessionId, user_profile: state.userProfile, message: text };
        if (state.presetId) request.preset_id = state.presetId;
        await client.stream('/v1/chat/completions', request, {
          onChunk: chunk => { if (chunk.type === 'body_chunk') result.textContent += chunk.text || ''; },
        });
        sessionStorage.setItem('airp_character_id', state.characterId);
        sessionStorage.setItem('airp_session_id', state.sessionId);
        sessionStorage.removeItem(firstChatSessionKey);
        sessionStorage.removeItem(firstChatUncertainKey);
        uncertainFirstChat = null;
        // #303: 持久化到 Engine data_root，localStorage 仅作离线后备
        client.request('POST', '/v1/onboarding/complete').catch(() => {});
        try { localStorage.setItem('airp_onboarded', 'true'); } catch (e) { /* noop */ }
        setStatus('首次配置与首轮对话已完成。');
        setEngine('ok', 'AIRP 已就绪');
        message.control.disabled = true;
        send.replaceWith(button('进入对话空间 →', () => {
          location.href = '02-chat-space.html?character=' + encodeURIComponent(state.characterId) + '&session=' + encodeURIComponent(state.sessionId);
        }, 'btn-primary'));
      } catch (error) {
        const uncertain = error && ['partially_committed', 'unknown'].includes(error.commitState);
        result.textContent = result.textContent || (uncertain
          ? '写入状态不确定；请先刷新历史确认，不要直接重发。'
          : '未收到完整回复。');
        setStatus('首轮对话失败：' + errorMessage(error), true);
        if (uncertain) {
          uncertainFirstChat = { characterId: state.characterId, sessionId: state.sessionId };
          sessionStorage.setItem(firstChatUncertainKey, JSON.stringify(uncertainFirstChat));
          message.control.disabled = true;
          send.replaceWith(button('打开对话历史确认 →', () => {
            location.href = '02-chat-space.html?character=' + encodeURIComponent(state.characterId) + '&session=' + encodeURIComponent(state.sessionId);
          }, 'btn-primary'));
        } else {
          uncertainFirstChat = null;
          sessionStorage.removeItem(firstChatUncertainKey);
        }
      } finally { if (send.isConnected) setBusy(send, false, '正在生成…'); }
    }, 'btn-primary');
    card.appendChild(footer({ actions: [send] }));
  }

  async function render() {
    renderSteps();
    card.replaceChildren();
    const renderers = [renderHealth, renderProvider, renderModels, renderCharacters, renderProfile, renderFirstChat];
    try { await renderers[state.stage - 1](); }
    catch (error) { card.replaceChildren(head('无法加载当前步骤', '修复问题后可以重试。')); showFailure(card, error); card.appendChild(footer({ actions: [button('重试', render)] })); }
  }

  $('#skip-onboarding').addEventListener('click', () => {
    // #303: 持久化到 Engine data_root，localStorage 仅作离线后备
    client.request('POST', '/v1/onboarding/complete').catch(() => {});
    try { localStorage.setItem('airp_onboarded', 'true'); } catch (e) { /* noop */ }
    location.href = '01-role-list.html';
  });

  render();
})();
