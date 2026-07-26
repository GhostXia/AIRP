// Phase 5.3: plugin-tools.js — 插件工具管理页面交互逻辑。
// 与 provider-management.js 同构：服务端权威 + 本地 draft + dialog 编辑器。
// 安全约束：
//  - 所有从服务端拿到的工具名/URL/路径仅通过 textContent 渲染（防 XSS）。
//  - 编辑器表单值用 textContent 写回 DOM，不使用 innerHTML。
//  - 测试参数由用户手工输入，仅 JSON.parse 后发到后端（后端再次校验）。

(function () {
  'use strict';
  const $ = s => document.querySelector(s);
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) {
    sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  }
  const base = sessionStorage.getItem('airp_engine_url') || location.origin;
  const bearer = sessionStorage.getItem('airp_bearer') || '';
  const client = AIRPApi.createClient({ base, bearer });

  // 与 console-runtime 一致的 pages 数组，末尾追加本页。
  const pages = [
    ['03','workbench','角色工作台','03-workbench.html'],
    ['04','worldbook','世界书','04-world-book.html'],
    ['05','presets','预设','05-presets.html'],
    ['06','persona','Persona','06-user-persona.html'],
    ['07','agent','Agent 运行','07-agent-runs.html'],
    ['08','settings','设置','08-settings.html'],
    ['17','memory','记忆与状态','17-memory-state.html'],
    ['18','scenes','多人场景','18-group-chat.html'],
    ['19','branches','分支与 Swipe','19-branch-tree.html'],
    ['20','preview','装配预览','20-assembly-preview.html'],
    ['21','quota','用量配额','21-usage-quota.html'],
    ['23','diagnostics','诊断','23-diagnostics.html'],
    ['32','style','风格系统','32-style-review.html'],
    ['34','graph','关系图谱','34-relationship-graph.html'],
    ['35','plotarc','剧情弧','35-plot-arc.html'],
    ['36','imagegen','图片生成','36-image-gen.html'],
    ['37','templates','模板库','37-character-templates.html'],
    ['38','stylelearn','风格迁移','38-style-learn.html'],
    ['39','dialoguegen','对话示例','39-dialogue-gen.html'],
    ['40','wbgraph','知识图谱','40-worldbook-graph.html'],
    ['41','timeline','时间线导出','41-timeline-export.html'],
    ['42','carddiff','版本对比','42-card-diff.html'],
    ['43','providers','多 Provider 路由','43-provider-management.html'],
    ['44','plugintools','插件工具','44-plugin-tools.html'],
  ];

  function pathWithState(path) {
    const url = new URL(path, location.href);
    if (client.base !== location.origin) url.searchParams.set('engine', client.base);
    return url.href;
  }

  function renderChrome() {
    $('#engine-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    $('#engine-address-ctx').textContent = client.base === location.origin ? '同源 Engine' : client.base;
    const nav = $('#console-nav');
    const group = document.createElement('div');
    group.className = 'nav-group';
    group.textContent = '工作区';
    nav.appendChild(group);
    const home = document.createElement('a');
    home.className = 'nav-link';
    home.textContent = '角色与会话';
    home.href = pathWithState('01-role-list.html');
    nav.appendChild(home);
    for (const [idx, id, title, href] of pages) {
      const link = document.createElement('a');
      link.className = 'nav-link' + (id === 'plugintools' ? ' active' : '');
      link.href = pathWithState(href);
      const s1 = document.createElement('span');
      s1.className = 'nav-index';
      s1.textContent = idx;
      const s2 = document.createElement('span');
      s2.textContent = title;
      link.append(s1, s2);
      nav.appendChild(link);
    }
    const related = $('#related-links');
    for (const [label, href] of [
      ['角色列表', '01-role-list.html'],
      ['设置', '08-settings.html'],
      ['诊断', '23-diagnostics.html'],
      ['Agent 运行', '07-agent-runs.html'],
    ]) {
      const a = document.createElement('a');
      a.className = 'context-link';
      a.textContent = label + ' →';
      a.href = pathWithState(href);
      related.appendChild(a);
    }
  }

  function setStatus(text, isError) {
    const el = $('#runtime-status');
    el.textContent = text;
    el.classList.toggle('error', Boolean(isError));
  }

  // ── HTML 转义（用于 textarea/code 中的展示） ──────────────────────────
  function escapeText(s) {
    return String(s == null ? '' : s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  // ── 状态 ─────────────────────────────────────────────────────────────
  let serverTools = []; // 服务端权威
  let editingIndex = -1; // -1 = 新增

  // ── 加载 ─────────────────────────────────────────────────────────────
  async function loadAll() {
    setStatus('加载中...', false);
    try {
      const resp = await client.request('GET', '/v1/plugin-tools');
      serverTools = Array.isArray(resp.tools) ? resp.tools : [];
      renderTools();
      renderTestSelect();
      setStatus(`已加载 ${serverTools.length} 个工具（启用 ${resp.enabled || 0}）`, false);
    } catch (err) {
      setStatus('加载失败：' + (err && err.message ? err.message : String(err)), true);
      serverTools = [];
      renderTools();
      renderTestSelect();
    }
  }

  // ── 渲染：工具列表 ───────────────────────────────────────────────────
  function renderTools() {
    const tbody = $('#pt-rows');
    tbody.replaceChildren();
    $('#pt-count').textContent = String(serverTools.length);

    const enabledBar = $('#pt-enabled-bar');
    const enabledTag = $('#pt-enabled-tag');
    const enabledHint = $('#pt-enabled-hint');
    if (serverTools.length > 0) {
      const enabledCount = serverTools.filter(t => t.enabled).length;
      enabledBar.classList.add('is-enabled');
      enabledTag.textContent = '已启用';
      enabledHint.textContent = `${enabledCount}/${serverTools.length} 个工具已启用，将进入 ToolRegistry。`;
    } else {
      enabledBar.classList.remove('is-enabled');
      enabledTag.textContent = '未启用';
      enabledHint.textContent = '尚未注册任何插件工具';
    }

    if (serverTools.length === 0) {
      $('#pt-table').hidden = true;
      $('#pt-empty').hidden = false;
      return;
    }
    $('#pt-table').hidden = false;
    $('#pt-empty').hidden = true;

    const tpl = $('#pt-row-template');
    for (let i = 0; i < serverTools.length; i++) {
      const t = serverTools[i];
      const row = tpl.content.firstElementChild.cloneNode(true);
      row.dataset.index = String(i);
      row.querySelector('.pt-cell-name').textContent = t.name;
      const invCode = row.querySelector('.pt-cell-invocation code');
      invCode.textContent = invocationSummary(t.invocation);
      const sideCell = row.querySelector('.pt-cell-side-effect');
      sideCell.textContent = t.side_effect;
      sideCell.dataset.side = t.side_effect;
      const toggle = row.querySelector('.pt-toggle');
      toggle.classList.toggle('is-on', Boolean(t.enabled));
      toggle.title = t.enabled ? '已启用' : '已禁用';
      row.querySelector('.pt-test').addEventListener('click', () => openTestFor(i));
      row.querySelector('.pt-edit').addEventListener('click', () => openEditor(i));
      row.querySelector('.pt-delete').addEventListener('click', () => deleteTool(i));
      tbody.appendChild(row);
    }
  }

  function invocationSummary(inv) {
    if (!inv || typeof inv !== 'object') return '(unknown)';
    if (inv.kind === 'webhook') {
      const headers = inv.headers_set ? ' +headers' : '';
      const t = inv.timeout_secs ? ` timeout=${inv.timeout_secs}s` : '';
      return `webhook ${inv.url}${headers}${t}`;
    }
    if (inv.kind === 'script') {
      const args = Array.isArray(inv.args) && inv.args.length ? ` args=[${inv.args.join(',')}]` : '';
      const t = inv.timeout_secs ? ` timeout=${inv.timeout_secs}s` : '';
      return `script ${inv.relative_path}${args}${t}`;
    }
    return `(unknown kind: ${inv.kind || '?'})`;
  }

  // ── 编辑器：打开 ─────────────────────────────────────────────────────
  function openEditor(index) {
    editingIndex = index;
    const dlg = $('#pt-editor');
    const title = $('#pt-editor-title');
    const hint = $('#pt-editor-hint');
    hint.textContent = '';
    hint.classList.remove('is-error');
    if (index === -1) {
      title.textContent = '新增工具';
      $('#pt-ed-name').value = '';
      $('#pt-ed-name').disabled = false;
      $('#pt-ed-description').value = '';
      $('#pt-ed-side-effect').value = 'readonly';
      $('#pt-ed-enabled').checked = true;
      $('#pt-ed-kind').value = 'webhook';
      $('#pt-ed-wh-url').value = '';
      $('#pt-ed-wh-timeout').value = '';
      $('#pt-ed-wh-headers').value = '';
      $('#pt-ed-sc-path').value = '';
      $('#pt-ed-sc-args').value = '';
      $('#pt-ed-sc-timeout').value = '';
    } else {
      const t = serverTools[index];
      title.textContent = '编辑工具: ' + t.name;
      $('#pt-ed-name').value = t.name;
      // 编辑时不允许改名（upsert 语义允许，但改名容易冲突，这里禁用以减少误操作）
      $('#pt-ed-name').disabled = true;
      $('#pt-ed-description').value = t.description || '';
      $('#pt-ed-side-effect').value = t.side_effect || 'readonly';
      $('#pt-ed-enabled').checked = Boolean(t.enabled);
      const inv = t.invocation || {};
      if (inv.kind === 'webhook') {
        $('#pt-ed-kind').value = 'webhook';
        $('#pt-ed-wh-url').value = inv.url || '';
        $('#pt-ed-wh-timeout').value = inv.timeout_secs || '';
        // Critical4, 2026-07-26: headers 不返回本体（仅 headers_set）。
        // 编辑时清空 textarea，提示用户：留空 = 保留原 headers，输入新内容 = 覆盖。
        $('#pt-ed-wh-headers').value = '';
        if (inv.headers_set) {
          hint.textContent = '已设置自定义 headers；留空表示保留原值，输入新内容会覆盖。';
        }
      } else if (inv.kind === 'script') {
        $('#pt-ed-kind').value = 'script';
        $('#pt-ed-sc-path').value = inv.relative_path || '';
        $('#pt-ed-sc-args').value = Array.isArray(inv.args) ? inv.args.join('\n') : '';
        $('#pt-ed-sc-timeout').value = inv.timeout_secs || '';
      } else {
        $('#pt-ed-kind').value = 'webhook';
      }
    }
    toggleInvocationBlocks();
    dlg.showModal();
  }

  function closeEditor() {
    const dlg = $('#pt-editor');
    if (dlg.open) dlg.close();
  }

  function toggleInvocationBlocks() {
    const kind = $('#pt-ed-kind').value;
    $('#pt-inv-webhook').hidden = kind !== 'webhook';
    $('#pt-inv-script').hidden = kind !== 'script';
  }

  // ── 编辑器：保存（提交到后端） ───────────────────────────────────────
  async function applyEditor() {
    const hint = $('#pt-editor-hint');
    hint.textContent = '校验中...';
    hint.classList.remove('is-error');

    const name = $('#pt-ed-name').value.trim();
    const description = $('#pt-ed-description').value.trim();
    const side_effect = $('#pt-ed-side-effect').value;
    const enabled = $('#pt-ed-enabled').checked;
    const kind = $('#pt-ed-kind').value;

    // Minor3, 2026-07-26: name 格式校验（与后端 `^[a-z0-9_]{1,64}$` 对齐）
    if (!name) {
      hint.textContent = 'name 不能为空';
      hint.classList.add('is-error');
      return;
    }
    if (!/^[a-z0-9_]{1,64}$/.test(name)) {
      hint.textContent = 'name 只能包含小写字母、数字和下划线，长度 1-64';
      hint.classList.add('is-error');
      return;
    }
    if (!description) {
      hint.textContent = 'description 不能为空';
      hint.classList.add('is-error');
      return;
    }
    // Major5, 2026-07-26: 新增工具时检查重名（编辑时 name disabled，无需检查）
    if (editingIndex === -1 && serverTools.some(t => t.name === name)) {
      hint.textContent = `工具名 "${name}" 已存在`;
      hint.classList.add('is-error');
      return;
    }

    let invocation;
    if (kind === 'webhook') {
      const url = $('#pt-ed-wh-url').value.trim();
      if (!url) {
        hint.textContent = 'webhook url 不能为空';
        hint.classList.add('is-error');
        return;
      }
      // Minor3: 抽取 timeout 校验为共享 helper
      const timeout_secs = validateTimeout($('#pt-ed-wh-timeout').value, hint);
      if (timeout_secs === null && hint.classList.contains('is-error')) return;
      const headersText = $('#pt-ed-wh-headers').value;
      const headers = parseHeaders(headersText);
      if (headers === null) {
        hint.textContent = 'headers 格式错误（每行 "Name: Value"）';
        hint.classList.add('is-error');
        return;
      }
      // Minor3: 单个 header name/value 长度限制
      for (const [hname, hvalue] of Object.entries(headers)) {
        if (hname.length > 64) {
          hint.textContent = `header name "${hname.slice(0, 16)}..." 超过 64 字符`;
          hint.classList.add('is-error');
          return;
        }
        if (hvalue.length > 4096) {
          hint.textContent = `header "${hname}" 的 value 超过 4096 字符`;
          hint.classList.add('is-error');
          return;
        }
      }
      invocation = {
        kind: 'webhook',
        url,
        headers,
        timeout_secs,
      };
    } else if (kind === 'script') {
      const relative_path = $('#pt-ed-sc-path').value.trim();
      if (!relative_path) {
        hint.textContent = 'relative_path 不能为空';
        hint.classList.add('is-error');
        return;
      }
      // Minor3: relative_path 长度限制
      if (relative_path.length > 512) {
        hint.textContent = 'relative_path 超过 512 字符';
        hint.classList.add('is-error');
        return;
      }
      const argsText = $('#pt-ed-sc-args').value;
      const args = argsText
        .split('\n')
        .map(s => s.trim())
        .filter(s => s.length > 0);
      if (args.length > 16) {
        hint.textContent = 'args 数量超过 16';
        hint.classList.add('is-error');
        return;
      }
      // Minor3: 单个 arg 长度限制
      for (const arg of args) {
        if (arg.length > 4096) {
          hint.textContent = '单个 arg 超过 4096 字符';
          hint.classList.add('is-error');
          return;
        }
      }
      // Minor3: 抽取 timeout 校验为共享 helper
      const timeout_secs = validateTimeout($('#pt-ed-sc-timeout').value, hint);
      if (timeout_secs === null && hint.classList.contains('is-error')) return;
      invocation = {
        kind: 'script',
        relative_path,
        args,
        timeout_secs,
      };
    } else {
      hint.textContent = '未知 invocation.kind: ' + kind;
      hint.classList.add('is-error');
      return;
    }

    const body = { name, description, side_effect, enabled, invocation };

    hint.textContent = '保存中...';
    try {
      await client.request('POST', '/v1/plugin-tools', body);
      closeEditor();
      await loadAll();
      setStatus(`已保存工具 ${name}`, false);
    } catch (err) {
      hint.textContent = '保存失败：' + (err && err.message ? err.message : String(err));
      hint.classList.add('is-error');
    }
  }

  // Minor3, 2026-07-26: 共享 timeout 校验 helper。
  // 返回解析后的 timeout_secs（null 表示未设置），或在校验失败时设置 hint 并返回 null + is-error。
  function validateTimeout(raw, hint) {
    const trimmed = raw.trim();
    if (!trimmed) return null;
    const num = Number(trimmed);
    if (!Number.isFinite(num) || num < 1 || num > 30) {
      hint.textContent = 'timeout_secs 必须在 1..30 之间';
      hint.classList.add('is-error');
      return null;
    }
    return Math.floor(num);
  }

  // 解析 "Name: Value" 多行 headers。返回 BTreeMap-like 对象，或 null（格式错误）。
  function parseHeaders(text) {
    const result = {};
    if (!text || !text.trim()) return result;
    const lines = text.split('\n');
    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      // 注释行允许（用户备注）
      if (trimmed.startsWith('#')) continue;
      const idx = trimmed.indexOf(':');
      if (idx <= 0) return null;
      const name = trimmed.slice(0, idx).trim();
      const value = trimmed.slice(idx + 1).trim();
      if (!name) return null;
      result[name] = value;
    }
    return result;
  }

  // ── 删除 ─────────────────────────────────────────────────────────────
  async function deleteTool(index) {
    const t = serverTools[index];
    if (!t) return;
    if (!confirm(`确认删除插件工具 "${t.name}"？此操作不可撤销。`)) return;
    setStatus(`删除 ${t.name} 中...`, false);
    try {
      await client.request('DELETE', '/v1/plugin-tools/' + encodeURIComponent(t.name));
      await loadAll();
      setStatus(`已删除工具 ${t.name}`, false);
    } catch (err) {
      setStatus('删除失败：' + (err && err.message ? err.message : String(err)), true);
    }
  }

  // ── 测试调用 ─────────────────────────────────────────────────────────
  function renderTestSelect() {
    const sel = $('#pt-test-name');
    sel.replaceChildren();
    if (serverTools.length === 0) {
      const opt = document.createElement('option');
      opt.value = '';
      opt.textContent = '(无可用工具)';
      sel.appendChild(opt);
      sel.disabled = true;
      return;
    }
    sel.disabled = false;
    for (const t of serverTools) {
      const opt = document.createElement('option');
      opt.value = t.name;
      opt.textContent = t.name;
      sel.appendChild(opt);
    }
  }

  function openTestFor(index) {
    const t = serverTools[index];
    if (!t) return;
    $('#pt-test-name').value = t.name;
    $('#pt-test-params').value = '{}';
    $('#pt-test-confirm').checked = false;
    $('#pt-test-output').textContent = '';
    // 滚动到测试区
    $('#pt-test-name').scrollIntoView({ behavior: 'smooth', block: 'center' });
  }

  async function runTest() {
    const name = $('#pt-test-name').value;
    if (!name) {
      setStatus('请先选择工具', true);
      return;
    }
    const paramsText = $('#pt-test-params').value;
    let params;
    try {
      params = paramsText.trim() ? JSON.parse(paramsText) : {};
    } catch (err) {
      $('#pt-test-output').textContent = 'params JSON 解析失败：' + (err && err.message ? err.message : String(err));
      return;
    }
    const confirm_flag = $('#pt-test-confirm').checked;
    const output = $('#pt-test-output');
    output.textContent = '调用中...';
    setStatus(`测试调用 ${name}...`, false);
    try {
      const resp = await client.request('POST', '/v1/plugin-tools/' + encodeURIComponent(name) + '/test', {
        params,
        confirm: confirm_flag,
      });
      output.textContent = JSON.stringify(resp, null, 2);
      setStatus(`测试完成 (dry_run=${resp.dry_run})`, false);
    } catch (err) {
      output.textContent = '调用失败：' + (err && err.message ? err.message : String(err));
      setStatus('测试调用失败', true);
    }
  }

  function clearTest() {
    $('#pt-test-params').value = '{}';
    $('#pt-test-confirm').checked = false;
    $('#pt-test-output').textContent = '';
  }

  // ── 引擎状态轮询 ─────────────────────────────────────────────────────
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
    $('#pt-add').addEventListener('click', () => openEditor(-1));
    $('#pt-reload').addEventListener('click', loadAll);
    $('#pt-ed-cancel').addEventListener('click', (ev) => { ev.preventDefault(); closeEditor(); });
    $('#pt-editor-form').addEventListener('submit', (ev) => {
      ev.preventDefault();
      applyEditor();
    });
    $('#pt-ed-kind').addEventListener('change', toggleInvocationBlocks);
    $('#pt-test-run').addEventListener('click', runTest);
    $('#pt-test-clear').addEventListener('click', clearTest);
  }

  // ── 启动 ─────────────────────────────────────────────────────────────
  function boot() {
    renderChrome();
    bindEvents();
    pollEngineStatus();
    setInterval(pollEngineStatus, 10000);
    loadAll();
  }

  boot();
})();
