// AIRP WebUI onboarding wizard — first-run引导 6-stage state machine.
//
// Exports: mountOnboarding(container, hostPort) → cleanup()
//
// 解耦契约（设计 spec §2.1, §3.6）：
// - 本文件零 import 指向 app.js / shared.js；仅依赖浏览器 API + hostPort 注入。
// - 向导对宿主的全部知识 = hostPort 对象（6 成员）。
// - 宿主对向导的全部知识 = mountOnboarding(container, hostPort) 签名 + 返回的 cleanup。
// - Port 不变量：api_key 永不作为成员、永不写入浏览器存储、永不出现在 URL。
//
// 失败分类（spec §6.2）：
// - F3 Port 版本不匹配 → mountOnboarding 入口 throw（宿主 catch）
// - F4 运行时崩溃 → 顶层 try/catch 渲染崩溃面板 + [重试向导]/[退回手动配置]
// - F5 HTTP 失败 / F6 SSE 中断 → 向导内阶段错误 + 重试，不降级

export function mountOnboarding(container, hostPort) {
  // F3: 单整数断言（spec §2.4 砍除项 #211：不做完整版本协商）
  if (!hostPort || hostPort.version !== 1) {
    throw new Error('onboarding: hostPort.version must be 1 (got ' + (hostPort && hostPort.version) + ')');
  }
  // Port 成员完整性检查（spec §3.6 不变量 2）
  const REQUIRED = ['version', 'mode', 'fetcher', 'formatError', 'onComplete', 'onSkip'];
  for (const k of REQUIRED) {
    if (typeof hostPort[k] === 'undefined') {
      throw new Error('onboarding: hostPort missing required member "' + k + '"');
    }
  }

  let stage = 1;
  let state = {};            // 向导内部状态；api_key 永不进入此对象
  const listeners = [];      // 注册的事件监听，cleanup 时遍历移除（spec §5.6）
  let sseAbort = null;       // Stage 6 SSE 中断用

  // ── listener 注册 helper（spec §5.6：记录所有 addEventListener 以便 cleanup 移除）
  function on(target, type, handler, opts) {
    target.addEventListener(type, handler, opts);
    listeners.push({ target: target, type: type, handler: handler, opts: opts });
  }

  // ── DOM helpers ──────────────────────────────────────────────────────────
  function el(tag, className, text) {
    const n = document.createElement(tag);
    if (className) n.className = className;
    if (text !== undefined) n.textContent = text;
    return n;
  }

  function clearContainer() {
    if (sseAbort) { try { sseAbort.abort(); } catch {} sseAbort = null; }
    container.innerHTML = '';
  }

  // ── 顶层渲染入口（F4 try/catch 包裹，spec §6.4）──────────────────────────
  function render() {
    try {
      renderStage();
    } catch (err) {
      console.error('[onboarding] runtime crash:', err);
      renderCrashFallback(err);
    }
  }

  function renderCrashFallback(err) {
    clearContainer();
    const panel = el('div', 'onb-crash');
    panel.appendChild(el('h2', '', '向导遇到问题'));
    panel.appendChild(el('p', 'onb-crash-msg', String((err && err.message) || err)));
    const btnRow = el('div', 'onb-btn-row');
    const btnRetry = el('button', 'btn-primary', '重试向导');
    const btnExit = el('button', 'btn-secondary', '退回手动配置');
    on(btnRetry, 'click', () => { stage = 1; state = {}; render(); });
    on(btnExit, 'click', () => {
      // F4 退回手动配置走 onSkip——宿主写 airp_onboarded=true（spec §6.4）
      try { hostPort.onSkip(); } catch (e) { console.error('[onboarding] onSkip threw:', e); }
    });
    btnRow.appendChild(btnRetry);
    btnRow.appendChild(btnExit);
    panel.appendChild(btnRow);
    container.appendChild(panel);
  }

  // ── 阶段渲染分发 ──────────────────────────────────────────────────────────
  function renderStage() {
    clearContainer();
    const wrap = el('div', 'onb-wizard');
    wrap.appendChild(renderHeader());
    const body = el('div', 'onb-body');
    const stageRenderers = { 1: renderStage1, 2: renderStage2, 3: renderStage3, 4: renderStage4, 5: renderStage5, 6: renderStage6 };
    const r = stageRenderers[stage];
    if (!r) throw new Error('onboarding: unknown stage ' + stage);
    body.appendChild(r());
    wrap.appendChild(body);
    wrap.appendChild(renderNav());
    container.appendChild(wrap);
  }

  function renderHeader() {
    const h = el('div', 'onb-header');
    h.appendChild(el('h1', '', 'AIRP 首次配置'));
    const steps = ['部署检查', 'Provider', '模型', '角色', 'Persona', '首聊'];
    const dots = el('div', 'onb-steps');
    for (let i = 0; i < steps.length; i++) {
      const s = i + 1;
      const cls = s === stage ? 'onb-step active' : (s < stage ? 'onb-step done' : 'onb-step');
      dots.appendChild(el('span', cls, steps[i]));
    }
    h.appendChild(dots);
    return h;
  }

  function renderNav() {
    const nav = el('div', 'onb-nav');
    if (stage > 1) {
      const back = el('button', 'btn-secondary', '上一步');
      on(back, 'click', () => { stage--; render(); });
      nav.appendChild(back);
    }
    const skip = el('button', 'btn-tertiary onb-skip', '跳过向导');
    on(skip, 'click', () => {
      // 任意阶段可跳过（spec §4.4）；Stage 6 跳过走 onComplete(firstChatCompleted:false)
      if (stage === 6) { finish(false); }
      else { try { hostPort.onSkip(); } catch (e) { console.error('[onboarding] onSkip threw:', e); } }
    });
    nav.appendChild(skip);
    return nav;
  }

  // ── 错误展示 helper（F5/F6 阶段错误，不降级，spec §6.5）
  function showError(parent, message, onRetry) {
    const box = el('div', 'onb-error');
    box.appendChild(el('p', '', message));
    if (onRetry) {
      const btn = el('button', 'btn-secondary', '重试');
      on(btn, 'click', onRetry);
      box.appendChild(btn);
    }
    parent.appendChild(box);
  }

  // 统一 fetcher + JSON 解析封装；返回 {ok,status,data,text}
  async function callApi(path, opts) {
    const res = await hostPort.fetcher(path, opts);
    const text = await res.text();
    let data;
    try { data = JSON.parse(text); } catch { data = text; }
    return { ok: res.ok, status: res.status, data: data, text: text };
  }

  // ══════════════════════════════════════════════════════════════════════════
  // Stage 1: 部署健康检查
  // ══════════════════════════════════════════════════════════════════════════
  function renderStage1() {
    const box = el('div', 'onb-stage');
    box.appendChild(el('h2', '', 'Step 1 · 部署健康检查'));
    const isDev = hostPort.mode !== 'production';

    if (isDev) {
      box.appendChild(el('p', 'onb-hint', '请输入 Engine URL 与 Bearer（可选），用于本地开发连接。'));
      const urlInput = el('input', 'onb-input');
      urlInput.type = 'text';
      urlInput.value = sessionStorage.getItem('airp_engine_url') || 'http://127.0.0.1:8000';
      urlInput.placeholder = 'http://127.0.0.1:8000';
      const bearerInput = el('input', 'onb-input');
      bearerInput.type = 'password';
      bearerInput.placeholder = 'Bearer（可选）';
      bearerInput.value = sessionStorage.getItem('airp_bearer') || '';
      bearerInput.autocomplete = 'off';
      box.appendChild(el('label', 'onb-label', 'Engine URL'));
      box.appendChild(urlInput);
      box.appendChild(el('label', 'onb-label', 'Bearer'));
      box.appendChild(bearerInput);
      const btn = el('button', 'btn-primary', '连接并检查');
      on(btn, 'click', async () => {
        // dev 模式 Stage 1 写 sessionStorage（spec §3.2），fetcher 每次调用读取
        sessionStorage.setItem('airp_engine_url', urlInput.value.replace(/\/+$/, ''));
        sessionStorage.setItem('airp_bearer', bearerInput.value || '');
        await runHealthCheck(box);
      });
      box.appendChild(btn);
    } else {
      box.appendChild(el('p', 'onb-hint', '生产模式：同源安全连接，网关注入认证。正在检查部署健康…'));
      setTimeout(() => { runHealthCheck(box); }, 0);
    }
    return box;
  }

  async function runHealthCheck(box) {
    const loading = el('p', 'onb-hint', '正在检查…');
    box.appendChild(loading);
    try {
      const vr = await callApi('/version');
      if (!vr.ok) {
        box.removeChild(loading);
        showError(box, 'Engine 连接失败：' + hostPort.formatError(vr.data, vr.text), () => { render(); });
        return;
      }
      const hr = await callApi('/health');
      box.removeChild(loading);
      if (!hr.ok || !hr.data || hr.data.engine !== 'ok') {
        showError(box, 'Engine 健康检查失败：' + hostPort.formatError(hr.data, hr.text), () => { render(); });
        return;
      }
      state.engineVersion = vr.data.version || vr.text;
      state.providerConfigured = !!hr.data.provider_configured;
      state.dataRootWritable = !!hr.data.data_root_writable;
      const summary = el('div', 'onb-summary');
      summary.appendChild(el('p', '', '✓ Engine 已连接（版本 ' + String(state.engineVersion).slice(0, 40) + '）'));
      summary.appendChild(el('p', state.providerConfigured ? '' : 'onb-warn',
        state.providerConfigured ? '✓ Provider 已配置' : '⚠ Provider 未配置（下一步将配置）'));
      summary.appendChild(el('p', state.dataRootWritable ? '' : 'onb-warn',
        state.dataRootWritable ? '✓ 数据目录可写' : '⚠ 数据目录不可写'));
      box.appendChild(summary);
      const next = el('button', 'btn-primary', '下一步');
      on(next, 'click', () => { stage = 2; render(); });
      box.appendChild(next);
    } catch (err) {
      if (loading.parentNode) box.removeChild(loading);
      showError(box, '健康检查异常：' + String(err.message || err), () => { render(); });
    }
  }

  // ══════════════════════════════════════════════════════════════════════════
  // Stage 2: provider 配置
  // ══════════════════════════════════════════════════════════════════════════
  function renderStage2() {
    const box = el('div', 'onb-stage');
    box.appendChild(el('h2', '', 'Step 2 · Provider 配置'));
    box.appendChild(el('p', 'onb-hint', '填入 Provider、Endpoint、API Key。API Key 仅运行时持有，不写入磁盘或浏览器存储。'));

    const providerInput = el('input', 'onb-input');
    providerInput.type = 'text';
    providerInput.placeholder = 'openai / anthropic / ...';
    const endpointInput = el('input', 'onb-input');
    endpointInput.type = 'text';
    endpointInput.placeholder = 'https://api.openai.com/v1';
    const modelInput = el('input', 'onb-input');
    modelInput.type = 'text';
    modelInput.placeholder = '可留空，下一步从列表选择';
    // api_key：永不预填（spec §4.3）；type=password + autocomplete=off
    const apiKeyInput = el('input', 'onb-input');
    apiKeyInput.type = 'password';
    apiKeyInput.placeholder = 'API Key（留空=不修改）';
    apiKeyInput.autocomplete = 'off';

    box.appendChild(el('label', 'onb-label', 'Provider'));
    box.appendChild(providerInput);
    box.appendChild(el('label', 'onb-label', 'Endpoint'));
    box.appendChild(endpointInput);
    box.appendChild(el('label', 'onb-label', 'Model（可选）'));
    box.appendChild(modelInput);
    box.appendChild(el('label', 'onb-label', 'API Key'));
    box.appendChild(apiKeyInput);

    // GET /v1/settings 回填非密字段
    const loading = el('p', 'onb-hint', '加载当前配置…');
    box.appendChild(loading);
    callApi('/v1/settings').then(r => {
      if (r.ok && r.data) {
        if (r.data.provider) providerInput.value = r.data.provider;
        if (r.data.endpoint) endpointInput.value = r.data.endpoint;
        if (r.data.model) modelInput.value = r.data.model;
        if (r.data.api_key_set) {
          const note = el('p', 'onb-hint', '✓ API Key 已配置（不回显；留空提交=不修改）');
          box.insertBefore(note, apiKeyInput);
        }
      }
      if (loading.parentNode) box.removeChild(loading);
    }).catch(() => { if (loading.parentNode) box.removeChild(loading); });

    const btn = el('button', 'btn-primary', '保存并验证');
    on(btn, 'click', async () => {
      // api_key 仅在非空时携带（spec §4.3；空字符串=不修改）
      const body = {};
      if (providerInput.value.trim()) body.provider = providerInput.value.trim();
      if (endpointInput.value.trim()) body.endpoint = endpointInput.value.trim();
      if (modelInput.value.trim()) body.model = modelInput.value.trim();
      if (apiKeyInput.value.trim()) body.api_key = apiKeyInput.value.trim();
      // 安全不变量：api_key 永不进入向导 state（spec §3.6, §4.3）
      try {
        const r = await callApi('/v1/settings', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        });
        if (!r.ok) {
          showError(box, '保存失败：' + hostPort.formatError(r.data, r.text), () => { render(); });
          return;
        }
        // POST 成功后立即清空 api_key 输入框（spec §4.3，复用 app.js:405 行为）
        apiKeyInput.value = '';
        state.provider = providerInput.value.trim() || state.provider;
        state.endpoint = endpointInput.value.trim() || state.endpoint;
        state.model = modelInput.value.trim() || state.model;
        state.apiKeySet = true;
        stage = 3;
        render();
      } catch (err) {
        showError(box, '保存异常：' + String(err.message || err), () => { render(); });
      }
    });
    box.appendChild(btn);
    return box;
  }

  // ══════════════════════════════════════════════════════════════════════════
  // Stage 3: 模型验证
  // ══════════════════════════════════════════════════════════════════════════
  function renderStage3() {
    const box = el('div', 'onb-stage');
    box.appendChild(el('h2', '', 'Step 3 · 模型验证'));
    box.appendChild(el('p', 'onb-hint', '从上游模型列表选择，或手动输入。'));

    if (state._stage3Error) {
      showError(box, state._stage3Error, () => { delete state._stage3Error; render(); });
    }

    const selectWrap = el('div', 'onb-model-picker');
    const loading = el('p', 'onb-hint', '拉取模型列表…');
    box.appendChild(loading);

    callApi('/v1/models').then(r => {
      if (loading.parentNode) box.removeChild(loading);
      if (!r.ok) {
        // F5：/v1/models 失败，提示回 Stage 2 修 endpoint（spec §4.1）
        showError(box, '拉取模型失败：' + hostPort.formatError(r.data, r.text) + '\n请返回 Step 2 检查 Endpoint。', () => { stage = 2; render(); });
        return;
      }
      const models = (r.data && r.data.data) ? r.data.data.map(m => m.id)
        : (Array.isArray(r.data) ? r.data.map(m => (m && m.id) || m) : []);
      if (models.length === 0) {
        // picker 降级：返回空 → 自由文本输入（spec §4.3）
        box.appendChild(el('p', 'onb-hint', '上游返回空模型列表，请手动输入 model id。'));
        const input = el('input', 'onb-input');
        input.type = 'text';
        input.placeholder = 'model id';
        if (state.model) input.value = state.model;
        box.appendChild(input);
        const btn = el('button', 'btn-primary', '保存并下一步');
        on(btn, 'click', () => saveModelAndAdvance(input.value.trim()));
        box.appendChild(btn);
        return;
      }
      // 真实 picker（spec §4.3"全面一点"取向）
      const select = el('select', 'onb-input');
      const placeholder = el('option', '', '— 选择模型 —');
      placeholder.value = '';
      select.appendChild(placeholder);
      for (const m of models) {
        const opt = el('option', '', m);
        opt.value = m;
        if (state.model === m) opt.selected = true;
        select.appendChild(opt);
      }
      selectWrap.appendChild(el('label', 'onb-label', '选择模型'));
      selectWrap.appendChild(select);
      box.appendChild(selectWrap);
      const btn = el('button', 'btn-primary', '保存并下一步');
      on(btn, 'click', () => saveModelAndAdvance(select.value));
      box.appendChild(btn);
    }).catch(err => {
      if (loading.parentNode) box.removeChild(loading);
      showError(box, '拉取异常：' + String(err.message || err), () => { render(); });
    });

    return box;
  }

  async function saveModelAndAdvance(model) {
    if (!model) return; // 用户需选/输入
    try {
      const r = await callApi('/v1/settings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ model: model }),
      });
      if (!r.ok) {
        state._stage3Error = '保存模型失败：' + hostPort.formatError(r.data, r.text);
        render();
        return;
      }
      state.model = model;
      stage = 4;
      render();
    } catch (err) {
      state._stage3Error = '保存异常：' + String(err.message || err);
      render();
    }
  }

  // ══════════════════════════════════════════════════════════════════════════
  // Stage 4: 角色导入
  // ══════════════════════════════════════════════════════════════════════════
  function renderStage4() {
    const box = el('div', 'onb-stage');
    box.appendChild(el('h2', '', 'Step 4 · 角色导入'));
    box.appendChild(el('p', 'onb-hint', '上传 PNG 角色卡或 character_book JSON。生产模式仅支持内容上传，card_path 被拒绝。'));

    const fileInput = el('input', 'onb-input');
    fileInput.type = 'file';
    fileInput.accept = '.png,.json,image/png,application/json';
    box.appendChild(fileInput);

    const btn = el('button', 'btn-primary', '导入');
    const skipBtn = el('button', 'btn-secondary', '跳过（选择已有角色）');
    on(btn, 'click', () => importCharacter(fileInput.files[0], box));
    on(skipBtn, 'click', () => { stage = 5; render(); });
    box.appendChild(btn);
    box.appendChild(skipBtn);
    return box;
  }

  async function importCharacter(file, box) {
    if (!file) { showError(box, '请选择文件', null); return; }
    // 客户端校验：10 MiB 字节计数（spec §4.3，复用 app.js:1807-1817 逻辑）
    if (file.size > 10 * 1024 * 1024) { showError(box, '文件超过 10 MiB 限制', null); return; }
    try {
      const buf = await file.arrayBuffer();
      const bytes = new Uint8Array(buf);
      let body;
      // PNG magic bytes 检测：89 50 4E 47 0D 0A 1A 0A
      if (bytes.length >= 8 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4E && bytes[3] === 0x47) {
        let binary = '';
        for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
        body = { card_png_base64: btoa(binary) };
      } else {
        const text = new TextDecoder().decode(bytes);
        let parsed;
        try { parsed = JSON.parse(text); } catch {
          showError(box, '文件既非 PNG 也非有效 JSON', null);
          return;
        }
        body = { card_json: JSON.stringify(parsed) };
      }
      // 生产模式不发送 card_path（spec §4.3，handlers 拒绝；向导侧也不构造）
      const r = await callApi('/v1/characters/import', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (!r.ok) { showError(box, '导入失败：' + hostPort.formatError(r.data, r.text), null); return; }
      const cid = r.data && (r.data.character_id || r.data.id || r.data.uuid);
      if (!cid) { showError(box, '导入响应缺少 character_id', null); return; }
      const lr = await callApi('/v1/characters');
      if (!lr.ok || !Array.isArray(lr.data) || !lr.data.some(c => (c.id || c.character_id) === cid)) {
        showError(box, '导入后角色列表未包含该 ID，可能持久化失败', null);
        return;
      }
      state.characterId = cid;
      state.characterName = (lr.data.find(c => (c.id || c.character_id) === cid) || {}).name || cid;
      stage = 5;
      render();
    } catch (err) {
      showError(box, '导入异常：' + String(err.message || err), null);
    }
  }

  // ══════════════════════════════════════════════════════════════════════════
  // Stage 5: Persona/Preset 选择
  // ══════════════════════════════════════════════════════════════════════════
  function renderStage5() {
    const box = el('div', 'onb-stage');
    box.appendChild(el('h2', '', 'Step 5 · Persona / Preset 选择'));
    box.appendChild(el('p', 'onb-hint', '选择 Persona 与 Preset，或保持默认。可预览 effective config。'));

    // 如果 Stage 4 跳过，需要选已有角色（spec §4.4）
    if (!state.characterId) {
      box.appendChild(el('p', 'onb-warn', '⚠ 未导入角色，请先选择已有角色或返回 Step 4 导入。'));
      const charSelect = el('select', 'onb-input');
      const ph = el('option', '', '— 选择已有角色 —');
      ph.value = '';
      charSelect.appendChild(ph);
      box.appendChild(charSelect);
      callApi('/v1/characters').then(r => {
        if (r.ok && Array.isArray(r.data)) {
          for (const c of r.data) {
            const opt = el('option', '', c.name || c.id || c.character_id);
            opt.value = c.id || c.character_id;
            charSelect.appendChild(opt);
          }
        }
      });
      const selBtn = el('button', 'btn-secondary', '确认角色');
      on(selBtn, 'click', () => {
        if (charSelect.value) {
          state.characterId = charSelect.value;
          state.characterName = charSelect.options[charSelect.selectedIndex].text;
          render();
        }
      });
      box.appendChild(selBtn);
      return box;
    }

    const personaWrap = el('div');
    const presetWrap = el('div');
    const previewWrap = el('div', 'onb-preview');
    box.appendChild(personaWrap);
    box.appendChild(presetWrap);
    box.appendChild(previewWrap);

    const loading = el('p', 'onb-hint', '加载 Persona / Preset 列表…');
    box.appendChild(loading);

    Promise.all([
      callApi('/v1/users/default/personas'),
      callApi('/v1/presets'),
    ]).then(([pr, pp]) => {
      if (loading.parentNode) box.removeChild(loading);
      // Persona picker（含 default，spec §4.3）
      personaWrap.appendChild(el('label', 'onb-label', 'Persona'));
      const pSelect = el('select', 'onb-input');
      const defaultOpt = el('option', '', 'default');
      defaultOpt.value = 'default';
      pSelect.appendChild(defaultOpt);
      if (pr.ok && Array.isArray(pr.data)) {
        for (const p of pr.data) {
          const opt = el('option', '', p.name || p.id);
          opt.value = p.id;
          pSelect.appendChild(opt);
        }
      }
      personaWrap.appendChild(pSelect);

      // Preset picker（含"不使用预设"=null，spec §4.3）
      presetWrap.appendChild(el('label', 'onb-label', 'Preset'));
      const ppSelect = el('select', 'onb-input');
      const noneOpt = el('option', '', '不使用预设');
      noneOpt.value = '';
      ppSelect.appendChild(noneOpt);
      if (pp.ok && Array.isArray(pp.data)) {
        for (const p of pp.data) {
          const opt = el('option', '', p.name || p.id);
          opt.value = p.id;
          ppSelect.appendChild(opt);
        }
      }
      presetWrap.appendChild(ppSelect);

      // 预览按钮（spec §4.3：chat/preview 只读、不创建 session、不返回 prompt body/secrets）
      const previewBtn = el('button', 'btn-secondary', '预览 effective config');
      on(previewBtn, 'click', () => previewEffective(pSelect.value, ppSelect.value || null, previewWrap));
      box.appendChild(previewBtn);

      const btn = el('button', 'btn-primary', '下一步');
      on(btn, 'click', () => {
        state.personaId = pSelect.value || 'default';
        state.presetId = ppSelect.value || null;
        stage = 6;
        render();
      });
      box.appendChild(btn);
    }).catch(err => {
      if (loading.parentNode) box.removeChild(loading);
      showError(box, '加载异常：' + String(err.message || err), () => { render(); });
    });
    return box;
  }

  async function previewEffective(personaId, presetId, wrap) {
    wrap.innerHTML = '';
    wrap.appendChild(el('p', 'onb-hint', '预览中…'));
    try {
      const body = { character_id: state.characterId, user_id: 'default', persona_id: personaId };
      if (presetId) body.preset_id = presetId;
      const r = await callApi('/v1/chat/preview', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      wrap.innerHTML = '';
      if (!r.ok) { showError(wrap, '预览失败：' + hostPort.formatError(r.data, r.text), null); return; }
      // 显示来源标签（spec §4.3，对齐 §2.4 L0 trace：card/persona/lorebook/state/preset/scene/memory/history/user）
      wrap.appendChild(el('p', '', 'Effective config 来源：'));
      const sources = (r.data && r.data.sources) || (r.data && r.data.assembly && r.data.assembly.sources) || [];
      if (Array.isArray(sources) && sources.length > 0) {
        const ul = el('ul', 'onb-source-list');
        for (const s of sources) {
          ul.appendChild(el('li', '', typeof s === 'string' ? s : (s.kind || s.source || JSON.stringify(s))));
        }
        wrap.appendChild(ul);
      } else {
        wrap.appendChild(el('p', 'onb-hint', '（无来源标签返回）'));
      }
    } catch (err) {
      wrap.innerHTML = '';
      showError(wrap, '预览异常：' + String(err.message || err), null);
    }
  }

  // ══════════════════════════════════════════════════════════════════════════
  // Stage 6: 首轮对话
  // ══════════════════════════════════════════════════════════════════════════
  function renderStage6() {
    const box = el('div', 'onb-stage');
    box.appendChild(el('h2', '', 'Step 6 · 首轮对话'));
    box.appendChild(el('p', 'onb-hint', '发送第一条消息，收到回复即完成。可选"完成向导，稍后聊天"。'));

    const input = el('textarea', 'onb-input onb-chat-input');
    input.rows = 3;
    input.placeholder = '输入第一条消息…';
    box.appendChild(input);

    const btn = el('button', 'btn-primary', '发送');
    const btnFinishLater = el('button', 'btn-secondary', '完成向导，稍后聊天');
    on(btn, 'click', () => sendFirstMessage(input.value.trim(), box));
    on(btnFinishLater, 'click', () => finish(false));
    box.appendChild(btn);
    box.appendChild(btnFinishLater);

    const replyWrap = el('div', 'onb-chat-reply');
    box.appendChild(replyWrap);
    return box;
  }

  async function sendFirstMessage(message, box) {
    if (!message) return;
    const replyWrap = box.querySelector('.onb-chat-reply');
    if (!replyWrap) return;
    replyWrap.innerHTML = '';
    replyWrap.appendChild(el('p', 'onb-hint', '发送中…'));
    try {
      // 懒创建 session（spec §4.1 Stage 6）
      const sr = await callApi('/v1/sessions/' + encodeURIComponent(state.characterId), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      });
      if (!sr.ok) {
        replyWrap.innerHTML = '';
        showError(replyWrap, '创建会话失败：' + hostPort.formatError(sr.data, sr.text), null);
        return;
      }
      const sessionId = sr.data && (sr.data.session_id || sr.data.uuid || sr.data);
      if (typeof sessionId !== 'string') {
        replyWrap.innerHTML = '';
        showError(replyWrap, '会话响应缺少 session_id', null);
        return;
      }
      state.sessionId = sessionId;

      // SSE 流式首聊（spec §4.3，走 Port.fetcher 注入 auth）
      sseAbort = new AbortController();
      const body = {
        character_id: state.characterId,
        session_id: sessionId,
        user_id: 'default',
        persona_id: state.personaId || 'default',
        user_profile: '',
        message: message,
      };
      if (state.presetId) body.preset_id = state.presetId;
      const res = await hostPort.fetcher('/v1/chat/completions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', 'Accept': 'text/event-stream' },
        body: JSON.stringify(body),
        signal: sseAbort.signal,
      });
      if (!res.ok) {
        const text = await res.text();
        let data; try { data = JSON.parse(text); } catch { data = text; }
        replyWrap.innerHTML = '';
        // F6：SSE 请求失败显示重试
        showError(replyWrap, '请求失败：' + hostPort.formatError(data, text), null);
        return;
      }
      // 解析 SSE
      replyWrap.innerHTML = '';
      const bodyP = el('div', 'onb-reply-body');
      replyWrap.appendChild(bodyP);
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = '';
      let done = false;
      while (!done) {
        const { value, done: rDone } = await reader.read();
        if (rDone) break;
        buf += decoder.decode(value, { stream: true });
        const lines = buf.split('\n');
        buf = lines.pop() || '';
        for (const line of lines) {
          if (!line.startsWith('data: ')) continue;
          const payload = line.slice(6);
          if (payload === '[DONE]') { done = true; break; }
          let chunk;
          try { chunk = JSON.parse(payload); } catch { continue; }
          // chunk 类型：body_chunk/think_chunk/plan/tool_call/tool_result/done
          // spec §4.3：不强制 Agent tool 调用——展示但不作通过条件
          if (chunk.type === 'body_chunk' && chunk.content) {
            bodyP.append(chunk.content);
          } else if (chunk.type === 'done') {
            done = true;
            break;
          }
          // think_chunk/plan/tool_call/tool_result 展示与否可选，首版不展示避免 UI 复杂
        }
      }
      // 收到 done → onComplete（spec §4.1 Stage 6 出口条件）
      finish(true);
    } catch (err) {
      // F6：SSE 中断
      if (replyWrap) {
        replyWrap.innerHTML = '';
        showError(replyWrap, '回复中断：' + String(err.message || err), null);
      }
    }
  }

  // ── 出口：onComplete / finish ─────────────────────────────────────────────
  function finish(firstChatCompleted) {
    const config = {
      provider: state.provider || '',
      model: state.model || '',
      character_id: state.characterId || '',
      persona_id: state.personaId || 'default',
      preset_id: state.presetId || null,
      user_id: 'default',
      firstChatCompleted: !!firstChatCompleted,
    };
    try { hostPort.onComplete(config); } catch (e) { console.error('[onboarding] onComplete threw:', e); }
  }

  // ── 启动初次渲染 ──────────────────────────────────────────────────────────
  render();

  // ── 返回 cleanup（spec §5.6）──────────────────────────────────────────────
  return function cleanup() {
    if (sseAbort) { try { sseAbort.abort(); } catch {} sseAbort = null; }
    listeners.forEach(({ target, type, handler, opts }) => {
      try { target.removeEventListener(type, handler, opts); } catch {}
    });
    listeners.length = 0;
    container.innerHTML = '';
  };
}
