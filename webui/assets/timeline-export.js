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
  let sessionId = params.get('session') || '';
  let characters = [];
  let sessions = [];
  let currentTimeline = null;
  // 缓存最近一次拿到的导出文本（Markdown / HTML），供预览复用
  let cachedMarkdown = '';
  let cachedHtml = '';

  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html'],['41','timeline','时间线导出','41-timeline-export.html'],['42','carddiff','版本对比','42-card-diff.html'],['43','providers','多 Provider 路由','43-provider-management.html']];
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
      link.className = 'nav-link' + (id === 'timeline' ? ' active' : '');
      link.href = pathWithState(href);
      const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx;
      const s2 = document.createElement('span'); s2.textContent = title;
      link.append(s1, s2);
      nav.appendChild(link);
    }
    const related = $('#related-links');
    for (const [label, href] of [['角色列表','01-role-list.html'],['剧情弧','35-plot-arc.html'],['诊断','23-diagnostics.html']]) {
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

  async function loadCharacters() {
    const sel = $('#tl-character');
    sel.replaceChildren();
    sel.appendChild(new Option('— 请选择角色 —', ''));
    try {
      characters = await client.request('GET', '/v1/characters');
      for (const id of characters) sel.appendChild(new Option(id, id));
      if (characterId && characters.includes(characterId)) {
        sel.value = characterId;
        await loadSessions();
      }
    } catch (error) {
      // 加载失败时保留"— 请选择角色 —"占位项，通过 status bar 提示用户
      characters = [];
      setStatus('加载角色失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function loadSessions() {
    const sel = $('#tl-session');
    sel.replaceChildren();
    sel.appendChild(new Option('— 请选择会话 —', ''));
    if (!characterId) return;
    try {
      sessions = await client.request('GET', '/v1/sessions/' + encodeURIComponent(characterId));
      for (const s of sessions) {
        const sid = s.session_id || s.id || s;
        const label = s.name ? (sid + ' · ' + s.name) : sid;
        sel.appendChild(new Option(label, sid));
      }
      if (sessionId && sessions.some(s => (s.session_id || s.id || s) === sessionId)) {
        sel.value = sessionId;
      } else if (sessions.length === 1) {
        sessionId = sessions[0].session_id || sessions[0].id || sessions[0];
        sel.value = sessionId;
      }
    } catch (error) {
      // 加载失败时保留"— 请选择会话 —"占位项，通过 status bar 提示用户
      sessions = [];
      setStatus('加载会话失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function loadTimeline() {
    if (!characterId) { setStatus('请先选择角色', true); return; }
    if (!sessionId) { setStatus('请先选择会话', true); return; }
    setStatus('加载中...', false);
    try {
      const tl = await client.request('GET', '/v1/sessions/' + encodeURIComponent(characterId) + '/' + encodeURIComponent(sessionId) + '/timeline');
      currentTimeline = tl;
      cachedMarkdown = '';
      cachedHtml = '';
      renderTimeline(tl);
      setStatus('已加载', false);
    } catch (error) {
      setStatus('加载时间线失败：' + AIRPApi.errorMessage(error.data, error.message), true);
      hideTimeline();
    }
  }

  function hideTimeline() {
    $('#tl-meta').hidden = true;
    $('#tl-stats').hidden = true;
    $('#tl-actions').hidden = true;
    $('#tl-entries').hidden = true;
    $('#tl-pending').hidden = true;
    $('#tl-preview').hidden = true;
  }

  function renderTimeline(tl) {
    // 元数据
    const metaList = $('#tl-meta-list');
    metaList.replaceChildren();
    const metaItems = [
      ['角色 ID', tl.character_id, true],
      ['Session ID', tl.session_id, true],
      ['Session 创建时间', tl.session_created_at, false],
      ['Session 更新时间', tl.session_updated_at, false],
      ['导出生成时间', tl.generated_at, false],
    ];
    if (tl.character && tl.character.name) metaItems.unshift(['角色名', tl.character.name, false]);
    if (tl.world_clock) metaItems.push(['世界时钟', tl.world_clock.display + ' (' + tl.world_clock.time_unit + ')', false]);
    for (const [k, v, isCode] of metaItems) {
      metaList.appendChild(node('dt', null, k));
      const dd = node('dd');
      if (isCode) {
        const code = node('code', null, v);
        dd.appendChild(code);
      } else {
        dd.textContent = v;
      }
      metaList.appendChild(dd);
    }
    $('#tl-meta').hidden = false;

    // 统计
    $('#tl-stat-messages').textContent = tl.message_count || 0;
    $('#tl-stat-triggered').textContent = tl.triggered_event_count || 0;
    $('#tl-stat-entries').textContent = (tl.entries || []).length;
    $('#tl-stats').hidden = false;

    // 操作按钮
    $('#tl-actions').hidden = false;

    // 条目列表
    const list = $('#tl-entry-list');
    list.replaceChildren();
    if (!tl.entries || tl.entries.length === 0) {
      const li = node('li', null, '（暂无消息）');
      list.appendChild(li);
    } else {
      for (const entry of tl.entries) {
        list.appendChild(renderEntry(entry));
      }
    }
    $('#tl-entries').hidden = false;

    // 附录：未触发事件
    const pendingList = $('#tl-pending-list');
    pendingList.replaceChildren();
    if (tl.pending_events && tl.pending_events.length > 0) {
      for (const e of tl.pending_events) {
        const li = node('li');
        const name = node('strong', null, e.name);
        const tt = node('span', 'tt', e.time_trigger != null ? ' · T+' + e.time_trigger : '');
        const desc = document.createTextNode(': ' + (e.description || ''));
        li.append(name, tt, desc);
        pendingList.appendChild(li);
      }
      $('#tl-pending').hidden = false;
    } else {
      $('#tl-pending').hidden = true;
    }

    // 隐藏预览
    $('#tl-preview').hidden = true;
  }

  function renderEntry(entry) {
    if (entry.kind === 'chat_message') {
      const cls = entry.role === 'user' ? 'msg-user' : entry.role === 'assistant' ? 'msg-assistant' : entry.role === 'system' ? 'msg-system' : 'msg-other';
      const li = node('li', cls);
      const meta = node('div', 'tl-item-meta');
      const ts = node('span', null, entry.ts || '(无时间戳)');
      const roleLabel = entry.role === 'user' ? '用户' : entry.role === 'assistant' ? '角色' : entry.role === 'system' ? '系统' : entry.role;
      const role = node('span', 'tl-item-role', roleLabel);
      meta.append(ts, role);
      const content = node('div', 'tl-item-content', entry.content || '');
      li.append(meta, content);
      return li;
    } else if (entry.kind === 'world_event') {
      const li = node('li', 'msg-event');
      const meta = node('div', 'tl-item-meta');
      const ts = node('span', null, entry.time_trigger != null ? '世界事件 · T+' + entry.time_trigger : '世界事件');
      const role = node('span', 'tl-item-role', '🌐 ' + (entry.name || ''));
      meta.append(ts, role);
      const desc = entry.description ? node('div', 'tl-item-desc', entry.description) : null;
      const content = node('div', 'tl-item-content', entry.content || '');
      if (desc) li.append(meta, desc, content); else li.append(meta, content);
      return li;
    }
    return node('li', null, '（未知条目类型）');
  }

  async function fetchExport(format) {
    if (!characterId || !sessionId) { setStatus('请先选择角色与会话', true); return null; }
    setStatus('导出中...', false);
    try {
      const url = client.base + '/v1/sessions/' + encodeURIComponent(characterId) + '/' + encodeURIComponent(sessionId) + '/timeline/export?format=' + encodeURIComponent(format);
      const headers = {};
      if (bearer) headers.Authorization = 'Bearer ' + bearer;
      const resp = await fetch(url, { headers });
      if (!resp.ok) {
        const text = await resp.text();
        throw new Error('导出失败 (' + resp.status + '): ' + text);
      }
      return resp;
    } catch (error) {
      setStatus('导出失败：' + error.message, true);
      return null;
    }
  }

  function triggerDownload(blob, filename) {
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  async function exportAs(format) {
    const resp = await fetchExport(format);
    if (!resp) return;
    try {
      const blob = await resp.blob();
      // 从 Content-Disposition 提取文件名
      let filename = 'timeline.' + format;
      const cd = resp.headers.get('Content-Disposition') || '';
      const matchStar = cd.match(/filename\*=UTF-8''([^;]+)/);
      const matchPlain = cd.match(/filename="([^"]+)"/);
      if (matchStar) {
        try { filename = decodeURIComponent(matchStar[1]); } catch { filename = matchStar[1]; }
      } else if (matchPlain) {
        filename = matchPlain[1];
      }
      triggerDownload(blob, filename);
      setStatus('已下载 ' + filename, false);
    } catch (error) {
      setStatus('保存文件失败：' + error.message, true);
    }
  }

  async function previewMarkdown() {
    if (!currentTimeline) { setStatus('请先加载时间线', true); return; }
    setStatus('生成 Markdown 预览...', false);
    if (!cachedMarkdown) {
      const resp = await fetchExport('markdown');
      if (!resp) return;
      cachedMarkdown = await resp.text();
    }
    $('#tl-preview-title').textContent = 'Markdown 预览';
    $('#tl-preview-md-content').textContent = cachedMarkdown;
    $('#tl-preview-md-content').hidden = false;
    $('#tl-preview-html-frame').hidden = true;
    $('#tl-preview').hidden = false;
    setStatus('已加载 Markdown 预览', false);
  }

  async function previewHtml() {
    if (!currentTimeline) { setStatus('请先加载时间线', true); return; }
    setStatus('生成 HTML 预览...', false);
    if (!cachedHtml) {
      const resp = await fetchExport('html');
      if (!resp) return;
      cachedHtml = await resp.text();
    }
    $('#tl-preview-title').textContent = 'HTML 预览（可通过浏览器打印为 PDF）';
    const frame = $('#tl-preview-html-frame');
    frame.hidden = false;
    $('#tl-preview-md-content').hidden = true;
    $('#tl-preview').hidden = false;
    // 用 srcdoc 注入完整 HTML 文档
    frame.srcdoc = cachedHtml;
    setStatus('已加载 HTML 预览', false);
  }

  function closePreview() {
    $('#tl-preview').hidden = true;
  }

  function bind() {
    $('#tl-character').addEventListener('change', async () => {
      characterId = $('#tl-character').value;
      sessionStorage.setItem('airp_character_id', characterId);
      $('#scope-character').textContent = characterId || '—';
      sessionId = '';
      hideTimeline();
      await loadSessions();
    });
    $('#tl-session').addEventListener('change', () => {
      sessionId = $('#tl-session').value;
      hideTimeline();
    });
    $('#tl-load').addEventListener('click', loadTimeline);
    $('#tl-export-json').addEventListener('click', () => exportAs('json'));
    $('#tl-export-md').addEventListener('click', () => exportAs('markdown'));
    $('#tl-export-html').addEventListener('click', () => exportAs('html'));
    $('#tl-preview-md').addEventListener('click', previewMarkdown);
    $('#tl-preview-html').addEventListener('click', previewHtml);
    $('#tl-preview-close').addEventListener('click', closePreview);
  }

  // R1: 转为 boot() 模式以与其它屏幕对齐——既填充 #scope-character/#scope-user，
  //     也通过 /health 检查更新 #engine-status（HTML 中的"正在连接"占位此前永不更新）。
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
    bind();
    await loadCharacters();
    $('#scope-character').textContent = characterId || '—';
    $('#scope-user').textContent = sessionStorage.getItem('airp_user_id') || 'default';
  }

  boot();
})();
