// AIRP Engine Console — backend validation harness (M1)
// Zero-build native JS.  plan: docs/WEBUI-BACKEND-PLAN.md

(function () {
  'use strict';

  // ── DOM refs ─────────────────────────────────────────────────────────────
  const $ = (s) => document.querySelector(s);
  const $$ = (s) => document.querySelectorAll(s);
  const engineUrl = $('#engine-url');
  const bearerToken = $('#bearer-token');
  const btnConnect = $('#btn-connect');
  const connStatus = $('#conn-status');
  const connText = $('#conn-text');
  const accessKeyWarning = $('#access-key-warning');
  const healthInfo = $('#health-info');
  const settingsDisplay = $('#settings-display');
  const modelsDisplay = $('#models-display');
  const btnRefreshModels = $('#btn-refresh-models');
  const charSelect = $('#char-select');
  const sessSelect = $('#sess-select');
  const chatLog = $('#chat-log');
  const chatInput = $('#chat-input');
  const btnSend = $('#btn-send');
  const btnStop = $('#btn-stop');
  const btnHistory = $('#btn-history');
  const btnRegen = $('#btn-regen');
  const btnRollback = $('#btn-rollback');
  const agentInput = $('#agent-input');
  const btnAgentRun = $('#btn-agent-run');
  const agentOutput = $('#agent-output');
  const eventLog = $('#event-log');
  const btnClearLog = $('#btn-clear-log');
  const btnRefreshChars = $('#btn-refresh-chars');
  const btnNewSession = $('#btn-new-session');
  const charAvatar = $('#char-avatar');
  const stateDisplay = $('#state-display');
  const stateHistoryDisplay = $('#state-history-display');
  const stateHistoryLimit = $('#state-history-limit');
  const btnRefreshState = $('#btn-refresh-state');
  const characterList = $('#character-list');
  const btnDeleteChar = $('#btn-delete-char');

  // ── state ────────────────────────────────────────────────────────────────
  let base = engineUrl.value.replace(/\/+$/, '');
  let bearer = '';
  let selectedChar = '';
  let selectedSess = '';
  let abortController = null;   // for chat SSE
  let agentAbort = null;        // for agent run SSE — 二次点击先 abort 前一个，防事件交错竞态（issue #43/#44 D）

  // ── event log ────────────────────────────────────────────────────────────
  function logEvent(method, path, status, ms, detail) {
    const el = document.createElement('div');
    el.className = 'event';
    const now = new Date().toLocaleTimeString();
    const code = Number(status);
    const cls = code >= 200 && code < 300 ? 'ok' : code >= 500 || code === 0 ? 'err' : 'unknown';
    appendInline(el, 'span', 'ts', now);
    el.append(' ');
    appendInline(el, 'span', 'status ' + cls, String(status));
    el.append(' ');
    appendInline(el, 'span', 'method', String(method));
    el.append(' ');
    appendInline(el, 'span', 'path', String(path));
    el.append(' ' + ms + 'ms');
    if (detail) {
      el.appendChild(document.createElement('br'));
      appendInline(el, 'span', 'detail', String(detail));
    }
    eventLog.prepend(el);
    if (eventLog.children.length > 200) eventLog.removeChild(eventLog.lastChild);
  }

  function appendInline(parent, tag, className, text) {
    const node = document.createElement(tag);
    node.className = className;
    node.textContent = text;
    parent.appendChild(node);
    return node;
  }

  // ── HTTP helpers ─────────────────────────────────────────────────────────
  function headers(extra) {
    const h = { 'Content-Type': 'application/json', ...extra };
    if (bearer) h['Authorization'] = 'Bearer ' + bearer;
    return h;
  }

  async function api(method, path, body) {
    const url = base + path;
    const t0 = performance.now();
    try {
      const opts = { method, headers: headers() };
      if (body !== undefined) opts.body = JSON.stringify(body);
      const res = await fetch(url, opts);
      const ms = Math.round(performance.now() - t0);
      const text = await res.text();
      logEvent(method, path, res.status, ms);
      let data;
      try { data = JSON.parse(text); } catch { data = text; }
      return { ok: res.ok, status: res.status, data, text };
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      logEvent(method, path, 0, ms, e.message);
      return { ok: false, status: 0, data: null, text: e.message };
    }
  }

  // ── connection ───────────────────────────────────────────────────────────
  // #67 #9 fix: formatError 用白名单展开已知字段，其余字段折叠为 raw JSON 显示。
  // 避免 engine 错误模型扩展（如 request_id / hint / suggestion）时 webui 自动丢失。
  function formatError(data, text) {
    if (data && typeof data === 'object' && data.error) {
      const err = data.error;
      const KNOWN_FIELDS = ['code', 'message', 'upstream_status', 'upstream_body', 'detail'];
      const lines = [err.code || 'error'];
      if (err.message) lines.push(err.message);
      if (err.upstream_status) lines.push('upstream_status=' + err.upstream_status);
      if (err.upstream_body) lines.push('upstream_body=' + err.upstream_body);
      if (err.detail) lines.push('detail=' + err.detail);
      // 折叠未知字段为 raw JSON（排除已知字段和已展开的字段）
      const extras = {};
      let hasExtra = false;
      for (const k of Object.keys(err)) {
        if (!KNOWN_FIELDS.includes(k)) {
          extras[k] = err[k];
          hasExtra = true;
        }
      }
      if (hasExtra) lines.push('extras=' + JSON.stringify(extras));
      return lines.join('\n');
    }
    if (typeof data === 'string' && data) return data;
    if (text) return text;
    return JSON.stringify(data);
  }

  async function connect() {
    // #68 #5: 任何路径进入 connect() 都取消 pending auto-connect，避免
    // keydown Enter / btn-click 后 300ms 又被 setTimeout 触发一次（重复请求）。
    cancelAutoConnect();
    base = engineUrl.value.replace(/\/+$/, '');
    bearer = bearerToken.value || '';
    // W-01: 持久化到 sessionStorage（关 tab 即清，缩短泄漏 token 的存活窗口）
    // 注意：sessionStorage 不缓解 XSS——同 tab 任意脚本仍可读。选它而非 localStorage
    // 只是为了让「tab 关闭后 token 失效」，降低意外跨会话复用的风险。
    try {
      sessionStorage.setItem('airp_engine_url', base);
      sessionStorage.setItem('airp_bearer', bearer);
    } catch {}
    connStatus.className = 'status-dot dot-unknown';
    connText.textContent = '连接中…';
    const r = await api('GET', '/version');
    if (r.ok) {
      // WEBUI-BACKEND-PLAN §4.2: /health 就绪探针，区分"engine 起了"与"能跑对话"
      const h = await api('GET', '/health');
      let detail = (r.data?.version || r.text).slice(0, 40);
      if (h.ok && h.data) {
        if (!h.data.provider_configured) detail += '  ⚠ provider 未配置';
        if (!h.data.data_root_writable) detail += '  ⚠ data_root 不可写';
      }
      connStatus.className = 'status-dot dot-ok';
      connText.textContent = '已连接  ' + detail;
      refreshAll();
    } else {
      connStatus.className = 'status-dot dot-err';
      connText.textContent = '连接失败: ' + formatError(r.data, r.text);
    }
  }

  btnConnect.addEventListener('click', connect);
  engineUrl.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });
  bearerToken.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });

  // #68 #5 fix: auto-connect 竞态保护。用户在 300ms 延迟内编辑 URL/bearer
  // 会触发 connect 读半截值，发请求到不存在的主机污染 event log。
  // 改成：用户输入时取消 pending auto-connect，由 Enter/click 显式触发。
  let pendingAutoConnect = null;
  function scheduleAutoConnect() {
    if (pendingAutoConnect) clearTimeout(pendingAutoConnect);
    pendingAutoConnect = setTimeout(() => {
      pendingAutoConnect = null;
      connect();
    }, 300);
  }
  function cancelAutoConnect() {
    if (pendingAutoConnect) {
      clearTimeout(pendingAutoConnect);
      pendingAutoConnect = null;
    }
  }
  engineUrl.addEventListener('input', cancelAutoConnect);
  bearerToken.addEventListener('input', cancelAutoConnect);

  // ── refresh all left-panel data ──────────────────────────────────────────
  async function refreshAll() {
    await Promise.all([refreshHealth(), refreshSettings(), refreshModels(), refreshChars()]);
    // 初次连接后自动加载当前角色的 chat history（PLAN §9 P1 "交互收口"）。
    // refreshChars 内部已设置 selectedChar；此处 await 完成后即可拉 history。
    if (selectedChar) loadHistory();
  }

  async function refreshHealth() {
    const r = await api('GET', '/version');
    if (r.ok) healthInfo.textContent = 'version: ' + (r.data?.version || r.text);
    else healthInfo.textContent = 'err: ' + formatError(r.data, r.text);
  }

  async function refreshSettings() {
    const r = await api('GET', '/v1/settings');
    if (r.ok) {
      const s = { ...r.data };
      if (s.api_key) s.api_key = maskSecret(s.api_key);
      if (s.access_api_key) s.access_api_key = maskSecret(s.access_api_key);
      settingsDisplay.textContent = JSON.stringify(s, null, 2);
      if (accessKeyWarning) {
        const unprotected = s.access_api_key_set === false;
        accessKeyWarning.hidden = !unprotected;
        accessKeyWarning.textContent = unprotected
          ? '未设置 Bearer；仅限受信的本地开发环境。'
          : '';
      }
    } else {
      settingsDisplay.textContent = 'err: ' + formatError(r.data, r.text);
      if (accessKeyWarning) accessKeyWarning.hidden = true;
    }
  }

  async function refreshModels() {
    if (!modelsDisplay) return;
    modelsDisplay.textContent = 'loading...';
    const r = await api('GET', '/v1/models');
    if (r.ok) {
      const models = Array.isArray(r.data?.data) ? r.data.data : null;
      modelsDisplay.textContent = models
        ? models.map(m => m.id || JSON.stringify(m)).join('\n')
        : JSON.stringify(r.data, null, 2);
    } else {
      modelsDisplay.textContent = 'err:\n' + formatError(r.data, r.text);
    }
  }

  function maskSecret(value) {
    const s = String(value);
    if (s.length <= 8) return '•'.repeat(Math.max(4, s.length));
    return s.slice(0, 4) + '…' + s.slice(-4);
  }

  async function refreshChars() {
    const r = await api('GET', '/v1/characters');
    if (r.ok) {
      const ids = Array.isArray(r.data) ? r.data : [];
      replaceOptions(charSelect, ids);
      if (ids.length && !selectedChar) { selectedChar = ids[0]; charSelect.value = ids[0]; }
      if (selectedChar && ids.includes(selectedChar)) charSelect.value = selectedChar;
      await renderCharacterCards(ids);
      refreshSessions();
      refreshAvatar();
      refreshStateAll();
    }
  }

  function replaceOptions(select, ids, labelFn) {
    select.textContent = '';
    ids.forEach(id => {
      const value = String(id);
      const option = document.createElement('option');
      option.value = value;
      option.textContent = labelFn ? labelFn(value) : value;
      // A4 fix: option 文本可能被 <select> 宽度截断（如完整 UUID），加 title
      // 让用户 hover 看完整值。零成本改善可读性。
      option.title = value;
      select.appendChild(option);
    });
  }

  // The V2 character page is a view over the same selected character state as
  // the accessibility-friendly select in the sidebar.  Keeping one source of
  // truth avoids duplicating the existing session, avatar, and state flows.
  async function renderCharacterCards(ids) {
    if (!characterList) return;
    characterList.textContent = '';
    if (!ids.length) {
      appendInline(characterList, 'p', 'empty-state', '没有可用角色。请先导入角色卡。');
      return;
    }
    const summaries = await Promise.all(ids.map(async id => {
      const characterId = String(id);
      const r = await api('GET', '/v1/characters/' + encodeURIComponent(characterId));
      const card = r.ok && r.data && typeof r.data === 'object' ? (r.data.data || r.data) : {};
      return {
        id: characterId,
        name: typeof card.name === 'string' && card.name.trim() ? card.name : characterId,
        description: typeof card.description === 'string' && card.description.trim()
          ? card.description
          : '选择后加载会话、聊天记录与状态。',
      };
    }));
    summaries.forEach(summary => {
      const characterId = summary.id;
      const card = document.createElement('button');
      card.type = 'button';
      card.className = 'character-card' + (characterId === selectedChar ? ' selected' : '');
      card.setAttribute('aria-pressed', characterId === selectedChar ? 'true' : 'false');
      appendInline(card, 'span', 'character-card-name', summary.name);
      appendInline(card, 'span', 'character-card-copy', summary.description);
      appendInline(card, 'span', 'character-card-meta', '打开对话空间 →');
      card.addEventListener('click', () => {
        if (charSelect.value !== characterId) {
          charSelect.value = characterId;
          charSelect.dispatchEvent(new Event('change'));
        }
        showView('session');
      });
      characterList.appendChild(card);
    });
  }

  function showView(name) {
    const view = document.getElementById('view-' + name);
    if (!view) return;
    document.querySelectorAll('.app-view').forEach(item => {
      const active = item === view;
      item.hidden = !active;
      item.classList.toggle('active', active);
    });
    document.querySelectorAll('.page-nav-item').forEach(item => {
      item.classList.toggle('active', item.dataset.view === name);
    });
    if (window.location.hash !== '#' + name) window.location.hash = name;
  }

  function showViewFromHash() {
    const view = window.location.hash.slice(1);
    showView(view === 'session' ? 'session' : 'characters');
  }

  document.querySelectorAll('[data-view]').forEach(control => {
    control.addEventListener('click', (event) => {
      event.preventDefault();
      showView(control.dataset.view);
    });
  });

  window.addEventListener('hashchange', showViewFromHash);
  showViewFromHash();

  // summary 内的 button 点击不触发 details toggle
  document.querySelectorAll('summary > button').forEach(b => {
    b.addEventListener('click', e => e.stopPropagation());
  });
  btnRefreshChars.addEventListener('click', refreshChars);
  if (btnRefreshModels) btnRefreshModels.addEventListener('click', refreshModels);

  async function refreshSessions() {
    const cid = charSelect.value;
    if (!cid) return;
    selectedChar = cid;
    const r = await api('GET', '/v1/sessions/' + encodeURIComponent(cid));
    if (r.ok) {
      const ids = Array.isArray(r.data) ? r.data.map(s => s.session_id || s) : [];
      // W-08 fix: 不再截断到前 12 字符——UUID 前 12 位区分度低，用户分不清多个 session。
      // 与 charSelect 保持一致，显示完整 ID；select 元素宽度由 CSS 控制。
      replaceOptions(sessSelect, ids);
      if (!ids.includes(selectedSess)) selectedSess = ids[0] || '';
      if (selectedSess) sessSelect.value = selectedSess;
    }
  }

  // POST /v1/sessions 返回引擎 `Json<SessionId>` 序列化的纯字符串（如
  // "550e8400-…"）；历史/兼容路径可能回 `{session_id}` 或 `{uuid}` 对象。
  // 抽出来供 btnNewSession / doSend 复用，消除上一轮 review 留下的两处复制。
  function extractSessionId(r) {
    const raw = r && r.data;
    if (!raw) return '';
    if (typeof raw === 'string') return raw;
    if (typeof raw === 'object') return raw.session_id || raw.uuid || String(raw);
    return String(raw);
  }

  // 终止在飞 chat/agent stream。Kimi-K2.7-Code 已修 chat 这条；
  // 同源 race 同样存在于 agent run（issue #43/#44 二次点击 abort 路径的姊妹），
  // 用户切 session/character 时不 abort 同样会让 agent event 写回已清空视图。
  function abortInFlightStreams() {
    if (abortController) {
      abortController.abort();
      abortController = null;
    }
    if (agentAbort) {
      agentAbort.abort();
      agentAbort = null;
    }
  }

  // 切换 session / character / 新建 session 时统一：终止在飞 stream + 清空视图。
  function clearChatView() {
    abortInFlightStreams();
    chatLog.innerHTML = '';
  }

  // ── avatar: fetch as blob with bearer, render via object URL ─────────────
  // <img src> 无法附 Authorization 头，故 fetch blob → createObjectURL。
  // 切换角色时 revoke 旧 URL 防泄漏。
  let avatarUrl = '';
  async function refreshAvatar() {
    if (avatarUrl) { URL.revokeObjectURL(avatarUrl); avatarUrl = ''; }
    const cid = selectedChar || charSelect.value;
    if (!cid || !charAvatar) { if (charAvatar) charAvatar.hidden = true; return; }
    try {
      const res = await fetch(base + '/v1/characters/' + encodeURIComponent(cid) + '/avatar', {
        headers: bearer ? { Authorization: 'Bearer ' + bearer } : {},
      });
      logEvent('GET', '/v1/characters/:id/avatar', res.status, 0);
      if (!res.ok) { charAvatar.hidden = true; return; }
      const blob = await res.blob();
      if (!blob.size || !blob.type.startsWith('image/')) { charAvatar.hidden = true; return; }
      avatarUrl = URL.createObjectURL(blob);
      charAvatar.src = avatarUrl;
      charAvatar.hidden = false;
    } catch (e) {
      charAvatar.hidden = true;
    }
  }

  // ── state: live.json + state history ─────────────────────────────────────
  // 404 = 角色尚未有 state；与空对象 {} 区分开显示（PLAN §2.1 L47）。
  async function refreshState() {
    if (!stateDisplay) return;
    const cid = selectedChar || charSelect.value;
    if (!cid) { stateDisplay.textContent = '—'; return; }
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(cid) + '/state');
    if (r.status === 404) {
      stateDisplay.textContent = '(404 — 该角色尚无 live.json)';
      return;
    }
    if (r.ok) {
      // try pretty-print; if not JSON, show raw
      try { stateDisplay.textContent = JSON.stringify(typeof r.data === 'string' ? JSON.parse(r.data) : r.data, null, 2); }
      catch { stateDisplay.textContent = String(r.data || r.text); }
    } else {
      stateDisplay.textContent = 'err: ' + formatError(r.data, r.text);
    }
  }

  async function refreshStateHistory() {
    if (!stateHistoryDisplay) return;
    const cid = selectedChar || charSelect.value;
    if (!cid) { stateHistoryDisplay.textContent = '—'; return; }
    const limit = Math.max(1, Math.min(1000, parseInt(stateHistoryLimit.value, 10) || 20));
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(cid) + '/state/history?limit=' + limit);
    if (r.status === 404) {
      stateHistoryDisplay.textContent = '(404 — 尚无 state history)';
      return;
    }
    if (r.ok) {
      const arr = Array.isArray(r.data) ? r.data : [];
      stateHistoryDisplay.textContent = arr.length
        ? arr.map((e, i) => '[' + i + '] ' + JSON.stringify(e)).join('\n')
        : '(empty array)';
    } else {
      stateHistoryDisplay.textContent = 'err: ' + formatError(r.data, r.text);
    }
  }

  function refreshStateAll() {
    refreshState();
    refreshStateHistory();
  }

  if (btnRefreshState) btnRefreshState.addEventListener('click', refreshStateAll);
  if (stateHistoryLimit) stateHistoryLimit.addEventListener('change', refreshStateHistory);

  charSelect.addEventListener('change', () => {
    selectedChar = charSelect.value;
    selectedSess = '';
    renderCharacterCards(Array.from(charSelect.options, option => option.value));
    clearChatView();
    refreshSessions();
    refreshAvatar();
    refreshStateAll();
    // 自动加载 history：切角色后立即拉取该角色已有 chat history，
    // 避免用户每次都需手点 History 按钮（PLAN §9 P1 "交互收口"）。
    loadHistory();
  });
  sessSelect.addEventListener('change', () => {
    selectedSess = sessSelect.value;
    clearChatView();
    loadHistory();
  });

  btnNewSession.addEventListener('click', async () => {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/sessions/' + encodeURIComponent(selectedChar));
    if (r.ok) {
      // 新建后自动选中该 session，省用户再手动点
      const newId = extractSessionId(r);
      if (newId) selectedSess = newId;
      clearChatView();
      await refreshSessions();
    }
  });

  // ── chat: send & stream ─────────────────────────────────────────────────
  // 极简 markdown 渲染（PLAN §4.1.1 "支持 streaming delta 和 markdown"）。
  // 零构建约束 → 不引入第三方库；手写覆盖 code fence / inline code / 标题 /
  // 粗体 / 斜体 / 段落换行，足够 chat 场景。安全策略：先 escapeHtml 全转义，
  // 再用 private-use Unicode 占位符抽出 code fence（防止内部 \n 被 <br> 替换），
  // 最后应用其它行内/块级转换。所有用户内容已被转义，不会注入 HTML。
  function escapeHtml(s) {
    return String(s)
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;')
      .replace(/"/g, '&quot;')
      .replace(/'/g, '&#39;');
  }

  function renderMarkdown(text) {
    if (text == null) return '';
    // 行级分块：先把 fenced code blocks 抽出（用占位符），再按行 split，
    // 块级（pre/h1-h3）独立包裹，避免非法 HTML 嵌套（<p><pre>、<p><h1>）。
    const codeBlocks = [];
    // F16/F17 fix: 用带随机 nonce 的 private-use 占位符，避免用户输入同形序列被误替换。
    const phNonce = Math.random().toString(36).slice(2, 10);
    const phPrefix = '\uF8FFCB' + phNonce + '_';
    const phSuffix = '\uF8FF';
    const phRegex = new RegExp(phPrefix + '(\\d+)' + phSuffix, 'g');
    const placeholder = (i) => '\n' + phPrefix + i + phSuffix + '\n';
    let s = escapeHtml(text);
    // F18 fix: 统一换行符，避免 CRLF 导致空行识别失败。
    s = s.replace(/\r\n/g, '\n').replace(/\r/g, '\n');
    // 1. fenced code blocks ```lang\n...\n``` 抽到占位行
    s = s.replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => {
      // lang 暂未用于高亮（避免引第三方）；保留以便未来加 highlight.js
      const i = codeBlocks.push('<pre class="md-code"><code>' + code.replace(/\n$/, '') + '</code></pre>') - 1;
      return placeholder(i);
    });
    // 2. 行内 markdown：inline code / bold / italic 在非占位行上。
    //    占位行使用随机 nonce，用户输入的同形序列不会被后续恢复步骤误匹配。
    s = s.replace(/`([^`\n]+)`/g, '<code class="md-code-inline">$1</code>');
    s = s.replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>');
    s = s.replace(/\*([^*\n]+)\*/g, '<em>$1</em>');
    // 3. 标题转块级（不被 <p> 包裹）。Fenced placeholder 行不含 #。
    s = s.replace(/^### (.+)$/gm, '<h3 class="md-h">$1</h3>');
    s = s.replace(/^## (.+)$/gm, '<h2 class="md-h">$1</h2>');
    s = s.replace(/^# (.+)$/gm, '<h1 class="md-h">$1</h1>');
    // 4. 块级切分：按行扫描，块级元素/占位行独占一段，blank line 切 paragraph
    const lines = s.split('\n');
    const out = [];
    let para = [];
    const flushPara = () => {
      if (para.length) {
        out.push('<p>' + para.join('<br>') + '</p>');
        para = [];
      }
    };
    const phLineRegex = new RegExp('^' + phPrefix + '\\d+' + phSuffix + '$');
    for (const line of lines) {
      const isBlock = /^(<h[1-3] )/.test(line) || phLineRegex.test(line);
      if (line === '') { flushPara(); continue; }
      if (isBlock) { flushPara(); out.push(line); }
      else { para.push(line); }
    }
    flushPara();
    // 5. 恢复 code blocks (顺序与 codeBlocks.push 一致)
    return out.join('\n').replace(phRegex, (_, i) => codeBlocks[+i]);
  }

  // appendMsg: 流式中用 textContent（保 cursor 动画 + 性能），完成后切 innerHTML 跑 markdown。
  // W-06 fix: 加可选 ts 参数（Date 或省略）。流式新消息传 new Date() 显示 HH:MM:SS；
  // loadHistory 不传（engine 的 chat_log.jsonl 不存消息时间戳，避免用加载时刻误导用户）。
  function appendMsg(role, text, isStreaming, ts) {
    const div = document.createElement('div');
    const safeRole = role === 'user' ? 'user' : 'assistant';
    div.className = 'msg ' + safeRole;
    if (ts instanceof Date) {
      const hh = String(ts.getHours()).padStart(2, '0');
      const mm = String(ts.getMinutes()).padStart(2, '0');
      const ss = String(ts.getSeconds()).padStart(2, '0');
      appendInline(div, 'span', 'ts', hh + ':' + mm + ':' + ss);
      div.append(' ');
    }
    appendInline(div, 'span', 'role', role);
    const textNode = appendInline(div, 'span', 'text' + (isStreaming ? ' streaming' : ''), '');
    if (isStreaming) {
      textNode.textContent = text;
    } else {
      textNode.innerHTML = renderMarkdown(text);
    }
    chatLog.appendChild(div);
    chatLog.scrollTop = chatLog.scrollHeight;
    return textNode;
  }

  async function doSend() {
    const text = chatInput.value.trim();
    if (!text || !selectedChar) return;
    chatInput.value = '';
    appendMsg('user', text, false, new Date());

    // create session if none
    if (!selectedSess) {
      const r = await api('POST', '/v1/sessions/' + encodeURIComponent(selectedChar));
      if (!r.ok) { appendMsg('assistant', '[session create failed]', false, new Date()); return; }
      const newId = extractSessionId(r);
      if (!newId) { appendMsg('assistant', '[session create: empty id]', false, new Date()); return; }
      selectedSess = newId;
      await refreshSessions();
    }

    // abort prior stream
    if (abortController) abortController.abort();
    // 用局部引用 ac：finally 清理时只清「仍是当前实例」的情况，避免旧请求 finally
    // 在新请求已开始后误清新请求的 abortController 与 btnStop 可见性（异步竞态）。
    const ac = new AbortController();
    abortController = ac;
    if (btnStop) btnStop.hidden = false;
    const url = base + '/v1/chat/completions';
    const t0 = performance.now();
    let msgEl = null;

    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify(buildChatPayload(text)),
        signal: ac.signal,
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', '/v1/chat/completions', res.status, Math.round(performance.now() - t0), errBody);
        appendMsg('assistant', '[HTTP ' + res.status + '] ' + errBody, false, new Date());
        return;
      }
      msgEl = appendMsg('assistant', '', true, new Date());
      let acc = '';
      const seq = await streamSse(res, (chunk, seq) => {
        // A2: 只把 body_chunk 渲染到正文。think_chunk（心理独白，应折叠）和
        // action_options（选项数组，content 是对象不是 string）都不应混入 body。
        if (chunk.type === 'body_chunk' && chunk.text) {
          acc += chunk.text;
          msgEl.textContent = acc;
          if (seq % 5 === 0) {
            logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'chunk#' + seq);
          }
        }
      });
      msgEl.classList.remove('streaming');
      // 流式期间用 textContent（保 cursor 动画 + 性能），完成后切 markdown innerHTML。
      if (acc) msgEl.innerHTML = renderMarkdown(acc);
      logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'done/' + seq + 'chunks');
    } catch (e) {
      if (msgEl) {
        msgEl.classList.remove('streaming');
        // 异常时若已有部分内容，仍切 markdown；否则保留 raw 错误文本
        if (acc) msgEl.innerHTML = renderMarkdown(acc);
      }
      if (e.name === 'AbortError') {
        logEvent('SSE', '/v1/chat/completions', 0, Math.round(performance.now() - t0), 'aborted');
        return;
      }
      if (e.kind === 'stream_interrupt') {
        logEvent('SSE', '/v1/chat/completions', 0, Math.round(performance.now() - t0), 'stream interrupted: ' + e.message);
        appendMsg('assistant', '[stream interrupted: engine disconnected] ' + e.message, false, new Date());
        return;
      }
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), e.message);
      appendMsg('assistant', '[fetch error] ' + e.message, false, new Date());
    } finally {
      // 只在全局仍是当前实例时清理，避免被旧请求 finally 误清新请求状态（race）
      if (abortController === ac) {
        abortController = null;
        if (btnStop) btnStop.hidden = true;
      }
    }
  }

  function buildChatPayload(text) {
    return buildSessionPayload({
      user_profile: { name: 'User', variables: {} },
      message: text,
    });
  }

  function buildSessionPayload(extra) {
    const payload = { character_id: selectedChar, ...(extra || {}) };
    if (selectedSess) payload.session_id = selectedSess;
    return payload;
  }

  async function streamSse(res, onChunk) {
    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let buf = '';
    let seq = 0;
    let sawDone = false;
    while (!sawDone) {
      let done, value;
      try {
        ({ done, value } = await reader.read());
      } catch (e) {
        // 主动 abort（用户取消 / timeout）保持原语义，向上抛 AbortError。
        if (e && e.name === 'AbortError') throw e;
        // 网络中途断开（reader.read 抛 TypeError: network error 等）转 typed error，
        // 让调用方区分「engine 断连」vs「主动取消」vs「其他 fetch error」（issue #47）。
        const err = new Error(e && e.message ? e.message : 'stream interrupted');
        err.kind = 'stream_interrupt';
        throw err;
      }
      if (done) break;
      buf += decoder.decode(value, { stream: true });
      const lines = buf.split('\n');
      buf = lines.pop() || '';
      for (const line of lines) {
        if (!line.startsWith('data: ')) continue;
        const data = line.slice(6).trim();
        if (data === '[DONE]') {
          sawDone = true;
          break;
        }
        try {
          const chunk = JSON.parse(data);
          seq++;
          onChunk(chunk, seq);
        } catch {}
      }
    }
    return seq;
  }

  btnSend.addEventListener('click', doSend);
  chatInput.addEventListener('keydown', e => { if (e.ctrlKey && e.key === 'Enter') doSend(); });

  // ── history / regen / rollback (P1: destructive confirm) ────────────────
  // loadHistory 抽出来供 charSelect/sessSelect 切换时自动复用，避免用户每次手点。
  // #73 方案A: 同时显示会话时间范围（created_at → updated_at），让用户能识别
  // 历史会话的"新鲜度"，避免误把陈旧会话当作活跃会话继续聊。
  function formatSessionTime(iso) {
    // ISO 8601 → 本地可读格式 "YYYY-MM-DD HH:MM"。无效输入返回空串。
    if (!iso || typeof iso !== 'string') return '';
    const d = new Date(iso);
    if (Number.isNaN(d.getTime())) return '';
    const yyyy = d.getFullYear();
    const mm = String(d.getMonth() + 1).padStart(2, '0');
    const dd = String(d.getDate()).padStart(2, '0');
    const hh = String(d.getHours()).padStart(2, '0');
    const mi = String(d.getMinutes()).padStart(2, '0');
    return yyyy + '-' + mm + '-' + dd + ' ' + hh + ':' + mi;
  }
  function renderSessionInfo(data) {
    const msgs = data?.messages;
    const hasMsgs = Array.isArray(msgs) && msgs.length > 0;
    const created = formatSessionTime(data?.created_at);
    const updated = formatSessionTime(data?.updated_at);
    if (!hasMsgs) {
      // 空会话：不显示时间条，避免在新建角色/会话时占用顶部空间。
      return null;
    }
    const div = document.createElement('div');
    div.className = 'session-info';
    let text;
    if (created && updated && created.slice(0, 10) === updated.slice(0, 10)) {
      // 同一天：只显示一次日期 + 时间范围
      text = '会话时间：' + created + ' → ' + updated.slice(11);
    } else if (created && updated) {
      text = '会话时间：' + created + ' → ' + updated;
    } else if (updated) {
      text = '会话时间：' + updated;
    } else {
      // 时间戳缺失（异常路径）：退化为消息数提示，不阻断渲染。
      text = '会话消息数：' + msgs.length;
    }
    div.textContent = text;
    return div;
  }
  async function loadHistory() {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/chat/history', buildSessionPayload());
    if (r.ok) {
      const data = r.data && typeof r.data === 'object' ? r.data : {};
      const msgs = data.messages || r.data || [];
      // #73 方案 B：消息级时间戳（与 messages 一一对应）。旧会话可能无 ts → null。
      const tss = Array.isArray(data.message_timestamps) ? data.message_timestamps : [];
      // A-3：长度不匹配是 engine bug，显式 warn 暴露而非静默降级
      if (tss.length !== msgs.length) {
        console.warn('engine bug: message_timestamps.length (' + tss.length + ') !== messages.length (' + msgs.length + ')');
      }
      chatLog.innerHTML = '';
      const info = renderSessionInfo(data);
      if (info) chatLog.appendChild(info);
      msgs.forEach((m, i) => {
        const tsRaw = tss[i];
        const ts = tsRaw ? new Date(tsRaw) : null;
        appendMsg(m.role || 'assistant', m.text || m.content || '', false, ts);
      });
    } else if (r.status === 404) {
      // #68 #8：404 = 角色无 history（engine #67 #4 已改 NotFound），视为无内容而非错误，
      // 静默清空 chatLog 避免用户无操作却见 [history err 404]。仅渲染 session info（如有）。
      chatLog.innerHTML = '';
    } else if (r.status !== 0) {
      // 0 = 网络层失败（已 logEvent），其它状态码显式提示
      appendMsg('assistant', '[history err ' + r.status + '] ' + formatError(r.data, r.text), false, new Date());
    }
  }

  btnHistory.addEventListener('click', loadHistory);

  btnRegen.addEventListener('click', async () => {
    if (!selectedChar) return;
    if (!window.confirm('Regenerate 会重写/删除最后一条 assistant 消息，不可撤销。继续？')) return;
    const r = await api('POST', '/v1/chat/regen', buildSessionPayload());
    if (r.ok) loadHistory();
  });

  btnRollback.addEventListener('click', async () => {
    if (!selectedChar) return;
    const index = prompt('Rollback to message index (0-based)：\n（将截断该 index 之后的所有消息，不可撤销）', '0');
    if (index === null) return;
    const idx = parseInt(index);
    if (Number.isNaN(idx) || idx < 0) { logEvent('POST', '/v1/chat/rollback', 0, 0, 'illegal index: ' + index); return; }
    if (!window.confirm('确认截断到 index ' + idx + '？此操作不可撤销。')) return;
    const r = await api('POST', '/v1/chat/rollback', buildSessionPayload({ message_index: idx }));
    if (r.ok) loadHistory();
  });

  // ── agent run (P1: classified event log + collapsible raw JSON) ─────────
  const agentStepCounter = $('#agent-step-counter');
  const agentMaxSteps = $('#agent-max-steps');
  const btnAgentClear = $('#btn-agent-clear');

  const AGENT_TYPE_LABEL = {
    plan: 'PLAN',
    tool_call: 'TOOL_CALL',
    tool_result: 'TOOL_RESULT',
    delta: 'DELTA',
    done: 'DONE',
  };
  // 上限 20 仅为防御性 UX cap；引擎本身无此限制（u32::MAX）。
  const AGENT_MAX_STEPS_CAP = 20;
  // stop_reason snake_case → 人类可读标签
  const STOP_REASON_LABEL = {
    converged: 'converged',
    step_cap: 'step cap reached',
    token_budget: 'token budget exhausted',
    wall_clock: 'wall clock timeout',
    cancelled: 'cancelled',
    upstream_error: 'upstream error',
  };
  const AGENT_TYPE_CLASS = {
    plan: 'ev-plan',
    tool_call: 'ev-tool',
    tool_result: 'ev-result',
    delta: 'ev-delta',
    done: 'ev-done',
  };

  function summarizeAgentEvent(chunk) {
    // 返回 {label, summary, isDone}；summary 是人类可读的一行
    // PlanAction 是 #[serde(rename_all = "snake_case")]，故 JSON 里是
    //   {"action":"generate"} / {"action":"finish"}
    //   {"action":{"call_tool":{"tool","params"}}}
    const t = chunk.type;
    if (t === 'plan') {
      const action = chunk.action;
      if (action && typeof action === 'object' && action.call_tool) {
        return { label: 'PLAN', summary: 'step ' + chunk.step + ' → call ' + action.call_tool.tool };
      }
      if (action === 'generate') return { label: 'PLAN', summary: 'step ' + chunk.step + ' → generate' };
      if (action === 'finish') return { label: 'PLAN', summary: 'step ' + chunk.step + ' → finish' };
      return { label: 'PLAN', summary: 'step ' + chunk.step };
    }
    if (t === 'tool_call') return { label: 'TOOL_CALL', summary: 'step ' + chunk.step + ' · ' + chunk.tool };
    if (t === 'tool_result') return { label: 'TOOL_RESULT', summary: 'step ' + chunk.step + ' · ' + chunk.tool };
    if (t === 'delta') return { label: 'DELTA', summary: 'step ' + chunk.step + ' · ' + (chunk.chunk || '').slice(0, 60) };
    if (t === 'done') return { label: 'DONE', summary: chunk.stop_reason + ' · steps=' + chunk.steps_taken + ' · tokens~' + chunk.tokens_estimated, isDone: true };
    return { label: (t || 'EVENT').toUpperCase(), summary: '' };
  }

  // agent output DOM 上限：长跑累积可膨胀，封顶防回流压力（issue F）
  const AGENT_OUTPUT_MAX_ROWS = 500;

  function appendAgentEvent(chunk) {
    const info = summarizeAgentEvent(chunk);
    const row = document.createElement('div');
    const cls = AGENT_TYPE_CLASS[chunk.type] || '';
    row.className = cls ? 'agent-ev ' + cls : 'agent-ev';
    appendInline(row, 'span', 'ev-label', info.label);
    row.append(' ');
    appendInline(row, 'span', 'ev-summary', info.summary || '');
    // 折叠 raw JSON，summary 带事件类型提示方便长流扫读（issue O）
    const details = document.createElement('details');
    details.className = 'ev-raw';
    const summary = document.createElement('summary');
    summary.textContent = 'raw (' + info.label.toLowerCase() + ')';
    details.appendChild(summary);
    const pre = document.createElement('pre');
    pre.className = 'mono';
    pre.textContent = JSON.stringify(chunk, null, 2);
    details.appendChild(pre);
    row.appendChild(details);
    agentOutput.appendChild(row);
    // DOM 上限：超则删最早行
    while (agentOutput.children.length > AGENT_OUTPUT_MAX_ROWS) {
      agentOutput.removeChild(agentOutput.firstChild);
    }
    agentOutput.scrollTop = agentOutput.scrollHeight;
    return info;
  }

  // agent run 客户端超时（30s；agent loop 比单轮 chat 慢，给宽点）
  const AGENT_RUN_TIMEOUT_MS = 30000;

  btnAgentRun.addEventListener('click', async () => {
    const input = agentInput.value.trim();
    if (!input || !selectedChar) return;
    // 二次点击先 abort 前一个 run，防 SSE 事件交错竞态（与 chat send 路径对齐）
    if (agentAbort) agentAbort.abort();
    agentAbort = new AbortController();
    const timeoutTimer = setTimeout(() => agentAbort.abort(), AGENT_RUN_TIMEOUT_MS);
    agentOutput.innerHTML = '';
    agentStepCounter.textContent = 'running…';
    const path = '/v1/agent/run';
    const t0 = performance.now();
    let stepCount = 0;
    let lastDone = null;
    try {
      const maxSteps = Math.max(1, Math.min(AGENT_MAX_STEPS_CAP, parseInt(agentMaxSteps.value, 10) || 3));
      const res = await fetch(base + path, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify({ ...buildChatPayload(input), max_steps: maxSteps }),
        signal: agentAbort.signal,
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', path, res.status, Math.round(performance.now() - t0), errBody);
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[HTTP ' + res.status + '] ' + errBody;
        agentOutput.appendChild(row);
        agentStepCounter.textContent = 'http ' + res.status;
        return;
      }
      const seq = await streamSse(res, (chunk, seq) => {
        // 防畸形 chunk：SSE 解析已 try/catch JSON.parse，但未知 type 走 fallback
        if (!chunk || typeof chunk !== 'object') {
          logEvent('SSE', path, 200, Math.round(performance.now() - t0), '#' + seq + ' invalid chunk');
          return;
        }
        const info = appendAgentEvent(chunk);
        if (chunk.type === 'plan') {
          stepCount = chunk.step;
          agentStepCounter.textContent = 'step ' + stepCount + ' · ' + seq + ' events · ' + Math.round(performance.now() - t0) + 'ms';
        }
        if (info.isDone) lastDone = chunk;
        logEvent('SSE', path, 200, Math.round(performance.now() - t0), '#' + seq + ' ' + info.label + (info.summary ? ' ' + info.summary : ''));
      });
      const ms = Math.round(performance.now() - t0);
      logEvent('SSE', path, 200, ms, 'done/' + seq + 'events');
      agentStepCounter.textContent = lastDone
        ? STOP_REASON_LABEL[lastDone.stop_reason] + ' · ' + lastDone.steps_taken + ' steps · ' + ms + 'ms'
        : (stepCount ? 'step ' + stepCount + ' · ' : '') + seq + ' events · ' + ms + 'ms';
    } catch (e) {
      if (e.name === 'AbortError') {
        logEvent('SSE', path, 0, Math.round(performance.now() - t0), 'aborted/timeout');
        agentStepCounter.textContent = stepCount ? 'aborted at step ' + stepCount : 'aborted';
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[aborted]';
        agentOutput.appendChild(row);
      } else if (e.kind === 'stream_interrupt') {
        logEvent('SSE', path, 0, Math.round(performance.now() - t0), 'stream interrupted: ' + e.message);
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[stream interrupted: engine disconnected] ' + e.message;
        agentOutput.appendChild(row);
        agentStepCounter.textContent = stepCount ? 'interrupted at step ' + stepCount : 'stream interrupted';
      } else {
        logEvent('POST', path, 0, Math.round(performance.now() - t0), e.message);
        const row = document.createElement('div');
        row.className = 'agent-ev ev-err';
        row.textContent = '[fetch error] ' + e.message;
        agentOutput.appendChild(row);
        agentStepCounter.textContent = 'fetch error';
      }
    } finally {
      clearTimeout(timeoutTimer);
      agentAbort = null;
    }
  });

  if (btnAgentClear) btnAgentClear.addEventListener('click', () => {
    agentOutput.textContent = '—';
    agentStepCounter.textContent = '';
  });

  // ── clear log ────────────────────────────────────────────────────────────
  btnClearLog.addEventListener('click', () => { eventLog.innerHTML = ''; });

  // ── M3: import via multipart/base64 (NEVER card_path) ───────────────────
  // 审计裁定：Web 永不走 card_path（RR-001）。浏览器读文件 → base64 →
  // card_png_base64 或 card_json。engine 侧 AIRP_ALLOW_LOCAL_PATH 未设时
  // card_path 被拒，此为纵深防御；WebUI 自身亦不发 card_path。
  const importFile = $('#import-file');
  const btnImport = $('#btn-import');
  const importResult = $('#import-result');

  btnImport.addEventListener('click', async () => {
    const file = importFile.files[0];
    if (!file) { importResult.textContent = '请先选文件'; return; }
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    const isPng = bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4E && bytes[3] === 0x47;
    // C3 fix: 客户端 size gate。engine import 路由 body limit = 10MB
    // (daemon/mod.rs:200 DefaultBodyLimit::max(10 * 1024 * 1024))。
    // 按路径动态计算最终 body 长度，零误差拦截：
    //   PNG 路径: body = {"card_png_base64":"<b64>"}，外壳 22 字节 + base64 膨胀 4/3
    //   JSON 路径: body = {"card_json":"<text>"}，外壳 16 字节，无 base64 膨胀
    // A2 fix: 改为动态精确计算，替代之前的 7.5MB 固定近似阈值（原阈值比安全值大 18 字节）。
    const ENGINE_BODY_LIMIT = 10 * 1024 * 1024; // 10 MB
    const PNG_WRAPPER = 22; // {"card_png_base64":""}
    const JSON_WRAPPER = 16; // {"card_json":""}
    const b64Len = isPng ? Math.ceil(file.size / 3) * 4 : 0;
    const wrapperLen = isPng ? PNG_WRAPPER : JSON_WRAPPER;
    const estBodyLen = (isPng ? b64Len : file.size) + wrapperLen;
    if (estBodyLen > ENGINE_BODY_LIMIT) {
      const limitMB = (ENGINE_BODY_LIMIT / 1024 / 1024).toFixed(1);
      importResult.textContent = '✗ 文件过大（' + (file.size / 1024 / 1024).toFixed(1) + 'MB' + (isPng ? ' PNG → base64 ' + (b64Len / 1024 / 1024).toFixed(1) + 'MB' : '') + '），上限 ' + limitMB + 'MB body';
      return;
    }
    importResult.textContent = '上传中…';
    let body;
    if (isPng) {
      const b64 = await readFileAsBase64(file);
      body = { card_png_base64: b64 };
    } else {
      // treat as JSON text
      const text = new TextDecoder().decode(bytes);
      body = { card_json: text };
    }
    const r = await api('POST', '/v1/characters/import', body);
    if (r.ok) {
      importResult.textContent = '✓ 导入成功: ' + (r.data?.character_id || '?');
      refreshChars();
    } else {
      importResult.textContent = '✗ ' + (r.status || 'err') + ': ' + formatError(r.data, r.text);
    }
  });

  function readFileAsBase64(file) {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const value = String(reader.result || '');
        resolve(value.includes(',') ? value.slice(value.indexOf(',') + 1) : value);
      };
      reader.onerror = () => reject(reader.error || new Error('file read failed'));
      reader.readAsDataURL(file);
    });
  }

  // ── M2: concurrent stream test ───────────────────────────────────────────
  // 启动两条并发 chat.send，验证 id-keyed chat state 不串扰（PR #6 修的 race）。
  const btnConcurrent = $('#btn-concurrent');
  const concurrentStatus = $('#concurrent-status');

  btnConcurrent.addEventListener('click', async () => {
    if (!selectedChar) { concurrentStatus.textContent = '请先选角色'; return; }
    concurrentStatus.textContent = '启动两条并发流…';
    const t0 = performance.now();
    const promises = [
      doSendText('并发流 A: 你好'),
      doSendText('并发流 B: 再见'),
    ];
    const results = await Promise.all(promises);
    const ms = Math.round(performance.now() - t0);
    const ok = results.every(r => r.ok);
    logEvent('CONCURRENT', '/v1/chat/completions ×2', ok ? 200 : 500, ms, ok ? '两条流均完成' : JSON.stringify(results));
    // #68 #9：补真断言——读 engine 端 /v1/chat/history 验证持久化顺序与串扰保护，
    // 不只看 results.every。回归 PR #6 的 id-keyed race 修复（原测试只校 ok flag，
    // 断言被悄悄废掉）。失败时显式列出 mismatch 供诊断。
    let assertion = '';
    if (ok) {
      try {
        const h = await api('POST', '/v1/chat/history', { character_id: selectedChar });
        if (h.ok) {
          const data = h.data && typeof h.data === 'object' ? h.data : {};
          const msgs = Array.isArray(data.messages) ? data.messages : [];
          const expectN = 4;
          if (msgs.length !== expectN) {
            assertion = '✗ history msgs=' + msgs.length + ' 期望 ' + expectN;
          } else {
            // 审计 CR2：engine chat_completion 先写 user 后写 assistant，并发两请求
            // 写入顺序非确定——可能产 [u,u,a,a]（两 user 先写两 assistant 后写）也合法。
            // 不硬校交替，只校"2 user + 2 assistant + 内容匹配 A/B"。
            const roles = msgs.map(m => m.role || 'assistant');
            const userCount = roles.filter(r => r === 'user').length;
            const asstCount = roles.filter(r => r === 'assistant').length;
            const roleOk = userCount === 2 && asstCount === 2;
            const userMsgs = msgs.filter(m => (m.role || 'assistant') === 'user');
            const hasA = userMsgs.some(m => (m.text || m.content || '').includes('并发流 A'));
            const hasB = userMsgs.some(m => (m.text || m.content || '').includes('并发流 B'));
            const contentOk = hasA && hasB;
            if (!roleOk) assertion = '✗ role 计数错: user=' + userCount + ' assistant=' + asstCount + ' 期望各 2';
            else if (!contentOk) assertion = '✗ user 内容不匹配 A/B';
            else assertion = '✓ history 含 2 user + 2 assistant，内容匹配 A/B，无串扰';
          }
        } else {
          assertion = '⚠ history 读取失败 ' + h.status + '（无法校持久化）';
        }
      } catch (e) { assertion = '⚠ history 读取异常: ' + e.message; }
    }
    concurrentStatus.textContent = (ok ? '✓' : '✗') + ' 两条流完成 (' + ms + 'ms)。' + (assertion || '检查 chat log 顺序：u-A → a-A → u-B → a-B 应基本交替，无串扰。');
  });

  // 抽出 doSend 的纯逻辑供并发复用（不发 user DOM、不读 input）
  async function doSendText(text) {
    if (!selectedChar) return { ok: false, status: 0, error: 'no character' };
    appendMsg('user', text, false, new Date());
    const url = base + '/v1/chat/completions';
    const t0 = performance.now();
    let msgEl = null;
    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify(buildChatPayload(text)),
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', '/v1/chat/completions', res.status, Math.round(performance.now() - t0), errBody);
        return { ok: false, status: res.status, error: errBody };
      }
      msgEl = appendMsg('assistant', '', true, new Date());
      let acc = '';
      const seq = await streamSse(res, (chunk) => {
        // A2: 同 doSend，只渲染 body_chunk；think_chunk / action_options 不混入 body。
        if (chunk.type === 'body_chunk' && chunk.text) {
          acc += chunk.text;
          msgEl.textContent = acc;
        }
      });
      msgEl.classList.remove('streaming');
      if (acc) msgEl.innerHTML = renderMarkdown(acc);
      return { ok: true, status: 200, chunks: seq };
    } catch (e) {
      if (msgEl) {
        msgEl.classList.remove('streaming');
        if (acc) msgEl.innerHTML = renderMarkdown(acc);
      }
      const interrupted = e.kind === 'stream_interrupt';
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), (interrupted ? 'stream interrupted: ' : '') + e.message);
      return { ok: false, status: 0, error: e.message, kind: interrupted ? 'stream_interrupt' : undefined };
    }
  }

  // ── P1: one-click diagnostics ────────────────────────────────────────────
  // 依次跑 version/settings/models，输出可复制的诊断摘要。
  // 不发真实 chat/agent run（避免消耗 provider quota）；只验证后端可达性。
  const btnDiag = $('#btn-diag');
  const btnDiagCopy = $('#btn-diag-copy');
  const diagOutput = $('#diag-output');
  let lastDiagText = '';

  // 诊断专用：带 timeout 的 api 包装。engine 卡死时 fail-fast 而非永悬
  // （诊断的本职就是探 engine 卡死，自己不能跟着卡）。
  // 用 AbortController 真切断 fetch，而非 Promise.race 留 fetch 悬跑。
  async function diagApi(method, path, timeoutMs = 5000) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    const t0 = performance.now();
    try {
      const url = base + path;
      const res = await fetch(url, { method, headers: headers(), signal: ctrl.signal });
      const text = await res.text();
      const ms = Math.round(performance.now() - t0);
      logEvent(method, path, res.status, ms);
      let data;
      try { data = JSON.parse(text); } catch { data = text; }
      return { ok: res.ok, status: res.status, data, text, ms };
    } catch (e) {
      const ms = Math.round(performance.now() - t0);
      const aborted = e.name === 'AbortError';
      const msg = aborted ? 'timeout after ' + timeoutMs + 'ms' : e.message;
      logEvent(method, path, 0, ms, msg);
      return { ok: false, status: 0, data: null, text: msg, ms };
    } finally {
      clearTimeout(timer);
    }
  }

  async function runDiagnostics() {
    if (!base) { diagOutput.textContent = '请先连接 engine'; return; }
    diagOutput.textContent = '诊断中…';
    const lines = [];
    lines.push('=== AIRP Engine Diagnostics ===');
    lines.push('time: ' + new Date().toISOString());
    lines.push('engine_url: ' + base);
    lines.push('bearer: ' + (bearer ? '(set, len=' + bearer.length + ')' : '(empty — engine 无鉴权或未配 bearer)'));
    lines.push('');

    // 1. version
    {
      const r = await diagApi('GET', '/version');
      lines.push('[1] GET /version  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) lines.push('    name=' + (r.data?.name || '?') + ' version=' + (r.data?.version || r.text || '?'));
      else lines.push('    err: ' + formatError(r.data, r.text));
    }
    // 1b. health
    {
      const r = await diagApi('GET', '/health');
      lines.push('[1b] GET /health  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) {
        const h = r.data || {};
        lines.push('    engine=' + (h.engine || '?') + ' provider_configured=' + (h.provider_configured ? 'true' : 'false') + ' data_root_writable=' + (h.data_root_writable ? 'true' : 'false'));
      } else {
        lines.push('    err: ' + formatError(r.data, r.text));
      }
    }
    // 2. settings
    {
      const r = await diagApi('GET', '/v1/settings');
      lines.push('[2] GET /v1/settings  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) {
        const s = r.data || {};
        const hasApiKey = !!(s.api_key && String(s.api_key).length);
        const hasAccessKey = !!(s.access_api_key && String(s.access_api_key).length);
        lines.push('    endpoint=' + (s.endpoint || '(unset)'));
        lines.push('    model=' + (s.model || '(unset)'));
        lines.push('    api_key=' + (hasApiKey ? '(set)' : '(MISSING — provider 调用会失败)'));
        lines.push('    access_api_key=' + (hasAccessKey ? '(set — 需 bearer)' : '(empty — 无鉴权)'));
      } else {
        lines.push('    err: ' + formatError(r.data, r.text));
      }
    }
    // 3. models (provider smoke)
    {
      const r = await diagApi('GET', '/v1/models');
      lines.push('[3] GET /v1/models  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) {
        const models = Array.isArray(r.data?.data) ? r.data.data.map(m => m.id) : null;
        lines.push('    models: ' + (models ? models.length + ' 个 → ' + models.slice(0, 5).join(', ') + (models.length > 5 ? ' …' : '') : JSON.stringify(r.data).slice(0, 80)));
      } else {
        lines.push('    err: ' + formatError(r.data, r.text));
      }
    }
    // 4. characters
    {
      const r = await diagApi('GET', '/v1/characters');
      lines.push('[4] GET /v1/characters  → ' + r.status + ' (' + r.ms + 'ms)');
      if (r.ok) lines.push('    count=' + (Array.isArray(r.data) ? r.data.length : 0));
      else lines.push('    err: ' + formatError(r.data, r.text));
    }
    lines.push('');
    lines.push('=== End ===');
    lastDiagText = lines.join('\n');
    diagOutput.textContent = lastDiagText;
  }

  if (btnDiag) btnDiag.addEventListener('click', runDiagnostics);
  if (btnDiagCopy) btnDiagCopy.addEventListener('click', async () => {
    if (!lastDiagText) { diagOutput.textContent = '先点「一键诊断」'; return; }
    try {
      await navigator.clipboard.writeText(lastDiagText);
      diagOutput.textContent = lastDiagText + '\n\n[已复制到剪贴板]';
    } catch {
      diagOutput.textContent = lastDiagText + '\n\n[剪贴板不可用，请手动选中复制]';
    }
  });

  // ── Workbench（角色卡 + 世界书编辑，PR F）─────────────────────────────────
  // 用户需求：导入角色卡后，点击「工作台」进入编辑视图，可改角色卡和世界书，
  // 然后在右侧 session 区新建对话。工作台是 overlay 面板，不挡 chat。
  const workbenchPanel = $('#workbench-panel');
  const wbCharName = $('#wb-char-name');
  const wbCardFields = $('#wb-card-fields');
  const wbLoreEntries = $('#wb-lore-entries');
  const wbCardStatus = $('#wb-card-status');
  const wbLoreStatus = $('#wb-lore-status');
  const btnWorkbench = $('#btn-workbench');
  const btnReextract = $('#btn-reextract');
  const btnWbClose = $('#btn-wb-close');
  const btnWbSaveCard = $('#btn-wb-save-card');
  const btnWbSaveLore = $('#btn-wb-save-lore');
  const btnWbAddLore = $('#btn-wb-add-lore');
  const wbDirtyDot = $('#wb-dirty-dot');

  // 当前工作台缓存的角色卡 / 世界书数据
  let wbCardData = null;
  let wbLoreData = null;
  let wbDirty = false;

  // 角色卡可编辑字段（TavernCardV2 data 层）
  const CARD_FIELDS = [
    { key: 'name', label: '名称', type: 'input' },
    { key: 'description', label: '描述', type: 'textarea', tall: true },
    { key: 'personality', label: '性格', type: 'textarea' },
    { key: 'scenario', label: '场景', type: 'textarea' },
    { key: 'first_mes', label: '开场白', type: 'textarea', tall: true },
    { key: 'system_prompt', label: '系统提示词', type: 'textarea', tall: true },
    { key: 'mes_example', label: '对话示例', type: 'textarea', tall: true },
  ];

  function openWorkbench() {
    if (!selectedChar) {
      alert('请先选择一个角色');
      return;
    }
    workbenchPanel.hidden = false;
    wbCharName.textContent = selectedChar;
    wbCardStatus.textContent = '加载中…';
    wbLoreStatus.textContent = '加载中…';
    setWbDirty(false);  // 切角色打开时清 dirty，避免上次残留
    loadWorkbenchCard();
    loadWorkbenchLorebook();
  }

  function setWbDirty(dirty) {
    wbDirty = dirty;
    if (wbDirtyDot) wbDirtyDot.hidden = !dirty;
  }

  function closeWorkbench() {
    if (wbDirty) {
      if (!confirm('工作台有未保存修改，关闭后修改将丢失。确定关闭？')) return;
    }
    workbenchPanel.hidden = true;
    wbCardData = null;
    wbLoreData = null;
    setWbDirty(false);
  }

  async function loadWorkbenchCard() {
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar));
    if (r.ok) {
      wbCardData = r.data;
      renderCardFields();
      wbCardStatus.textContent = '已加载（修改后点保存写回）';
    } else {
      wbCardStatus.textContent = '加载失败: ' + formatError(r.data, r.text);
      wbCardFields.innerHTML = '';
    }
  }

  function renderCardFields() {
    if (!wbCardData) return;
    const data = wbCardData.data || wbCardData;
    wbCardFields.innerHTML = '';

    // name 单独一行
    const nameField = CARD_FIELDS.find(f => f.key === 'name');
    if (nameField) wbCardFields.appendChild(makeCardField(nameField, data));

    // 提示词类字段分组
    const promptFields = CARD_FIELDS.filter(f => f.key !== 'name');
    if (promptFields.length) {
      const group = document.createElement('div');
      group.className = 'wb-group';
      const title = document.createElement('div');
      title.className = 'wb-group-title';
      title.textContent = '提示词与背景';
      group.appendChild(title);
      for (const f of promptFields) group.appendChild(makeCardField(f, data));
      wbCardFields.appendChild(group);
    }
  }

  function makeCardField(f, data) {
    const wrap = document.createElement('div');
    wrap.className = 'wb-field';
    const lbl = document.createElement('label');
    lbl.textContent = f.label;
    const el = document.createElement(f.type === 'textarea' ? 'textarea' : 'input');
    if (f.type === 'textarea' && f.tall) el.classList.add('wb-tall');
    el.value = data[f.key] || '';
    el.dataset.field = f.key;
    el.addEventListener('input', () => setWbDirty(true));
    wrap.appendChild(lbl);
    wrap.appendChild(el);
    return wrap;
  }

  async function saveWorkbenchCard() {
    if (!wbCardData || !selectedChar) return;
    const data = wbCardData.data || wbCardData;
    // A-01 修复：保存前深拷贝表单原始值。保存失败时恢复 wbCardData，
    // 让用户能「回到上次保存的版本」而不是带着失败的 mutation。
    const snapshot = JSON.parse(JSON.stringify(wbCardData));
    // 收集表单值
    for (const f of CARD_FIELDS) {
      const el = wbCardFields.querySelector('[data-field="' + f.key + '"]');
      if (el) data[f.key] = el.value;
    }
    wbCardStatus.textContent = '保存中…';
    btnWbSaveCard.disabled = true;
    const r = await api('PUT', '/v1/characters/' + encodeURIComponent(selectedChar), wbCardData);
    btnWbSaveCard.disabled = false;
    if (r.ok) {
      wbCardStatus.textContent = '已保存 ✓';
      setWbDirty(false);
      refreshAvatar();
    } else {
      // 保存失败：恢复 wbCardData 到保存前状态，并把表单字段也回滚
      wbCardData = snapshot;
      const data2 = wbCardData.data || wbCardData;
      for (const f of CARD_FIELDS) {
        const el = wbCardFields.querySelector('[data-field="' + f.key + '"]');
        if (el) el.value = data2[f.key] || '';
      }
      wbCardStatus.textContent = '保存失败: ' + formatError(r.data, r.text);
    }
  }

  async function loadWorkbenchLorebook() {
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar) + '/lorebook');
    if (r.status === 404) {
      wbLoreData = { entries: [] };
      renderLoreEntries();
      wbLoreStatus.textContent = '该角色尚无世界书（可新建条目后保存）';
    } else if (r.ok) {
      wbLoreData = r.data;
      if (!wbLoreData.entries) wbLoreData.entries = [];
      renderLoreEntries();
      wbLoreStatus.textContent = '已加载 ' + wbLoreData.entries.length + ' 条条目';
    } else {
      wbLoreStatus.textContent = '加载失败: ' + formatError(r.data, r.text);
    }
  }

  function renderLoreEntries() {
    if (!wbLoreData) return;
    wbLoreEntries.innerHTML = '';
    wbLoreData.entries.forEach((entry, i) => {
      wbLoreEntries.appendChild(renderLoreEntry(entry, i));
    });
  }

  function renderLoreEntry(entry, index) {
    const div = document.createElement('div');
    div.className = 'wb-lore-entry collapsed';
    div.dataset.index = String(index);

    const head = document.createElement('div');
    head.className = 'wb-lore-head';

    // index + 展开切换
    const toggle = document.createElement('button');
    toggle.className = 'wb-lore-toggle';
    toggle.textContent = '▸';
    toggle.title = '展开/折叠';
    toggle.addEventListener('click', () => {
      div.classList.toggle('collapsed');
      toggle.textContent = div.classList.contains('collapsed') ? '▸' : '▾';
    });
    head.appendChild(toggle);

    const lbl = document.createElement('span');
    lbl.className = 'wb-lore-index';
    lbl.textContent = '条目 #' + (index + 1);
    head.appendChild(lbl);

    // keys 行内编辑
    const keysInput = document.createElement('input');
    keysInput.className = 'wb-lore-keys';
    keysInput.type = 'text';
    keysInput.value = (entry.keys || []).join(', ');
    keysInput.placeholder = '关键词（逗号分隔）';
    keysInput.addEventListener('input', (e) => {
      entry.keys = e.target.value.split(',').map(s => s.trim()).filter(Boolean);
      setWbDirty(true);
    });
    head.appendChild(keysInput);

    // priority
    const priInput = document.createElement('input');
    priInput.className = 'wb-lore-priority';
    priInput.type = 'number';
    priInput.min = '0';
    priInput.step = '1';
    priInput.value = entry.priority ?? 10;
    priInput.title = '优先级';
    priInput.addEventListener('input', (e) => {
      const v = parseInt(e.target.value, 10);
      entry.priority = isNaN(v) ? 10 : v;
      setWbDirty(true);
    });
    head.appendChild(priInput);

    // enabled
    const enLbl = document.createElement('label');
    enLbl.className = 'wb-lore-enabled';
    const enCb = document.createElement('input');
    enCb.type = 'checkbox';
    enCb.checked = entry.enabled !== false;
    enCb.addEventListener('change', () => {
      entry.enabled = enCb.checked;
      setWbDirty(true);
    });
    enLbl.appendChild(enCb);
    enLbl.appendChild(document.createTextNode('启用'));
    head.appendChild(enLbl);

    // delete
    const del = document.createElement('button');
    del.className = 'wb-lore-del';
    del.textContent = '✕';
    del.title = '删除此条目';
    // delete：只移除该条目 DOM + 数据，不全量重渲染（A-02 修复）
    // 全量重渲染会丢失其他条目的展开/折叠状态与未保存的 input 值。
    del.addEventListener('click', () => {
      wbLoreData.entries.splice(index, 1);
      div.remove();
      // 重编后续条目的序号显示（dataset.index + lbl 文本），保持视觉一致
      wbLoreEntries.querySelectorAll('.wb-lore-entry').forEach((e, i) => {
        e.dataset.index = String(i);
        const lbl = e.querySelector('.wb-lore-index');
        if (lbl) lbl.textContent = '条目 #' + (i + 1);
      });
      setWbDirty(true);
    });
    head.appendChild(del);

    div.appendChild(head);

    // body
    const body = document.createElement('div');
    body.className = 'wb-lore-body';

    const contentTa = document.createElement('textarea');
    contentTa.placeholder = '注入内容';
    contentTa.value = entry.content || '';
    contentTa.addEventListener('input', (e) => {
      entry.content = e.target.value;
      setWbDirty(true);
    });
    body.appendChild(contentTa);

    const cmtInput = document.createElement('input');
    cmtInput.className = 'wb-lore-comment';
    cmtInput.type = 'text';
    cmtInput.placeholder = '注释（可选）';
    cmtInput.value = entry.comment || '';
    cmtInput.addEventListener('input', (e) => {
      entry.comment = e.target.value || null;
      setWbDirty(true);
    });
    body.appendChild(cmtInput);

    div.appendChild(body);
    return div;
  }

  function addLoreEntry() {
    if (!wbLoreData) wbLoreData = { entries: [] };
    wbLoreData.entries.push({
      keys: [],
      content: '',
      enabled: true,
      priority: 10,
      comment: null,
    });
    setWbDirty(true);
    renderLoreEntries();
    // 自动滚动到新条目并展开
    const entries = wbLoreEntries.querySelectorAll('.wb-lore-entry');
    const last = entries[entries.length - 1];
    if (last) {
      last.classList.remove('collapsed');
      const toggle = last.querySelector('.wb-lore-toggle');
      if (toggle) toggle.textContent = '▾';
      last.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
  }

  async function saveWorkbenchLore() {
    if (!wbLoreData || !selectedChar) return;
    wbLoreStatus.textContent = '保存中…';
    btnWbSaveLore.disabled = true;
    const r = await api('PUT', '/v1/characters/' + encodeURIComponent(selectedChar) + '/lorebook', wbLoreData);
    btnWbSaveLore.disabled = false;
    if (r.ok) {
      wbLoreStatus.textContent = '已保存 ✓（' + (r.data?.entries_count ?? '?') + ' 条）';
      setWbDirty(false);
    } else {
      wbLoreStatus.textContent = '保存失败: ' + formatError(r.data, r.text);
    }
  }

  // ── Decompose tab：拆解 / analysis 文件列表 / enhance / apply ──────────────
  // A4 修复：所有 analysis MD 内容（来自 LLM 或用户编辑，属 untrusted data）
  // 一律用 textContent 渲染，绝不用 innerHTML 注入。原始内容显示在 <pre>（只读），
  // 增强后内容显示在 <textarea>（可编辑，apply 时写盘）。
  const wbDecomposeStatus = $('#wb-decompose-status');
  const wbAnalysisFiles = $('#wb-analysis-files');
  const wbAnalysisViewer = $('#wb-analysis-viewer');
  const wbAnalysisFilename = $('#wb-analysis-filename');
  const wbAnalysisOriginal = $('#wb-analysis-original');
  const wbAnalysisEnhanced = $('#wb-analysis-enhanced');
  const btnWbDecompose = $('#btn-wb-decompose');
  const btnWbListAnalysis = $('#btn-wb-list-analysis');
  const btnWbAnalysisEnhance = $('#btn-wb-analysis-enhance');
  const btnWbAnalysisApply = $('#btn-wb-analysis-apply');
  const btnWbAnalysisClose = $('#btn-wb-analysis-close');
  const wbDecomposeForce = $('#wb-decompose-force');
  // L4 修复（issue #92）：预设拆解按钮 + 预设选择器
  const btnWbDecomposePreset = $('#btn-wb-decompose-preset');
  const wbPresetSelect = $('#wb-preset-select');
  let wbAnalysisCurrentFilename = null;

  async function decomposeCharacter() {
    if (!selectedChar) return;
    const force = wbDecomposeForce && wbDecomposeForce.checked;
    wbDecomposeStatus.textContent = '拆解中…';
    btnWbDecompose.disabled = true;
    const path = '/v1/characters/' + encodeURIComponent(selectedChar) + '/decompose' + (force ? '?force=true' : '');
    const r = await api('POST', path);
    btnWbDecompose.disabled = false;
    if (r.ok) {
      const data = r.data || {};
      wbDecomposeStatus.textContent = '已拆解 ✓（' + (data.files_written?.length ?? '?') + ' 个文件'
        + (data.lorebook_decomposed ? '，含世界书' : '') + '）';
      loadAnalysisFileList();
    } else {
      wbDecomposeStatus.textContent = '拆解失败: ' + formatError(r.data, r.text);
    }
  }

  async function loadAnalysisFileList() {
    if (!selectedChar) return;
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis');
    wbAnalysisFiles.innerHTML = '';
    if (!r.ok) {
      wbDecomposeStatus.textContent = '加载文件列表失败: ' + formatError(r.data, r.text);
      return;
    }
    const files = (r.data && r.data.files) || [];
    if (!files.length) {
      const empty = document.createElement('div');
      empty.className = 'hint';
      empty.textContent = '尚无 analysis 文件（点"拆解角色卡"生成）';
      wbAnalysisFiles.appendChild(empty);
      return;
    }
    wbDecomposeStatus.textContent = '已加载 ' + files.length + ' 个文件';
    for (const f of files) {
      const row = document.createElement('div');
      row.className = 'wb-analysis-file';
      const name = document.createElement('span');
      name.className = 'wb-af-name';
      // A4 修复：filename 来自服务端但仍是 untrusted，用 textContent
      name.textContent = f.filename;
      const size = document.createElement('span');
      size.className = 'wb-af-size';
      size.textContent = f.size + ' B';
      row.appendChild(name);
      row.appendChild(size);
      row.addEventListener('click', () => loadAnalysisFile(f.filename));
      wbAnalysisFiles.appendChild(row);
    }
  }

  async function loadAnalysisFile(filename) {
    if (!selectedChar) return;
    // filename 已通过服务端白名单校验（[a-z0-9_/.-]+\.md），直接拼接到 URL 路径。
    // 不用 encodeURIComponent——它会编码 / 为 %2F，axum /*filename 通配符不解码。
    const r = await api('GET', '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis/' + filename);
    if (!r.ok) {
      wbDecomposeStatus.textContent = '读文件失败: ' + formatError(r.data, r.text);
      return;
    }
    wbAnalysisCurrentFilename = filename;
    // A4 修复：用 textContent 渲染 untrusted MD 内容，绝不用 innerHTML
    wbAnalysisFilename.textContent = filename;
    wbAnalysisOriginal.textContent = r.data.content || '';
    wbAnalysisEnhanced.value = r.data.content || '';
    btnWbAnalysisApply.hidden = true;
    wbAnalysisViewer.hidden = false;
    wbDecomposeStatus.textContent = '已加载 ' + filename;
  }

  async function enhanceAnalysis() {
    if (!selectedChar || !wbAnalysisCurrentFilename) return;
    wbDecomposeStatus.textContent = '生成 diff 预览中…';
    btnWbAnalysisEnhance.disabled = true;
    const body = { action: 'enhance' };
    const r = await api('POST',
      '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis/' + wbAnalysisCurrentFilename,
      body);
    btnWbAnalysisEnhance.disabled = false;
    if (!r.ok) {
      wbDecomposeStatus.textContent = 'enhance 失败: ' + formatError(r.data, r.text);
      return;
    }
    const data = r.data || {};
    // A4 修复：enhanced_md 来自 LLM/占位，仍是 untrusted，用 .value 设置（textarea）
    wbAnalysisEnhanced.value = data.enhanced_md || '';
    btnWbAnalysisApply.hidden = false;
    wbDecomposeStatus.textContent = data.has_changes
      ? '已生成 diff 预览（有变化，确认后点 Apply 写盘）'
      : '已生成 diff 预览（无变化）';
  }

  async function applyEnhanced() {
    if (!selectedChar || !wbAnalysisCurrentFilename) return;
    const enhanced_md = wbAnalysisEnhanced.value;
    if (!confirm('确认把 enhanced_md 写入 ' + wbAnalysisCurrentFilename + ' ？此操作覆盖原文件。')) return;
    wbDecomposeStatus.textContent = '写盘中…';
    btnWbAnalysisApply.disabled = true;
    const body = { action: 'apply', enhanced_md };
    const r = await api('POST',
      '/v1/characters/' + encodeURIComponent(selectedChar) + '/analysis/' + wbAnalysisCurrentFilename,
      body);
    btnWbAnalysisApply.disabled = false;
    if (r.ok) {
      wbDecomposeStatus.textContent = '已写盘 ✓';
      // 重新加载原文件内容
      wbAnalysisOriginal.textContent = enhanced_md;
      btnWbAnalysisApply.hidden = true;
    } else {
      wbDecomposeStatus.textContent = 'apply 失败: ' + formatError(r.data, r.text);
    }
  }

  function closeAnalysisViewer() {
    wbAnalysisViewer.hidden = true;
    wbAnalysisCurrentFilename = null;
  }

  // L4 修复（issue #92）：预设拆解 — 复用 decomposeCharacter 的 force 选项 + 状态栏
  async function decomposePreset() {
    const presetId = wbPresetSelect.value;
    if (!presetId) {
      wbDecomposeStatus.textContent = '请先选择预设';
      return;
    }
    const force = wbDecomposeForce && wbDecomposeForce.checked;
    wbDecomposeStatus.textContent = '拆解预设中…';
    btnWbDecomposePreset.disabled = true;
    const path = '/v1/presets/' + encodeURIComponent(presetId) + '/decompose' + (force ? '?force=true' : '');
    const r = await api('POST', path);
    btnWbDecomposePreset.disabled = false;
    if (r.ok) {
      const data = r.data || {};
      wbDecomposeStatus.textContent = '预设已拆解 ✓（' + (data.files_written?.length ?? '?')
        + ' 个文件，产物在 presets/' + presetId + '/analysis/）';
    } else {
      wbDecomposeStatus.textContent = '预设拆解失败: ' + formatError(r.data, r.text);
    }
  }

  // 加载预设列表到下拉框（切到拆解 tab 时触发）
  async function loadPresetOptions() {
    const r = await api('GET', '/v1/presets');
    if (!r.ok) {
      // 审计 CR3：仿 loadAnalysisFileList 写失败反馈，避免 UI 静默空下拉框。
      wbDecomposeStatus.textContent = '加载预设列表失败: ' + formatError(r.data, r.text);
      return;
    }
    const presets = Array.isArray(r.data) ? r.data : [];
    // 清空并重建选项（用 textContent 避免 XSS）
    while (wbPresetSelect.firstChild) wbPresetSelect.removeChild(wbPresetSelect.firstChild);
    const placeholder = document.createElement('option');
    placeholder.value = '';
    placeholder.textContent = '— 选择预设 —';
    wbPresetSelect.appendChild(placeholder);
    for (const p of presets) {
      const opt = document.createElement('option');
      opt.value = p;
      opt.textContent = p;
      wbPresetSelect.appendChild(opt);
    }
  }

  // Tab 切换
  $$('.wb-tab').forEach(tab => {
    tab.addEventListener('click', () => {
      $$('.wb-tab').forEach(t => t.classList.remove('active'));
      tab.classList.add('active');
      const target = tab.dataset.tab;
      $('#wb-tab-card').hidden = target !== 'card';
      $('#wb-tab-lorebook').hidden = target !== 'lorebook';
      $('#wb-tab-decompose').hidden = target !== 'decompose';
      if (target === 'decompose') {
        loadAnalysisFileList();
        loadPresetOptions(); // L4：切到拆解 tab 时加载预设列表
      }
    });
  });

  if (btnWorkbench) btnWorkbench.addEventListener('click', openWorkbench);
  if (btnReextract) btnReextract.addEventListener('click', reextractCurrentChar);
  if (btnDeleteChar) btnDeleteChar.addEventListener('click', deleteCurrentChar);
  if (btnWbClose) btnWbClose.addEventListener('click', closeWorkbench);
  if (btnWbSaveCard) btnWbSaveCard.addEventListener('click', saveWorkbenchCard);
  if (btnWbSaveLore) btnWbSaveLore.addEventListener('click', saveWorkbenchLore);
  if (btnWbAddLore) btnWbAddLore.addEventListener('click', addLoreEntry);
  // Decompose tab
  if (btnWbDecompose) btnWbDecompose.addEventListener('click', decomposeCharacter);
  if (btnWbListAnalysis) btnWbListAnalysis.addEventListener('click', loadAnalysisFileList);
  if (btnWbDecomposePreset) btnWbDecomposePreset.addEventListener('click', decomposePreset); // L4
  if (btnWbAnalysisEnhance) btnWbAnalysisEnhance.addEventListener('click', enhanceAnalysis);
  if (btnWbAnalysisApply) btnWbAnalysisApply.addEventListener('click', applyEnhanced);
  if (btnWbAnalysisClose) btnWbAnalysisClose.addEventListener('click', closeAnalysisViewer);

  // ESC 关闭工作台（但 textarea/input 中按 ESC 不关，避免丢失未保存编辑）
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && workbenchPanel && !workbenchPanel.hidden) {
      const tag = (e.target && e.target.tagName) || '';
      if (tag === 'INPUT' || tag === 'TEXTAREA') return;
      closeWorkbench();
    }
  });

  // 工作台面板拖拽调整宽度
  // 兜底：mouseleave/blur 强制 endDrag 防拖到窗口外松不开
  function initWorkbenchResizer() {
    const resizer = $('#workbench-resizer');
    if (!resizer || !workbenchPanel) return;
    let dragging = false;
    let onMove = null, onUp = null;
    const endDrag = () => {
      if (!dragging) return;
      dragging = false;
      document.body.style.userSelect = '';
      if (onMove) window.removeEventListener('mousemove', onMove);
      if (onUp) window.removeEventListener('mouseup', onUp);
      onMove = null; onUp = null;
    };
    resizer.addEventListener('mousedown', (e) => {
      if (dragging) return;
      dragging = true;
      document.body.style.userSelect = 'none';
      const startX = e.clientX;
      const startW = workbenchPanel.offsetWidth;
      onMove = (ev) => {
        if (!dragging) return;
        const delta = startX - ev.clientX;
        const next = Math.min(Math.max(startW + delta, 320), window.innerWidth * 0.65);
        workbenchPanel.style.width = next + 'px';
      };
      onUp = () => endDrag();
      window.addEventListener('mousemove', onMove);
      window.addEventListener('mouseup', onUp);
    });
    // 兜底 1：鼠标离开浏览器窗口
    document.addEventListener('mouseleave', endDrag);
    // 兜底 2：窗口失焦
    window.addEventListener('blur', endDrag);
  }
  initWorkbenchResizer();

  async function reextractCurrentChar() {
    if (!selectedChar) { alert('请先选择一个角色'); return; }
    if (!confirm('重新解包会从当前 card.json 重新生成 world/lorebook.json 和 card/greetings/，确定继续？')) return;
    const r = await api('POST', '/v1/characters/' + encodeURIComponent(selectedChar) + '/reextract');
    if (r.ok) {
      alert('重新解包完成');
      loadWorkbenchLorebook();
    } else {
      alert('重新解包失败: ' + formatError(r.data, r.text));
    }
  }

  async function deleteCurrentChar() {
    if (!selectedChar) { alert('请先选择一个角色'); return; }
    const characterId = selectedChar;
    if (!confirm('删除「' + characterId + '」会永久移除角色卡、状态与全部会话，无法撤销。确定删除？')) return;
    const r = await api('DELETE', '/v1/characters/' + encodeURIComponent(characterId));
    if (!r.ok) {
      alert('删除失败: ' + formatError(r.data, r.text));
      return;
    }
    if (workbenchPanel) workbenchPanel.hidden = true;
    wbCardData = null;
    wbLoreData = null;
    setWbDirty(false);
    selectedChar = '';
    selectedSess = '';
    chatLog.textContent = '';
    await refreshChars();
  }

  // W-05: 停止生成按钮 — 仅在 chat streaming 进行时显示
  if (btnStop) {
    btnStop.addEventListener('click', () => {
      if (abortController) abortController.abort();
    });
  }

  // W-01: 页面加载时从 sessionStorage 恢复连接参数（关 tab 即清）
  try {
    const savedUrl = sessionStorage.getItem('airp_engine_url');
    const savedBearer = sessionStorage.getItem('airp_bearer');
    if (savedUrl) engineUrl.value = savedUrl;
    if (savedBearer) bearerToken.value = savedBearer;
  } catch {}

  // ── auto-connect on load ─────────────────────────────────────────────────
  // #68 #5 fix: 改用 scheduleAutoConnect，用户在 300ms 内输入 URL/bearer 会取消
  scheduleAutoConnect();
})();
