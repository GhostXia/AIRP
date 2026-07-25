(function () {
  'use strict';
  const $ = s => document.querySelector(s);
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  const base = sessionStorage.getItem('airp_engine_url') || location.origin;
  const bearer = sessionStorage.getItem('airp_bearer') || '';
  const client = AIRPApi.createClient({ base, bearer });
  let characterId = params.get('character') || sessionStorage.getItem('airp_character_id') || '';
  let sessionId = params.get('session') || sessionStorage.getItem('airp_session_id') || '';
  let characters = [];
  let sessions = [];

  // ── Chrome ──
  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html'],['41','timeline','时间线导出','41-timeline-export.html'],['42','carddiff','版本对比','42-card-diff.html'],['43','providers','多 Provider 路由','43-provider-management.html']];
  function pathWithState(path) { const url = new URL(path, location.href); if (characterId) url.searchParams.set('character', characterId); if (sessionId) url.searchParams.set('session', sessionId); if (client.base !== location.origin) url.searchParams.set('engine', client.base); return url.href; }
  function renderChrome() {
    $('#engine-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    const nav = $('#console-nav');
    const group = document.createElement('div'); group.className = 'nav-group'; group.textContent = '工作区'; nav.appendChild(group);
    const home = document.createElement('a'); home.className = 'nav-link'; home.textContent = '角色与会话'; home.href = pathWithState('01-role-list.html'); nav.appendChild(home);
    for (const [idx, id, title, href] of pages) { const link = document.createElement('a'); link.className = 'nav-link' + (id === 'imagegen' ? ' active' : ''); link.href = pathWithState(href); const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx; const s2 = document.createElement('span'); s2.textContent = title; link.append(s1, s2); nav.appendChild(link); }
    const related = $('#related-links');
    for (const [label, href] of [['对话空间','02-chat-space.html'],['角色列表','01-role-list.html'],['诊断','23-diagnostics.html']]) { const a = document.createElement('a'); a.className = 'context-link'; a.textContent = label + ' →'; a.href = pathWithState(href); related.appendChild(a); }
  }

  function setStatus(text, isError) {
    const el = $('#runtime-status');
    el.textContent = text;
    el.classList.toggle('error', Boolean(isError));
  }

  function node(tag, className, text) {
    const v = document.createElement(tag);
    if (className) v.className = className;
    if (text !== undefined) v.textContent = text;
    return v;
  }

  // ── 加载角色 / 会话下拉 ──
  async function loadCharacters() {
    try {
      characters = await client.request('GET', '/v1/characters').catch(() => []);
      const sel = $('#image-character');
      sel.replaceChildren();
      for (const id of characters) { const opt = document.createElement('option'); opt.value = id; opt.textContent = id; sel.appendChild(opt); }
      if (!characters.includes(characterId)) characterId = characters[0] || '';
      sel.value = characterId;
      $('#scope-character').textContent = characterId || '未选择';
    } catch (error) {
      setStatus('加载角色失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function loadSessions() {
    const sel = $('#image-session');
    sel.replaceChildren();
    const none = document.createElement('option'); none.value = ''; none.textContent = '— 不绑定 session —'; sel.appendChild(none);
    if (!characterId) { sessions = []; return; }
    try {
      sessions = await client.request('GET', '/v1/sessions/' + encodeURIComponent(characterId)).catch(() => ({ sessions: [] }));
      const list = (sessions && sessions.sessions) || [];
      for (const s of list) { const opt = document.createElement('option'); opt.value = s.session_id; opt.textContent = s.name || s.session_id; sel.appendChild(opt); }
      if (sessionId && list.some(s => s.session_id === sessionId)) sel.value = sessionId; else sessionId = '';
    } catch (error) {
      // 会话列表加载失败不阻塞主流程
    }
  }

  // ── 生成图片 ──
  async function generateImage() {
    if (!characterId) { setStatus('请先选择角色', true); return; }
    const prompt = $('#image-prompt').value.trim();
    if (!prompt) { setStatus('Prompt 不能为空', true); return; }
    const body = {
      character_id: characterId,
      session_id: sessionId || null,
      prompt,
      size: $('#image-size').value,
      style: $('#image-style').value,
      download: $('#image-download').checked,
    };
    const model = $('#image-model').value.trim();
    if (model) body.image_model = model;

    const btn = $('#image-generate-btn');
    btn.disabled = true;
    const original = btn.textContent;
    btn.textContent = '生成中…';
    setStatus('正在调用上游图片生成 API…');
    try {
      const resp = await client.request('POST', '/v1/image/generate', body);
      renderCurrent(resp);
      setStatus(resp.success ? '图片已生成' : '上游 API 未返回图片（可能 prompt 被拒绝）', !resp.success);
      if (body.download) refreshList();
    } catch (error) {
      setStatus('生成失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    } finally {
      btn.disabled = false;
      btn.textContent = original;
    }
  }

  function renderCurrent(resp) {
    const wrap = $('#imagegen-result');
    const container = $('#imagegen-current');
    container.replaceChildren();
    if (!resp) { wrap.hidden = true; return; }
    wrap.hidden = false;

    const img = node('img');
    if (resp.image_path) {
      // 本地资产路径走 Engine 同源
      img.src = new URL(resp.image_path, client.base.replace(/\/$/, '') + '/').href;
    } else if (resp.image_url) {
      img.src = resp.image_url;
    } else {
      img.alt = '无图片';
    }
    container.appendChild(img);

    const meta = node('dl', 'meta');
    const addRow = (k, v) => { const row = node('div'); row.appendChild(node('dt', null, k + ':')); row.appendChild(node('dd', null, v || '—')); meta.appendChild(row); };
    addRow('本地路径', resp.image_path);
    addRow('上游 URL', resp.image_url ? '（已隐藏，已下载）' : '—');
    addRow('revised_prompt', resp.revised_prompt);
    if (resp.meta) {
      addRow('文件名', resp.meta.filename);
      addRow('大小', resp.meta.size);
      addRow('时间戳', String(resp.meta.timestamp));
    }
    container.appendChild(meta);
  }

  // ── 已生成图片列表 ──
  async function refreshList() {
    const container = $('#imagegen-list');
    container.replaceChildren();
    if (!characterId) { container.appendChild(node('div', 'empty', '请先选择角色')); return; }
    container.appendChild(node('div', 'empty', '加载中…'));
    try {
      const path = '/v1/characters/' + encodeURIComponent(characterId) + '/images' + (sessionId ? '?session_id=' + encodeURIComponent(sessionId) : '');
      const list = await client.request('GET', path);
      container.replaceChildren();
      if (!list || list.length === 0) { container.appendChild(node('div', 'empty', '尚无已生成图片')); return; }
      for (const item of list) {
        const card = node('div', 'image-card');
        const img = node('img');
        // 本地资产路径：characters/{id}/sessions/{sid}/images/{filename}
        const imgUrl = new URL('characters/' + encodeURIComponent(characterId) + '/' + (sessionId ? 'sessions/' + encodeURIComponent(sessionId) + '/' : '') + 'images/' + encodeURIComponent(item.filename), client.base.replace(/\/$/, '') + '/').href;
        img.src = imgUrl;
        img.alt = item.prompt || item.filename;
        card.appendChild(img);
        const meta = node('div', 'image-meta');
        meta.appendChild(node('span', 'image-prompt', item.prompt || ''));
        meta.appendChild(node('span', null, new Date(item.timestamp * 1000).toLocaleString()));
        meta.appendChild(node('span', null, item.size));
        card.appendChild(meta);
        container.appendChild(card);
      }
    } catch (error) {
      container.replaceChildren();
      container.appendChild(node('div', 'empty', '加载失败：' + AIRPApi.errorMessage(error.data, error.message)));
    }
  }

  // ── Boot ──
  async function boot() {
    renderChrome();
    try {
      const health = await client.request('GET', '/health');
      $('#engine-status').className = 'status-pill ok';
      $('#engine-status').lastChild.textContent = health.provider_configured ? 'Engine 就绪' : 'Engine 就绪 · Provider 待配置';
    } catch (error) {
      $('#engine-status').className = 'status-pill danger';
      $('#engine-status').lastChild.textContent = '连接失败';
      setStatus('无法连接 Engine：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
    await loadCharacters();
    await loadSessions();
    $('#scope-session').textContent = sessionId || '—';
    $('#scope-user').textContent = sessionStorage.getItem('airp_user_id') || 'default';

    $('#image-character').addEventListener('change', async () => {
      characterId = $('#image-character').value;
      sessionId = '';
      sessionStorage.setItem('airp_character_id', characterId);
      $('#scope-character').textContent = characterId || '未选择';
      $('#scope-session').textContent = '—';
      await loadSessions();
      refreshList();
    });
    $('#image-session').addEventListener('change', () => {
      sessionId = $('#image-session').value;
      sessionStorage.setItem('airp_session_id', sessionId);
      $('#scope-session').textContent = sessionId || '—';
      refreshList();
    });
    $('#image-generate-btn').addEventListener('click', generateImage);
    $('#image-refresh-btn').addEventListener('click', refreshList);

    refreshList();
  }
  boot();
})();
