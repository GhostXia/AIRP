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
  let characters = [];

  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html'],['41','timeline','时间线导出','41-timeline-export.html'],['42','carddiff','版本对比','42-card-diff.html']];
  function pathWithState(path) {
    const url = new URL(path, location.href);
    if (characterId) url.searchParams.set('character', characterId);
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
      link.className = 'nav-link' + (id === 'stylelearn' ? ' active' : '');
      link.href = pathWithState(href);
      const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx;
      const s2 = document.createElement('span'); s2.textContent = title;
      link.append(s1, s2);
      nav.appendChild(link);
    }
    const related = $('#related-links');
    for (const [label, href] of [['风格审查','32-style-review.html'],['角色列表','01-role-list.html'],['诊断','23-diagnostics.html']]) {
      const a = document.createElement('a'); a.className = 'context-link'; a.textContent = label + ' →'; a.href = pathWithState(href); related.appendChild(a);
    }
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

  // ── 加载角色下拉 ──
  async function loadCharacters() {
    const sel = $('#learn-character');
    sel.replaceChildren();
    sel.appendChild(new Option('不绑定角色', ''));
    try {
      characters = await client.request('GET', '/v1/characters');
      for (const id of characters) sel.appendChild(new Option(id, id));
      if (characterId && characters.includes(characterId)) sel.value = characterId;
    } catch (error) {
      // 加载失败时保留"不绑定角色"占位项，通过 status bar 提示用户
      characters = [];
      setStatus('加载角色失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  // ── 字符计数 ──
  function updateCharCount() {
    const text = $('#learn-text').value;
    $('#learn-char-count').textContent = String([...text].length);
  }

  // ── 提交风格学习 ──
  async function submitLearn() {
    const text = $('#learn-text').value;
    const charCount = [...text].length;
    if (charCount < 100) { setStatus('文本样本过短（至少 100 字符，当前 ' + charCount + '）', true); return; }
    if (charCount > 20000) { setStatus('文本样本过长（最多 20000 字符，当前 ' + charCount + '）', true); return; }

    const profileId = $('#learn-profile-id').value.trim() || 'default';
    const characterIdSelected = $('#learn-character').value;
    const body = { text, profile_id: profileId };
    if (characterIdSelected) body.character_id = characterIdSelected;

    const btn = $('#learn-submit');
    btn.disabled = true;
    const original = btn.textContent;
    btn.textContent = '提取中…';
    setStatus('正在调用 LLM 提取风格特征…');
    try {
      const resp = await client.request('POST', '/v1/style/learn', body);
      renderResult(resp);
      setStatus('Profile 已写入：' + resp.profile_path + '（' + resp.features_count + ' 条特征）');
      refreshProfiles();
    } catch (error) {
      setStatus('学习失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    } finally {
      btn.disabled = false;
      btn.textContent = original;
    }
  }

  function renderResult(resp) {
    const wrap = $('#learn-result');
    const meta = $('#learn-result-meta');
    const content = $('#learn-result-content');
    meta.replaceChildren();
    content.textContent = '';
    wrap.hidden = false;
    meta.appendChild(node('span', null, 'Profile 路径：' + (resp.profile_path || '—')));
    meta.appendChild(node('span', null, ' · 特征条目数：' + resp.features_count));
    content.textContent = resp.profile_content || '（无内容）';
  }

  // ── 清空表单 ──
  function clearForm() {
    $('#learn-text').value = '';
    $('#learn-profile-id').value = '';
    $('#learn-character').value = '';
    updateCharCount();
    $('#learn-result').hidden = true;
    $('#learn-result-content').textContent = '';
    setStatus('');
  }

  // ── Profile 列表 ──
  async function refreshProfiles() {
    const container = $('#profiles-list');
    container.replaceChildren();
    container.appendChild(node('div', 'empty', '加载中…'));
    try {
      const list = await client.request('GET', '/v1/style/profiles');
      container.replaceChildren();
      if (!list || list.length === 0) { container.appendChild(node('div', 'empty', '尚无已学习的 profile')); return; }
      for (const item of list) {
        const card = node('div', 'profile-card');
        card.addEventListener('click', () => showProfile(item.profile_id));
        const left = node('div');
        left.appendChild(node('div', 'profile-id', item.profile_id));
        left.appendChild(node('div', 'profile-meta', '大小：' + (item.size_bytes || 0) + ' bytes'));
        card.appendChild(left);
        const right = node('div', 'profile-meta');
        const ts = item.modified_timestamp ? new Date(item.modified_timestamp * 1000).toLocaleString() : '—';
        right.appendChild(node('span', null, '修改于 ' + ts));
        card.appendChild(right);
        container.appendChild(card);
      }
    } catch (error) {
      container.replaceChildren();
      container.appendChild(node('div', 'empty', '加载失败：' + AIRPApi.errorMessage(error.data, error.message)));
    }
  }

  async function showProfile(profileId) {
    const wrap = $('#profile-detail');
    const title = $('#profile-detail-title');
    const content = $('#profile-detail-content');
    title.textContent = 'Profile 详情：' + profileId;
    content.textContent = '加载中…';
    wrap.hidden = false;
    try {
      const resp = await client.request('GET', '/v1/style/profiles/' + encodeURIComponent(profileId));
      content.textContent = resp.content || '（无内容）';
    } catch (error) {
      content.textContent = '加载失败：' + AIRPApi.errorMessage(error.data, error.message);
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
      return;
    }
    await loadCharacters();
    $('#scope-character').textContent = characterId || '—';
    $('#scope-session').textContent = '—';
    $('#scope-user').textContent = sessionStorage.getItem('airp_user_id') || 'default';

    $('#learn-text').addEventListener('input', updateCharCount);
    $('#learn-submit').addEventListener('click', submitLearn);
    $('#learn-clear').addEventListener('click', clearForm);
    $('#profiles-refresh').addEventListener('click', refreshProfiles);
    $('#profile-detail-close').addEventListener('click', () => { $('#profile-detail').hidden = true; });

    refreshProfiles();
  }
  boot();
})();
