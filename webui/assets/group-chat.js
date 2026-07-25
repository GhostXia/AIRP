(function () {
  'use strict';
  const $ = s => document.querySelector(s);
  const params = new URLSearchParams(location.search);
  const requestedEngine = params.get('engine');
  if (requestedEngine && /^https?:\/\//i.test(requestedEngine)) sessionStorage.setItem('airp_engine_url', requestedEngine.replace(/\/+$/, ''));
  const base = sessionStorage.getItem('airp_engine_url') || location.origin;
  // CodeRabbit #7：仅同源 Engine 接收 stored bearer，跨源不带（防钓鱼链接窃取令牌）。
  const storedBearer = sessionStorage.getItem('airp_bearer') || '';
  const bearer = (base === location.origin) ? storedBearer : '';
  const client = AIRPApi.createClient({ base, bearer });

  let scenes = [];
  let activeScene = null;
  let activeCharacters = [];
  let sessionId = '';
  let streaming = false;

  const flow = $('#group-flow');
  const input = $('#group-input');
  const sendBtn = $('#group-send');

  function charColor(idx) { return 'char-color-' + (idx % 8); }

  function appendGroupMessage(speaker, text, isUser, colorIdx) {
    const row = document.createElement('div');
    row.className = 'gmsg-row' + (isUser ? ' user' : '');
    const avatar = document.createElement('div');
    avatar.className = 'gmsg-avatar ' + (isUser ? 'char-color-0' : charColor(colorIdx || 0));
    avatar.textContent = (speaker || '?').slice(0, 1).toUpperCase();
    const bubble = document.createElement('div');
    bubble.className = 'gmsg-bubble';
    if (!isUser) {
      const name = document.createElement('div');
      name.className = 'gmsg-name';
      name.textContent = speaker;
      bubble.appendChild(name);
    }
    const content = document.createElement('div');
    content.textContent = text;
    bubble.appendChild(content);
    const meta = document.createElement('div');
    meta.className = 'gmsg-meta';
    meta.textContent = new Date().toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });
    bubble.appendChild(meta);
    row.append(avatar, bubble);
    flow.appendChild(row);
    flow.scrollTop = flow.scrollHeight;
    return { row, content, bubble };
  }

  function renderScenes() {
    const list = $('#scene-list');
    list.replaceChildren();
    if (!scenes.length) {
      list.appendChild(document.createTextNode('尚无场景。请在多人场景页创建。'));
      return;
    }
    scenes.forEach(scene => {
      const item = document.createElement('div');
      item.className = 'scene-item' + (activeScene && activeScene.scene_id === scene ? ' active' : '');
      item.textContent = scene;
      item.addEventListener('click', () => selectScene(scene));
      list.appendChild(item);
    });
  }

  async function selectScene(sceneId) {
    try {
      activeScene = await client.request('GET', '/v1/scenes/' + encodeURIComponent(sceneId));
      activeCharacters = (activeScene.characters || []).map(c => typeof c === 'string' ? c : c.character_id);
      renderScenes();
      renderCharTags();
      // 创建或复用 session
      if (!sessionId) {
        const firstChar = activeCharacters[0];
        if (firstChar) {
          sessionId = await client.request('POST', '/v1/sessions/' + encodeURIComponent(firstChar));
          sessionId = String(sessionId);
        }
      }
      $('#group-status').textContent = '场景: ' + sceneId + ' · ' + activeCharacters.length + ' 个角色';
      flow.replaceChildren();
      const welcome = document.createElement('div');
      welcome.className = 'gmsg-meta';
      welcome.style.textAlign = 'center';
      welcome.style.padding = '12px';
      welcome.textContent = '— 场景「' + (activeScene.description || sceneId) + '」已就绪 · ' + activeCharacters.join(', ') + ' —';
      flow.appendChild(welcome);
    } catch (error) {
      $('#group-status').textContent = '加载场景失败: ' + AIRPApi.errorMessage(error.data, error.message);
    }
  }

  function renderCharTags() {
    const container = $('#scene-chars');
    container.replaceChildren();
    activeCharacters.forEach((cid, idx) => {
      const tag = document.createElement('span');
      tag.className = 'scene-char-tag';
      tag.textContent = cid;
      tag.style.borderLeft = '3px solid ' + ['#6366f1','#ec4899','#f59e0b','#22c55e','#06b6d4','#8b5cf6','#ef4444','#14b8a6'][idx % 8];
      container.appendChild(tag);
    });
  }

  async function sendGroupMessage() {
    const message = input.value.trim();
    if (!message || streaming) return;
    if (!activeScene || !activeCharacters.length) {
      $('#group-status').textContent = '请先选择一个场景';
      return;
    }
    input.value = '';
    appendGroupMessage('User', message, true, 0);
    streaming = true;
    sendBtn.disabled = true;

    // 轮流让每个角色回复
    for (let i = 0; i < activeCharacters.length; i++) {
      const cid = activeCharacters[i];
      const msg = appendGroupMessage(cid, '', false, i);
      msg.content.textContent = '…';
      try {
        let text = '';
        const userProfile = { name: 'User', variables: {} };
        await client.stream('/v1/chat/completions', {
          character_id: cid,
          session_id: sessionId,
          scene_id: activeScene.scene_id,
          user_profile: userProfile,
          message: message,
        }, {
          onChunk: chunk => {
            if (chunk.type === 'body_chunk') {
              text += chunk.text || '';
              msg.content.textContent = text;
              flow.scrollTop = flow.scrollHeight;
            }
          },
          onDone: () => {},
        });
        if (!text) msg.content.textContent = '（无回复）';
      } catch (error) {
        msg.content.textContent = '⚠ ' + AIRPApi.errorMessage(error.data, error.message);
        msg.bubble.style.borderColor = 'var(--danger)';
      }
    }
    streaming = false;
    sendBtn.disabled = false;
  }

  // ── Boot ──
  async function boot() {
    try {
      await client.request('GET', '/health');
      $('#engine-status').className = 'status-pill ok';
      $('#engine-status').lastChild.textContent = 'Engine 就绪';
      scenes = await client.request('GET', '/v1/scenes').catch(() => []);
      renderScenes();
      if (scenes.length) selectScene(scenes[0]);
    } catch (error) {
      $('#engine-status').className = 'status-pill danger';
      $('#engine-status').lastChild.textContent = '连接失败';
      $('#group-status').textContent = '无法连接 Engine';
    }
  }

  sendBtn.addEventListener('click', sendGroupMessage);
  input.addEventListener('keydown', e => {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); sendGroupMessage(); }
  });
  boot();
})();
