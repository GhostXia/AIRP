(function () {
  'use strict';

  const $ = selector => document.querySelector(selector);
  const status = $('#engine-status');
  const address = $('#engine-address');
  const pageStatus = $('#page-status');
  const grid = $('#character-grid');
  const search = $('#character-search');
  const fileInput = $('#character-file');
  let characters = [];

  function resolveConnection() {
    const params = new URLSearchParams(location.search);
    const requested = params.get('engine');
    if (requested && /^https?:\/\//i.test(requested)) sessionStorage.setItem('airp_engine_url', requested.replace(/\/+$/, ''));
    const base = sessionStorage.getItem('airp_engine_url') || location.origin;
    const bearer = sessionStorage.getItem('airp_bearer') || '';
    return { base, bearer };
  }

  const connection = resolveConnection();
  const client = AIRPApi.createClient({ base: connection.base, bearer: connection.bearer });
  address.textContent = client.base === location.origin ? '同源 Engine' : client.base;

  function setConnection(kind, text) {
    status.className = 'status-pill' + (kind ? ' ' + kind : '');
    status.lastChild.textContent = text;
  }

  function setPageStatus(text, error) {
    pageStatus.textContent = text;
    pageStatus.classList.toggle('error', Boolean(error));
  }

  function firstText(value, fallback) {
    return typeof value === 'string' && value.trim() ? value.trim() : fallback;
  }

  function cardData(raw, id, sessionCount) {
    const card = raw && typeof raw === 'object' ? (raw.data || raw) : {};
    return {
      id,
      name: firstText(card.name, id),
      description: firstText(card.description, '尚未填写角色简介。'),
      sessions: sessionCount,
    };
  }

  function openCharacter(id) {
    sessionStorage.setItem('airp_character_id', id);
    const target = new URL('02-chat-space.html', location.href);
    target.searchParams.set('character', id);
    if (client.base !== location.origin) target.searchParams.set('engine', client.base);
    location.href = target.href;
  }

  async function deleteCharacter(id, name) {
    if (!window.confirm('确定删除角色「' + name + '」？\n此操作不可撤销，角色卡、世界书、会话历史将全部移除。')) return;
    setPageStatus('正在删除 ' + name + '…');
    try {
      await client.request('DELETE', '/v1/characters/' + encodeURIComponent(id));
      setPageStatus('已删除角色 ' + name + '。');
      await load();
    } catch (error) {
      setPageStatus('删除失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    }
  }

  function renderCards() {
    const query = search.value.trim().toLocaleLowerCase();
    const visible = characters.filter(item => (item.name + '\n' + item.id + '\n' + item.description).toLocaleLowerCase().includes(query));
    grid.replaceChildren();
    if (!visible.length) {
      const empty = document.createElement('div');
      empty.className = 'empty-state runtime-empty';
      const title = document.createElement('h2');
      title.className = 'empty-title';
      title.textContent = query ? '没有匹配的角色' : '还没有角色';
      const copy = document.createElement('p');
      copy.className = 'empty-desc';
      copy.textContent = query ? '换一个名称或角色 ID 搜索。' : '导入 PNG 或 JSON 角色卡后即可开始对话。';
      empty.append(title, copy);
      grid.appendChild(empty);
      return;
    }
    for (const item of visible) {
      const card = document.createElement('div');
      card.className = 'char-card';

      const open = document.createElement('button');
      open.type = 'button';
      open.className = 'cc-open';
      open.addEventListener('click', () => openCharacter(item.id));

      const head = document.createElement('span');
      head.className = 'cc-head';
      const avatar = document.createElement('span');
      avatar.className = 'avatar';
      avatar.textContent = item.name.slice(0, 1) || 'A';
      const identity = document.createElement('div');
      const name = document.createElement('div');
      name.className = 'cc-name';
      name.textContent = item.name;
      const meta = document.createElement('div');
      meta.className = 'cc-meta';
      meta.textContent = item.id + ' · ' + item.sessions + ' 个会话';
      identity.append(name, meta);
      head.append(avatar, identity);

      const description = document.createElement('span');
      description.className = 'cc-desc';
      description.textContent = item.description;
      const foot = document.createElement('span');
      foot.className = 'cc-foot';
      const state = document.createElement('span');
      state.className = 'tag tag-neutral';
      state.textContent = item.sessions ? '已有会话' : '可开始';
      const action = document.createElement('span');
      action.className = 'cc-model';
      action.textContent = '打开对话 →';
      foot.append(state, action);
      open.append(head, description, foot);

      const del = document.createElement('button');
      del.type = 'button';
      del.className = 'cc-delete';
      del.textContent = '删除';
      del.setAttribute('aria-label', '删除角色 ' + item.name);
      del.addEventListener('click', () => deleteCharacter(item.id, item.name));

      card.append(open, del);
      grid.appendChild(card);
    }
  }

  function asVersion(value) {
    const text = firstText(value && value.version, typeof value === 'string' ? value : '—');
    return text.length > 12 ? text.slice(0, 12) : text;
  }

  async function load() {
    setConnection('', '正在连接');
    setPageStatus('正在从 Engine 加载角色…');
    try {
      const [version, health, settings, ids] = await Promise.all([
        client.request('GET', '/version'),
        client.request('GET', '/health'),
        client.request('GET', '/v1/settings'),
        client.request('GET', '/v1/characters'),
      ]);
      const provider = settings && settings.provider || {};
      const configured = Boolean(health && health.provider_configured);
      $('#provider-state').className = 'tag ' + (configured ? 'tag-success' : 'tag-warning');
      $('#provider-state').textContent = configured ? '已就绪' : '待配置';
      $('#provider-action').hidden = configured;
      $('#provider-model').textContent = firstText(provider.model || (settings && settings.model), '—');
      $('#provider-temperature').textContent = settings.temperature == null ? '—' : String(settings.temperature);
      $('#stat-provider').textContent = configured ? '就绪' : '待配置';
      $('#stat-version').textContent = asVersion(version);
      $('#stat-characters').textContent = Array.isArray(ids) ? String(ids.length) : '0';

      const requestedIds = Array.isArray(ids) ? ids : [];
      const summaries = (await Promise.all(requestedIds.map(async id => {
        try {
          const [raw, sessions] = await Promise.all([
            client.request('GET', '/v1/characters/' + encodeURIComponent(id)),
            client.request('GET', '/v1/sessions/' + encodeURIComponent(id)).catch(() => []),
          ]);
          return cardData(raw, String(id), Array.isArray(sessions) ? sessions.length : 0);
        } catch (error) {
          console.warn('Skipping unreadable character', id, error);
          return null;
        }
      }))).filter(Boolean);
      characters = summaries;
      const sessionTotal = summaries.reduce((total, item) => total + item.sessions, 0);
      $('#stat-sessions').textContent = String(sessionTotal);
      setConnection('ok', configured ? 'Engine 已连接' : '已连接 · Provider 待配置');
      const skipped = requestedIds.length - summaries.length;
      setPageStatus(summaries.length
        ? '已加载 ' + summaries.length + ' 个角色。' + (skipped ? ' 跳过 ' + skipped + ' 个无法读取的角色。' : '')
        : (skipped ? '角色列表存在，但均无法读取。' : 'Engine 已连接；导入第一张角色卡开始使用。'), skipped > 0);
      renderCards();
    } catch (error) {
      setConnection('danger', '连接失败');
      setPageStatus('无法连接 Engine：' + AIRPApi.errorMessage(error.data, error.message) + '。确认 Engine 已启动后重试。', true);
      characters = [];
      renderCards();
    }
  }

  function bytesToBase64(bytes) {
    let result = '';
    const size = 0x8000;
    for (let i = 0; i < bytes.length; i += size) result += String.fromCharCode.apply(null, bytes.subarray(i, i + size));
    return btoa(result);
  }

  async function importCard(file) {
    if (!file) return;
    setPageStatus('正在导入 ' + file.name + '…');
    try {
      let body;
      if (file.type === 'image/png' || file.name.toLocaleLowerCase().endsWith('.png')) {
        body = { card_png_base64: bytesToBase64(new Uint8Array(await file.arrayBuffer())) };
      } else {
        body = { card_json: await file.text() };
      }
      const result = await client.request('POST', '/v1/characters/import', body);
      setPageStatus('已导入角色 ' + firstText(result && result.character_id, file.name) + '。');
      await load();
    } catch (error) {
      setPageStatus('导入失败：' + AIRPApi.errorMessage(error.data, error.message), true);
    } finally {
      fileInput.value = '';
    }
  }

  $('#refresh-engine').addEventListener('click', load);
  $('#side-import').addEventListener('click', () => fileInput.click());
  $('#main-import').addEventListener('click', () => fileInput.click());
  fileInput.addEventListener('change', () => importCard(fileInput.files[0]));
  search.addEventListener('input', renderCards);
  load();
})();
