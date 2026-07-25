(function () {
  'use strict';
  const $ = s => document.querySelector(s);
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  const base = sessionStorage.getItem('airp_engine_url') || location.origin;
  // CodeRabbit #7：仅同源 Engine 接收 stored bearer。跨源 engine（可能由
  // ?engine= 查询参数注入）不带 bearer，防止钓鱼链接窃取会话令牌。跨源
  // engine 用户须在该 engine 域名单独登录。
  const storedBearer = sessionStorage.getItem('airp_bearer') || '';
  const bearer = (base === location.origin) ? storedBearer : '';
  const client = AIRPApi.createClient({ base, bearer });
  let characterId = params.get('character') || sessionStorage.getItem('airp_character_id') || '';
  let templates = [];
  let selectedTemplate = null;
  // CodeRabbit outside-diff #1：showDetail 请求序号，防旧响应覆盖新选择。
  // 两次快速 showDetail 调用可能 out-of-order resolve；用递增 token 标识
  // 最新请求，stale 响应（成功或失败）一律忽略，避免 selectedTemplate 被旧
  // 响应设成错误的 id 导致 instantiate 创建到非用户当前选择的模板。
  let detailRequestId = 0;

  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html'],['41','timeline','时间线导出','41-timeline-export.html']];
  function pathWithState(path) { const url = new URL(path, location.href); if (characterId) url.searchParams.set('character', characterId); if (client.base !== location.origin) url.searchParams.set('engine', client.base); return url.href; }
  function renderChrome() {
    $('#engine-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    const nav = $('#console-nav');
    const group = document.createElement('div'); group.className = 'nav-group'; group.textContent = '工作区'; nav.appendChild(group);
    const home = document.createElement('a'); home.className = 'nav-link'; home.textContent = '角色与会话'; home.href = pathWithState('01-role-list.html'); nav.appendChild(home);
    for (const [idx, id, title, href] of pages) { const link = document.createElement('a'); link.className = 'nav-link' + (id === 'templates' ? ' active' : ''); link.href = pathWithState(href); const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx; const s2 = document.createElement('span'); s2.textContent = title; link.append(s1, s2); nav.appendChild(link); }
    const related = $('#related-links');
    for (const [label, href] of [['角色列表','01-role-list.html'],['对话空间','02-chat-space.html'],['诊断','23-diagnostics.html']]) { const a = document.createElement('a'); a.className = 'context-link'; a.textContent = label + ' →'; a.href = pathWithState(href); related.appendChild(a); }
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

  function renderGrid() {
    const grid = $('#template-grid');
    grid.replaceChildren();
    const filterCat = $('#template-category-filter').value;
    const q = $('#template-search').value.trim().toLowerCase();
    const filtered = templates.filter(t => {
      if (filterCat && t.category !== filterCat) return false;
      if (q && !(t.name.toLowerCase().includes(q) || t.description.toLowerCase().includes(q))) return false;
      return true;
    });
    if (filtered.length === 0) { grid.appendChild(node('div', 'empty', '没有匹配的模板')); return; }
    for (const t of filtered) {
      // CodeRabbit #8：改 <button type="button"> 使模板卡键盘可达（Tab + Enter）。
      const card = node('button', 'template-card');
      card.type = 'button';
      card.addEventListener('click', () => showDetail(t.id));
      const head = node('div', 'tc-head');
      head.appendChild(node('span', 'tc-name', t.name));
      head.appendChild(node('span', 'tc-category', t.category));
      card.appendChild(head);
      card.appendChild(node('div', 'tc-desc', t.description));
      const tags = node('div', 'tc-tags');
      for (const tag of (t.tags || [])) tags.appendChild(node('span', 'tc-tag', tag));
      card.appendChild(tags);
      grid.appendChild(card);
    }
  }

  function populateCategoryFilter() {
    const sel = $('#template-category-filter');
    const cats = Array.from(new Set(templates.map(t => t.category))).sort();
    sel.replaceChildren();
    sel.appendChild(new Option('全部', ''));
    for (const c of cats) sel.appendChild(new Option(c, c));
  }

  async function showDetail(id) {
    const wrap = $('#template-detail');
    const body = $('#detail-body');
    body.replaceChildren();
    wrap.hidden = false;
    // CodeRabbit #6：await 前先清 selectedTemplate，加载失败时不保留上次选择，
    // 避免用户误用旧 id 触发 instantiate。
    selectedTemplate = null;
    // CodeRabbit outside-diff #1：本调用的请求 token。await 期间若用户又点了
    // 别的模板，detailRequestId 会再 ++，本 token 即过期；resolve 后比对，
    // 不匹配则丢弃响应。
    const requestId = ++detailRequestId;
    setStatus('加载模板详情…');
    try {
      const card = await client.request('GET', '/v1/character-templates/' + encodeURIComponent(id));
      if (requestId !== detailRequestId) return; // stale，丢弃
      selectedTemplate = id;
      $('#detail-title').textContent = card.data.name + '（' + card.data.extensions.airp_template_category + '）';
      body.appendChild(node('h3', null, '描述'));
      body.appendChild(node('p', null, card.data.description));
      body.appendChild(node('h3', null, '人格'));
      body.appendChild(node('p', null, card.data.personality));
      body.appendChild(node('h3', null, '场景'));
      body.appendChild(node('p', null, card.data.scenario));
      body.appendChild(node('h3', null, '开场白'));
      const pre1 = node('pre'); pre1.textContent = card.data.first_mes; body.appendChild(pre1);
      body.appendChild(node('h3', null, '对话示例'));
      const pre2 = node('pre'); pre2.textContent = card.data.mes_example; body.appendChild(pre2);
      $('#detail-name-override').value = '';
      $('#detail-char-id').value = '';
      setStatus('');
    } catch (error) {
      if (requestId !== detailRequestId) return; // stale，丢弃
      setStatus('加载失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function instantiate() {
    if (!selectedTemplate) { setStatus('请先选择模板', true); return; }
    const body = {};
    const id = $('#detail-char-id').value.trim();
    const name = $('#detail-name-override').value.trim();
    if (id) body.character_id = id;
    if (name) body.name_override = name;
    const btn = $('#detail-instantiate');
    btn.disabled = true;
    const original = btn.textContent;
    btn.textContent = '创建中…';
    try {
      const resp = await client.request('POST', '/v1/character-templates/' + encodeURIComponent(selectedTemplate) + '/instantiate', body);
      setStatus('已创建角色：' + resp.character_id + '（格式：' + resp.card_format + '）');
      // 跳转到角色列表，便于查看新角色
      setTimeout(() => { location.href = pathWithState('01-role-list.html'); }, 1200);
    } catch (error) {
      setStatus('创建失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    } finally {
      btn.disabled = false;
      btn.textContent = original;
    }
  }

  async function boot() {
    renderChrome();
    $('#scope-character').textContent = characterId || '—';
    $('#scope-session').textContent = '—';
    $('#scope-user').textContent = sessionStorage.getItem('airp_user_id') || 'default';
    try {
      const health = await client.request('GET', '/health');
      $('#engine-status').className = 'status-pill ok';
      $('#engine-status').lastChild.textContent = health.provider_configured ? 'Engine 就绪' : 'Engine 就绪 · Provider 待配置';
    } catch (error) {
      $('#engine-status').className = 'status-pill danger';
      $('#engine-status').lastChild.textContent = '连接失败';
      setStatus('无法连接 Engine：' + AIRPApi.errorMessage(error.data, error.message), true);
      return;
    }
    try {
      templates = await client.request('GET', '/v1/character-templates');
      populateCategoryFilter();
      renderGrid();
    } catch (error) {
      setStatus('加载模板列表失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
    $('#template-category-filter').addEventListener('change', renderGrid);
    $('#template-search').addEventListener('input', renderGrid);
    $('#detail-instantiate').addEventListener('click', instantiate);
    $('#detail-close').addEventListener('click', () => { $('#template-detail').hidden = true; selectedTemplate = null; });
  }
  boot();
})();
