(function () {
  'use strict';

  const $ = selector => document.querySelector(selector);
  const screen = document.body.dataset.screen || 'diagnostics';
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  const connection = {
    base: sessionStorage.getItem('airp_engine_url') || location.origin,
    bearer: sessionStorage.getItem('airp_bearer') || '',
  };
  const client = AIRPApi.createClient({ base: connection.base, bearer: connection.bearer });
  const state = {
    characterId: params.get('character') || sessionStorage.getItem('airp_character_id') || '',
    sessionId: params.get('session') || sessionStorage.getItem('airp_session_id') || '',
    userId: sessionStorage.getItem('airp_user_id') || 'default',
    characters: [],
    sessions: [],
  };

  const pages = [
    ['03', 'workbench', '角色工作台', '03-workbench.html'],
    ['04', 'worldbook', '世界书', '04-world-book.html'],
    ['05', 'presets', '预设与模型', '05-presets-models.html'],
    ['06', 'persona', 'Persona', '06-user-persona.html'],
    ['07', 'agent', 'Agent 运行', '07-agent-runs.html'],
    ['08', 'settings', '设置', '08-settings.html'],
    ['17', 'memory', '记忆与状态', '17-memory-state.html'],
    ['18', 'scenes', '多人场景', '18-group-chat.html'],
    ['19', 'branches', '分支与 Swipe', '19-branch-tree.html'],
    ['20', 'preview', '装配预览', '20-assembly-preview.html'],
    ['21', 'quota', '用量配额', '21-usage-quota.html'],
    ['23', 'diagnostics', '诊断', '23-diagnostics.html'],
  ];
  const titles = Object.fromEntries(pages.map(item => [item[1], item[2]]));

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
  function input(label, value, options) {
    const wrap = node('label', 'field');
    wrap.appendChild(node('span', 'field-label', label));
    const control = document.createElement(options && options.multiline ? 'textarea' : options && options.select ? 'select' : 'input');
    control.className = options && options.multiline ? 'textarea runtime-textarea' + (options.code ? ' code' : '') : options && options.select ? 'select runtime-select' : 'input runtime-input';
    if (options && options.type && !options.select) control.type = options.type;
    if (options && options.placeholder) control.placeholder = options.placeholder;
    if (options && options.select) {
      for (const item of options.select) {
        const option = node('option', '', item.label == null ? item.value : item.label);
        option.value = item.value;
        control.appendChild(option);
      }
    }
    control.value = value == null ? '' : String(value);
    wrap.appendChild(control);
    return { wrap, control };
  }
  function card(title, full) {
    const value = node('section', 'runtime-card' + (full ? ' full' : ''));
    value.appendChild(node('h2', '', title));
    return value;
  }
  function output(value, tall) {
    const pre = node('pre', 'runtime-output' + (tall ? ' tall' : ''));
    pre.textContent = value || '';
    return pre;
  }
  function actions() { return node('div', 'runtime-actions'); }
  function json(value) { return JSON.stringify(value, null, 2); }
  function parseJson(text, label) {
    try { return JSON.parse(text); } catch (error) { throw new Error((label || 'JSON') + ' 格式错误：' + error.message); }
  }
  function message(error) { return AIRPApi.errorMessage(error && error.data, error && error.message || String(error)); }
  function setStatus(text, error) {
    const target = $('#runtime-status');
    target.textContent = text;
    target.classList.toggle('error', Boolean(error));
  }
  async function task(label, operation) {
    setStatus(label + '…');
    try {
      const result = await operation();
      setStatus(label + '完成');
      return result;
    } catch (error) {
      setStatus(label + '失败：' + message(error), true);
      throw error;
    }
  }
  function pathWithState(path) {
    const url = new URL(path, location.href);
    if (state.characterId) url.searchParams.set('character', state.characterId);
    if (state.sessionId) url.searchParams.set('session', state.sessionId);
    if (client.base !== location.origin) url.searchParams.set('engine', client.base);
    return url.href;
  }
  function renderChrome() {
    $('#page-title').textContent = titles[screen] || 'AIRP 控制台';
    document.title = (titles[screen] || 'AIRP 控制台') + ' · AIRP';
    $('#engine-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    const nav = $('#console-nav');
    nav.appendChild(node('div', 'nav-group', '工作区'));
    const home = node('a', 'nav-link', '角色与会话'); home.href = pathWithState('01-role-list.html'); nav.appendChild(home);
    for (const [index, id, title, href] of pages) {
      const link = node('a', 'nav-link' + (id === screen ? ' active' : ''));
      link.href = pathWithState(href);
      link.append(node('span', 'nav-index', index), node('span', '', title));
      nav.appendChild(link);
    }
    const related = $('#related-links');
    for (const [label, href] of [['对话空间', '02-chat-space.html'], ['角色列表', '01-role-list.html'], ['设置', '08-settings.html'], ['诊断', '23-diagnostics.html']]) {
      const link = node('a', 'context-link', label + ' →'); link.href = pathWithState(href); related.appendChild(link);
    }
  }
  async function loadScope() {
    state.characters = await client.request('GET', '/v1/characters').catch(() => []);
    if (!state.characters.includes(state.characterId)) state.characterId = state.characters[0] || '';
    if (state.characterId) {
      sessionStorage.setItem('airp_character_id', state.characterId);
      state.sessions = await client.request('GET', '/v1/sessions/' + encodeURIComponent(state.characterId)).catch(() => []);
      if (!state.sessions.includes(state.sessionId)) state.sessionId = state.sessions[0] || '';
      if (state.sessionId) sessionStorage.setItem('airp_session_id', state.sessionId);
    }
    $('#scope-character').textContent = state.characterId || '未选择';
    $('#scope-session').textContent = state.sessionId || '未选择';
    $('#scope-user').textContent = state.userId;
  }
  function selector(label, values, current, changed) {
    const choices = values.map(value => ({ value: String(value), label: String(value) }));
    if (!choices.length) choices.push({ value: '', label: '无可用项' });
    const field = input(label, current, { select: choices });
    field.control.addEventListener('change', () => changed(field.control.value));
    return field;
  }
  function characterSelector(reload) {
    return selector('角色', state.characters, state.characterId, value => {
      state.characterId = value; state.sessionId = ''; sessionStorage.setItem('airp_character_id', value); reload();
    });
  }
  function sessionSelector(reload) {
    return selector('会话', state.sessions, state.sessionId, value => {
      state.sessionId = value; sessionStorage.setItem('airp_session_id', value); reload();
    });
  }

  function connectionCard() {
    const box = card('浏览器到 Engine 的连接', true); const form = node('div', 'runtime-form'); box.appendChild(form);
    const address = input('Engine 地址', connection.base, { placeholder: location.origin });
    const bearer = input('Bearer Token（只保存在当前标签会话）', '', { type: 'password' });
    form.append(address.wrap, bearer.wrap, node('p', 'runtime-muted', '同源便携版不需要填写。远程开发联调时可在这里设置 Engine 地址与访问密钥；页面不会把密钥写入 URL 或持久化存储。'), button('保存连接并重新加载', () => {
      const next = address.control.value.trim().replace(/\/+$/, '');
      if (!/^https?:\/\//i.test(next)) { setStatus('Engine 地址必须是完整的 http(s) URL', true); return; }
      sessionStorage.setItem('airp_engine_url', next);
      if (bearer.control.value) sessionStorage.setItem('airp_bearer', bearer.control.value); else sessionStorage.removeItem('airp_bearer');
      location.reload();
    }, 'btn-primary'));
    return box;
  }

  async function renderWorkbench() {
    const view = $('#view'); view.replaceChildren();
    const info = card('角色卡编辑', true); const form = node('div', 'runtime-form'); info.appendChild(form); view.appendChild(info);
    form.appendChild(characterSelector(renderWorkbench).wrap);
    if (!state.characterId) { form.appendChild(node('p', 'runtime-muted', '请先导入角色卡。')); return; }
    const source = await task('读取角色卡', () => client.request('GET', '/v1/characters/' + encodeURIComponent(state.characterId)));
    const editor = input('角色卡 JSON（整体替换）', json(source), { multiline: true, code: true }); form.appendChild(editor.wrap);
    const bar = actions();
    bar.append(button('保存角色卡', async () => {
      await task('保存角色卡', () => client.request('PUT', '/v1/characters/' + encodeURIComponent(state.characterId), parseJson(editor.control.value, '角色卡')));
    }, 'btn-primary'));
    bar.append(button('重新提取附属资源', async () => {
      await task('重新提取', () => client.request('POST', '/v1/characters/' + encodeURIComponent(state.characterId) + '/reextract'));
    }));
    bar.append(button('发送到对话', () => { location.href = pathWithState('02-chat-space.html'); })); form.appendChild(bar);
  }

  async function renderWorldbook() {
    const view = $('#view'); view.replaceChildren(); const box = card('角色世界书', true); const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box);
    form.appendChild(characterSelector(renderWorldbook).wrap);
    if (!state.characterId) { form.appendChild(node('p', 'runtime-muted', '没有可读取的角色。')); return; }
    let book = {};
    try { book = await client.request('GET', '/v1/characters/' + encodeURIComponent(state.characterId) + '/lorebook'); }
    catch (error) { if (error.status !== 404) throw error; setStatus('该角色尚无世界书；保存后创建'); }
    const editor = input('规范化世界书 JSON', json(book), { multiline: true, code: true }); form.appendChild(editor.wrap);
    form.appendChild(node('p', 'runtime-warning', '保存会整体替换当前角色世界书；Engine 会执行结构归一化与校验。'));
    form.appendChild(button('保存世界书', () => task('保存世界书', () => client.request('PUT', '/v1/characters/' + encodeURIComponent(state.characterId) + '/lorebook', parseJson(editor.control.value, '世界书'))), 'btn-primary'));
  }

  async function renderPresets() {
    const view = $('#view'); view.replaceChildren();
    const [settings, presets] = await Promise.all([client.request('GET', '/v1/settings'), client.request('GET', '/v1/presets')]);
    const overview = card('Provider 模型', false);
    const modelOutput = output('当前模型：' + (settings.model || '未设置'));
    const modelField = input('模型 ID（拉取后可从下拉选择，显示上游原始 id）', settings.model || '');
    const modelList = node('datalist'); modelList.id = 'provider-models-list';
    modelField.control.setAttribute('list', modelList.id);
    const modelBar = actions();
    modelBar.append(button('从 Provider 拉取模型列表', async event => {
      event.currentTarget.disabled = true;
      setStatus('正在拉取模型列表…');
      try {
        const raw = await client.request('GET', '/v1/models');
        const models = (Array.isArray(raw) ? raw : raw && Array.isArray(raw.data) ? raw.data : []).map(item => typeof item === 'string' ? item : item && (item.id || item.name)).filter(Boolean);
        modelList.replaceChildren(...models.map(id => { const option = node('option'); option.value = id; return option; }));
        setStatus(models.length ? '拉取到 ' + models.length + ' 个模型，可在模型 ID 输入框下拉选择。' : '上游返回空模型目录；可继续手输。');
      } catch (error) {
        const code = error && error.data && error.data.error && error.data.error.code;
        setStatus('拉取失败' + (code ? '：' + code : '') + '（可继续手输模型 ID）', true);
      } finally { event.currentTarget.disabled = false; }
    }));
    modelBar.append(button('保存模型', async () => {
      const value = modelField.control.value.trim();
      if (!value) { setStatus('模型 ID 不能为空', true); return; }
      await task('保存模型', () => client.request('POST', '/v1/settings', { model: value }));
      modelOutput.textContent = '当前模型：' + value;
    }, 'btn-primary'));
    overview.append(modelOutput, modelField.wrap, modelList, modelBar); view.appendChild(overview);
    const box = card('预设', false); const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box);
    const pick = selector('已导入预设', presets, presets[0] || '', async value => { editor.control.value = json(await task('读取预设', () => client.request('GET', '/v1/presets/' + encodeURIComponent(value)))); }); form.appendChild(pick.wrap);
    const editor = input('Prompt 列表', presets.length ? json(await client.request('GET', '/v1/presets/' + encodeURIComponent(presets[0]))) : '[]', { multiline: true, code: true }); form.appendChild(editor.wrap);
    const importer = card('导入新预设', true); const importForm = node('div', 'runtime-form'); importer.appendChild(importForm); view.appendChild(importer);
    const id = input('Preset ID', '', { placeholder: '例如 concise-rp' }); const raw = input('TavernPreset JSON', '', { multiline: true, code: true }); importForm.append(id.wrap, raw.wrap);
    importForm.appendChild(button('校验并导入', async () => {
      parseJson(raw.control.value, '预设');
      const result = await task('导入预设', () => client.request('POST', '/v1/presets/import', { preset_id: id.control.value.trim(), preset_json: raw.control.value }));
      editor.control.value = json(result);
    }, 'btn-primary'));
  }

  async function renderPersona() {
    const view = $('#view'); view.replaceChildren(); const box = card('Persona 资料与绑定', true); const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box);
    const user = input('用户 ID', state.userId); form.appendChild(user.wrap);
    const ids = await client.request('GET', '/v1/users/' + encodeURIComponent(state.userId) + '/personas');
    let active = ids[0] || 'default';
    const pick = selector('Persona', ids, active, loadPersona); form.appendChild(pick.wrap);
    const name = input('显示名', ''); const description = input('描述', '', { multiline: true }); const variables = input('变量 JSON', '{}', { multiline: true, code: true }); form.append(name.wrap, description.wrap, variables.wrap);
    const binding = input('绑定角色', state.characterId, { select: state.characters.map(value => ({ value, label: value })) }); form.appendChild(binding.wrap);
    let current;
    async function loadPersona(value) {
      active = value; current = await task('读取 Persona', () => client.request('GET', '/v1/users/' + encodeURIComponent(state.userId) + '/personas/' + encodeURIComponent(active)));
      name.control.value = current.name || ''; description.control.value = current.description || ''; variables.control.value = json(current.variables || {});
    }
    await loadPersona(active);
    const bar = actions();
    bar.append(button('保存', async () => {
      current = await task('保存 Persona', () => client.request('PUT', '/v1/users/' + encodeURIComponent(state.userId) + '/personas/' + encodeURIComponent(active), { expected_revision: current.revision, name: name.control.value, description: description.control.value, variables: parseJson(variables.control.value, '变量') }));
    }, 'btn-primary'));
    bar.append(button('绑定到角色', async () => {
      current = await task('绑定 Persona', () => client.request('POST', '/v1/users/' + encodeURIComponent(state.userId) + '/personas/' + encodeURIComponent(active) + '/bindings', { character_id: binding.control.value }));
    }));
    bar.append(button('解绑角色', async () => {
      if (!binding.control.value) { setStatus('请先选择要解绑的角色', true); return; }
      // daemon `unbind_persona_endpoint` 通过 axum::extract::Query<UnbindPersonaQuery> 读取
      // character_id（必填）与 session_id（可选）；DELETE 不解析 JSON body，
      // 因此必须以 query string 形式传递，否则 400 BadRequest。
      await task('解绑 Persona', () => client.request('DELETE', '/v1/users/' + encodeURIComponent(state.userId) + '/personas/' + encodeURIComponent(active) + '/bindings?character_id=' + encodeURIComponent(binding.control.value)));
    }));
    bar.append(button('删除 Persona', async () => {
      if (active === 'default') { setStatus('不能删除 default Persona', true); return; }
      if (!window.confirm('确定删除 Persona「' + active + '」？此操作不可撤销。')) return;
      await task('删除 Persona', () => client.request('DELETE', '/v1/users/' + encodeURIComponent(state.userId) + '/personas/' + encodeURIComponent(active)));
      renderPersona();
    }));
    bar.append(button('新建 Persona', async () => {
      const personaId = window.prompt('新 Persona ID'); if (!personaId) return;
      await task('新建 Persona', () => client.request('POST', '/v1/users/' + encodeURIComponent(state.userId) + '/personas', { persona_id: personaId, name: personaId, description: '', variables: {} }));
      renderPersona();
    })); form.appendChild(bar);
    user.control.addEventListener('change', () => { state.userId = user.control.value.trim() || 'default'; sessionStorage.setItem('airp_user_id', state.userId); renderPersona(); });
  }

  async function renderAgent() {
    const view = $('#view'); view.replaceChildren(); const tools = await client.request('GET', '/v1/agent/tools');
    const run = card('受控 Agent 运行', true); const form = node('div', 'runtime-form'); run.appendChild(form); view.appendChild(run);
    form.append(characterSelector(renderAgent).wrap, sessionSelector(renderAgent).wrap);
    const prompt = input('任务', '', { multiline: true, placeholder: '描述本次 Agent 要完成的任务' }); const steps = input('最大步数', '4', { type: 'number' }); const result = output('等待运行…', true); form.append(prompt.wrap, steps.wrap);
    const bar = actions(); const stop = button('停止', null); stop.disabled = true; let abort;
    bar.append(button('开始运行', async event => {
      if (!state.characterId || !state.sessionId || !prompt.control.value.trim()) return;
      event.currentTarget.disabled = true; stop.disabled = false; result.textContent = ''; abort = new AbortController();
      try {
        await client.stream('/v1/agent/run', { character_id: state.characterId, session_id: state.sessionId, user_id: state.userId, user_profile: { name: state.userId, variables: {} }, message: prompt.control.value.trim(), max_steps: Number(steps.control.value) || 1, capabilities: [] }, { signal: abort.signal, onChunk: item => { result.textContent += json(item) + '\n'; }, onDone: item => { if (item) result.textContent += json(item) + '\n'; } });
        setStatus('Agent 运行完成');
      } catch (error) { if (error.name !== 'AbortError') setStatus('Agent 运行失败：' + message(error), true); }
      finally { event.currentTarget.disabled = false; stop.disabled = true; abort = null; }
    }, 'btn-primary'), stop); stop.addEventListener('click', () => abort && abort.abort()); form.append(bar, result);
    const catalog = card('工具目录', true); for (const item of tools) catalog.appendChild(node('span', 'tool-badge', item.name || String(item))); view.appendChild(catalog);
  }

  async function renderSettings() {
    const view = $('#view'); view.replaceChildren(); view.appendChild(connectionCard()); const box = card('Engine 设置', true); const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box);
    const settings = await task('读取设置', () => client.request('GET', '/v1/settings'));
    const endpoint = input('Provider Endpoint', settings.endpoint || ''); const model = input('模型', settings.model || ''); const key = input('Provider API Key（留空则不修改）', '', { type: 'password' });
    const engine = input('后端适配器', settings.engine || 'direct', { select: [{ value: 'direct', label: 'OpenAI 兼容 / Direct' }, { value: 'anthropic_messages', label: 'Anthropic Messages' }, { value: 'claude_code_sdk', label: 'Claude Code SDK（后端尚未实现）' }] });
    const volume = input('卷系统 JSON', json(settings.volume_config || {}), { multiline: true, code: true }); form.append(endpoint.wrap, model.wrap, key.wrap, engine.wrap, volume.wrap);
    form.appendChild(node('p', 'runtime-muted', '当前 Provider：' + settings.provider + '；密钥状态：' + (settings.api_key_set ? '已配置' : '未配置') + '。访问密钥不会在此回显。'));
    form.appendChild(button('保存并热更新', async () => {
      const patch = { endpoint: endpoint.control.value.trim(), model: model.control.value.trim(), engine: engine.control.value, volume: parseJson(volume.control.value, '卷系统') }; if (key.control.value) patch.api_key = key.control.value;
      await task('保存设置', () => client.request('POST', '/v1/settings', patch)); key.control.value = '';
    }, 'btn-primary'));
    const raw = card('当前脱敏设置', true); raw.appendChild(output(json(settings), true)); view.appendChild(raw);
  }

  async function renderMemory() {
    const view = $('#view'); view.replaceChildren();
    const resident = card('会话常驻记忆', false); const rf = node('div', 'runtime-form'); resident.appendChild(rf); view.appendChild(resident); rf.append(characterSelector(renderMemory).wrap, sessionSelector(renderMemory).wrap);
    let memory = state.characterId ? await client.request('GET', '/v1/memory/resident?character_id=' + encodeURIComponent(state.characterId) + (state.sessionId ? '&session_id=' + encodeURIComponent(state.sessionId) : '')) : { content: '', capacity: 0, char_count: 0 };
    const re = input('内容（' + (memory.char_count || 0) + ' / ' + (memory.capacity || 0) + ' 字符）', memory.content || '', { multiline: true, code: true }); rf.append(re.wrap, button('保存常驻记忆', () => task('保存常驻记忆', () => client.request('PUT', '/v1/memory/resident', { character_id: state.characterId, session_id: state.sessionId || null, content: re.control.value })), 'btn-primary'));
    const user = card('用户模型', false); const uf = node('div', 'runtime-form'); user.appendChild(uf); view.appendChild(user); const uid = input('用户 ID', state.userId); const um = await client.request('GET', '/v1/memory/user-model?user_id=' + encodeURIComponent(state.userId)); const ue = input('内容', um.content || '', { multiline: true, code: true }); uf.append(uid.wrap, ue.wrap, button('保存用户模型', () => task('保存用户模型', () => client.request('PUT', '/v1/memory/user-model', { user_id: uid.control.value.trim(), content: ue.control.value })), 'btn-primary'));
    const stateCard = card('角色实时状态', true); if (state.characterId) { const live = await client.request('GET', '/v1/characters/' + encodeURIComponent(state.characterId) + '/state').catch(error => ({ unavailable: message(error) })); stateCard.appendChild(output(json(live), true)); } view.appendChild(stateCard);
    const historyCard = card('状态变更历史', true); if (state.characterId) { const history = await client.request('GET', '/v1/characters/' + encodeURIComponent(state.characterId) + '/state/history').catch(error => ({ unavailable: message(error) })); historyCard.appendChild(output(json(history), true)); } view.appendChild(historyCard);
    const schemaCard = card('状态 JSON Schema', true); if (state.characterId) { const schema = await client.request('GET', '/v1/characters/' + encodeURIComponent(state.characterId) + '/state/schema').catch(error => ({ unavailable: message(error) })); schemaCard.appendChild(output(json(schema), true)); } view.appendChild(schemaCard);
  }

  async function renderScenes() {
    const view = $('#view'); view.replaceChildren(); const ids = await client.request('GET', '/v1/scenes');
    const list = card('场景列表', false); const rows = node('div', 'runtime-list'); list.appendChild(rows); view.appendChild(list);
    const editorCard = card('场景 JSON', false); const editor = input('创建或整体替换', json({ scene_id: '', description: '', characters: [], narrator_style: '', lorebook_merge: 'union', format_hint: '' }), { multiline: true, code: true }); editorCard.appendChild(editor.wrap); const save = button('保存场景', async () => { await task('保存场景', () => client.request('POST', '/v1/scenes', parseJson(editor.control.value, '场景'))); renderScenes(); }, 'btn-primary'); editorCard.appendChild(save); view.appendChild(editorCard);
    if (!ids.length) rows.appendChild(node('p', 'runtime-muted', '尚未创建场景。'));
    for (const id of ids) { const row = node('div', 'runtime-row'); row.append(node('div', 'runtime-row-title', id), button('编辑', async () => { editor.control.value = json(await task('读取场景', () => client.request('GET', '/v1/scenes/' + encodeURIComponent(id)))); })); rows.appendChild(row); }
  }

  async function renderBranches() {
    const view = $('#view'); view.replaceChildren(); const box = card('会话分支与候选回复', true); const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box); form.append(characterSelector(renderBranches).wrap, sessionSelector(renderBranches).wrap);
    if (!state.characterId || !state.sessionId) { form.appendChild(node('p', 'runtime-muted', '请选择已有会话。')); return; }
    const data = await task('读取分支', () => client.request('POST', '/v1/chat/history', { character_id: state.characterId, session_id: state.sessionId, limit: 200 }));
    const list = node('div', 'runtime-list'); form.appendChild(list);
    (data.messages || []).forEach((item, index) => {
      const id = data.message_ids[index]; const candidates = data.message_candidates[index] || []; const active = data.message_swipe_index[index] || 0; const row = node('div', 'runtime-row'); const copy = node('div', 'runtime-row-main'); copy.append(node('div', 'runtime-row-title', (item.role || 'message') + ' · ' + String(item.content || '').slice(0, 100)), node('div', 'runtime-row-meta', id + ' · parent ' + (data.message_parents[index] || 'root') + (data.active_path.includes(id) ? ' · active' : '')));
      const bar = actions(); if (candidates.length > 1) candidates.forEach((candidate, candidateIndex) => bar.appendChild(button((candidateIndex === active ? '● ' : '') + (candidateIndex + 1) + '/' + candidates.length, async () => { await task('切换候选', () => client.request('POST', '/v1/chat/swipe', { character_id: state.characterId, session_id: state.sessionId, message_id: id, index: candidateIndex })); renderBranches(); })));
      const isLeaf = !(data.message_parents || []).includes(id); if (isLeaf && id !== data.active_leaf) bar.appendChild(button('切到此分支', async () => { await task('切换分支', () => client.request('POST', '/v1/chat/branch/switch', { character_id: state.characterId, session_id: state.sessionId, target_leaf_id: id })); renderBranches(); })); row.append(copy, bar); list.appendChild(row);
    });
  }

  async function renderPreview() {
    const view = $('#view'); view.replaceChildren();
    const box = card('Prompt 装配预览（无模型调用、无写入）', true);
    const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box);
    form.append(characterSelector(renderPreview).wrap, sessionSelector(renderPreview).wrap);
    const prompt = input('模拟用户消息', '测试消息', { multiline: true });
    const result = output('点击预览后显示脱敏装配轨迹。', true);
    form.append(prompt.wrap, button('生成预览', async () => {
      const data = await task('生成装配预览', () => client.request('POST', '/v1/chat/preview', {
        character_id: state.characterId,
        session_id: state.sessionId || null,
        user_id: state.userId,
        user_profile: { name: state.userId, variables: {} },
        message: prompt.control.value,
      }));
      result.textContent = json(data);
    }, 'btn-primary'), result);
  }

  async function renderQuota() {
    const view = $('#view'); view.replaceChildren(); const settings = await client.request('GET', '/v1/settings'); const box = card('每日配额策略', true); const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box); const editor = input('Quota JSON', json(settings.quota || {}), { multiline: true, code: true }); form.append(editor.wrap, node('p', 'runtime-warning', '这是 Engine 的配额策略，不是实时计费账单。保存后影响后续请求。'), button('保存配额策略', () => task('保存配额', () => client.request('POST', '/v1/settings', { quota: parseJson(editor.control.value, 'Quota') })), 'btn-primary'));
  }

  async function renderDiagnostics() {
    const view = $('#view'); view.replaceChildren(); const box = card('Engine 诊断快照', true); view.appendChild(box); const result = {};
    for (const [name, path] of [['version', '/version'], ['health', '/health'], ['settings', '/v1/settings'], ['characters', '/v1/characters'], ['presets', '/v1/presets'], ['scenes', '/v1/scenes'], ['agent_tools', '/v1/agent/tools']]) {
      try { result[name] = { ok: true, data: await client.request('GET', path) }; } catch (error) { result[name] = { ok: false, error: message(error), status: error.status || 0 }; }
    }
    box.appendChild(output(json(result), true));
  }

  async function renderNotes() {
    const view = $('#view'); view.replaceChildren();
    const box = card('本机连接备注', true); const form = node('div', 'runtime-form'); box.appendChild(form); view.appendChild(box);
    const editor = input('只保存在当前浏览器中的备注', localStorage.getItem('airp_console_notes') || '', { multiline: true, code: true });
    form.append(editor.wrap, node('p', 'runtime-muted', '备注不会发送给 Engine，也不会进入角色、会话或备份。'), button('保存本机备注', () => { localStorage.setItem('airp_console_notes', editor.control.value); setStatus('本机备注已保存'); }, 'btn-primary'));
  }

  async function renderUnavailable(kind) {
    const view = $('#view'); view.replaceChildren(); const box = card(kind === 'backup' ? '备份与恢复' : '插件管理', true); box.append(node('div', 'runtime-warning', kind === 'backup' ? '当前 Engine 没有备份/恢复 HTTP API。为避免制造“已备份”的假象，本页不提供不可验证的操作。请先通过文件系统或部署层备份 AIRP 数据目录。' : '当前 Engine 没有插件发现、安装或权限管理 API。本页只声明能力缺口，不伪造插件状态。'), node('p', 'runtime-muted', '后端提供正式契约后，可在此接入并加入 smoke 验收。')); view.appendChild(box);
  }

  async function boot() {
    renderChrome();
    $('#heading-title').textContent = titles[screen] || (screen === 'onboarding' ? '首次连接' : screen === 'backup' ? '备份与恢复' : screen === 'plugins' ? '插件管理' : screen === 'notes' ? '备注与连接' : 'AIRP 控制台');
    try {
      const health = await client.request('GET', '/health'); $('#engine-status').className = 'status-pill ok'; $('#engine-status').lastChild.textContent = health.provider_configured ? 'Engine 与 Provider 就绪' : 'Engine 就绪 · Provider 待配置';
      await loadScope();
      const renderers = { workbench: renderWorkbench, worldbook: renderWorldbook, presets: renderPresets, persona: renderPersona, agent: renderAgent, settings: renderSettings, memory: renderMemory, scenes: renderScenes, branches: renderBranches, preview: renderPreview, quota: renderQuota, diagnostics: renderDiagnostics, backup: () => renderUnavailable('backup'), plugins: () => renderUnavailable('plugins'), notes: renderNotes };
      await (renderers[screen] || renderDiagnostics)();
    } catch (error) {
      $('#engine-status').className = 'status-pill danger'; $('#engine-status').lastChild.textContent = '连接或加载失败'; setStatus(message(error), true);
      const view = $('#view'); if (!view.children.length) { const box = card('无法加载页面', true); box.appendChild(node('p', 'runtime-warning', message(error))); view.append(box, connectionCard()); }
    }
  }
  boot();
})();
