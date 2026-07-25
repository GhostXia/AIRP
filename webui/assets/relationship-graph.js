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

  // ── Chrome ──
  const pages = [['03','workbench','角色工作台','03-workbench.html'],['04','worldbook','世界书','04-world-book.html'],['17','memory','记忆与状态','17-memory-state.html'],['18','scenes','多人场景','18-group-chat.html'],['32','style','风格系统','32-style-review.html'],['34','graph','关系图谱','34-relationship-graph.html'],['35','plotarc','剧情弧','35-plot-arc.html'],['36','imagegen','图片生成','36-image-gen.html'],['37','templates','模板库','37-character-templates.html'],['38','stylelearn','风格迁移','38-style-learn.html'],['39','dialoguegen','对话示例','39-dialogue-gen.html'],['40','wbgraph','知识图谱','40-worldbook-graph.html']];
  function pathWithState(path) { const url = new URL(path, location.href); if (characterId) url.searchParams.set('character', characterId); if (client.base !== location.origin) url.searchParams.set('engine', client.base); return url.href; }
  function renderChrome() {
    $('#engine-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    const nav = $('#console-nav');
    const group = document.createElement('div'); group.className = 'nav-group'; group.textContent = '工作区'; nav.appendChild(group);
    const home = document.createElement('a'); home.className = 'nav-link'; home.textContent = '角色与会话'; home.href = pathWithState('01-role-list.html'); nav.appendChild(home);
    for (const [idx, id, title, href] of pages) { const link = document.createElement('a'); link.className = 'nav-link' + (id === 'graph' ? ' active' : ''); link.href = pathWithState(href); const spanIdx = document.createElement('span'); spanIdx.className = 'nav-index'; spanIdx.textContent = idx; const spanTitle = document.createElement('span'); spanTitle.textContent = title; link.append(spanIdx, spanTitle); nav.appendChild(link); }
    const related = $('#related-links');
    for (const [label, href] of [['对话空间','02-chat-space.html'],['角色列表','01-role-list.html'],['诊断','23-diagnostics.html']]) { const a = document.createElement('a'); a.className = 'context-link'; a.textContent = label + ' →'; a.href = pathWithState(href); related.appendChild(a); }
  }

  // ── Force-directed graph ──
  const canvas = $('#graph-canvas');
  const ctx = canvas.getContext('2d');
  let nodes = []; // { id, x, y, vx, vy, radius }
  let edges = []; // { source, target, type, intensity }
  let dragging = null;
  let animFrame = null;
  let simRunning = false;

  function resizeCanvas() {
    const box = $('#graph-box');
    canvas.width = box.clientWidth * (window.devicePixelRatio || 1);
    canvas.height = box.clientHeight * (window.devicePixelRatio || 1);
    ctx.setTransform(window.devicePixelRatio || 1, 0, 0, window.devicePixelRatio || 1, 0, 0);
  }

  function initGraph(relData) {
    const nodeSet = new Set();
    const parsedEdges = [];
    for (const [key, val] of Object.entries(relData)) {
      const parts = key.split('->');
      if (parts.length !== 2) continue;
      const [from, to] = parts;
      nodeSet.add(from); nodeSet.add(to);
      parsedEdges.push({ source: from, target: to, type: (val && val.type) || 'neutral', intensity: (val && val.intensity) || 0.5 });
    }
    if (!nodeSet.size) { nodes = []; edges = []; return; }
    const w = canvas.width / (window.devicePixelRatio || 1);
    const h = canvas.height / (window.devicePixelRatio || 1);
    const cx = w / 2, cy = h / 2;
    const ids = Array.from(nodeSet);
    nodes = ids.map((id, i) => {
      const angle = (2 * Math.PI * i) / ids.length;
      const r = Math.min(w, h) * 0.3;
      return { id, x: cx + r * Math.cos(angle) + (Math.random() - 0.5) * 20, y: cy + r * Math.sin(angle) + (Math.random() - 0.5) * 20, vx: 0, vy: 0, radius: 22 };
    });
    edges = parsedEdges;
    $('#graph-empty').hidden = true;
    simRunning = true;
    if (!animFrame) tick();
  }

  const COLORS = { primary: '#6366f1', edge: '#e67e22', text: '#1e293b' };
  const TYPE_COLORS = { friend: '#22c55e', enemy: '#ef4444', family: '#3b82f6', lover: '#ec4899', rival: '#f59e0b', neutral: '#94a3b8' };

  function simulate() {
    const w = canvas.width / (window.devicePixelRatio || 1);
    const h = canvas.height / (window.devicePixelRatio || 1);
    const k = 0.01;
    const repulsion = 3000;
    const damping = 0.85;
    const centerPull = 0.002;
    for (let i = 0; i < nodes.length; i++) {
      for (let j = i + 1; j < nodes.length; j++) {
        let dx = nodes[j].x - nodes[i].x;
        let dy = nodes[j].y - nodes[i].y;
        let dist = Math.sqrt(dx * dx + dy * dy) || 1;
        let force = repulsion / (dist * dist);
        let fx = (dx / dist) * force;
        let fy = (dy / dist) * force;
        nodes[i].vx -= fx; nodes[i].vy -= fy;
        nodes[j].vx += fx; nodes[j].vy += fy;
      }
    }
    const nodeMap = Object.fromEntries(nodes.map(n => [n.id, n]));
    for (const e of edges) {
      const s = nodeMap[e.source], t = nodeMap[e.target];
      if (!s || !t) continue;
      let dx = t.x - s.x, dy = t.y - s.y;
      let dist = Math.sqrt(dx * dx + dy * dy) || 1;
      let idealLen = 120 + (1 - e.intensity) * 80;
      let force = (dist - idealLen) * k;
      let fx = (dx / dist) * force, fy = (dy / dist) * force;
      s.vx += fx; s.vy += fy;
      t.vx -= fx; t.vy -= fy;
    }
    let totalV = 0;
    for (const n of nodes) {
      n.vx += (w / 2 - n.x) * centerPull;
      n.vy += (h / 2 - n.y) * centerPull;
      n.vx *= damping; n.vy *= damping;
      if (n === dragging) continue;
      n.x += n.vx; n.y += n.vy;
      n.x = Math.max(n.radius, Math.min(w - n.radius, n.x));
      n.y = Math.max(n.radius, Math.min(h - n.radius, n.y));
      totalV += Math.abs(n.vx) + Math.abs(n.vy);
    }
    if (totalV < 0.5 && !dragging) simRunning = false;
  }

  function draw() {
    const w = canvas.width / (window.devicePixelRatio || 1);
    const h = canvas.height / (window.devicePixelRatio || 1);
    ctx.clearRect(0, 0, w, h);
    const nodeMap = Object.fromEntries(nodes.map(n => [n.id, n]));
    for (const e of edges) {
      const s = nodeMap[e.source], t = nodeMap[e.target];
      if (!s || !t) continue;
      ctx.beginPath();
      ctx.moveTo(s.x, s.y);
      ctx.lineTo(t.x, t.y);
      ctx.strokeStyle = TYPE_COLORS[e.type] || COLORS.edge;
      ctx.lineWidth = 1 + e.intensity * 3;
      ctx.globalAlpha = 0.5 + e.intensity * 0.4;
      ctx.stroke();
      ctx.globalAlpha = 1;
      const mx = (s.x + t.x) / 2, my = (s.y + t.y) / 2;
      ctx.font = '9px sans-serif';
      ctx.fillStyle = '#64748b';
      ctx.textAlign = 'center';
      ctx.fillText(e.type + ' (' + e.intensity.toFixed(1) + ')', mx, my - 4);
    }
    for (const n of nodes) {
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.radius, 0, Math.PI * 2);
      ctx.fillStyle = COLORS.primary;
      ctx.fill();
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 2;
      ctx.stroke();
      ctx.font = '11px sans-serif';
      ctx.fillStyle = COLORS.text;
      ctx.textAlign = 'center';
      ctx.fillText(n.id.length > 8 ? n.id.slice(0, 7) + '…' : n.id, n.x, n.y + n.radius + 14);
      ctx.font = 'bold 12px sans-serif';
      ctx.fillStyle = '#fff';
      ctx.fillText(n.id.slice(0, 1).toUpperCase(), n.x, n.y + 4);
    }
  }

  function tick() {
    if (simRunning) simulate();
    draw();
    animFrame = requestAnimationFrame(tick);
  }

  // Drag interaction
  function getMousePos(e) {
    const rect = canvas.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  }
  canvas.addEventListener('mousedown', e => {
    const pos = getMousePos(e);
    for (const n of nodes) {
      const dx = pos.x - n.x, dy = pos.y - n.y;
      if (dx * dx + dy * dy < n.radius * n.radius) { dragging = n; simRunning = true; break; }
    }
  });
  canvas.addEventListener('mousemove', e => {
    if (!dragging) return;
    const pos = getMousePos(e);
    dragging.x = pos.x; dragging.y = pos.y;
    dragging.vx = 0; dragging.vy = 0;
  });
  canvas.addEventListener('mouseup', () => { dragging = null; });
  canvas.addEventListener('mouseleave', () => { dragging = null; });
  window.addEventListener('resize', () => { resizeCanvas(); simRunning = true; });

  // ── Data loading ──
  async function loadGraph() {
    if (!characterId) { $('#graph-info').textContent = '请先选择角色'; return; }
    $('#graph-info').textContent = '加载中…';
    try {
      const stateData = await client.request('GET', '/v1/characters/' + encodeURIComponent(characterId) + '/state');
      const rel = stateData && stateData.relationships;
      if (!rel || typeof rel !== 'object' || !Object.keys(rel).length) {
        nodes = []; edges = [];
        $('#graph-empty').hidden = false;
        $('#graph-info').textContent = '该角色没有关系数据（state.relationships 为空）';
        return;
      }
      resizeCanvas();
      initGraph(rel);
      $('#graph-info').textContent = Object.keys(rel).length + ' 条关系 · ' + nodes.length + ' 个节点';
    } catch (error) {
      $('#graph-info').textContent = '加载失败：' + AIRPApi.errorMessage(error.data, error.message);
    }
  }

  // ── Boot ──
  async function boot() {
    renderChrome();
    resizeCanvas();
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
      const sel = $('#graph-character');
      sel.replaceChildren();
      for (const id of characters) { const opt = document.createElement('option'); opt.value = id; opt.textContent = id; sel.appendChild(opt); }
      sel.value = characterId;
      sel.addEventListener('change', () => { characterId = sel.value; sessionStorage.setItem('airp_character_id', characterId); loadGraph(); });
      $('#graph-refresh').addEventListener('click', loadGraph);
      if (characterId) loadGraph();
    } catch (error) {
      $('#engine-status').className = 'status-pill danger';
      $('#engine-status').lastChild.textContent = '连接失败';
      $('#graph-info').textContent = '无法连接 Engine：' + AIRPApi.errorMessage(error.data, error.message);
    }
  }
  boot();
})();
