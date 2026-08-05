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
  // C-P2：函数形态 bearer——每次请求时解析当前 sessionStorage 值，
  // token 续期（rotation）后新值即刻生效，无需重建 client。
  const bearer = () => sessionStorage.getItem('airp_bearer') || '';
  let characterId = params.get('character') || sessionStorage.getItem('airp_character_id') || '';
  let characterName = '';
  let sessionId = params.get('session') || sessionStorage.getItem('airp_session_id') || '';
  let sessions = [];
  let messageCount = 0;
  let lastHistory = null;
  let streamController = null;
  let branchMeta = null; // { activePath: Set, activeLeaf, parents: [] }
  let historyCursor = null; // { hasMore, oldestId } — #122 窗口化分页
  let coordinatorPhase = 'idle';
  let coordinatorGenerationId = null;
  let coordinatorStateSupported = true;
  let coordinatorStatePending = false;

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
    onRequest: entry => {
      if (entry.path !== '/v1/chat/session-state') log('http.' + entry.method.toLocaleLowerCase(), entry.path + ' · ' + (entry.status || 'network') + ' · ' + entry.ms + 'ms');
    },
    // C-P2：401 续期单次重试（含流启动前）。无 key 模式 renew 403 →
    // 钩子返回 false → 回退既有 401 错误展示行为。
    onUnauthorized: () => AIRPDesktopSession.renewDesktopSession({ base }),
  });
  $('#connection-address').textContent = client.base === location.origin ? '同源 Engine' : client.base;

  function setConnection(kind, text) {
    engineStatus.className = 'status-pill' + (kind ? ' ' + kind : '');
    engineStatus.lastChild.textContent = text;
  }

  function setComposer(enabled) {
    const available = enabled && coordinatorPhase === 'idle';
    input.disabled = !available;
    sendButton.disabled = !available;
    $('#continue-message').disabled = !available || messageCount === 0;
    $('#regen-message').disabled = !available || !lastHistory || !lastHistory.messages?.length || String(lastHistory.messages.at(-1)?.role).toLowerCase() === 'user';
    input.placeholder = enabled ? '向 ' + (characterName || '角色') + ' 发送消息…' : '选择或新建会话后发送消息…';
  }

  function sessionMutationBlocked() {
    return Boolean(streamController) || coordinatorPhase !== 'idle';
  }

  function setCoordinatorState(state) {
    coordinatorPhase = state && typeof state.phase === 'string' ? state.phase : 'idle';
    coordinatorGenerationId = state && typeof state.generation_id === 'string' ? state.generation_id : null;
    const status = $('#session-operation-status');
    const labels = { generating: '会话：生成中', committing: '会话：提交中', recovering: '会话：恢复中' };
    status.textContent = labels[coordinatorPhase] || '';
    status.hidden = coordinatorPhase === 'idle';
    status.className = 'tag mono session-operation-status ' + (coordinatorPhase === 'recovering' ? 'tag-danger' : 'tag-warning');
    renderSessionRecoverAction();
    document.querySelectorAll('.message-action, .swipe-btn').forEach(button => { button.disabled = coordinatorPhase !== 'idle'; });
    if (!streamController) setComposer(Boolean(sessionId));
  }

  // BUG-2 缓解切片：会话被 TurnCommit marker fail-closed 锁死时，给用户提供
  // 一键恢复入口（归档 marker，不删除数据；replay 尚未交付）。
  function renderSessionRecoverAction() {
    const status = $('#session-operation-status');
    const existing = $('#session-recover');
    if (coordinatorPhase !== 'recovering') {
      if (existing) existing.remove();
      return;
    }
    if (existing) return;
    const button = document.createElement('button');
    button.type = 'button';
    button.id = 'session-recover';
    button.className = 'btn btn-secondary session-recover-btn';
    button.textContent = '尝试恢复会话';
    button.addEventListener('click', recoverSession);
    status.insertAdjacentElement('afterend', button);
  }

  async function recoverSession() {
    if (!characterId || !sessionId || streamController) return;
    if (!window.confirm('该会话因上次写入中断被保护性锁定。\n恢复会隔离未完成的提交标记（不会删除任何消息数据），然后允许继续对话。继续吗？')) return;
    const button = $('#session-recover');
    if (button) { button.disabled = true; button.textContent = '正在恢复…'; }
    try {
      const resp = await client.request('POST', '/v1/chat/session-recover', { character_id: characterId, session_id: sessionId });
      log('session.recover', '标记已隔离：' + (resp && resp.quarantined_marker || 'ok'));
      $('#stream-status').textContent = '会话已恢复，可继续对话';
      await refreshCoordinatorState();
      await loadHistory();
    } catch (error) {
      const msg = AIRPApi.errorMessage(error.data, error.message);
      log('session.recover.error', msg);
      $('#stream-status').textContent = '恢复失败：' + msg;
      if (button) { button.disabled = false; button.textContent = '尝试恢复会话'; }
    }
  }

  async function cancelActiveGeneration(controller) {
    const requestedCharacter = characterId;
    const requestedSession = sessionId;
    let abortLocalStream = true;
    try {
      let generationId = coordinatorGenerationId;
      if (!generationId) {
        const state = await client.request('POST', '/v1/chat/session-state', {
          character_id: requestedCharacter,
          session_id: requestedSession,
        });
        generationId = state && state.generation_id;
      }
      if (generationId) {
        await client.request('POST', '/v1/chat/cancel', {
          character_id: requestedCharacter,
          session_id: requestedSession,
          generation_id: generationId,
        });
        // Keep reading until Engine reports the authoritative cancellation
        // commit_state. Aborting here would discard that safety contract.
        abortLocalStream = false;
      }
    } catch (error) {
      const message = AIRPApi.errorMessage(error.data, error.message);
      if (message.includes('generation_committing')) {
        abortLocalStream = false;
        $('#stream-status').textContent = '正在提交，无法取消';
      } else if (!message.includes('stale_generation')) {
        log('llm.stream.cancel.error', message);
      }
    } finally {
      if (abortLocalStream) controller.abort();
    }
  }

  async function refreshCoordinatorState() {
    if (!characterId || !sessionId) { setCoordinatorState({ phase: 'idle' }); return; }
    if (!coordinatorStateSupported || coordinatorStatePending) return;
    const requestedCharacter = characterId;
    const requestedSession = sessionId;
    coordinatorStatePending = true;
    try {
      const state = await client.request('POST', '/v1/chat/session-state', {
        character_id: requestedCharacter,
        session_id: requestedSession,
      });
      if (characterId === requestedCharacter && sessionId === requestedSession) setCoordinatorState(state);
    } catch (error) {
      if (error && error.status === 404) {
        coordinatorStateSupported = false;
        setCoordinatorState({ phase: 'idle' });
        return;
      }
      log('chat.session_state.error', AIRPApi.errorMessage(error.data, error.message));
    } finally {
      coordinatorStatePending = false;
    }
  }

  function handleMutationError(type, error) {
    const message = AIRPApi.errorMessage(error.data, error.message);
    log(type, message);
    if (error && error.status === 409 && (message.includes('session_busy') || message.includes('session_recovery_required'))) refreshCoordinatorState();
  }

  function setStreamState(active) {
    if (active) {
      setCoordinatorState({ phase: 'generating' });
      sendButton.disabled = false;
      sendButton.classList.add('stop');
      sendButton.setAttribute('aria-label', '停止生成');
      sendButton.querySelector('.ico').textContent = '■';
      $('#stream-status').textContent = '正在生成 · 点击停止';
      input.disabled = true;
      $('#continue-message').disabled = true;
      $('#regen-message').disabled = true;
      // Phase 3.5: 流式输出时头像动画
      const avatar = $('#character-avatar');
      if (avatar) avatar.classList.add('streaming');
    } else {
      sendButton.classList.remove('stop');
      sendButton.setAttribute('aria-label', '发送消息');
      sendButton.querySelector('.ico').textContent = '?';
      $('#stream-status').textContent = 'Enter 发送 · Shift+Enter 换行';
      setComposer(Boolean(sessionId));
      refreshCoordinatorState();
      // Phase 3.5: 停止头像动画
      const avatar = $('#character-avatar');
      if (avatar) avatar.classList.remove('streaming');
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

  // P4: 错误可行动化——根据错误类型给出具体操作建议
  function actionableHint(error, msg) {
    const text = (msg || '').toLowerCase();
    if (text.includes('upstream') || text.includes('provider') || text.includes('api_key')) return '检查设置页的 Provider 配置和 API Key 是否正确。';
    if (text.includes('quota') || text.includes('429')) return '今日配额已用尽，明天再试或调整配额设置。';
    if (text.includes('timeout') || text.includes('idle')) return 'Provider 响应超时，可能是网络问题或模型负载高，稍后重试。';
    if (text.includes('model') && text.includes('not')) return '模型不存在或不可用，请在设置页更换模型。';
    if (error && error.status === 401) return 'API Key 无效或已过期，请更新设置。';
    if (error && error.status === 404) return '角色或会话不存在，请刷新页面重新选择。';
    if (text.includes('network') || text.includes('fetch')) return 'Engine 连接失败，确认 Engine 已启动并刷新页面。';
    return '';
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
    // Phase 2.2: 检测世界推进消息类型，添加差异化样式。
    // 仅对非 user 消息应用 world-progression 样式，防止用户文本被误标记。
    let worldClass = '';
    if (role !== 'user') {
      if (text && text.includes('[NPC行动')) worldClass = ' npc-action';
      else if (text && text.includes('[世界事件')) worldClass = ' world-event';
      else if (text && text.includes('[剧情推进')) worldClass = ' plot-advance';
    }
    bubble.className = 'bubble ' + (role === 'user' ? 'user' : 'ai') + worldClass + (options && options.error ? ' runtime-error' : '') + (options && options.onActivePath === false ? ' branch-inactive' : '');
    const content = document.createElement('div');
    content.className = 'bubble-text';
    content.textContent = text || '';
    const meta = document.createElement('div');
    meta.className = 'meta';
    meta.textContent = messageTime(options && options.timestamp) || (options && options.streaming ? '正在生成' : '');
    bubble.append(content, meta);
    // Swipe 内联：多候选时显示切换控件
    const candidates = options && options.candidates;
    const swipeIndex = options && options.swipeIndex || 0;
    if (role !== 'user' && candidates && candidates.length > 1 && options && options.messageId) {
      const swipeBar = document.createElement('div');
      swipeBar.className = 'swipe-bar';
      const prev = document.createElement('button'); prev.type = 'button'; prev.className = 'swipe-btn'; prev.textContent = '‹'; prev.setAttribute('aria-label', '上一个候选');
      const indicator = document.createElement('span'); indicator.className = 'swipe-indicator'; indicator.textContent = (swipeIndex + 1) + '/' + candidates.length;
      const next = document.createElement('button'); next.type = 'button'; next.className = 'swipe-btn'; next.textContent = '›'; next.setAttribute('aria-label', '下一个候选');
      prev.disabled = coordinatorPhase !== 'idle';
      next.disabled = coordinatorPhase !== 'idle';
      const doSwipe = async (newIndex) => {
        if (sessionMutationBlocked()) return;
        try {
          const resp = await client.request('POST', '/v1/chat/swipe', { character_id: characterId, session_id: sessionId, message_id: options.messageId, index: newIndex });
          content.textContent = resp.content || candidates[newIndex] || '';
          indicator.textContent = (resp.index + 1) + '/' + resp.candidates_count;
          log('chat.swipe', options.messageId + ' → ' + (resp.index + 1) + '/' + resp.candidates_count);
        } catch (error) { handleMutationError('chat.swipe.error', error); }
      };
      prev.addEventListener('click', () => { const cur = parseInt(indicator.textContent) - 1; if (cur > 0) doSwipe(cur - 1); });
      next.addEventListener('click', () => { const cur = parseInt(indicator.textContent) - 1; if (cur < candidates.length - 1) doSwipe(cur + 1); });
      swipeBar.append(prev, indicator, next);
      bubble.appendChild(swipeBar);
    }
    if (options && options.messageId) {
      row.dataset.messageId = options.messageId;
      const controls = document.createElement('div');
      controls.className = 'message-actions';
      const addAction = (label, action) => {
        const control = document.createElement('button'); control.type = 'button'; control.className = 'message-action'; control.textContent = label; control.addEventListener('click', action); controls.appendChild(control);
        control.disabled = coordinatorPhase !== 'idle';
      };
      addAction('回滚到这里', () => rollbackTo(options.messageId));
      addAction('删除', () => deleteMessage(options.messageId));
      if (role === 'user') addAction('编辑', () => editMessage(options.messageId, content.textContent));
      // Phase 3.2: TTS 朗读按钮（仅助手消息）
      if (role !== 'user' && 'speechSynthesis' in window) addAction('🔊 朗读', () => speakText(content.textContent));
      // Phase 3.6: 对话片段分享卡片
      addAction('📷 分享', () => shareAsCard(role, content.textContent, options));
      // Branch 内联：非活动分支的叶节点显示“切到此分支”
      if (options.onActivePath === false && branchMeta && branchMeta.ids) {
        const isLeaf = !branchMeta.parents.some(p => p === options.messageId);
        if (isLeaf && options.messageId !== branchMeta.activeLeaf) {
          addAction('⎇ 切到此分支', () => switchBranch(options.messageId));
        }
      }
      bubble.appendChild(controls);
    }
    row.appendChild(bubble);
    flow.appendChild(row);
    // Phase 3.5: 消息到达动画
    bubble.classList.add('arriving');
    bubble.addEventListener('animationend', () => bubble.classList.remove('arriving'), { once: true });
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
      del.addEventListener('click', event => { event.stopPropagation(); deleteSession(item.id, del); });
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
      // #122: 初始加载最近 50 条，而非 200 条全量重建
      const data = await client.request('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId, limit: 50 });
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
      const allCandidates = Array.isArray(data && data.message_candidates) ? data.message_candidates : [];
      const allSwipeIdx = Array.isArray(data && data.message_swipe_index) ? data.message_swipe_index : [];
      const allParents = Array.isArray(data && data.message_parents) ? data.message_parents : [];
      const activePath = Array.isArray(data && data.active_path) ? data.active_path : ids.slice();
      const activeLeaf = data && data.active_leaf || null;
      branchMeta = { activePath: new Set(activePath), activeLeaf, parents: allParents, ids };
      historyCursor = { hasMore: Boolean(data && data.has_more), oldestId: data && data.oldest_id || null };
      renderLoadMore();
      messages.forEach((message, index) => appendMessage(String(message.role).toLocaleLowerCase() === 'user' ? 'user' : 'assistant', message.content || message.text || '', { timestamp: timestamps[index], messageId: ids[index], candidates: allCandidates[index], swipeIndex: allSwipeIdx[index] || 0, onActivePath: activePath.includes(ids[index]) }));
      setComposer(true);
    } catch (error) {
      emptyState('历史加载失败', AIRPApi.errorMessage(error.data, error.message));
      setComposer(true);
    }
  }

  // #122: “加载更早”按钮渲染与增量 prepend
  function renderLoadMore() {
    const existing = flow.querySelector('.load-more-row');
    if (existing) existing.remove();
    if (!historyCursor || !historyCursor.hasMore) return;
    const row = document.createElement('div');
    row.className = 'load-more-row';
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'load-more-btn';
    btn.textContent = '↑ 加载更早的消息';
    btn.addEventListener('click', loadMore);
    row.appendChild(btn);
    flow.prepend(row);
  }

  async function loadMore() {
    if (!sessionId || !historyCursor || !historyCursor.oldestId || streamController) return;
    const btn = flow.querySelector('.load-more-btn');
    if (btn) { btn.disabled = true; btn.textContent = '正在加载…'; }
    try {
      const data = await client.request('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId, limit: 50, before: historyCursor.oldestId });
      const messages = Array.isArray(data && data.messages) ? data.messages : [];
      const timestamps = Array.isArray(data && data.message_timestamps) ? data.message_timestamps : [];
      const ids = Array.isArray(data && data.message_ids) ? data.message_ids : [];
      const allCandidates = Array.isArray(data && data.message_candidates) ? data.message_candidates : [];
      const allSwipeIdx = Array.isArray(data && data.message_swipe_index) ? data.message_swipe_index : [];
      const activePath = branchMeta ? Array.from(branchMeta.activePath) : [];
      historyCursor = { hasMore: Boolean(data && data.has_more), oldestId: data && data.oldest_id || null };
      renderLoadMore();
      // 增量 prepend：保持滚动位置不跳
      const anchor = flow.querySelector('.msg-row');
      const prevTop = anchor ? anchor.getBoundingClientRect().top : 0;
      const fragment = document.createDocumentFragment();
      messages.forEach((message, index) => {
        const temp = appendMessageToFragment(fragment, String(message.role).toLocaleLowerCase() === 'user' ? 'user' : 'assistant', message.content || message.text || '', { timestamp: timestamps[index], messageId: ids[index], candidates: allCandidates[index], swipeIndex: allSwipeIdx[index] || 0, onActivePath: activePath.includes(ids[index]) });
      });
      const loadMoreRow = flow.querySelector('.load-more-row');
      if (loadMoreRow) loadMoreRow.after(fragment);
      else flow.prepend(fragment);
      // 滚动位置补偿
      if (anchor) {
        const newTop = anchor.getBoundingClientRect().top;
        flow.scrollTop += newTop - prevTop;
      }
      log('chat.loadMore', messages.length + ' 条更早消息');
    } catch (error) {
      log('chat.loadMore.error', AIRPApi.errorMessage(error.data, error.message));
      if (btn) { btn.disabled = false; btn.textContent = '↑ 加载更早的消息'; }
    }
  }

  // 辅助：创建消息行并追加到 fragment（不直接插入 flow）
  function appendMessageToFragment(fragment, role, text, options) {
    const row = document.createElement('div');
    row.className = 'msg-row' + (role === 'user' ? ' user' : '');
    if (role !== 'user') {
      const avatar = document.createElement('span');
      avatar.className = 'avatar';
      avatar.textContent = characterName.slice(0, 1) || 'A';
      row.appendChild(avatar);
    }
    const bubble = document.createElement('div');
    bubble.className = 'bubble ' + (role === 'user' ? 'user' : 'ai') + (options && options.onActivePath === false ? ' branch-inactive' : '');
    const content = document.createElement('div');
    content.className = 'bubble-text';
    content.textContent = text || '';
    const meta = document.createElement('div');
    meta.className = 'meta';
    meta.textContent = messageTime(options && options.timestamp);
    bubble.append(content, meta);
    if (options && options.messageId) row.dataset.messageId = options.messageId;
    row.appendChild(bubble);
    fragment.appendChild(row);
    return row;
  }

  async function rollbackTo(messageId) {
    if (!sessionId || sessionMutationBlocked() || !window.confirm('回滚会丢弃这条消息之后的全部内容。继续吗？')) return;
    try {
      await client.request('POST', '/v1/chat/rollback', { character_id: characterId, session_id: sessionId, message_id: messageId });
      log('chat.rollback', messageId); await loadSessions(); await loadHistory();
    } catch (error) { handleMutationError('chat.rollback.error', error); }
  }

  async function deleteMessage(messageId) {
    if (!sessionId || sessionMutationBlocked() || !window.confirm('确定删除这条消息？')) return;
    try {
      await client.request('POST', '/v1/chat/delete', { character_id: characterId, session_id: sessionId, message_id: messageId });
      log('chat.delete', messageId); await loadSessions(); await loadHistory();
    } catch (error) { handleMutationError('chat.delete.error', error); }
  }

  async function editMessage(messageId, current) {
    if (!sessionId || sessionMutationBlocked()) return;
    const content = window.prompt('编辑用户消息', current); if (content == null || !content.trim() || content === current) return;
    try {
      await client.request('PUT', '/v1/chat/message', { character_id: characterId, session_id: sessionId, message_id: messageId, content: content.trim() });
      log('chat.edit', messageId); await loadHistory();
    } catch (error) { handleMutationError('chat.edit.error', error); }
  }

  async function switchBranch(targetLeafId) {
    if (!sessionId || sessionMutationBlocked()) return;
    try {
      await client.request('POST', '/v1/chat/branch/switch', { character_id: characterId, session_id: sessionId, target_leaf_id: targetLeafId });
      log('chat.branch.switch', targetLeafId);
      await loadHistory();
    } catch (error) { handleMutationError('chat.branch.error', error); }
  }

  async function streamMutation(path, label) {
    if (!sessionId || sessionMutationBlocked()) return;
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
      if (error.name !== 'AbortError') { assistant.content.textContent = text || AIRPApi.errorMessage(error.data, error.message); assistant.row.querySelector('.bubble').classList.add('runtime-error'); handleMutationError(label + '.error', error); }
    } finally { streamController = null; setStreamState(false); }
  }

  async function deleteSession(id, btn) {
    if (streamController || !window.confirm('确定删除会话 ' + id.slice(0, 8) + '？\n全部消息将不可恢复。')) return;
    if (btn) btn.disabled = true;
    try {
      await client.request('DELETE', '/v1/sessions/' + encodeURIComponent(characterId) + '/' + encodeURIComponent(id));
      log('session.delete', id);
      if (sessionId === id) { sessionId = ''; sessionStorage.removeItem('airp_session_id'); }
      await loadSessions();
      await loadHistory();
    } catch (error) { log('session.delete.error', AIRPApi.errorMessage(error.data, error.message)); if (btn) btn.disabled = false; }
  }

  async function selectSession(id) {
    if (streamController) return;
    sessionId = id;
    sessionStorage.setItem('airp_session_id', id);
    renderSessions();
    flow.classList.add('switching');
    await refreshCoordinatorState();
    await loadHistory();
    flow.classList.remove('switching');
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
      const controller = streamController;
      $('#stream-status').textContent = '正在停止…';
      void cancelActiveGeneration(controller);
      return;
    }
    if (coordinatorPhase !== 'idle') return;
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
      // #303: 持久化到 Engine data_root，localStorage 仅作离线后备
      client.request('POST', '/v1/onboarding/complete').catch(() => {});
      try { localStorage.setItem('airp_onboarded', 'true'); } catch (e) { /* noop */ }
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
        const rawMsg = AIRPApi.errorMessage(error.data, error.message);
        const suggestion = actionableHint(error, rawMsg);
        assistant.content.textContent = text || (uncertain ? '生成中断，本轮写入状态不确定。请刷新历史确认，不要直接重发。' : rawMsg + (suggestion ? '\n\n建议：' + suggestion : ''));
        assistant.row.querySelector('.bubble').classList.add('runtime-error');
        assistant.meta.textContent = uncertain ? '状态不确定 · 请刷新历史' : '生成失败';
        log('llm.stream.error', rawMsg);
      }
    } finally {
      streamController = null;
      setStreamState(false);
    }
  }

  // C-P1 widget slots：把真实连接状态推给 chat.panel-right 槽内的
  // status-pill widget（引导由 assets/widgets/boot.js 完成，这里只推 state）。
  // boot.js 是 module 脚本（defer），晚于本经典脚本执行；轮询等它把
  // 就绪 Promise 挂到 window.__airpWidgetBoot，超时静默跳过。
  function pushChatWidgetState(label, on) {
    const started = Date.now();
    const timer = window.setInterval(() => {
      if (window.__airpWidgetBoot) {
        window.clearInterval(timer);
        window.__airpWidgetBoot.then(api => {
          if (api) api.pushSlotState('chat.panel-right', { label, on });
        }).catch(() => {});
      } else if (Date.now() - started > 5000) {
        window.clearInterval(timer);
      }
    }, 100);
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
      pushChatWidgetState(health && health.provider_configured ? 'Engine · Provider 就绪' : 'Engine 就绪 · Provider 待配置', true);
      log('engine.ready', (version && version.version || version || 'ready').toString());
      await loadSessions();
      await refreshCoordinatorState();
      await loadHistory();
      startHud(); // Phase 1.5: 启动状态 HUD 轮询
      window.setInterval(refreshCoordinatorState, 2000);
    } catch (error) {
      setConnection('danger', '连接失败');
      pushChatWidgetState('Engine 连接失败', false);
      emptyState('无法连接 Engine', AIRPApi.errorMessage(error.data, error.message) + '。确认 Engine 已启动后刷新页面。');
      log('engine.error', AIRPApi.errorMessage(error.data, error.message));
    }
  }

  // ── Phase 1.5: 角色情感状态 HUD（聊天侧栏状态条） ──────────────────────
  let hudTimer = null;
  const HUD_INTERVAL = 15000; // 15s 轮询

  function renderHud(stateData) {
    const hud = $('#state-hud');
    const body = $('#hud-body');
    if (!stateData || typeof stateData !== 'object' || stateData.unavailable) {
      hud.hidden = true;
      return;
    }
    const entries = Object.entries(stateData).filter(([k]) => !k.startsWith('_'));
    if (!entries.length) { hud.hidden = true; return; }
    hud.hidden = false;
    body.replaceChildren();
    for (const [key, value] of entries) {
      const row = document.createElement('div');
      row.className = 'hud-row';
      const label = document.createElement('span');
      label.className = 'hud-key';
      label.textContent = key;
      const val = document.createElement('span');
      val.className = 'hud-val';
      if (typeof value === 'number') {
        // N-B 修复：仅对已知百分比字段（0-100 刻度）渲染进度条。
        // 原 impl 把任意数值（0-1 比例 / 1-10 等级 / >100 计数）都当百分比，导致误导。
        // 启发式：字段名含 percent/_pct/_ratio 且值在 [0,100]；或值在 [0,1] 视为比例（×100 显示）。
        const lowerKey = key.toLowerCase();
        const isPercentField = /percent|_pct|_p$/.test(lowerKey) && value >= 0 && value <= 100;
        const isRatioField = /_ratio|_rate|mood|affinity|trust|confidence|arousal/.test(lowerKey) && value >= 0 && value <= 1;
        if (isPercentField || isRatioField) {
          const pct = isRatioField ? value * 100 : value;
          const bar = document.createElement('div');
          bar.className = 'hud-bar';
          const fill = document.createElement('div');
          fill.className = 'hud-fill';
          fill.style.width = Math.min(100, Math.max(0, pct)) + '%';
          bar.appendChild(fill);
          val.textContent = isRatioField ? pct.toFixed(0) + '%' : String(value);
          row.append(label, bar, val);
        } else {
          // 非百分比数值按原值显示，不强制套进度条
          val.textContent = String(value);
          row.append(label, val);
        }
      } else {
        val.textContent = typeof value === 'object' ? JSON.stringify(value) : String(value);
        row.append(label, val);
      }
      body.appendChild(row);
    }
  }

  async function pollState() {
    if (!characterId) return;
    try {
      const data = await client.request('GET', '/v1/characters/' + encodeURIComponent(characterId) + '/state');
      renderHud(data);
      suggestBgm(data); // Phase 3.4: 根据状态推荐 BGM
    } catch (error) {
      // N-L 修复：原静默吞掉错误导致调试困难，至少留一条 warn 级别日志
      console.warn('[AIRP] state HUD 轮询失败，隐藏状态面板', error);
      $('#state-hud').hidden = true;
      $('#bgm-hud').hidden = true;
    }
  }

  function startHud() {
    stopHud();
    pollState();
    hudTimer = setInterval(pollState, HUD_INTERVAL);
  }
  function stopHud() { if (hudTimer) { clearInterval(hudTimer); hudTimer = null; } }

  // ── Phase 3.4: 氛围 BGM 建议 ──────────────────────────────────────────────

  // ── Phase 3.2: TTS 朗读（Web Speech API） ─────────────────────────────────
  let ttsVoice = null;
  function initTtsVoice() {
    if (!('speechSynthesis' in window)) return;
    const voices = speechSynthesis.getVoices();
    // 优先选择中文女声
    ttsVoice = voices.find(v => v.lang.startsWith('zh') && v.name.includes('Female'))
      || voices.find(v => v.lang.startsWith('zh'))
      || voices[0] || null;
  }
  if ('speechSynthesis' in window) {
    speechSynthesis.onvoiceschanged = initTtsVoice;
    initTtsVoice();
  }
  function speakText(text) {
    if (!('speechSynthesis' in window) || !text) return;
    speechSynthesis.cancel();
    // 清理 markdown 标记和特殊符号
    const clean = text.replace(/\[.*?\]/g, '').replace(/[*_#`]/g, '').replace(/\n{2,}/g, '\n').trim();
    if (!clean) return;
    const utterance = new SpeechSynthesisUtterance(clean);
    if (ttsVoice) utterance.voice = ttsVoice;
    utterance.lang = 'zh-CN';
    utterance.rate = 0.9;
    utterance.pitch = 1.0;
    speechSynthesis.speak(utterance);
    log('tts.speak', clean.slice(0, 30) + '…');
  }

  // ── Phase 3.6: 对话片段分享卡片 ─────────────────────────────────────────
  // B1 修复（审计 PR #317）：speaker / characterName 来自用户可控的角色卡，
  // 必须像 text 一样做 HTML 转义，否则角色名包含 <script> 等标签会在下载的
  // HTML 文件被打开时执行（XSS）。time 来自 Date.toLocaleString，安全。
  function escapeHtml(s) {
    return String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#39;');
  }
  function shareAsCard(role, text, options) {
    if (!text) return;
    const speaker = role === 'user' ? (sessionStorage.getItem('airp_user_name') || 'User') : (characterName || 'Assistant');
    const time = options && options.timestamp ? new Date(options.timestamp).toLocaleString('zh-CN') : new Date().toLocaleString('zh-CN');
    // 构建 HTML 卡片。所有外部字符串均经 escapeHtml。
    const cardHtml = '<!DOCTYPE html><html><head><meta charset="UTF-8"><style>'
      + 'body{margin:0;padding:24px;background:#f0f2f5;display:flex;justify-content:center;align-items:center;min-height:100vh;font-family:system-ui,-apple-system,sans-serif}'
      + '.card{max-width:480px;width:100%;background:#fff;border-radius:16px;padding:24px;box-shadow:0 4px 24px rgba(0,0,0,.08)}'
      + '.card-head{display:flex;align-items:center;gap:10px;margin-bottom:16px}'
      + '.card-avatar{width:36px;height:36px;border-radius:50%;background:#6366f1;color:#fff;display:flex;align-items:center;justify-content:center;font-weight:700;font-size:14px}'
      + '.card-name{font-weight:600;font-size:14px;color:#1e293b}'
      + '.card-time{font-size:11px;color:#94a3b8}'
      + '.card-body{font-size:14px;line-height:1.8;color:#334155;white-space:pre-wrap;word-break:break-word}'
      + '.card-foot{margin-top:16px;padding-top:12px;border-top:1px solid #e2e8f0;font-size:10px;color:#94a3b8;display:flex;justify-content:space-between}'
      + '</style></head><body><div class="card">'
      + '<div class="card-head"><div class="card-avatar">' + escapeHtml(speaker.slice(0, 1)) + '</div><div><div class="card-name">' + escapeHtml(speaker) + '</div><div class="card-time">' + escapeHtml(time) + '</div></div></div>'
      + '<div class="card-body">' + escapeHtml(text) + '</div>'
      + '<div class="card-foot"><span>AIRP · ' + escapeHtml(characterName || '') + '</span><span>' + escapeHtml(new Date().toLocaleDateString('zh-CN')) + '</span></div>'
      + '</div></body></html>';
    const blob = new Blob([cardHtml], { type: 'text/html;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'AIRP_share_' + Date.now() + '.html';
    document.body.appendChild(a); a.click();
    document.body.removeChild(a); URL.revokeObjectURL(url);
    log('share.card', speaker + ' 的消息已导出为分享卡片');
  }
  const BGM_RULES = [
    { keywords: ['战斗', '危机', '紧张', '危险', 'combat', 'danger'], mood: '紧张', tracks: ['Two Steps From Hell - Heart of Courage', 'Hans Zimmer - Time', '进击的巨人 OST - ətˈæk 0N tάɪtn'] },
    { keywords: ['欢乐', '开心', '日常', '温馨', 'happy', 'peaceful'], mood: '欢快', tracks: ['久石让 - Summer', 'Yiruma - River Flows in You', 'Clannad OST - 楽しい会話'] },
    { keywords: ['悲伤', '离别', '孤独', 'sad', 'lonely'], mood: '伤感', tracks: ['Secret Garden - Song from a Secret Garden', 'Hans Zimmer - Now We Are Free', 'Unravel - Tokyo Ghoul'] },
    { keywords: ['神秘', '探索', '未知', 'mystery', 'explore'], mood: '神秘', tracks: ['Vangelis - Blade Runner Blues', 'Interstellar Main Theme', 'The Elder Scrolls V - Far Horizons'] },
    { keywords: ['浪漫', '爱情', '温柔', 'romance', 'love'], mood: '浪漫', tracks: ['Yiruma - Kiss The Rain', 'Ludovico Einaudi - Nuvole Bianche', 'Your Name OST - なんでもないや'] },
    { keywords: ['史诗', '壮阔', '战争', 'epic', 'war'], mood: '史诗', tracks: ['Two Steps From Hell - Victory', 'Hans Zimmer - Gladiator Suite', 'Lord of the Rings - The Bridge of Khazad Dum'] },
  ];

  function suggestBgm(stateData) {
    const hud = $('#bgm-hud');
    const body = $('#bgm-body');
    if (!stateData || typeof stateData !== 'object') { hud.hidden = true; return; }
    // 从状态中提取关键词
    const stateText = JSON.stringify(stateData).toLowerCase();
    let matched = null;
    for (const rule of BGM_RULES) {
      // CodeRabbit #10：ASCII 关键词用 \b 词边界匹配，避免命中 key 名或子串
      // （如 {"mood":"not combat"} 不该命中 "combat"）。中文无词边界概念，仍用 includes。
      const hit = rule.keywords.some(kw => {
        const k = kw.toLowerCase();
        // 简单 ASCII 判定：含 a-z0-9 视为 ASCII 关键词
        if (/^[a-z0-9\s]+$/i.test(kw)) {
          return new RegExp('\\b' + k.replace(/[.*+?^${}()|[\]\\]/g, '\\$&') + '\\b', 'i').test(stateText);
        }
        return stateText.includes(k);
      });
      if (hit) {
        matched = rule;
        break;
      }
    }
    if (!matched) { hud.hidden = true; return; }
    hud.hidden = false;
    body.replaceChildren();
    const tag = document.createElement('span');
    tag.className = 'bgm-tag';
    tag.textContent = matched.mood;
    body.appendChild(tag);
    matched.tracks.forEach(track => {
      const item = document.createElement('div');
      item.className = 'bgm-item';
      const name = document.createElement('span');
      name.className = 'bgm-name';
      name.textContent = track;
      name.title = track;
      item.appendChild(name);
      item.addEventListener('click', () => {
        window.open('https://www.youtube.com/results?search_query=' + encodeURIComponent(track), '_blank');
      });
      body.appendChild(item);
    });
  }

  // ── Phase 1.1: 对话导出（Markdown / JSON 一键下载） ──────────────────────
  function downloadBlob(content, filename, mime) {
    const blob = new Blob([content], { type: mime + ';charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = filename;
    document.body.appendChild(a); a.click();
    document.body.removeChild(a); URL.revokeObjectURL(url);
  }

  async function exportConversation(format) {
    if (!characterId || !sessionId) { log('export.error', '请先选择角色和会话'); return; }
    try {
      // B3 修复：不传 limit 字段 → HistoryQuery.limit=None → handler 走 legacy 全量分支
      // 传 limit（含 9999）会被 history_window 的 unwrap_or(50).clamp(1,200) 钳为 200，长对话静默截断
      const data = await client.request('POST', '/v1/chat/history', { character_id: characterId, session_id: sessionId });
      const messages = Array.isArray(data && data.messages) ? data.messages : [];
      const timestamps = Array.isArray(data && data.message_timestamps) ? data.message_timestamps : [];
      if (!messages.length) { log('export.empty', '当前会话无消息'); return; }
      const stamp = new Date().toISOString().slice(0, 19).replace(/[T:]/g, '-');
      const base = (characterName || characterId) + '_' + sessionId.slice(0, 8) + '_' + stamp;
      if (format === 'json') {
        const payload = { character: characterName || characterId, character_id: characterId, session_id: sessionId, exported_at: new Date().toISOString(), total: messages.length, messages: messages.map((m, i) => ({ role: m.role, content: m.content || m.text || '', timestamp: timestamps[i] || null })) };
        downloadBlob(JSON.stringify(payload, null, 2), base + '.json', 'application/json');
      } else {
        let md = '# ' + (characterName || characterId) + ' — 对话记录\n\n';
        md += '> 会话: ' + sessionId + '  \n> 导出时间: ' + new Date().toLocaleString('zh-CN') + '  \n> 消息数: ' + messages.length + '\n\n---\n\n';
        messages.forEach((m, i) => {
          const role = String(m.role || 'assistant').toLowerCase() === 'user' ? (sessionStorage.getItem('airp_user_name') || 'User') : (characterName || 'Assistant');
          const time = timestamps[i] ? new Date(timestamps[i]).toLocaleString('zh-CN') : '';
          md += '### ' + role + (time ? ' · ' + time : '') + '\n\n' + (m.content || m.text || '') + '\n\n';
        });
        downloadBlob(md, base + '.md', 'text/markdown');
      }
      log('export.' + format, messages.length + ' 条消息已导出');
    } catch (error) { log('export.error', AIRPApi.errorMessage(error.data, error.message)); }
  }

  async function searchHistory() {
    const query = ($('#search-input') && $('#search-input').value || '').trim();
    if (!query || !characterId) return;
    log('chat.search', query);
    try {
      const data = await client.request('POST', '/v1/chat/search', { character_id: characterId, session_id: sessionId || null, query, limit: 20 });
      const results = Array.isArray(data && data.results) ? data.results : [];
      if (!results.length) { log('chat.search.empty', '无匹配结果'); emptyState('无匹配结果', '没有找到包含“' + query + '”的历史消息。'); return; }
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
  $('#export-md').addEventListener('click', () => exportConversation('md'));
  $('#export-json').addEventListener('click', () => exportConversation('json'));
  const searchInput = $('#search-input');
  if (searchInput) {
    searchInput.addEventListener('keydown', event => { if (event.key === 'Enter') { event.preventDefault(); searchHistory(); } });
    const searchBtn = $('#search-button');
    if (searchBtn) searchBtn.addEventListener('click', searchHistory);
  }
  boot();
})();
