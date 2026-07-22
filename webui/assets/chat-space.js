(function () {
  'use strict';

  const $ = selector => document.querySelector(selector);
  const flow = $('#message-flow');
  const sessionList = $('#session-list');
  const input = $('#message-input');
  const sendButton = $('#send-message');
  const engineStatus = $('#engine-status');
  const eventLog = $('#event-log');
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  const base = sessionStorage.getItem('airp_engine_url') || location.origin;
  const bearer = sessionStorage.getItem('airp_bearer') || '';
  let characterId = params.get('character') || sessionStorage.getItem('airp_character_id') || '';
  let characterName = '';
  let sessionId = params.get('session') || sessionStorage.getItem('airp_session_id') || '';
  let sessions = [];
  let messageCount = 0;
  let lastHistory = null;
  let streamController = null;

  function log(type, detail) {
    const row = document.createElement('div');
    row.className = 'log-item';
    const time = document.createElement('span');
    time.className = 't';
    time.textContent = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    const event = document.createElement('span');
    event.className = 'e';
    event.textContent = type;
    const copy = document.createElement('span');
    copy.className = 'd';
    copy.textContent = detail;
    row.append(time, event, copy);
    eventLog.prepend(row);
  }

  const client = AIRPApi.createClient({
    base,
    bearer,
    onRequest: entry => log('http.' + entry.method.toLocaleLowerCase(), entry.path + ' · ' + (entry.status || 'network') + ' · ' + entry.ms + 'ms'),
  });
  $('#connection-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;

  function setConnection(kind, text) {
    engineStatus.className = 'status-pill' + (kind ? ' ' + kind : '');
    engineStatus.lastChild.textContent = text;
  }

  function setComposer(enabled) {
    input.disabled = !enabled;
    sendButton.disabled = !enabled;
    $('#continue-message').disabled = !enabled || messageCount === 0;
    $('#regen-message').disabled = !enabled || !lastHistory || !lastHistory.messages?.length || String(lastHistory.messages.at(-1)?.role).toLowerCase() === 'user';
    input.placeholder = enabled ? '向 ' + (characterName || '角色') + ' 发送消息…' : '选择或新建会话后发送消息…';
  }

  function setStreamState(active) {
    if (active) {
      sendButton.disabled = false;
      sendButton.classList.add('stop');
      sendButton.setAttribute('aria-label', '停止生成');
      sendButton.querySelector('.ico').textContent = '■';
      $('#stream-status').textContent = '正在生成 · 点击停止';
      input.disabled = true;
      $('#continue-message').disabled = true;
      $('#regen-message').disabled = true;
    } else {
      sendButton.classList.remove('stop');
      sendButton.setAttribute('aria-label', '发送消息');
      sendButton.querySelector('.ico').textContent = '?';
      $('#stream-status').textContent = 'Enter 发送 · Shift+Enter 换行';
      setComposer(Boolean(sessionId));
    }
  }

  function emptyState(title, description) {
    flow.replaceChildren();
    const empty = document.createElement('div');
    empty.className = 'empty-state runtime-empty';
    const heading = document.createElement('h2');
    heading.className = 'empty-title';
    heading.textContent = title;
    const copy = document.createElement('p');
    copy.className = 'empty-desc';
    copy.textContent = description;
    empty.append(heading, copy);
    flow.appendChild(empty);
  }

  function messageTime(value) {
    if (!value) return '';
    const date = new Date(value);
    return Number.isNaN(date.getTime()) ? '' : date.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });
  }

  function appendMessage(role, text, options) {
    if (flow.querySelector('.runtime-empty')) flow.replaceChildren();
    const row = document.createElement('div');
    row.className = 'msg-row' + (role === 'user' ? ' user' : '');
    if (role !== 'user') {
      const avatar = document.createElement('span');
      avatar.className = 'avatar';
      avatar.textContent = characterName.slice(0, 1) || 'A';
      row.appendChild(avatar);
    }
    const bubble = document.createElement('div');
    bubble.className = 'bubble ' + (role === 'user' ? 'user' : 'ai') + (options && options.error ? ' runtime-error' : '');
    const content = document.createElement('div');
    content.className = 'bubble-text';
    content.textContent = text || '';
    const meta = document.createElement('div');
    meta.className = 'meta';
    meta.textContent = messageTime(options && options.timestamp) || (options && options.streaming ? '正在生成' : '');
    bubble.append(content, meta);
    if (options && options.messageId) {
      row.dataset.messageId = options.messageId;
      const controls = document.createElement('div');
      controls.className = 'message-actions';
      const addAction = (label, action) => {
        const control = document.createElement('button'); control.type = 'button'; control.className = 'message-action'; control.textContent = label; control.addEventListener('click', action); controls.appendChild(control);
      };
      addAction('回滚到这里', () => rollbackTo(options.messageId));
      addAction('删除', () => deleteMessage(options.messageId));
      if (role === 'user') addAction('编辑', () => editMessage(options.messageId, content.textContent));
      bubble.appendChild(controls);
    }
    row.appendChild(bubble);
    flow.appendChild(row);
    flow.scrollTop = flow.scrollHeight;
    return { row, content, meta };
  }

  function renderSessions() {
    sessionList.replaceChildren();
    if (!sessions.length) {
      const copy = document.createElement('p');
      copy.className = 't-note';
      copy.textContent = '还没有命名会话。';
      sessionList.appendChild(copy);
      return;
    }
    for (const item of sessions) {
      const row = document.createElement('div');
      row.className = 'session-row' + (item.id === sessionId ? ' active' : '');
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'pane-item session-btn';
      const title = document.createElement('span');
      title.className = 'pi-title';
      title.textContent = '会话 ' + item.id.slice(0, 8);
      const sub = document.createElement('span');
      sub.className = 'pi-sub';
      sub.textContent = item.total == null ? item.id : item.total + ' 条消息';
      button.append(title, sub);
      button.addEventListener('click', () => selectSession(item.id));
      const del = document.createElement('button');
      del.type = 'button';
      del.className = 'session-delete';
      del.textContent = '×';
      del.setAttribute('aria-label', '删除会话 ' + item.id.slice(0, 8));
      del.addEventListener('click', event => { event.stopPropagation(); deleteSession(item.id); });
      row.append(button, del);
      sessionList.appendChild(row);
    }
  }

  async function loadSessions() {
    const ids = await client.request('GET', '/v1/sessions/' + encodeURIComponent(characterId));
    const values = Array.isArray(ids) ? ids.map(String) : [];
    sessions = values.map(id => ({ id, total: null }));
    if (!values.includes(sessionId)) sessionId = values[0] || '';
    if (sessionId) sessionStorage.setItem('airp_session_id', sessionId);
    else sessionStorage.removeItem('airp_session_id');
    renderSessions();
  }

  async function loadHistory() {
    if (!sessionId) {
      messageCount = 0;
      $('#context-count').textContent = '上下文 0 条';
      emptyState('新建一个会话', '创建命名会话后即可开始与 ' + characterName + ' 对话。');
      setComposer(false);
      return;
    }
    setComposer(false);
    emptyState('正在加载历史', '从 Engine 读取当前会话。');
    try {
      const data = await client.request('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId, limit: 200 });
      lastHistory = data;
      const messages = Array.isArray(data && data.messages) ? data.messages : [];
      const timestamps = Array.isArray(data && data.message_timestamps) ? data.message_timestamps : [];
      flow.replaceChildren();
      messageCount = Number(data && data.total) || messages.length;
      const activeSession = sessions.find(item => item.id === sessionId);
      if (activeSession) activeSession.total = messageCount;
      renderSessions();
      $('#context-count').textContent = '上下文 ' + messageCount + ' 条';
      if (!messages.length) emptyState('会话已就绪', '发送第一条消息，开始这段对话。');
      const ids = Array.isArray(data && data.message_ids) ? data.message_ids : [];
      messages.forEach((message, index) => appendMessage(String(message.role).toLocaleLowerCase() === 'user' ? 'user' : 'assistant', message.content || message.text || '', { timestamp: timestamps[index], messageId: ids[index] }));
      setComposer(true);
    } catch (error) {
      emptyState('历史加载失败', AIRPApi.errorMessage(error.data, error.message));
      setComposer(true);
    }
  }

  async function rollbackTo(messageId) {
    if (!sessionId || streamController || !window.confirm('回滚会丢弃这条消息之后的全部内容。继续吗？')) return;
    try {
      await client.request('POST', '/v1/chat/rollback', { character_id: characterId, session_id: sessionId, message_id: messageId });
      log('chat.rollback', messageId); await loadSessions(); await loadHistory();
    } catch (error) { log('chat.rollback.error', AIRPApi.errorMessage(error.data, error.message)); }
  }

  async function deleteMessage(messageId) {
    if (!sessionId || streamController || !window.confirm('确定删除这条消息？')) return;
    try {
      await client.request('POST', '/v1/chat/delete', { character_id: characterId, session_id: sessionId, message_id: messageId });
      log('chat.delete', messageId); await loadSessions(); await loadHistory();
    } catch (error) { log('chat.delete.error', AIRPApi.errorMessage(error.data, error.message)); }
  }

  async function editMessage(messageId, current) {
    if (!sessionId || streamController) return;
    const content = window.prompt('编辑用户消息', current); if (content == null || !content.trim() || content === current) return;
    try {
      await client.request('PUT', '/v1/chat/message', { character_id: characterId, session_id: sessionId, message_id: messageId, content: content.trim() });
      log('chat.edit', messageId); await loadHistory();
    } catch (error) { log('chat.edit.error', AIRPApi.errorMessage(error.data, error.message)); }
  }

  async function streamMutation(path, label) {
    if (!sessionId || streamController) return;
    const assistant = appendMessage('assistant', '', { streaming: true }); let text = '';
    streamController = new AbortController(); setStreamState(true);
    try {
      await client.stream(path, { character_id: characterId, session_id: sessionId }, {
        signal: streamController.signal,
        onChunk: chunk => { if (chunk.type === 'body_chunk') { text += chunk.text || ''; assistant.content.textContent = text; } },
        onDone: () => log(label + '.complete', text.length + ' 字符'),
      });
      await loadSessions(); await loadHistory();
    } catch (error) {
      if (error.name !== 'AbortError') { assistant.content.textContent = text || AIRPApi.errorMessage(error.data, error.message); assistant.row.querySelector('.bubble').classList.add('runtime-error'); log(label + '.error', AIRPApi.errorMessage(error.data, error.message)); }
    } finally { streamController = null; setStreamState(false); }
  }

  async function deleteSession(id) {
    if (streamController || !window.confirm('确定删除会话 ' + id.slice(0, 8) + '？\n全部消息将不可恢复。')) return;
    try {
      await client.request('DELETE', '/v1/sessions/' + encodeURIComponent(characterId) + '/' + encodeURIComponent(id));
      log('session.delete', id);
      if (sessionId === id) { sessionId = ''; sessionStorage.removeItem('airp_session_id'); }
      await loadSessions();
      await loadHistory();
    } catch (error) { log('session.delete.error', AIRPApi.errorMessage(error.data, error.message)); }
  }

  async function selectSession(id) {
    if (streamController) return;
    sessionId = id;
    sessionStorage.setItem('airp_session_id', id);
    renderSessions();
    await loadHistory();
  }

  async function createSession() {
    if (!characterId) return;
    $('#new-session').disabled = true;
    try {
      const id = await client.request('POST', '/v1/sessions/' + encodeURIComponent(characterId));
      sessionId = String(id);
      sessionStorage.setItem('airp_session_id', sessionId);
      log('session.create', sessionId);
      await loadSessions();
      await loadHistory();
    } catch (error) {
      log('session.error', AIRPApi.errorMessage(error.data, error.message));
    } finally {
      $('#new-session').disabled = false;
    }
  }

  async function send() {
    if (streamController) {
      streamController.abort();
      return;
    }
    const message = input.value.trim();
    if (!message || !characterId || !sessionId) return;
    input.value = '';
    appendMessage('user', message, { timestamp: new Date().toISOString() });
    const assistant = appendMessage('assistant', '', { streaming: true });
    streamController = new AbortController();
    setStreamState(true);
    let text = '';
    try {
      let userProfile = { name: 'User', variables: {} };
      try {
        const savedProfile = JSON.parse(sessionStorage.getItem('airp_user_profile') || 'null');
        if (savedProfile && typeof savedProfile.name === 'string' && savedProfile.variables && typeof savedProfile.variables === 'object') userProfile = savedProfile;
      } catch {}
      const request = {
        character_id: characterId,
        session_id: sessionId,
        user_profile: userProfile,
        message,
      };
      const presetId = sessionStorage.getItem('airp_preset_id');
      if (presetId) request.preset_id = presetId;
      await client.stream('/v1/chat/completions', request, {
        signal: streamController.signal,
        onChunk: chunk => {
          if (chunk.type === 'body_chunk') {
            text += chunk.text || '';
            assistant.content.textContent = text;
            flow.scrollTop = flow.scrollHeight;
          } else if (chunk.type === 'think_chunk') {
            log('llm.reasoning', '收到隐藏推理片段');
          } else if (chunk.type === 'action_options') {
            log('story.actions', '收到剧情选项');
          }
        },
        onDone: () => log('llm.stream.complete', text.length + ' 字符'),
      });
      try { localStorage.setItem('airp_onboarded', 'true'); } catch {}
      sessionStorage.removeItem('airp_onboarding_session_id');
      sessionStorage.removeItem('airp_onboarding_commit_uncertain');
      await loadSessions();
      await loadHistory();
    } catch (error) {
      if (error && error.name === 'AbortError') {
        assistant.meta.textContent = '已停止；用户消息可能已写入';
        log('llm.stream.cancel', '用户停止生成');
      } else {
        const uncertain = error && ['partially_committed', 'unknown'].includes(error.commitState);
        assistant.content.textContent = text || (uncertain ? '生成中断，本轮写入状态不确定。请刷新历史确认，不要直接重发。' : AIRPApi.errorMessage(error.data, error.message));
        assistant.row.querySelector('.bubble').classList.add('runtime-error');
        assistant.meta.textContent = uncertain ? '状态不确定 · 请刷新历史' : '生成失败';
        log('llm.stream.error', AIRPApi.errorMessage(error.data, error.message));
      }
    } finally {
      streamController = null;
      setStreamState(false);
    }
  }

  async function boot() {
    setConnection('', '正在连接');
    setComposer(false);
    try {
      const [version, health, ids] = await Promise.all([
        client.request('GET', '/version'),
        client.request('GET', '/health'),
        client.request('GET', '/v1/characters'),
      ]);
      const values = Array.isArray(ids) ? ids.map(String) : [];
      if (!values.includes(characterId)) characterId = values[0] || '';
      if (!characterId) {
        setConnection('warn', '没有角色');
        emptyState('还没有角色', '返回角色列表导入角色卡后再开始对话。');
        return;
      }
      sessionStorage.setItem('airp_character_id', characterId);
      const [raw, settings] = await Promise.all([
        client.request('GET', '/v1/characters/' + encodeURIComponent(characterId)),
        client.request('GET', '/v1/settings'),
      ]);
      const card = raw && typeof raw === 'object' ? (raw.data || raw) : {};
      characterName = typeof card.name === 'string' && card.name.trim() ? card.name.trim() : characterId;
      const provider = settings && settings.provider || {};
      $('#character-name').textContent = characterName;
      $('#character-avatar').textContent = characterName.slice(0, 1) || 'A';
      $('#character-model').textContent = (provider.model || (settings && settings.model) || '未设置模型') + (!settings || settings.temperature == null ? '' : ' · T' + settings.temperature);
      $('#chat-crumb').textContent = '对话空间 / ' + characterName;
      setConnection('ok', health && health.provider_configured ? 'Engine 已连接' : '已连接 · Provider 待配置');
      log('engine.ready', (version && version.version || version || 'ready').toString());
      await loadSessions();
      await loadHistory();
    } catch (error) {
      setConnection('danger', '连接失败');
      emptyState('无法连接 Engine', AIRPApi.errorMessage(error.data, error.message) + '。确认 Engine 已启动后刷新页面。');
      log('engine.error', AIRPApi.errorMessage(error.data, error.message));
    }
  }

  async function searchHistory() {
    const query = ($('#search-input') && $('#search-input').value || '').trim();
    if (!query || !characterId) return;
    log('chat.search', query);
    try {
      const data = await client.request('POST', '/v1/chat/search', { character_id: characterId, session_id: sessionId || null, query, limit: 20 });
      const results = Array.isArray(data && data.results) ? data.results : [];
      if (!results.length) { log('chat.search.empty', '无匹配结果'); return; }
      flow.replaceChildren();
      const heading = document.createElement('div');
      heading.className = 'search-heading';
      heading.textContent = '搜索“' + query + '”—— ' + results.length + ' 条结果';
      flow.appendChild(heading);
      for (const item of results) {
        appendMessage(String(item.role || 'assistant').toLocaleLowerCase() === 'user' ? 'user' : 'assistant', item.content || item.text || '', { timestamp: item.timestamp, messageId: item.message_id || null });
      }
    } catch (error) { log('chat.search.error', AIRPApi.errorMessage(error.data, error.message)); }
  }

  $('#new-session').addEventListener('click', createSession);
  $('#refresh-history').addEventListener('click', loadHistory);
  $('#continue-message').addEventListener('click', () => streamMutation('/v1/chat/continue', 'chat.continue'));
  $('#regen-message').addEventListener('click', () => streamMutation('/v1/chat/regen', 'chat.regen'));
  sendButton.addEventListener('click', send);
  input.addEventListener('keydown', event => {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      send();
    }
  });
  $('#clear-log').addEventListener('click', () => eventLog.replaceChildren());
  $('#toggle-log').addEventListener('click', () => { $('.pane-right').hidden = !$('.pane-right').hidden; });
  const searchInput = $('#search-input');
  if (searchInput) {
    searchInput.addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); searchHistory(); } });
    const searchBtn = $('#search-button');
    if (searchBtn) searchBtn.addEventListener('click', searchHistory);
  }
  boot();
})();
