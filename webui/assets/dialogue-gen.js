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
  // 缓存最近一次 dry_run 生成的对话示例，供"写入角色卡"按钮使用
  let lastGenerated = '';

  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html']];
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
      link.className = 'nav-link' + (id === 'dialoguegen' ? ' active' : '');
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

  async function loadCharacters() {
    const sel = $('#gen-character');
    sel.replaceChildren();
    sel.appendChild(new Option('— 请选择角色 —', ''));
    try {
      characters = await client.request('GET', '/v1/characters').catch(() => []);
      for (const id of characters) sel.appendChild(new Option(id, id));
      if (characterId && characters.includes(characterId)) {
        sel.value = characterId;
        loadCurrentMesExample();
      }
    } catch (error) {
      setStatus('加载角色失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function loadCurrentMesExample() {
    const wrap = $('#gen-current');
    const content = $('#gen-current-content');
    if (!characterId) { wrap.hidden = true; return; }
    try {
      const card = await client.request('GET', '/v1/characters/' + encodeURIComponent(characterId) + '/card');
      const data = card.data || card;
      const mes = data.mes_example || '';
      content.textContent = mes || '（角色卡尚未设置 mes_example）';
      wrap.hidden = false;
    } catch (error) {
      content.textContent = '加载失败：' + AIRPApi.errorMessage(error.data, error.message);
      wrap.hidden = false;
    }
  }

  async function submitGenerate() {
    if (!characterId) { setStatus('请先选择角色', true); return; }
    const turns = parseInt($('#gen-turns').value, 10);
    const stance = $('#gen-stance').value.trim();
    const scenario = $('#gen-scenario').value.trim();
    const userName = $('#gen-username').value.trim();
    const dryRun = $('#gen-dry-run').checked;
    const append = $('#gen-append').checked;

    const body = { turns, dry_run: dryRun, append };
    if (stance) body.user_stance = stance;
    if (scenario) body.scenario_override = scenario;
    if (userName) body.user_name = userName;

    const btn = $('#gen-submit');
    btn.disabled = true;
    const original = btn.textContent;
    btn.textContent = '生成中…';
    setStatus('正在调用 LLM 生成对话示例…');
    try {
      const resp = await client.request('POST', '/v1/characters/' + encodeURIComponent(characterId) + '/dialogue-examples', body);
      renderResult(resp);
      lastGenerated = resp.mes_example || '';
      if (resp.written) {
        setStatus('已写入角色卡 mes_example（' + resp.turns_generated + ' 轮）');
        $('#gen-write').disabled = true;
        loadCurrentMesExample();
      } else {
        setStatus('生成完成（' + resp.turns_generated + ' 轮，预览模式未写盘）');
        $('#gen-write').disabled = false;
      }
    } catch (error) {
      setStatus('生成失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    } finally {
      btn.disabled = false;
      btn.textContent = original;
    }
  }

  function renderResult(resp) {
    const wrap = $('#gen-result');
    const meta = $('#gen-result-meta');
    const content = $('#gen-result-content');
    meta.replaceChildren();
    content.textContent = '';
    wrap.hidden = false;
    meta.appendChild(node('span', null, '角色：' + resp.character_id));
    meta.appendChild(node('span', null, '轮数：' + resp.turns_generated));
    meta.appendChild(node('span', null, resp.written ? '已写入角色卡' : '预览模式（未写入）'));
    if (resp.previous_mes_example !== null && resp.previous_mes_example !== undefined) {
      meta.appendChild(node('span', null, '旧值已备份（' + resp.previous_mes_example.length + ' 字符）'));
    }
    content.textContent = resp.mes_example || '（无内容）';
  }

  async function writeGenerated() {
    if (!characterId || !lastGenerated) { setStatus('无可写入的生成内容', true); return; }
    const append = $('#gen-append').checked;
    const turns = parseInt($('#gen-turns').value, 10);
    const body = { turns, dry_run: false, append };
    const stance = $('#gen-stance').value.trim();
    const scenario = $('#gen-scenario').value.trim();
    const userName = $('#gen-username').value.trim();
    if (stance) body.user_stance = stance;
    if (scenario) body.scenario_override = scenario;
    if (userName) body.user_name = userName;

    const btn = $('#gen-write');
    btn.disabled = true;
    const original = btn.textContent;
    btn.textContent = '写入中…';
    setStatus('正在写入角色卡 mes_example…');
    try {
      const resp = await client.request('POST', '/v1/characters/' + encodeURIComponent(characterId) + '/dialogue-examples', body);
      renderResult(resp);
      setStatus('已写入角色卡（' + resp.turns_generated + ' 轮）');
      loadCurrentMesExample();
    } catch (error) {
      setStatus('写入失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    } finally {
      btn.disabled = false;
      btn.textContent = original;
    }
  }

  function clearForm() {
    $('#gen-stance').value = '';
    $('#gen-scenario').value = '';
    $('#gen-username').value = '';
    $('#gen-dry-run').checked = true;
    $('#gen-append').checked = false;
    $('#gen-turns').value = '3';
    $('#gen-result').hidden = true;
    $('#gen-result-content').textContent = '';
    $('#gen-write').disabled = true;
    lastGenerated = '';
    setStatus('');
  }

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
    $('#scope-user').textContent = sessionStorage.getItem('airp_user_id') || 'default';

    $('#gen-character').addEventListener('change', () => {
      characterId = $('#gen-character').value;
      sessionStorage.setItem('airp_character_id', characterId);
      $('#scope-character').textContent = characterId || '—';
      loadCurrentMesExample();
    });
    $('#gen-submit').addEventListener('click', submitGenerate);
    $('#gen-clear').addEventListener('click', clearForm);
    $('#gen-write').addEventListener('click', writeGenerated);
    $('#gen-dry-run').addEventListener('change', () => {
      // 切换 dry_run 时重置 write 按钮可用性
      if ($('#gen-dry-run').checked) {
        $('#gen-write').disabled = !lastGenerated;
      } else {
        // 非 dry_run 模式：提交即写入，write 按钮无意义
        $('#gen-write').disabled = true;
      }
    });
  }
  boot();
})();
