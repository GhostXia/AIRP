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
    const cls = status < 300 ? 'ok' : status < 500 ? 'unknown' : 'err';
    el.innerHTML = `<span class="ts">${now}</span> <span class="status ${cls}">${status}</span> ` +
      `<span class="method">${method}</span> <span class="path">${path}</span> ${ms}ms` +
      (detail ? `<br/>${detail}` : '');
    eventLog.prepend(el);
    if (eventLog.children.length > 200) eventLog.removeChild(eventLog.lastChild);
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
      if (s.api_key) s.api_key = s.api_key.slice(0, 4) + '…' + s.api_key.slice(-4);
      settingsDisplay.textContent = JSON.stringify(s, null, 2);
    } else {
      settingsDisplay.textContent = 'err: ' + (r.data || r.text);
    }
  }

  async function refreshChars() {
    const r = await api('GET', '/v1/characters');
    if (r.ok) {
      const ids = Array.isArray(r.data) ? r.data : [];
      charSelect.innerHTML = ids.map(id => `<option value="${id}">${id}</option>`).join('');
      if (ids.length && !selectedChar) { selectedChar = ids[0]; charSelect.value = ids[0]; }
      if (selectedChar && ids.includes(selectedChar)) charSelect.value = selectedChar;
      refreshSessions();
    }
  }

  btnRefreshChars.addEventListener('click', refreshChars);

  async function refreshSessions() {
    const cid = charSelect.value;
    if (!cid) return;
    selectedChar = cid;
    const r = await api('GET', '/v1/sessions/' + cid);
    if (r.ok) {
      const ids = Array.isArray(r.data) ? r.data.map(s => s.session_id || s) : [];
      sessSelect.innerHTML = ids.map(id => `<option value="${id}">${id.slice(0, 12)}</option>`).join('');
      if (ids.length && !selectedSess) selectedSess = ids[0];
    }
  }

  charSelect.addEventListener('change', () => { selectedChar = charSelect.value; refreshSessions(); });

  btnNewSession.addEventListener('click', async () => {
    if (!selectedChar) return;
    const r = await api('POST', '/v1/sessions/' + selectedChar);
    if (r.ok) refreshSessions();
  });

  // ── chat: send & stream ─────────────────────────────────────────────────
  function appendMsg(role, text, isStreaming) {
    const div = document.createElement('div');
    div.className = 'msg ' + role;
    div.innerHTML = `<span class="role">${role}</span><span class="text${isStreaming ? ' streaming' : ''}">${escHtml(text)}</span>`;
    chatLog.appendChild(div);
    chatLog.scrollTop = chatLog.scrollHeight;
    return div.querySelector('.text');
  }

  function escHtml(s) { return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;'); }

  async function doSend() {
    const text = chatInput.value.trim();
    if (!text || !selectedChar) return;
    chatInput.value = '';
    appendMsg('user', text, false);
    const userMsg = document.querySelector('.msg.user:last-child .text');

    // create session if none
    if (!selectedSess) {
      const r = await api('POST', '/v1/sessions/' + selectedChar);
      if (!r.ok) { appendMsg('assistant', '[session create failed]', false); return; }
      await refreshSessions();
      selectedSess = r.data?.session_id || sessSelect.value;
    }

    // abort prior stream
    if (abortController) abortController.abort();
    abortController = new AbortController();
    const url = base + '/v1/chat/completions';
    const t0 = performance.now();

    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify({ character_id: selectedChar, messages: [{ role: 'user', content: text }] }),
        signal: abortController.signal,
      });
      if (!res.ok) {
        const errBody = await res.text();
        logEvent('POST', '/v1/chat/completions', res.status, Math.round(performance.now() - t0), errBody);
        appendMsg('assistant', '[HTTP ' + res.status + '] ' + errBody, false);
        return;
      }
      const msgEl = appendMsg('assistant', '', true);
      let acc = '';
      let seq = 0;
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = '';

      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        const lines = buf.split('\n');
        buf = lines.pop() || '';
        for (const line of lines) {
          if (line.startsWith('data: ')) {
            const data = line.slice(6).trim();
            if (data === '[DONE]') break;
            try {
              const chunk = JSON.parse(data);
              if (chunk.body_chunk !== undefined) {
                acc += chunk.body_chunk;
                msgEl.textContent = acc;
                seq++;
                if (seq % 5 === 0) {  // log every 5th chunk
                  logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'chunk#' + seq);
                }
              }
            } catch {}
          }
        }
      }
      msgEl.classList.remove('streaming');
      logEvent('SSE', '/v1/chat/completions', 200, Math.round(performance.now() - t0), 'done/' + seq + 'chunks');
    } catch (e) {
      if (e.name === 'AbortError') return;
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), e.message);
      appendMsg('assistant', '[fetch error] ' + e.message, false);
    }
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
    const r = await api('POST', '/v1/chat/rollback', { character_id: selectedChar, index: parseInt(index) });
    if (r.ok) btnHistory.click();
  });

  // ── agent run ────────────────────────────────────────────────────────────
  btnAgentRun.addEventListener('click', async () => {
    const input = agentInput.value.trim();
    if (!input) return;
    agentOutput.textContent = 'Running…';
    const r = await api('POST', '/v1/agent/run', {
      character_id: selectedChar || '',
      messages: [{ role: 'user', content: input }],
    });
    if (r.ok) {
      const text = typeof r.data === 'string' ? r.data : JSON.stringify(r.data, null, 2);
      agentOutput.textContent = text;
    } else {
      agentOutput.textContent = 'err: ' + (r.data || r.text);
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
      // base64 encode
      let bin = '';
      for (let i = 0; i < bytes.length; i++) bin += String.fromCharCode(bytes[i]);
      const b64 = btoa(bin);
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
    await Promise.all(promises);
    const ms = Math.round(performance.now() - t0);
    logEvent('CONCURRENT', '/v1/chat/completions ×2', 200, ms, '两条流均完成');
    concurrentStatus.textContent = '✓ 两条流完成 (' + ms + 'ms)。检查 chat log 顺序：u-A → a-A → u-B → a-B 应基本交替，无串扰。';
  });

  // 抽出 doSend 的纯逻辑供并发复用（不发 user DOM、不读 input）
  async function doSendText(text) {
    if (!selectedChar) return;
    const url = base + '/v1/chat/completions';
    const t0 = performance.now();
    try {
      const res = await fetch(url, {
        method: 'POST',
        headers: headers(),
        body: JSON.stringify({ character_id: selectedChar, messages: [{ role: 'user', content: text }] }),
      });
      if (!res.ok) {
        logEvent('POST', '/v1/chat/completions', res.status, Math.round(performance.now() - t0));
        return;
      }
      const msgEl = appendMsg('assistant', '', true);
      let acc = '';
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = '';
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buf += decoder.decode(value, { stream: true });
        const lines = buf.split('\n');
        buf = lines.pop() || '';
        for (const line of lines) {
          if (line.startsWith('data: ')) {
            const data = line.slice(6).trim();
            if (data === '[DONE]') break;
            try {
              const chunk = JSON.parse(data);
              if (chunk.body_chunk !== undefined) {
                acc += chunk.body_chunk;
                msgEl.textContent = acc;
              }
            } catch {}
          }
        }
      }
      msgEl.classList.remove('streaming');
    } catch (e) {
      logEvent('POST', '/v1/chat/completions', 0, Math.round(performance.now() - t0), e.message);
    }
  }

  // ── auto-connect on load ─────────────────────────────────────────────────
  setTimeout(connect, 300);
})();
