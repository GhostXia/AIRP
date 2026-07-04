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

  // ── state ────────────────────────────────────────────────────────────────
  let base = engineUrl.value.replace(/\/+$/, '');
  let bearer = '';
  let selectedChar = '';
  let selectedSess = '';
  let abortController = null;   // for chat SSE

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
      connText.textContent = '连接失败: ' + (r.data || r.text);
    }
  }

  btnConnect.addEventListener('click', connect);
  engineUrl.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });
  bearerToken.addEventListener('keydown', e => { if (e.key === 'Enter') connect(); });

  // ── refresh all left-panel data ──────────────────────────────────────────
  async function refreshAll() {
    await Promise.all([refreshHealth(), refreshSettings(), refreshChars()]);
  }

  async function refreshHealth() {
    const r = await api('GET', '/version');
    if (r.ok) healthInfo.textContent = 'version: ' + (r.data?.version || r.text);
    else healthInfo.textContent = 'err: ' + (r.data || r.text);
  }

  async function refreshSettings() {
    const r = await api('GET', '/v1/settings');
    if (r.ok) {
      const s = { ...r.data };
      if (s.api_key) s.api_key = maskSecret(s.api_key);
      if (s.access_api_key) s.access_api_key = maskSecret(s.access_api_key);
      settingsDisplay.textContent = JSON.stringify(s, null, 2);
    } else {
      settingsDisplay.textContent = 'err: ' + (r.data || r.text);
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

  charSelect.addEventListener('change', () => { selectedChar = charSelect.value; refreshSessions(); });
  sessSelect.addEventListener('change', () => { selectedSess = sessSelect.value; });

  btnNewSession.addEventListener('click', async () => {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/sessions/' + encodeURIComponent(selectedChar));
    if (r.ok) refreshSessions();
  });

  // ── chat: send & stream ─────────────────────────────────────────────────
  function appendMsg(role, text, isStreaming) {
    const div = document.createElement('div');
    const safeRole = role === 'user' ? 'user' : 'assistant';
    div.className = 'msg ' + safeRole;
    appendInline(div, 'span', 'role', role);
    const textNode = appendInline(div, 'span', 'text' + (isStreaming ? ' streaming' : ''), text);
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
      await refreshSessions();
      selectedSess = r.data?.session_id || sessSelect.value;
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
        const body = chunk.type === 'body_chunk' ? chunk.text : chunk.text;
        if (body) {
          acc += body;
          msgEl.textContent = acc;
          if (seq % 5 === 0) {
            logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'chunk#' + seq);
          }
        }
      });
      msgEl.classList.remove('streaming');
      logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'done/' + seq + 'chunks');
    } catch (e) {
      if (msgEl) msgEl.classList.remove('streaming');
      if (e.name === 'AbortError') {
        logEvent('SSE', '/v1/chat/completions', 0, Math.round(performance.now() - t0), 'aborted');
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
      const { done, value } = await reader.read();
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

  // ── history / regen / rollback ───────────────────────────────────────────
  btnHistory.addEventListener('click', async () => {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/chat/history', { character_id: selectedChar });
    if (r.ok) {
      const msgs = r.data?.messages || r.data || [];
      chatLog.innerHTML = '';
      msgs.forEach(m => appendMsg(m.role || 'assistant', m.text || m.content || '', false));
    }
  });

  btnRegen.addEventListener('click', async () => {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/chat/regen', { character_id: selectedChar });
    if (r.ok) btnHistory.click();
  });

  btnRollback.addEventListener('click', async () => {
    if (!selectedChar) return;
    const index = prompt('Rollback to message index (0-based):', '0');
    if (index === null) return;
    const r = await api('POST', '/v1/chat/rollback', { character_id: selectedChar, message_index: parseInt(index) });
    if (r.ok) btnHistory.click();
  });

  // ── agent run ────────────────────────────────────────────────────────────
  btnAgentRun.addEventListener('click', async () => {
    const input = agentInput.value.trim();
    if (!input) return;
    agentOutput.textContent = 'Running…';
    const path = '/v1/agent/run';
    const t0 = performance.now();
    try {
      const res = await fetch(base + path, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify({ ...buildChatPayload(input), max_steps: 3 }),
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', path, res.status, Math.round(performance.now() - t0), errBody);
        agentOutput.textContent = '[HTTP ' + res.status + '] ' + errBody;
        return;
      }
      const events = [];
      const seq = await streamSse(res, (chunk, seq) => {
        events.push(chunk);
        const label = chunk.type || 'event';
        logEvent('SSE', path, 200, Math.round(performance.now() - t0), '#' + seq + ' ' + label);
        agentOutput.textContent = events.map(e => JSON.stringify(e)).join('\n');
      });
      logEvent('SSE', path, 200, Math.round(performance.now() - t0), 'done/' + seq + 'events');
    } catch (e) {
      logEvent('POST', path, 0, Math.round(performance.now() - t0), e.message);
      agentOutput.textContent = '[fetch error] ' + e.message;
    }
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
      importResult.textContent = '✗ ' + (r.status || 'err') + ': ' + (r.data || r.text);
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
        const body = chunk.type === 'body_chunk' ? chunk.text : chunk.text;
        if (body) {
          acc += body;
          msgEl.textContent = acc;
        }
      });
      msgEl.classList.remove('streaming');
      return { ok: true, status: 200, chunks: seq };
    } catch (e) {
      if (msgEl) msgEl.classList.remove('streaming');
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), e.message);
      return { ok: false, status: 0, error: e.message };
    }
  }

  // ── auto-connect on load ─────────────────────────────────────────────────
  setTimeout(connect, 300);
})();
