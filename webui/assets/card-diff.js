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
  let revA = '';
  let revB = '';
  let characters = [];
  let currentDiff = null;
  // 缓存最近一次拿到的导出文本（Markdown / HTML），供预览复用
  let cachedMarkdown = '';
  let cachedHtml = '';

  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html'],['41','timeline','时间线导出','41-timeline-export.html'],['42','carddiff','版本对比','42-card-diff.html'],['43','providers','多 Provider 路由','43-provider-management.html'],['44','plugintools','插件工具','44-plugin-tools.html']];
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
      link.className = 'nav-link' + (id === 'carddiff' ? ' active' : '');
      link.href = pathWithState(href);
      const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx;
      const s2 = document.createElement('span'); s2.textContent = title;
      link.append(s1, s2);
      nav.appendChild(link);
    }
    const related = $('#related-links');
    for (const [label, href] of [['角色列表','01-role-list.html'],['模板库','37-character-templates.html'],['诊断','23-diagnostics.html']]) {
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

  function shortHash(hash) {
    if (!hash) return '—';
    return hash.length > 16 ? hash.slice(0, 16) + '...' : hash;
  }

  function formatJson(value) {
    try {
      return JSON.stringify(value, null, 2);
    } catch {
      return String(value);
    }
  }

  async function loadCharacters() {
    const sel = $('#cd-character');
    sel.replaceChildren();
    sel.appendChild(new Option('— 请选择角色 —', ''));
    try {
      characters = await client.request('GET', '/v1/characters');
      for (const id of characters) sel.appendChild(new Option(id, id));
      if (characterId && characters.includes(characterId)) {
        sel.value = characterId;
        await loadRevisions();
      }
    } catch (error) {
      // 加载失败时保留"— 请选择角色 —"占位项，并通过 status bar 提示用户
      characters = [];
      setStatus('加载角色失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function loadRevisions() {
    const selA = $('#cd-rev-a');
    const selB = $('#cd-rev-b');
    selA.replaceChildren();
    selB.replaceChildren();
    selA.appendChild(new Option('— 请选择 —', ''));
    selB.appendChild(new Option('— 请选择 —', ''));
    if (!characterId) return;
    hideDiff();
    try {
      const resp = await client.request('GET', '/v1/characters/' + encodeURIComponent(characterId) + '/revisions');
      const revisions = (resp && resp.revisions) || [];
      if (revisions.length === 0) {
        setStatus('该角色暂无 revision（请先 commit 一个版本）', false);
        return;
      }
      for (const r of revisions) {
        const label = 'rev ' + r;
        selA.appendChild(new Option(label, String(r)));
        selB.appendChild(new Option(label, String(r)));
      }
      // 默认选最旧 vs 最新
      if (revisions.length >= 2) {
        selA.value = String(revisions[0]);
        selB.value = String(revisions[revisions.length - 1]);
        revA = selA.value;
        revB = selB.value;
      }
      setStatus('已加载 ' + revisions.length + ' 个 revision', false);
    } catch (error) {
      setStatus('加载 revisions 失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function compareRevisions() {
    if (!characterId) { setStatus('请先选择角色', true); return; }
    revA = $('#cd-rev-a').value;
    revB = $('#cd-rev-b').value;
    if (!revA || !revB) { setStatus('请选择两个 revision', true); return; }
    if (revA === revB) { setStatus('请选择不同的 revision', true); return; }
    setStatus('对比中...', false);
    try {
      const url = '/v1/characters/' + encodeURIComponent(characterId) + '/revisions/diff?rev_a=' + encodeURIComponent(revA) + '&rev_b=' + encodeURIComponent(revB) + '&format=json';
      const diff = await client.request('GET', url);
      currentDiff = diff;
      cachedMarkdown = '';
      cachedHtml = '';
      renderDiff(diff);
      setStatus('已对比', false);
    } catch (error) {
      setStatus('对比失败：' + AIRPApi.errorMessage(error.data, error.message), true);
      hideDiff();
    }
  }

  function hideDiff() {
    $('#cd-meta').hidden = true;
    $('#cd-stats').hidden = true;
    $('#cd-actions').hidden = true;
    $('#cd-changes').hidden = true;
    $('#cd-preview').hidden = true;
  }

  function renderDiff(diff) {
    // 元数据
    const sa = diff.snapshot_a || {};
    const sb = diff.snapshot_b || {};
    $('#cd-meta-a-head').textContent = 'rev ' + (sa.revision ?? diff.revision_a);
    $('#cd-meta-b-head').textContent = 'rev ' + (sb.revision ?? diff.revision_b);
    $('#cd-meta-a-rev').textContent = sa.revision ?? diff.revision_a ?? '—';
    $('#cd-meta-b-rev').textContent = sb.revision ?? diff.revision_b ?? '—';
    $('#cd-meta-a-time').textContent = sa.created_at || '—';
    $('#cd-meta-b-time').textContent = sb.created_at || '—';
    $('#cd-meta-a-hash').replaceChildren(node('code', null, shortHash(sa.tree_sha256)));
    $('#cd-meta-b-hash').replaceChildren(node('code', null, shortHash(sb.tree_sha256)));
    $('#cd-meta').hidden = false;

    // 统计
    $('#cd-stat-added').textContent = diff.added_count || 0;
    $('#cd-stat-removed').textContent = diff.removed_count || 0;
    $('#cd-stat-changed').textContent = diff.changed_count || 0;
    $('#cd-stats').hidden = false;

    // 操作按钮
    $('#cd-actions').hidden = false;

    // 变更明细
    const list = $('#cd-change-list');
    list.replaceChildren();
    if (!diff.changes || diff.changes.length === 0) {
      const li = node('li', 'cd-empty', '（两个 revision 的 card.json 内容完全相同）');
      list.appendChild(li);
    } else {
      for (const change of diff.changes) {
        list.appendChild(renderChange(change));
      }
    }
    $('#cd-changes').hidden = false;

    // 隐藏预览
    $('#cd-preview').hidden = true;
  }

  function renderChange(change) {
    if (change.op === 'added') {
      const li = node('li', 'cd-added');
      const meta = node('div', 'cd-change-meta');
      meta.append(
        node('span', 'cd-op cd-op-added', '🟢 Added'),
        node('code', 'cd-path', change.path),
      );
      const value = node('pre', 'cd-value', formatJson(change.value));
      li.append(meta, value);
      return li;
    } else if (change.op === 'removed') {
      const li = node('li', 'cd-removed');
      const meta = node('div', 'cd-change-meta');
      meta.append(
        node('span', 'cd-op cd-op-removed', '🔴 Removed'),
        node('code', 'cd-path', change.path),
      );
      const value = node('pre', 'cd-value', formatJson(change.value));
      li.append(meta, value);
      return li;
    } else if (change.op === 'changed') {
      const li = node('li', 'cd-changed');
      const meta = node('div', 'cd-change-meta');
      meta.append(
        node('span', 'cd-op cd-op-changed', '🟡 Changed'),
        node('code', 'cd-path', change.path),
      );
      const pair = node('div', 'cd-value-pair');
      const oldBox = node('div', 'cd-value-old');
      oldBox.append(node('span', 'cd-value-label', 'old'), node('pre', 'cd-value', formatJson(change.old_value)));
      const newBox = node('div', 'cd-value-new');
      newBox.append(node('span', 'cd-value-label', 'new'), node('pre', 'cd-value', formatJson(change.new_value)));
      pair.append(oldBox, newBox);
      li.append(meta, pair);
      return li;
    }
    return node('li', null, '（未知变更类型）');
  }

  async function fetchExport(format) {
    if (!characterId || !revA || !revB) { setStatus('请先选择角色与两个 revision', true); return null; }
    setStatus('导出中...', false);
    try {
      const url = client.base + '/v1/characters/' + encodeURIComponent(characterId) + '/revisions/diff?rev_a=' + encodeURIComponent(revA) + '&rev_b=' + encodeURIComponent(revB) + '&format=' + encodeURIComponent(format);
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
      let filename = 'card-diff.' + format;
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
    if (!currentDiff) { setStatus('请先对比版本', true); return; }
    setStatus('生成 Markdown 预览...', false);
    if (!cachedMarkdown) {
      const resp = await fetchExport('markdown');
      if (!resp) return;
      cachedMarkdown = await resp.text();
    }
    $('#cd-preview-title').textContent = 'Markdown 预览';
    $('#cd-preview-md-content').textContent = cachedMarkdown;
    $('#cd-preview-md-content').hidden = false;
    $('#cd-preview-html-frame').hidden = true;
    $('#cd-preview').hidden = false;
    setStatus('已加载 Markdown 预览', false);
  }

  async function previewHtml() {
    if (!currentDiff) { setStatus('请先对比版本', true); return; }
    setStatus('生成 HTML 预览...', false);
    if (!cachedHtml) {
      const resp = await fetchExport('html');
      if (!resp) return;
      cachedHtml = await resp.text();
    }
    $('#cd-preview-title').textContent = 'HTML 预览（可通过浏览器打印为 PDF）';
    const frame = $('#cd-preview-html-frame');
    frame.hidden = false;
    $('#cd-preview-md-content').hidden = true;
    $('#cd-preview').hidden = false;
    // 用 srcdoc 注入完整 HTML 文档
    frame.srcdoc = cachedHtml;
    setStatus('已加载 HTML 预览', false);
  }

  function closePreview() {
    $('#cd-preview').hidden = true;
  }

  function bind() {
    $('#cd-character').addEventListener('change', async () => {
      characterId = $('#cd-character').value;
      sessionStorage.setItem('airp_character_id', characterId);
      $('#scope-character').textContent = characterId || '—';
      revA = '';
      revB = '';
      hideDiff();
      await loadRevisions();
    });
    $('#cd-rev-a').addEventListener('change', () => {
      revA = $('#cd-rev-a').value;
      hideDiff();
    });
    $('#cd-rev-b').addEventListener('change', () => {
      revB = $('#cd-rev-b').value;
      hideDiff();
    });
    $('#cd-compare').addEventListener('click', compareRevisions);
    $('#cd-export-json').addEventListener('click', () => exportAs('json'));
    $('#cd-export-md').addEventListener('click', () => exportAs('markdown'));
    $('#cd-export-html').addEventListener('click', () => exportAs('html'));
    $('#cd-preview-md').addEventListener('click', previewMarkdown);
    $('#cd-preview-html').addEventListener('click', previewHtml);
    $('#cd-preview-close').addEventListener('click', closePreview);
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
