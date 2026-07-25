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
  let graphData = null;
  // 力导向布局节点状态
  let simNodes = [];
  let simEdges = [];
  let dragNode = null;
  let hoverNode = null;
  let selectedNode = null;

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
      link.className = 'nav-link' + (id === 'wbgraph' ? ' active' : '');
      link.href = pathWithState(href);
      const s1 = document.createElement('span'); s1.className = 'nav-index'; s1.textContent = idx;
      const s2 = document.createElement('span'); s2.textContent = title;
      link.append(s1, s2);
      nav.appendChild(link);
    }
    const related = $('#related-links');
    for (const [label, href] of [['世界书','04-world-book.html'],['关系图谱','34-relationship-graph.html'],['诊断','23-diagnostics.html']]) {
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
    const sel = $('#graph-character');
    sel.replaceChildren();
    sel.appendChild(new Option('— 请选择角色 —', ''));
    try {
      characters = await client.request('GET', '/v1/characters').catch(() => []);
      for (const id of characters) sel.appendChild(new Option(id, id));
      if (characterId && characters.includes(characterId)) {
        sel.value = characterId;
        loadGraph();
      }
    } catch (error) {
      setStatus('加载角色失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  async function loadGraph() {
    if (!characterId) { setStatus('请先选择角色', true); return; }
    const includeKeyOverlap = $('#opt-key-overlap').checked;
    const includeReferences = $('#opt-references').checked;
    const detectConflicts = $('#opt-conflicts').checked;
    const minWeight = parseInt($('#opt-min-weight').value, 10) || 1;
    const query = new URLSearchParams({
      include_key_overlap: includeKeyOverlap ? 'true' : 'false',
      include_references: includeReferences ? 'true' : 'false',
      detect_conflicts: detectConflicts ? 'true' : 'false',
      min_weight: String(minWeight),
    }).toString();
    setStatus('正在加载知识图谱…');
    try {
      const graph = await client.request('GET', '/v1/characters/' + encodeURIComponent(characterId) + '/lorebook/graph?' + query);
      graphData = graph;
      renderGraph(graph);
      setStatus('图谱加载完成：' + graph.node_count + ' 节点 · ' + graph.edge_count + ' 边 · ' + graph.conflicts.length + ' 冲突');
    } catch (error) {
      graphData = null;
      simNodes = [];
      simEdges = [];
      clearCanvas();
      $('#graph-empty').textContent = '加载失败：' + AIRPApi.errorMessage(error.data, error.message);
      $('#graph-empty').style.display = 'block';
      setStatus('加载失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  function renderGraph(graph) {
    $('#graph-stats').textContent = '节点 ' + graph.node_count + ' · 边 ' + graph.edge_count + ' · 冲突 ' + graph.conflicts.length;
    // 冲突列表
    const conflictsWrap = $('#graph-conflicts');
    const conflictsList = $('#conflicts-list');
    conflictsList.replaceChildren();
    if (graph.conflicts.length === 0) {
      conflictsWrap.hidden = true;
    } else {
      conflictsWrap.hidden = false;
      for (const c of graph.conflicts) {
        const li = node('li');
        li.appendChild(node('span', 'conflict-key', c.key));
        li.appendChild(node('span', 'conflict-entries', '被 entry ' + c.entry_indices.join(', ') + ' 同时引用'));
        conflictsList.appendChild(li);
      }
    }
    // 初始化力导向布局
    initSimulation(graph);
    $('#graph-empty').style.display = graph.nodes.length === 0 ? 'block' : 'none';
    runSimulation();
  }

  function initSimulation(graph) {
    const canvas = $('#graph-canvas');
    const W = canvas.width;
    const H = canvas.height;
    const cx = W / 2;
    const cy = H / 2;
    // 节点初始位置：圆形分布
    simNodes = graph.nodes.map((n, i) => {
      const angle = (i / Math.max(graph.nodes.length, 1)) * Math.PI * 2;
      const radius = Math.min(W, H) * 0.35;
      return {
        id: n.id,
        label: n.label,
        keys: n.keys,
        constant: n.constant,
        enabled: n.enabled,
        contentLength: n.content_length,
        priority: n.priority,
        x: cx + Math.cos(angle) * radius + (Math.random() - 0.5) * 20,
        y: cy + Math.sin(angle) * radius + (Math.random() - 0.5) * 20,
        vx: 0,
        vy: 0,
        radius: 18 + Math.min(n.keys.length * 2, 12),
      };
    });
    simEdges = graph.edges.map(e => ({
      source: e.source,
      target: e.target,
      kind: e.kind,
      weight: e.weight,
      sharedKeys: e.shared_keys,
    }));
  }

  // 简化版 Fruchterman-Reingold 力导向算法（AIRP 独立实现）
  function runSimulation() {
    if (simNodes.length === 0) { clearCanvas(); return; }
    const iterations = 300;
    const canvas = $('#graph-canvas');
    const W = canvas.width;
    const H = canvas.height;
    const k = Math.sqrt((W * H) / Math.max(simNodes.length, 1)) * 0.6;
    const temperature = W / 10;
    let cool = temperature;
    for (let iter = 0; iter < iterations; iter++) {
      // 计算排斥力
      for (const a of simNodes) {
        a.fx = 0; a.fy = 0;
        for (const b of simNodes) {
          if (a === b) continue;
          let dx = a.x - b.x;
          let dy = a.y - b.y;
          let dist = Math.sqrt(dx * dx + dy * dy) || 0.1;
          const repulsion = (k * k) / dist;
          a.fx += (dx / dist) * repulsion;
          a.fy += (dy / dist) * repulsion;
        }
      }
      // 计算吸引力（边）
      for (const e of simEdges) {
        const a = simNodes[e.source];
        const b = simNodes[e.target];
        if (!a || !b) continue;
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let dist = Math.sqrt(dx * dx + dy * dy) || 0.1;
        const attraction = (dist * dist) / k;
        const fx = (dx / dist) * attraction;
        const fy = (dy / dist) * attraction;
        a.fx += fx; a.fy += fy;
        b.fx -= fx; b.fy -= fy;
      }
      // 居中引力
      const cx = W / 2;
      const cy = H / 2;
      for (const a of simNodes) {
        a.fx += (cx - a.x) * 0.01;
        a.fy += (cy - a.y) * 0.01;
      }
      // 应用位移（限制最大步长）
      for (const a of simNodes) {
        if (a === dragNode) continue;
        let dx = a.fx;
        let dy = a.fy;
        let dist = Math.sqrt(dx * dx + dy * dy) || 0.1;
        const step = Math.min(dist, cool);
        a.x += (dx / dist) * step;
        a.y += (dy / dist) * step;
        // 边界约束
        a.x = Math.max(a.radius, Math.min(W - a.radius, a.x));
        a.y = Math.max(a.radius, Math.min(H - a.radius, a.y));
      }
      cool *= 0.95;
    }
    drawCanvas();
  }

  function clearCanvas() {
    const canvas = $('#graph-canvas');
    const ctx = canvas.getContext('2d');
    ctx.clearRect(0, 0, canvas.width, canvas.height);
  }

  function drawCanvas() {
    const canvas = $('#graph-canvas');
    const ctx = canvas.getContext('2d');
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    // 画边
    for (const e of simEdges) {
      const a = simNodes[e.source];
      const b = simNodes[e.target];
      if (!a || !b) continue;
      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      if (e.kind === 'key_overlap') {
        ctx.strokeStyle = '#4a90d9';
        ctx.lineWidth = 1 + Math.min(e.weight, 4);
      } else {
        ctx.strokeStyle = '#d94a4a';
        ctx.lineWidth = 1 + Math.min(e.weight, 3);
      }
      ctx.stroke();
      // 引用边画箭头
      if (e.kind === 'reference') {
        drawArrow(ctx, a.x, a.y, b.x, b.y, b.radius);
      }
    }
    // 画节点
    for (const n of simNodes) {
      // 外圈（constant 高亮）
      if (n.constant) {
        ctx.beginPath();
        ctx.arc(n.x, n.y, n.radius + 4, 0, Math.PI * 2);
        ctx.strokeStyle = '#e0a800';
        ctx.lineWidth = 2;
        ctx.stroke();
      }
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.radius, 0, Math.PI * 2);
      const isHover = n === hoverNode;
      const isSelected = n === selectedNode;
      if (isSelected) {
        ctx.fillStyle = '#2d7a2d';
      } else if (isHover) {
        ctx.fillStyle = '#6ba36b';
      } else if (!n.enabled) {
        ctx.fillStyle = '#999';
      } else {
        ctx.fillStyle = '#4a90d9';
      }
      ctx.fill();
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 2;
      ctx.stroke();
      // 标签
      ctx.fillStyle = '#222';
      ctx.font = '12px sans-serif';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'top';
      ctx.fillText(n.label, n.x, n.y + n.radius + 4);
    }
  }

  function drawArrow(ctx, x1, y1, x2, y2, radius) {
    const angle = Math.atan2(y2 - y1, x2 - x1);
    const ex = x2 - Math.cos(angle) * (radius + 4);
    const ey = y2 - Math.sin(angle) * (radius + 4);
    ctx.beginPath();
    ctx.moveTo(ex, ey);
    ctx.lineTo(ex - 8 * Math.cos(angle - 0.4), ey - 8 * Math.sin(angle - 0.4));
    ctx.lineTo(ex - 8 * Math.cos(angle + 0.4), ey - 8 * Math.sin(angle + 0.4));
    ctx.closePath();
    ctx.fillStyle = '#d94a4a';
    ctx.fill();
  }

  function findNodeAt(x, y) {
    for (let i = simNodes.length - 1; i >= 0; i--) {
      const n = simNodes[i];
      const dx = x - n.x;
      const dy = y - n.y;
      if (dx * dx + dy * dy <= n.radius * n.radius) return n;
    }
    return null;
  }

  function showNodeDetail(n) {
    selectedNode = n;
    const wrap = $('#graph-detail');
    const content = $('#graph-detail-content');
    if (!n) { wrap.hidden = true; return; }
    wrap.hidden = false;
    const lines = [];
    lines.push('Label: ' + n.label);
    lines.push('ID: #' + n.id);
    lines.push('Keys: ' + (n.keys.length ? n.keys.join(', ') : '（无）'));
    lines.push('Constant: ' + (n.constant ? '是' : '否'));
    lines.push('Enabled: ' + (n.enabled ? '是' : '否'));
    lines.push('Content 长度: ' + n.contentLength + ' 字符');
    lines.push('Priority: ' + n.priority);
    content.textContent = lines.join('\n');
    drawCanvas();
  }

  function setupCanvas() {
    const canvas = $('#graph-canvas');
    function getPos(evt) {
      const rect = canvas.getBoundingClientRect();
      const scaleX = canvas.width / rect.width;
      const scaleY = canvas.height / rect.height;
      return { x: (evt.clientX - rect.left) * scaleX, y: (evt.clientY - rect.top) * scaleY };
    }
    canvas.addEventListener('mousedown', evt => {
      const p = getPos(evt);
      const n = findNodeAt(p.x, p.y);
      if (n) {
        dragNode = n;
        canvas.style.cursor = 'grabbing';
      } else {
        selectedNode = null;
        showNodeDetail(null);
        drawCanvas();
      }
    });
    canvas.addEventListener('mousemove', evt => {
      const p = getPos(evt);
      if (dragNode) {
        dragNode.x = p.x;
        dragNode.y = p.y;
        drawCanvas();
      } else {
        const n = findNodeAt(p.x, p.y);
        if (n !== hoverNode) {
          hoverNode = n;
          canvas.style.cursor = n ? 'pointer' : 'grab';
          drawCanvas();
        }
      }
    });
    canvas.addEventListener('mouseup', () => {
      if (dragNode) {
        dragNode = null;
        canvas.style.cursor = 'grab';
        runSimulation();
      }
    });
    canvas.addEventListener('mouseleave', () => {
      if (dragNode) {
        dragNode = null;
        canvas.style.cursor = 'grab';
        runSimulation();
      }
      hoverNode = null;
      drawCanvas();
    });
    canvas.addEventListener('click', evt => {
      const p = getPos(evt);
      const n = findNodeAt(p.x, p.y);
      if (n) showNodeDetail(n);
    });
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

    $('#graph-character').addEventListener('change', () => {
      characterId = $('#graph-character').value;
      sessionStorage.setItem('airp_character_id', characterId);
      $('#scope-character').textContent = characterId || '—';
      if (characterId) loadGraph();
    });
    $('#graph-refresh').addEventListener('click', loadGraph);
    setupCanvas();
  }
  boot();
})();
