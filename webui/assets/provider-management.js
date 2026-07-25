(function () {
  'use strict';
  const $ = s => document.querySelector(s);
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  const base = sessionStorage.getItem('airp_engine_url') || location.origin;
  const bearer = sessionStorage.getItem('airp_bearer') || '';
  const client = AIRPApi.createClient({ base, bearer });

  // 路由 pages 数组与其它 console 页面保持一致，并在末尾追加本页。
  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html'],['41','timeline','时间线导出','41-timeline-export.html'],['42','carddiff','版本对比','42-card-diff.html'],['43','providers','多 Provider 路由','43-provider-management.html']];
  function pathWithState(path) {
    const url = new URL(path, location.href);
    if (client.base !== location.origin) url.searchParams.set('engine', client.base);
    return url.href;
  }
  function renderChrome() {
    $('#engine-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    const nav = $('#console-nav');
    const group = document.createElement('div'); group.className = 'nav-group'; group.textContent = '工作区'; nav.appendChild(group);
    const home = document.createElement('a'); home.className = 'nav-link'; home.textContent = '角色与会话'; home.href = pathWithState('01-role-list.html'); nav.appendChild(home);
    for (const [idx, id, title, href] of pages) {
      const link = document.createElement('a');
      link.className = 'nav-link' + (id === 'providers' ? ' active' : '');
      link.href = pathWithState(href);
      const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx;
      const s2 = document.createElement('span'); s2.textContent = title;
      link.append(s1, s2);
      nav.appendChild(link);
    }
    const related = $('#related-links');
    for (const [label, href] of [['角色列表','01-role-list.html'],['设置','08-settings.html'],['诊断','23-diagnostics.html']]) {
      const a = document.createElement('a'); a.className = 'context-link'; a.textContent = label + ' →'; a.href = pathWithState(href); related.appendChild(a);
    }
  }

  function setStatus(text, isError) {
    const el = $('#runtime-status');
    el.textContent = text;
    el.classList.toggle('error', Boolean(isError));
  }

  // ── 状态 ────────────────────────────────────────────────────────────────
  // 服务端权威状态
  let serverEntries = [];
  let serverRouting = { default_provider: null, by_character: {}, by_scene_role: {}, by_task_kind: {} };
  let serverEnabled = false;
  // 本地编辑缓冲（在用户点击 “保存” 前不会写盘）
  let draftEntries = [];
  let draftRouting = { default_provider: null, by_character: {}, by_scene_role: {}, by_task_kind: {} };
  // 编辑器中正在修改的 entry 索引；-1 表示新增
  let editingIndex = -1;

  function cloneDraftFromServer() {
    draftEntries = serverEntries.map(e => ({...e, api_key: ''}));
    draftRouting = JSON.parse(JSON.stringify(serverRouting));
  }

  function providerNames() {
    return draftEntries.map(e => e.name);
  }

  function providerSelectOptions(selected) {
    return providerNames()
      .map(n => `<option value="${escapeAttr(n)}"${n === selected ? ' selected' : ''}>${escapeText(n)}</option>`)
      .join('');
  }

  // ── 工具：HTML 转义 ────────────────────────────────────────────────────
  function escapeText(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }
  function escapeAttr(s) { return escapeText(s); }

  // ── 渲染：Provider 列表 ────────────────────────────────────────────────
  function renderEntries() {
    const tbody = $('#pm-rows');
    tbody.innerHTML = '';
    const tpl = $('#pm-row-template');
    $('#pm-count').textContent = String(draftEntries.length);

    const enabledBar = $('#pm-enabled-bar');
    const enabledTag = $('#pm-enabled-tag');
    const enabledHint = $('#pm-enabled-hint');
    if (draftEntries.length > 0) {
      enabledBar.classList.add('is-enabled');
      enabledTag.textContent = '已启用';
      enabledHint.textContent = '多 provider 路由已生效；空 RouteContext 会走 default 路径。';
    } else {
      enabledBar.classList.remove('is-enabled');
      enabledTag.textContent = '未启用';
      enabledHint.textContent = 'provider 数组为空，daemon 走 legacy 单 provider 路径';
    }

    if (draftEntries.length === 0) {
      $('#pm-table').hidden = true;
      $('#pm-empty').hidden = false;
      return;
    }
    $('#pm-table').hidden = false;
    $('#pm-empty').hidden = true;

    for (let i = 0; i < draftEntries.length; i++) {
      const e = draftEntries[i];
      const row = tpl.content.firstElementChild.cloneNode(true);
      row.dataset.index = String(i);
      row.querySelector('.pm-cell-name').textContent = e.name;
      const endpointCode = row.querySelector('.pm-cell-endpoint code');
      endpointCode.textContent = e.endpoint;
      row.querySelector('.pm-cell-model').textContent = e.model;
      row.querySelector('.pm-cell-engine').textContent = e.engine;
      const defaultCell = row.querySelector('.pm-cell-default');
      defaultCell.classList.toggle('is-not-default', !e.is_default);
      const keyCell = row.querySelector('.pm-cell-key');
      const hasKey = Boolean(e.api_key); // draftEntries 中 api_key='' 表示“未修改/未设置”
      keyCell.classList.toggle('has-key', hasKey);
      // 显示 server 端真实状态而非 draft 中的空字符串
      if (i < serverEntries.length && serverEntries[i].name === e.name) {
        keyCell.classList.toggle('has-key', Boolean(serverEntries[i].api_key));
      }
      row.querySelector('.pm-edit').addEventListener('click', () => openEditor(i));
      row.querySelector('.pm-delete').addEventListener('click', () => deleteEntry(i));
      tbody.appendChild(row);
    }
  }

  // ── 渲染：路由策略 ──────────────────────────────────────────────────────
  function renderRouting() {
    // default_provider 下拉
    const dpSelect = $('#pm-default-provider');
    const currentDefault = draftRouting.default_provider;
    dpSelect.innerHTML = '<option value="">(无)</option>' + providerSelectOptions(currentDefault || '');

    // 三个映射的下拉
    for (const [selId, listId, mapKey] of [
      ['#pm-bc-value', '#pm-bc-list', 'by_character'],
      ['#pm-bsr-value', '#pm-bsr-list', 'by_scene_role'],
      ['#pm-btk-value', '#pm-btk-list', 'by_task_kind'],
    ]) {
      const sel = $(selId);
      sel.innerHTML = providerSelectOptions('');
      // 清空 key 输入
      const keyInput = sel.parentElement.querySelector('input');
      if (keyInput) keyInput.value = '';
      // 渲染已有映射
      const list = $(listId);
      list.innerHTML = '';
      const map = draftRouting[mapKey] || {};
      for (const [k, v] of Object.entries(map)) {
        const li = document.createElement('li');
        const isDirty = !serverRouting[mapKey] || serverRouting[mapKey][k] !== v;
        if (isDirty) li.classList.add('is-dirty');
        const pair = document.createElement('span');
        pair.className = 'pm-routing-pair';
        pair.innerHTML = `<b>${escapeText(k)}</b> → ${escapeText(v)}`;
        const btn = document.createElement('button');
        btn.className = 'btn btn-secondary pm-routing-remove';
        btn.type = 'button';
        btn.textContent = '移除';
        btn.addEventListener('click', () => {
          delete draftRouting[mapKey][k];
          renderRouting();
        });
        li.append(pair, btn);
        list.appendChild(li);
      }
    }
  }

  // ── 编辑器：新增 / 编辑 Provider ───────────────────────────────────────
  function openEditor(index) {
    editingIndex = index;
    const dlg = $('#pm-editor');
    const title = $('#pm-editor-title');
    const hint = $('#pm-editor-hint');
    if (index === -1) {
      title.textContent = '新增 Provider';
      $('#pm-ed-name').value = '';
      $('#pm-ed-endpoint').value = '';
      $('#pm-ed-model').value = '';
      $('#pm-ed-engine').value = 'direct';
      $('#pm-ed-default').checked = draftEntries.length === 0; // 第一个默认勾选
      $('#pm-ed-apikey').value = '';
      hint.textContent = '';
    } else {
      const e = draftEntries[index];
      title.textContent = '编辑 Provider: ' + e.name;
      $('#pm-ed-name').value = e.name;
      $('#pm-ed-endpoint').value = e.endpoint;
      $('#pm-ed-model').value = e.model;
      $('#pm-ed-engine').value = e.engine;
      $('#pm-ed-default').checked = e.is_default;
      $('#pm-ed-apikey').value = '';
      // 提示当前 key 是否已在服务端设置
      const serverEntry = serverEntries.find(s => s.name === e.name);
      if (serverEntry && serverEntry.api_key) {
        hint.textContent = '已存在 api_key；留空表示保持不变，输入新值会覆盖。';
      } else {
        hint.textContent = '尚未设置 api_key；输入新值会写入。';
      }
    }
    dlg.showModal();
  }

  function closeEditor() {
    const dlg = $('#pm-editor');
    if (dlg.open) dlg.close();
    editingIndex = -1;
  }

  function applyEditor() {
    const name = $('#pm-ed-name').value.trim();
    const endpoint = $('#pm-ed-endpoint').value.trim();
    const model = $('#pm-ed-model').value.trim();
    const engine = $('#pm-ed-engine').value;
    const isDefault = $('#pm-ed-default').checked;
    const apiKey = $('#pm-ed-apikey').value;

    if (!name) { setStatus('name 不能为空', true); return; }
    if (!endpoint) { setStatus('endpoint 不能为空', true); return; }
    if (!model) { setStatus('model 不能为空', true); return; }

    // 检查重名（排除自身）
    for (let i = 0; i < draftEntries.length; i++) {
      if (i !== editingIndex && draftEntries[i].name === name) {
        setStatus(`Provider name "${name}" 已存在`, true);
        return;
      }
    }

    if (editingIndex === -1) {
      // 新增
      draftEntries.push({
        name, endpoint, model, engine, is_default: isDefault,
        api_key: apiKey || null,
      });
    } else {
      const prev = draftEntries[editingIndex];
      // api_key 留空表示不修改：保留原值（如有）
      const preservedKey = apiKey ? apiKey : (prev.api_key || null);
      draftEntries[editingIndex] = {
        name, endpoint, model, engine, is_default: isDefault,
        api_key: preservedKey,
      };
    }

    // is_default 互斥：勾选了当前 entry 时，其它 entry 的 is_default 自动取消
    if (isDefault) {
      for (let i = 0; i < draftEntries.length; i++) {
        if (i !== editingIndex && i !== draftEntries.length - 1 && editingIndex === -1) {
          draftEntries[i].is_default = false;
        } else if (i !== editingIndex && editingIndex >= 0) {
          draftEntries[i].is_default = false;
        }
      }
    }
    // 没有 is_default 时，自动把第一个设为 default（保存时后端会校验）
    if (!draftEntries.some(e => e.is_default) && draftEntries.length > 0) {
      draftEntries[0].is_default = true;
    }

    closeEditor();
    renderEntries();
    renderRouting();
    setStatus('Provider 列表已修改，点击下方 “保存” 提交到服务端。', false);
  }

  function deleteEntry(index) {
    const e = draftEntries[index];
    if (!confirm(`确认删除 Provider "${e.name}"？相关路由规则也会一并失效。`)) return;
    const removedName = e.name;
    draftEntries.splice(index, 1);
    // 清理引用该 name 的路由规则
    for (const mapKey of ['by_character', 'by_scene_role', 'by_task_kind']) {
      for (const k of Object.keys(draftRouting[mapKey] || {})) {
        if (draftRouting[mapKey][k] === removedName) {
          delete draftRouting[mapKey][k];
        }
      }
    }
    if (draftRouting.default_provider === removedName) {
      draftRouting.default_provider = null;
    }
    // 没有 is_default 时，把第一个设为 default
    if (!draftEntries.some(x => x.is_default) && draftEntries.length > 0) {
      draftEntries[0].is_default = true;
    }
    renderEntries();
    renderRouting();
    setStatus(`已删除 "${removedName}"，点击 “保存” 提交变更。`, false);
  }

  // ── 路由策略编辑：新增映射 ─────────────────────────────────────────────
  function addRoutingEntry(mapKey, keyInputId, valueSelectId) {
    const key = $(keyInputId).value.trim();
    const value = $(valueSelectId).value;
    if (!key) { setStatus('映射键不能为空', true); return; }
    if (!value) { setStatus('请选择目标 provider', true); return; }
    if (!draftRouting[mapKey]) draftRouting[mapKey] = {};
    draftRouting[mapKey][key] = value;
    $(keyInputId).value = '';
    $(valueSelectId).value = '';
    renderRouting();
    setStatus(`已添加路由规则：${key} → ${value}，点击 “保存路由策略” 提交。`, false);
  }

  // ── 持久化：保存全部 providers + routing ────────────────────────────────
  async function saveAllProviders() {
    if (draftEntries.length === 0) {
      if (!confirm('provider 数组将为空，将禁用多 provider 路由并回退到 legacy 单 provider。继续？')) return;
    }
    // 校验：至少一个 is_default
    if (draftEntries.length > 0 && !draftEntries.some(e => e.is_default)) {
      setStatus('providers 非空时至少必须有一个 entry 的 is_default = true', true);
      return;
    }
    setStatus('正在保存 providers...', false);
    try {
      // 构造请求体；api_key 字段空字符串视为未设置
      const entriesPayload = draftEntries.map(e => ({
        name: e.name,
        endpoint: e.endpoint,
        model: e.model,
        engine: e.engine,
        is_default: e.is_default,
        // 后端 ProviderEntry.api_key 是 Option<String>，序列化时 serde skip 不会读，
        // 但反序列化时如果 JSON 中没有该字段就为 None。这里始终带上。
        api_key: e.api_key || '',
      }));
      const body = {
        entries: entriesPayload,
        routing: draftRouting,
      };
      const resp = await client.request('POST', '/v1/providers', body);
      applyServerState(resp);
      renderEntries();
      renderRouting();
      setStatus('Providers 已保存。', false);
    } catch (err) {
      setStatus('保存失败: ' + (err && err.message ? err.message : String(err)), true);
    }
  }

  // ── 持久化：仅保存路由策略 ─────────────────────────────────────────────
  async function saveRoutingOnly() {
    if (serverEntries.length === 0) {
      setStatus('providers 数组为空，无法保存 routing（请先添加 provider）', true);
      return;
    }
    setStatus('正在保存路由策略...', false);
    try {
      const resp = await client.request('PUT', '/v1/provider-routing', { routing: draftRouting });
      serverRouting = resp;
      draftRouting = JSON.parse(JSON.stringify(serverRouting));
      renderRouting();
      setStatus('路由策略已保存。', false);
    } catch (err) {
      setStatus('保存路由策略失败: ' + (err && err.message ? err.message : String(err)), true);
    }
  }

  function applyServerState(resp) {
    serverEntries = (resp.entries || []).map(e => ({
      name: e.name,
      endpoint: e.endpoint,
      model: e.model,
      engine: e.engine,
      is_default: e.is_default,
      // 服务端只返回 api_key_set，不返回 key 本体
      // 为了让 draftEntries.api_key 字段有意义，我们用占位符表示“已设置”
      api_key: e.api_key_set ? '__server_set__' : null,
    }));
    serverRouting = resp.routing || { default_provider: null, by_character: {}, by_scene_role: {}, by_task_kind: {} };
    serverEnabled = Boolean(resp.enabled);
    cloneDraftFromServer();
    // draft 中 api_key 用空字符串替代占位符，让编辑器逻辑能识别“未修改”
    draftEntries.forEach(e => { e.api_key = ''; });
  }

  // ── 加载 ──────────────────────────────────────────────────────────────
  async function loadAll() {
    setStatus('正在加载 provider 配置...', false);
    try {
      const resp = await client.request('GET', '/v1/providers');
      applyServerState(resp);
      renderEntries();
      renderRouting();
      setStatus('已加载 provider 配置。', false);
    } catch (err) {
      setStatus('加载失败: ' + (err && err.message ? err.message : String(err)), true);
    }
  }

  // ── 路由解析测试 ───────────────────────────────────────────────────────
  async function runResolve() {
    const characterId = $('#pm-rs-char').value.trim();
    const sceneRole = $('#pm-rs-role').value.trim();
    const taskKind = $('#pm-rs-task').value.trim();
    const qs = new URLSearchParams();
    if (characterId) qs.set('character_id', characterId);
    if (sceneRole) qs.set('scene_role', sceneRole);
    if (taskKind) qs.set('task_kind', taskKind);
    const result = $('#pm-rs-result');
    const status = $('#pm-rs-status');
    const rule = $('#pm-rs-rule');
    const entry = $('#pm-rs-entry');
    result.hidden = false;
    status.textContent = '解析中...';
    status.className = 'pm-resolve-status';
    rule.textContent = '';
    entry.textContent = '';
    try {
      const resp = await client.request('GET', '/v1/providers/resolve?' + qs.toString());
      if (resp.matched) {
        status.textContent = '命中';
        status.classList.add('is-matched');
        rule.textContent = 'rule=' + (resp.matched_rule || '?');
        const e = resp.entry;
        entry.textContent = `→ ${e.name} (model=${e.model}, endpoint=${e.endpoint})`;
      } else {
        status.textContent = '未命中';
        status.classList.add('is-unmatched');
        rule.textContent = 'no provider matched';
        entry.textContent = '（请检查 provider 列表或 routing 配置）';
      }
    } catch (err) {
      status.textContent = '错误';
      status.classList.add('is-unmatched');
      rule.textContent = '';
      entry.textContent = (err && err.message ? err.message : String(err));
    }
  }

  // ── 引擎状态轮询 ───────────────────────────────────────────────────────
  async function pollEngineStatus() {
    try {
      const health = await client.request('GET', '/health');
      const status = $('#engine-status');
      const ok = health && health.ok !== false;
      status.innerHTML = '<i class="dot"></i>' + (ok ? 'Engine 在线' : 'Engine 异常');
      status.classList.toggle('ok', ok);
      status.classList.toggle('error', !ok);
    } catch {
      const status = $('#engine-status');
      status.innerHTML = '<i class="dot"></i>Engine 离线';
      status.classList.remove('ok');
      status.classList.add('error');
    }
  }

  // ── 绑定事件 ──────────────────────────────────────────────────────────
  function bindEvents() {
    $('#pm-add').addEventListener('click', () => openEditor(-1));
    $('#pm-reload').addEventListener('click', loadAll);
    $('#pm-ed-cancel').addEventListener('click', (ev) => { ev.preventDefault(); closeEditor(); });
    $('#pm-editor-form').addEventListener('submit', (ev) => {
      // form method=dialog 会自动 close；这里先 applyEditor，如果校验失败则阻止
      ev.preventDefault();
      applyEditor();
    });
    $('#pm-ed-save').addEventListener('click', (ev) => {
      ev.preventDefault();
      applyEditor();
    });

    $('#pm-bc-add').addEventListener('click', () => addRoutingEntry('by_character', '#pm-bc-key', '#pm-bc-value'));
    $('#pm-bsr-add').addEventListener('click', () => addRoutingEntry('by_scene_role', '#pm-bsr-key', '#pm-bsr-value'));
    $('#pm-btk-add').addEventListener('click', () => addRoutingEntry('by_task_kind', '#pm-btk-key', '#pm-btk-value'));

    $('#pm-save-routing').addEventListener('click', saveRoutingOnly);
    $('#pm-reset-routing').addEventListener('click', () => {
      cloneDraftFromServer();
      draftEntries.forEach(e => { e.api_key = ''; });
      renderEntries();
      renderRouting();
      setStatus('已放弃本地修改。', false);
    });

    $('#pm-rs-run').addEventListener('click', runResolve);

    // 提供一个隐式的 “保存全部 providers” 入口：
    // 当用户修改了 provider 列表（新增/编辑/删除）后，必须显式保存。
    // 把 “+ 新增 Provider” 之后的操作都视为 draft，按 “保存 Provider 列表” 按钮提交。
    // 这里没有单独的 “保存 Provider 列表” 按钮 —— 改为在编辑/删除后即时提示，
    // 并复用 “保存路由策略” 的位置加一个 “保存全部” 按钮。
    // 为了不破坏现有 DOM，我们在运行时注入按钮。
    const saveAllBtn = document.createElement('button');
    saveAllBtn.className = 'btn btn-primary';
    saveAllBtn.type = 'button';
    saveAllBtn.id = 'pm-save-all';
    saveAllBtn.textContent = '保存 Provider 列表';
    saveAllBtn.addEventListener('click', saveAllProviders);
    // 插入到 “+ 新增 Provider” 之后
    const sectionActions = document.querySelector('.pm-section-actions');
    if (sectionActions) sectionActions.appendChild(saveAllBtn);

    // ESC 关闭 dialog 时也要重置 editingIndex
    $('#pm-editor').addEventListener('close', () => { editingIndex = -1; });
  }

  // ── 启动 ──────────────────────────────────────────────────────────────
  function start() {
    renderChrome();
    bindEvents();
    pollEngineStatus();
    setInterval(pollEngineStatus, 5000);
    loadAll();
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', start);
  } else {
    start();
  }
})();
