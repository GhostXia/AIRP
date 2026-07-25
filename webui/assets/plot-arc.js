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
  let arc = null;

  // ── Chrome (复用 console-runtime 导航模式) ──
  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html'],['41','timeline','时间线导出','41-timeline-export.html'],['42','carddiff','版本对比','42-card-diff.html'],['43','providers','多 Provider 路由','43-provider-management.html']];
  function pathWithState(path) { const url = new URL(path, location.href); if (characterId) url.searchParams.set('character', characterId); if (client.base !== location.origin) url.searchParams.set('engine', client.base); return url.href; }
  function renderChrome() {
    $('#engine-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    const nav = $('#console-nav');
    const group = document.createElement('div'); group.className = 'nav-group'; group.textContent = '工作区'; nav.appendChild(group);
    const home = document.createElement('a'); home.className = 'nav-link'; home.textContent = '角色与会话'; home.href = pathWithState('01-role-list.html'); nav.appendChild(home);
    for (const [idx, id, title, href] of pages) { const link = document.createElement('a'); link.className = 'nav-link' + (id === 'plotarc' ? ' active' : ''); link.href = pathWithState(href); const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx; const s2 = document.createElement('span'); s2.textContent = title; link.append(s1, s2); nav.appendChild(link); }
    const related = $('#related-links');
    for (const [label, href] of [['对话空间','02-chat-space.html'],['角色列表','01-role-list.html'],['诊断','23-diagnostics.html']]) { const a = document.createElement('a'); a.className = 'context-link'; a.textContent = label + ' →'; a.href = pathWithState(href); related.appendChild(a); }
  }

  function setStatus(text, isError) {
    const el = $('#runtime-status');
    el.textContent = text;
    el.classList.toggle('error', Boolean(isError));
  }

  // ── 渲染剧情弧 ──
  function renderArc() {
    const container = $('#arc-view');
    container.replaceChildren();
    if (!arc) { container.appendChild(document.createTextNode('加载中…')); return; }

    // Header
    const header = document.createElement('div'); header.className = 'arc-header';
    const titleInput = document.createElement('input');
    titleInput.className = 'input arc-title-input';
    titleInput.value = arc.title || '';
    titleInput.placeholder = '故事标题';
    titleInput.addEventListener('change', () => { arc.title = titleInput.value; });
    const progress = document.createElement('div'); progress.className = 'arc-progress';
    const totalPhases = arc.phases.length;
    const completedPhases = arc.phases.filter(p => p.completed).length;
    const overallPct = totalPhases ? Math.round(completedPhases / totalPhases * 100) : 0;
    const bar = document.createElement('div'); bar.className = 'arc-progress-bar';
    const fill = document.createElement('div'); fill.className = 'arc-progress-fill'; fill.style.width = overallPct + '%';
    bar.appendChild(fill);
    progress.append(bar, document.createTextNode(overallPct + '% · 第 ' + arc.turn_count + ' 轮'));
    header.append(titleInput, progress);
    container.appendChild(header);

    // Phase list
    const list = document.createElement('div'); list.className = 'phase-list';
    arc.phases.forEach((phase, idx) => {
      const card = document.createElement('div');
      card.className = 'phase-card' + (phase.id === arc.current_phase ? ' active' : '') + (phase.completed ? ' completed' : '');
      const head = document.createElement('div'); head.className = 'phase-head';
      const badge = document.createElement('span'); badge.className = 'phase-badge';
      badge.textContent = phase.completed ? '✓ 完成' : phase.id === arc.current_phase ? '▶ 进行中' : '待开始';
      const name = document.createElement('span'); name.className = 'phase-name';
      name.textContent = phase.name + '（' + phase.id + '）';
      head.append(badge, name);
      const desc = document.createElement('div'); desc.className = 'phase-desc';
      desc.textContent = phase.description;
      const meta = document.createElement('div'); meta.className = 'phase-meta';
      meta.textContent = '目标 ' + phase.target_turns + ' 轮';
      if (phase.id === arc.current_phase) {
        const prog = document.createElement('div'); prog.className = 'phase-progress';
        const pbar = document.createElement('div'); pbar.className = 'phase-bar';
        const pfill = document.createElement('div'); pfill.className = 'phase-fill';
        pfill.style.width = Math.round((arc.progress || 0) * 100) + '%';
        pbar.appendChild(pfill);
        prog.append(pbar, document.createTextNode(Math.round((arc.progress || 0) * 100) + '%'));
        meta.appendChild(prog);
      }
      card.append(head, desc, meta);
      list.appendChild(card);
    });
    container.appendChild(list);

    // Save button
    const saveBtn = document.createElement('button');
    saveBtn.className = 'btn btn-primary';
    saveBtn.type = 'button';
    saveBtn.textContent = '保存剧情弧';
    saveBtn.addEventListener('click', saveArc);
    container.appendChild(saveBtn);

    // JSON editor (advanced)
    const editorSection = document.createElement('div'); editorSection.className = 'arc-editor';
    const editorLabel = document.createElement('div'); editorLabel.className = 'runtime-muted';
    editorLabel.textContent = '高级：JSON 整体编辑';
    const textarea = document.createElement('textarea');
    textarea.className = 'textarea runtime-textarea code';
    textarea.rows = 10;
    textarea.value = JSON.stringify(arc, null, 2);
    const jsonSave = document.createElement('button');
    jsonSave.className = 'btn btn-secondary';
    jsonSave.type = 'button';
    jsonSave.textContent = '保存 JSON';
    jsonSave.addEventListener('click', async () => {
      try {
        arc = JSON.parse(textarea.value);
        await saveArc();
      } catch (e) { setStatus('JSON 格式错误：' + e.message, true); }
    });
    editorSection.append(editorLabel, textarea, jsonSave);
    container.appendChild(editorSection);
  }

  async function saveArc() {
    if (!characterId || !arc) return;
    try {
      await client.request('PUT', '/v1/characters/' + encodeURIComponent(characterId) + '/plot-arc', arc);
      setStatus('剧情弧已保存');
      renderArc();
    } catch (error) {
      setStatus('保存失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function loadArc() {
    if (!characterId) { setStatus('请先选择角色', true); return; }
    try {
      arc = await client.request('GET', '/v1/characters/' + encodeURIComponent(characterId) + '/plot-arc');
      setStatus('');
      renderArc();
    } catch (error) {
      setStatus('加载失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  // ── Boot ──
  async function boot() {
    renderChrome();
    try {
      const health = await client.request('GET', '/health');
      $('#engine-status').className = 'status-pill ok';
      $('#engine-status').lastChild.textContent = health.provider_configured ? 'Engine 就绪' : 'Engine 就绪 · Provider 待配置';
      characters = await client.request('GET', '/v1/characters').catch(() => []);
      if (!characters.includes(characterId)) characterId = characters[0] || '';
      if (characterId) sessionStorage.setItem('airp_character_id', characterId);
      $('#scope-character').textContent = characterId || '未选择';
      $('#scope-session').textContent = params.get('session') || '—';
      $('#scope-user').textContent = sessionStorage.getItem('airp_user_id') || 'default';
      const sel = $('#arc-character');
      sel.replaceChildren();
      for (const id of characters) { const opt = document.createElement('option'); opt.value = id; opt.textContent = id; sel.appendChild(opt); }
      sel.value = characterId;
      sel.addEventListener('change', () => { characterId = sel.value; sessionStorage.setItem('airp_character_id', characterId); loadArc(); });
      if (characterId) loadArc();
    } catch (error) {
      $('#engine-status').className = 'status-pill danger';
      $('#engine-status').lastChild.textContent = '连接失败';
      setStatus('无法连接 Engine：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }
  boot();
})();
