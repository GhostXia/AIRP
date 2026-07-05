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
  const healthInfo = $('#health-info');
  const settingsDisplay = $('#settings-display');
  const modelsDisplay = $('#models-display');
  const btnRefreshModels = $('#btn-refresh-models');
  const charSelect = $('#char-select');
  const sessSelect = $('#sess-select');
  const chatLog = $('#chat-log');
  const chatInput = $('#chat-input');
  const btnSend = $('#btn-send');
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
  function formatError(data, text) {
    if (data && typeof data === 'object' && data.error) {
      const err = data.error;
      const lines = [err.code || 'error'];
      if (err.message) lines.push(err.message);
      if (err.upstream_status) lines.push('upstream_status=' + err.upstream_status);
      if (err.upstream_body) lines.push('upstream_body=' + err.upstream_body);
      if (err.detail) lines.push('detail=' + err.detail);
      return lines.join('\n');
    }
    if (typeof data === 'string' && data) return data;
    if (text) return text;
    return JSON.stringify(data);
  }

  async function connect() {
    base = engineUrl.value.replace(/\/+$/, '');
    bearer = bearerToken.value || '';
    connStatus.className = 'status-dot dot-unknown';
    connText.textContent = '连接中…';
    const r = await api('GET', '/version');
    if (r.ok) {
      connStatus.className = 'status-dot dot-ok';
      connText.textContent = '已连接  ' + (r.data?.version || r.text).slice(0, 40);
      refreshAll();
    } else {
      connStatus.className = 'status-dot dot-err';
      connText.textContent = '连接失败: ' + formatError(r.data, r.text);
    }
  }

  btnConnect.addEventListener('click', connect);
  engineUrl.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });
  bearerToken.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });

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
    } else {
      settingsDisplay.textContent = 'err: ' + formatError(r.data, r.text);
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
      select.appendChild(option);
    });
  }

  btnRefreshChars.addEventListener('click', refreshChars);
  if (btnRefreshModels) btnRefreshModels.addEventListener('click', refreshModels);

  async function refreshSessions() {
    const cid = charSelect.value;
    if (!cid) return;
    selectedChar = cid;
    const r = await api('GET', '/v1/sessions/' + encodeURIComponent(cid));
    if (r.ok) {
      const ids = Array.isArray(r.data) ? r.data.map(s => s.session_id || s) : [];
      replaceOptions(sessSelect, ids, id => id.slice(0, 12));
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
    let s = escapeHtml(text);
    // 1. fenced code blocks ```lang\n...\n``` 抽到占位行
    s = s.replace(/```(\w*)\n?([\s\S]*?)```/g, (_, lang, code) => {
      // lang 暂未用于高亮（避免引第三方）；保留以便未来加 highlight.js
      const i = codeBlocks.push('<pre class="md-code"><code>' + code.replace(/\n$/, '') + '</code></pre>') - 1;
      return '\n\uF8FFCB' + i + '\uF8FF\n';
    });
    // 2. 行内 markdown：inline code / bold / italic 在非占位行上。
    //    占位行只有 \uF8FF 包装，无 ** / ` / * 风险，replace 仍安全。
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
    for (const line of lines) {
      const isBlock = /^(<h[1-3] )/.test(line) || /^\uF8FFCB\d+\uF8FF$/.test(line);
      if (line === '') { flushPara(); continue; }
      if (isBlock) { flushPara(); out.push(line); }
      else { para.push(line); }
    }
    flushPara();
    // 5. 恢复 code blocks (顺序与 codeBlocks.push 一致)
    return out.join('\n').replace(/\uF8FFCB(\d+)\uF8FF/g, (_, i) => codeBlocks[+i]);
  }

  // appendMsg: 流式中用 textContent（保 cursor 动画 + 性能），完成后切 innerHTML 跑 markdown。
  function appendMsg(role, text, isStreaming) {
    const div = document.createElement('div');
    const safeRole = role === 'user' ? 'user' : 'assistant';
    div.className = 'msg ' + safeRole;
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
    appendMsg('user', text, false);

    // create session if none
    if (!selectedSess) {
      const r = await api('POST', '/v1/sessions/' + encodeURIComponent(selectedChar));
      if (!r.ok) { appendMsg('assistant', '[session create failed]', false); return; }
      const newId = extractSessionId(r);
      if (!newId) { appendMsg('assistant', '[session create: empty id]', false); return; }
      selectedSess = newId;
      await refreshSessions();
    }

    // abort prior stream
    if (abortController) abortController.abort();
    abortController = new AbortController();
    const url = base + '/v1/chat/completions';
    const t0 = performance.now();
    let msgEl = null;

    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify(buildChatPayload(text)),
        signal: abortController.signal,
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', '/v1/chat/completions', res.status, Math.round(performance.now() - t0), errBody);
        appendMsg('assistant', '[HTTP ' + res.status + '] ' + errBody, false);
        return;
      }
      msgEl = appendMsg('assistant', '', true);
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
        appendMsg('assistant', '[stream interrupted: engine disconnected] ' + e.message, false);
        return;
      }
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), e.message);
      appendMsg('assistant', '[fetch error] ' + e.message, false);
    }
  }

  function buildChatPayload(text) {
    const payload = {
      character_id: selectedChar,
      user_profile: { name: 'User', variables: {} },
      message: text,
    };
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
  async function loadHistory() {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/chat/history', { character_id: selectedChar });
    if (r.ok) {
      const msgs = r.data?.messages || r.data || [];
      chatLog.innerHTML = '';
      msgs.forEach(m => appendMsg(m.role || 'assistant', m.text || m.content || '', false));
    } else if (r.status !== 0) {
      // 0 = 网络层失败（已 logEvent），其它状态码显式提示
      appendMsg('assistant', '[history err ' + r.status + '] ' + formatError(r.data, r.text), false);
    }
  }

  btnHistory.addEventListener('click', loadHistory);

  btnRegen.addEventListener('click', async () => {
    if (!selectedChar) return;
    if (!window.confirm('Regenerate 会重写/删除最后一条 assistant 消息，不可撤销。继续？')) return;
    const r = await api('POST', '/v1/chat/regen', { character_id: selectedChar });
    if (r.ok) loadHistory();
  });

  btnRollback.addEventListener('click', async () => {
    if (!selectedChar) return;
    const index = prompt('Rollback to message index (0-based)：\n（将截断该 index 之后的所有消息，不可撤销）', '0');
    if (index === null) return;
    const idx = parseInt(index);
    if (Number.isNaN(idx) || idx < 0) { logEvent('POST', '/v1/chat/rollback', 0, 0, 'illegal index: ' + index); return; }
    if (!window.confirm('确认截断到 index ' + idx + '？此操作不可撤销。')) return;
    const r = await api('POST', '/v1/chat/rollback', { character_id: selectedChar, message_index: idx });
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
    importResult.textContent = '上传中…';
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    const isPng = bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4E && bytes[3] === 0x47;
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
    concurrentStatus.textContent = (ok ? '✓' : '✗') + ' 两条流完成 (' + ms + 'ms)。检查 chat log 顺序：u-A → a-A → u-B → a-B 应基本交替，无串扰。';
  });

  // 抽出 doSend 的纯逻辑供并发复用（不发 user DOM、不读 input）
  async function doSendText(text) {
    if (!selectedChar) return { ok: false, status: 0, error: 'no character' };
    appendMsg('user', text, false);
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
      msgEl = appendMsg('assistant', '', true);
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

  // ── auto-connect on load ─────────────────────────────────────────────────
  setTimeout(connect, 300);
})();
