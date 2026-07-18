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
  let sendInFlight = false;  // Stage 6 sendFirstMessage 单飞保护（防双击/重试并发）
  let preMountFocus = null;  // dialog 打开前焦点，cleanup 时恢复（spec §4.2 a11y）

  // 焦点保存（spec §4.2 a11y）：mount 时记录当前焦点元素，cleanup 时恢复
  try {
    preMountFocus = document.activeElement;
  } catch { preMountFocus = null; }

  // ── listener 注册 helper（spec §5.6：记录所有 addEventListener 以便 cleanup 移除）
  function on(target, type, handler, opts) {
    target.addEventListener(type, handler, opts);
    listeners.push({ target: target, type: type, handler: handler, opts: opts });
  }

  // ── 移除所有已注册 listener（retry / cleanup 共用，spec §6.4 retry 不可残留）
  function removeListeners() {
    listeners.forEach(({ target, type, handler, opts }) => {
      try { target.removeEventListener(type, handler, opts); } catch {}
    });
    listeners.length = 0;
  }

  // ── 安全回调包装（F4 后续 stage 回调也走崩溃面板，spec §6.4 修正）
  // 任何 event handler / async continuation 的异常都路由到 renderCrashFallback，
  // 而不是变成浏览器 uncaught error。
  function safeSync(fn, label) {
    return function (...args) {
      try { return fn.apply(this, args); }
      catch (err) {
        console.error('[onboarding] handler crashed (' + (label || 'unknown') + '):', err);
        try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
      }
    };
  }
  async function safeAsync(fn, label) {
    try { return await fn(); }
    catch (err) {
      console.error('[onboarding] async crashed (' + (label || 'unknown') + '):', err);
      try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
    }
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
    // retry 不可残留旧 listener（spec §6.4 修正：先 removeListeners 再清 DOM）
    removeListeners();
    if (sseAbort) { try { sseAbort.abort(); } catch {} sseAbort = null; }
    sendInFlight = false;
    clearContainer();
    const panel = el('div', 'onb-crash');
    panel.appendChild(el('h2', '', '向导遇到问题'));
    panel.appendChild(el('p', 'onb-crash-msg', String((err && err.message) || err)));
    const btnRow = el('div', 'onb-btn-row');
    const btnRetry = el('button', 'btn-primary', '重试向导');
    const btnExit = el('button', 'btn-secondary', '退回手动配置');
    on(btnRetry, 'click', safeSync(() => {
      // 重置向导状态再渲染（spec §6.4 retry 入口）
      stage = 1; state = {}; sendInFlight = false;
      render();
    }, 'crash-retry'));
    on(btnExit, 'click', safeSync(() => {
      // F4 退回手动配置走 onSkip——宿主写 airp_onboarded=true（spec §6.4）
      try { hostPort.onSkip(); } catch (e) { console.error('[onboarding] onSkip threw:', e); }
    }, 'crash-exit'));
    btnRow.appendChild(btnRetry);
    btnRow.appendChild(btnExit);
    panel.appendChild(btnRow);
    container.appendChild(panel);
    // focus 入口（spec §4.2 a11y）：crash 面板聚焦重试按钮
    try { btnRetry.focus(); } catch {}
  }

  // ── 阶段渲染分发 ──────────────────────────────────────────────────────────
  function renderStage() {
    // 释放上一阶段的 listener 引用，避免跨阶段导航时闭包泄漏（CodeRabbit id=3602857760）
    removeListeners();
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
      on(back, 'click', safeSync(() => { stage--; render(); }, 'nav-back'));
      nav.appendChild(back);
    }
    const skip = el('button', 'btn-tertiary onb-skip', '跳过向导');
    on(skip, 'click', safeSync(() => {
      // 任意阶段可跳过（spec §4.4）；Stage 6 跳过走 onComplete(firstChatCompleted:false)
      // Stage 6 跳过时必须先中止流式，防止 SSE 完成后再次触发 finish(true)（CodeRabbit id=3602857770）
      if (stage === 6) {
        if (sseAbort) { try { sseAbort.abort(); } catch {} sseAbort = null; }
        finish(false);
      }
      else { try { hostPort.onSkip(); } catch (e) { console.error('[onboarding] onSkip threw:', e); } }
    }, 'nav-skip'));
    nav.appendChild(skip);
    return nav;
  }

  // ── 错误展示 helper（F5/F6 阶段错误，不降级，spec §6.5）
  function showError(parent, message, onRetry, actionLabel) {
    const box = el('div', 'onb-error');
    box.appendChild(el('p', '', message));
    if (onRetry) {
      const btn = el('button', 'btn-secondary', actionLabel || '重试');
      on(btn, 'click', safeSync(onRetry, 'error-retry'));
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
      on(btn, 'click', safeSync(() => {
        // dev 模式 Stage 1 写 sessionStorage（spec §3.2），fetcher 每次调用读取
        sessionStorage.setItem('airp_engine_url', urlInput.value.replace(/\/+$/, ''));
        sessionStorage.setItem('airp_bearer', bearerInput.value || '');
        safeAsync(() => runHealthCheck(box), 'stage1-health-check');
      }, 'stage1-connect'));
      box.appendChild(btn);
    } else {
      box.appendChild(el('p', 'onb-hint', '生产模式：同源安全连接，网关注入认证。正在检查部署健康…'));
      setTimeout(() => { safeAsync(() => runHealthCheck(box), 'stage1-health-check-prod'); }, 0);
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
      on(next, 'click', safeSync(() => { stage = 2; render(); }, 'stage1-next'));
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
    // .then / .catch 走 safeAsync 边界：异常 → renderCrashFallback（spec §6.4 修正）
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
    }).catch(err => {
      console.error('[onboarding] stage2 settings GET failed:', err);
      try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
    });

    const btn = el('button', 'btn-primary', '保存并验证');
    on(btn, 'click', safeSync(() => {
      // 异步保存走 safeAsync 边界（spec §6.4）
      safeAsync(async () => {
        // api_key 仅在非空时携带（spec §4.3；空字符串=不修改）
        const body = {};
        if (providerInput.value.trim()) body.provider = providerInput.value.trim();
        if (endpointInput.value.trim()) body.endpoint = endpointInput.value.trim();
        if (modelInput.value.trim()) body.model = modelInput.value.trim();
        if (apiKeyInput.value.trim()) body.api_key = apiKeyInput.value.trim();
        // 安全不变量：api_key 永不进入向导 state（spec §3.6, §4.3）
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
      }, 'stage2-save');
    }, 'stage2-save-entry'));
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
        on(btn, 'click', safeSync(() => saveModelAndAdvance(input.value.trim()), 'stage3-save-text'));
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
      on(btn, 'click', safeSync(() => saveModelAndAdvance(select.value), 'stage3-save-pick'));
      box.appendChild(btn);
    }).catch(err => {
      console.error('[onboarding] stage3 models GET failed:', err);
      try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
    });

    return box;
  }

  async function saveModelAndAdvance(model) {
    if (!model) return; // 用户需选/输入
    // 整个 async 流程走 safeAsync，异常 → renderCrashFallback（spec §6.4）
    // 此处保留 try/catch 仅做"已知失败" → stage state 设置；真正未预期异常由 safeAsync 兜底
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
    box.appendChild(el('p', 'onb-hint', '上传 PNG 角色卡、角色卡 JSON 或 Preset JSON。生产模式仅支持内容上传，card_path 被拒绝。'));

    const fileInput = el('input', 'onb-input');
    fileInput.type = 'file';
    fileInput.accept = '.png,.json,image/png,application/json';
    box.appendChild(fileInput);

    const btn = el('button', 'btn-primary', '导入');
    const skipBtn = el('button', 'btn-secondary', '跳过（选择已有角色）');
    // 单飞守卫：导入是非幂等操作，双击会创建重复角色/预设。导入期间禁用按钮（CodeRabbit id=3602857791）
    on(btn, 'click', safeSync(() => {
      btn.disabled = true;
      safeAsync(() => importCharacterOrPreset(fileInput.files[0], box)
        .finally(() => { btn.disabled = false; }), 'stage4-import');
    }, 'stage4-import-entry'));
    on(skipBtn, 'click', safeSync(() => { stage = 5; render(); }, 'stage4-skip'));
    box.appendChild(btn);
    box.appendChild(skipBtn);
    return box;
  }

  // Stage 4 文件分发：PNG / 角色 JSON / Preset JSON 三路径（spec §4.3 修正）
  // Preset JSON 启发式检测：含 `prompts` 数组 → 路由 /v1/presets/import；
  // 否则按角色卡处理（含 chara_card_v2 spec 字段或裸 data 对象）。
  async function importCharacterOrPreset(file, box) {
    if (!file) { showError(box, '请选择文件', null); return; }
    // 客户端校验：10 MiB 字节计数（spec §4.3，复用 app.js:1807-1817 逻辑）
    if (file.size > 10 * 1024 * 1024) { showError(box, '文件超过 10 MiB 限制', null); return; }
    const buf = await file.arrayBuffer();
    const bytes = new Uint8Array(buf);
    // PNG magic bytes 检测：89 50 4E 47 0D 0A 1A 0A
    if (bytes.length >= 8 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4E && bytes[3] === 0x47) {
      let binary = '';
      for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
      await importCharacterCard(box, { card_png_base64: btoa(binary) });
      return;
    }
    // JSON 路径：先解析，再按 shape 分发
    const text = new TextDecoder().decode(bytes);
    let parsed;
    try { parsed = JSON.parse(text); } catch {
      showError(box, '文件既非 PNG 也非有效 JSON', null);
      return;
    }
    // Preset JSON 启发式：Tavern preset 顶层含 `prompts` 数组（spec §4.3）
    if (parsed && Array.isArray(parsed.prompts)) {
      await importPresetJson(box, parsed);
      return;
    }
    // 否则按角色卡处理（card_json 走 stringified）
    await importCharacterCard(box, { card_json: JSON.stringify(parsed) });
  }

  async function importCharacterCard(box, body) {
    // 生产模式不发送 card_path（spec §4.3，handlers 拒绝；向导侧也不构造）
    const r = await callApi('/v1/characters/import', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(body),
    });
    if (!r.ok) { showError(box, '角色卡导入失败：' + hostPort.formatError(r.data, r.text), null); return; }
    const cid = r.data && (r.data.character_id || r.data.id || r.data.uuid);
    if (!cid) { showError(box, '导入响应缺少 character_id', null); return; }
    // 列表校验：支持 bare array 与 {characters: [...]} 两种 shape（对齐 smoke.mjs:233 既有约定）
    const lr = await callApi('/v1/characters');
    const list = Array.isArray(lr.data) ? lr.data : (lr.ok && lr.data && Array.isArray(lr.data.characters) ? lr.data.characters : null);
    if (!lr.ok || !list || !list.some(c => (typeof c === 'string' ? c : (c.id || c.character_id)) === cid)) {
      showError(box, '导入后角色列表未包含该 ID，可能持久化失败', null);
      return;
    }
    state.characterId = cid;
    state.characterName = (list.find(c => (c.id || c.character_id) === cid) || {}).name || cid;
    stage = 5;
    render();
  }

  async function importPresetJson(box, presetObj) {
    // Preset 导入：POST /v1/presets/import，body = { preset_json: <stringified>, preset_id?: <id> }
    // preset_id 缺省时 engine 应生成；此处用文件名或 fallback 'onb-imported'
    const presetId = (presetObj && typeof presetObj.name === 'string' && presetObj.name)
      || 'onb-' + Date.now();
    const r = await callApi('/v1/presets/import', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ preset_id: presetId, preset_json: JSON.stringify(presetObj) }),
    });
    if (!r.ok) { showError(box, 'Preset 导入失败：' + hostPort.formatError(r.data, r.text), null); return; }
    const pid = r.data && (r.data.preset_id || r.data.id);
    if (!pid) { showError(box, 'Preset 导入响应缺少 preset_id', null); return; }
    state.presetId = pid;
    // Preset 导入后仍进入 Stage 5，让用户选 Persona / 确认 Preset 选择
    stage = 5;
    render();
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
      // 角色列表失败需有可行动错误 + 重试（CodeRabbit id=3602857767）
      callApi('/v1/characters').then(r => {
        // 列表 shape 兼容：bare array ['id1', ...] 与 {characters: [{id, ...}, ...]}
        const list = Array.isArray(r.data) ? r.data
          : (r.ok && r.data && Array.isArray(r.data.characters) ? r.data.characters : null);
        if (!r.ok || !list) {
          showError(box, '加载角色失败：' + hostPort.formatError(r.data, r.text), () => { render(); });
          return;
        }
        for (const c of list) {
          const id = typeof c === 'string' ? c : (c.id || c.character_id);
          const name = typeof c === 'string' ? c : (c.name || c.id || c.character_id);
          const opt = el('option', '', name);
          opt.value = id;
          charSelect.appendChild(opt);
        }
      }).catch(err => {
        console.error('[onboarding] stage5 char-list GET failed:', err);
        try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
      });
      const selBtn = el('button', 'btn-secondary', '确认角色');
      on(selBtn, 'click', safeSync(() => {
        if (charSelect.value) {
          state.characterId = charSelect.value;
          state.characterName = charSelect.options[charSelect.selectedIndex].text;
          render();
        }
      }, 'stage5-pick-char'));
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
      // 预选 Stage 4 导入的 preset，避免 Next 覆盖为 null（CodeRabbit id=3602857795）
      if (state.presetId) {
        const matched = Array.from(ppSelect.options).some(o => o.value === state.presetId);
        if (matched) ppSelect.value = state.presetId;
      }
      presetWrap.appendChild(ppSelect);

      // 预览按钮（spec §4.3：chat/preview 只读、不创建 session、不返回 prompt body/secrets）
      const previewBtn = el('button', 'btn-secondary', '预览 effective config');
      on(previewBtn, 'click', safeSync(() => safeAsync(() => previewEffective(pSelect.value, ppSelect.value || null, previewWrap), 'stage5-preview'), 'stage5-preview-entry'));
      box.appendChild(previewBtn);

      const btn = el('button', 'btn-primary', '下一步');
      on(btn, 'click', safeSync(() => {
        state.personaId = pSelect.value || 'default';
        state.presetId = ppSelect.value || null;
        stage = 6;
        render();
      }, 'stage5-next'));
      box.appendChild(btn);
    }).catch(err => {
      // 列表加载未预期异常 → 崩溃面板（spec §6.4 修正）
      console.error('[onboarding] stage5 Promise.all failed:', err);
      try { renderCrashFallback(err); } catch (e) { console.error('[onboarding] crash fallback threw:', e); }
    });
    return box;
  }

  async function previewEffective(personaId, presetId, wrap) {
    wrap.innerHTML = '';
    wrap.appendChild(el('p', 'onb-hint', '预览中…'));
    // 整个 async 流程由调用方 safeAsync 兜底；此处 try/catch 仅做"已知失败" → 阶段错误
    try {
      const body = {
        character_id: state.characterId,
        user_id: 'default',
        persona_id: personaId,
        user_profile: { name: '', variables: {} },
        message: '',
      };
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
    // 单飞期间禁用输入（spec §4.3 Stage 6 防双击/重试并发）
    input.disabled = sendInFlight;
    box.appendChild(input);

    const btn = el('button', 'btn-primary', sendInFlight ? '发送中…' : '发送');
    btn.disabled = sendInFlight;
    const btnFinishLater = el('button', 'btn-secondary', '完成向导，稍后聊天');
    btnFinishLater.disabled = sendInFlight;
    // 点击走 safeSync 边界 → safeAsync 包 sendFirstMessage（spec §6.4 修正）
    on(btn, 'click', safeSync(() => {
      if (sendInFlight) return;           // 单飞保护：重复点击直接忽略
      const msg = input.value.trim();
      if (!msg) return;
      safeAsync(() => sendFirstMessage(msg, box), 'stage6-send');
    }, 'stage6-send-click'));
    on(btnFinishLater, 'click', safeSync(() => { finish(false); }, 'stage6-finish-later'));
    box.appendChild(btn);
    box.appendChild(btnFinishLater);

    const replyWrap = el('div', 'onb-chat-reply');
    box.appendChild(replyWrap);
    return box;
  }

  async function sendFirstMessage(message, box) {
    // 单飞保护：进入即置位，finally 释放（防双击/重试并发，spec §4.3 Stage 6）
    if (sendInFlight) return;
    sendInFlight = true;
    // 立即把按钮切到 disabled + "发送中…"（避免在 await 期间用户再点）
    const sendBtn = box.querySelector('.btn-primary');
    const laterBtn = box.querySelector('.btn-secondary');
    const inputEl = box.querySelector('.onb-chat-input');
    if (sendBtn) { sendBtn.disabled = true; sendBtn.textContent = '发送中…'; }
    if (laterBtn) laterBtn.disabled = true;
    if (inputEl) inputEl.disabled = true;

    const replyWrap = box.querySelector('.onb-chat-reply');
    if (!replyWrap) { sendInFlight = false; return; }
    replyWrap.innerHTML = '';
    replyWrap.appendChild(el('p', 'onb-hint', '发送中…'));
    try {
      // 懒创建 session（spec §4.1 Stage 6）；若已有 sessionId 则复用，避免孤儿会话
      let sessionId = state.sessionId;
      if (!sessionId) {
        const sr = await callApi('/v1/sessions/' + encodeURIComponent(state.characterId), {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({}),
        });
        if (!sr.ok) {
          replyWrap.innerHTML = '';
          showError(replyWrap, '创建会话失败：' + hostPort.formatError(sr.data, sr.text), () => sendFirstMessage(message, box));
          return;
        }
        sessionId = sr.data && (sr.data.session_id || sr.data.uuid || sr.data);
        if (typeof sessionId !== 'string') {
          replyWrap.innerHTML = '';
          showError(replyWrap, '会话响应缺少 session_id', () => sendFirstMessage(message, box));
          return;
        }
        state.sessionId = sessionId;
      }

      // SSE 流式首聊（spec §4.3，走 Port.fetcher 注入 auth）
      if (sseAbort) { try { sseAbort.abort(); } catch {} }
      sseAbort = new AbortController();
      const body = {
        character_id: state.characterId,
        session_id: sessionId,
        user_id: 'default',
        persona_id: state.personaId || 'default',
        user_profile: { name: '', variables: {} },
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
        // F6：SSE 请求失败显示重试（保留 sessionId，重试时复用）
        showError(replyWrap, '请求失败：' + hostPort.formatError(data, text), () => sendFirstMessage(message, box));
        return;
      }
      // 解析 SSE
      replyWrap.innerHTML = '';
      const bodyP = el('div', 'onb-reply-body');
      replyWrap.appendChild(bodyP);
      const reader = res.body.getReader();
      const decoder = new TextDecoder();
      let buf = '';
      // SSE 终止标记跟踪（spec §6.6 修正）：
      // reader.read() done=true 不等于流正常完成；只有 [DONE] sentinel 或 done chunk 才算正常出口。
      // 提前 EOF（reader done 但未见 sentinel）提交状态不确定，不得盲目重发。
      let completed = false;
      let receivedAny = false;
      let eventName = 'message';
      while (true) {
        const { value, done: rDone } = await reader.read();
        if (rDone) break;
        buf += decoder.decode(value, { stream: true });
        const lines = buf.split('\n');
        buf = lines.pop() || '';
        for (const line of lines) {
          if (line.startsWith('event:')) {
            eventName = line.slice(6).trim() || 'message';
            continue;
          }
          if (line.trim() === '') {
            eventName = 'message';
            continue;
          }
          if (!line.startsWith('data: ')) continue;
          const payload = line.slice(6);
          if (payload === '[DONE]') { completed = true; receivedAny = true; break; }
          let chunk;
          try { chunk = JSON.parse(payload); } catch { continue; }
          if (eventName === 'error') {
            const detail = chunk.error || chunk;
            const error = new Error(detail.message || chunk.text || 'stream failed');
            error.kind = 'stream_error';
            error.code = detail.code || 'stream_error';
            error.retryable = detail.retryable === true;
            error.commitState = detail.commit_state || 'ambiguous';
            throw error;
          }
          receivedAny = true;
          // chunk 类型：body_chunk/think_chunk/plan/tool_call/tool_result/done
          // spec §4.3：不强制 Agent tool 调用——展示但不作通过条件
          if (chunk.type === 'body_chunk' && chunk.text) {
            bodyP.append(chunk.text);
          } else if (chunk.type === 'done') {
            completed = true;
            break;
          }
          // think_chunk/plan/tool_call/tool_result 展示与否可选，首版不展示避免 UI 复杂
        }
        if (completed) break;
      }
      // SSE 终止标记判定（spec §6.6 修正）：
      // - completed=true → 收到 [DONE] sentinel 或 done chunk，走 onComplete 出口
      // - completed=false → 提前 EOF，提交状态不确定，只允许进入聊天检查历史
      if (completed) {
        finish(true);
      } else {
        replyWrap.innerHTML = '';
        showError(replyWrap,
          receivedAny ? '回复中断：流提前结束（未收到 done 标记）' : '回复中断：未收到任何流数据',
          () => finish(false), '进入聊天检查记录');
      }
    } catch (err) {
      // F6：SSE 中断（网络异常 / abort）
      if (replyWrap) {
        replyWrap.innerHTML = '';
        const suffix = err.commitState ? '（提交状态：' + err.commitState + '）' : '（提交状态不确定）';
        const canResend = err.retryable === true && err.commitState === 'uncommitted';
        showError(replyWrap,
          '回复中断：' + hostPort.formatError(null, String(err.message || err)) + suffix,
          canResend ? () => sendFirstMessage(message, box) : () => finish(false),
          canResend ? '重试' : '进入聊天检查记录');
      }
    } finally {
      // 单飞释放（spec §4.3 Stage 6）：无论成功/失败/异常都恢复按钮状态。
      // completed=true 时已走 finish(true)→onComplete，宿主会卸载向导，UI 恢复无意义但安全。
      sendInFlight = false;
      if (sendBtn) { sendBtn.disabled = false; sendBtn.textContent = '发送'; }
      if (laterBtn) laterBtn.disabled = false;
      if (inputEl) inputEl.disabled = false;
      if (sseAbort) { try { sseAbort.abort(); } catch {} sseAbort = null; }
    }
  }

  // ── 出口：onComplete / finish ─────────────────────────────────────────────
  // finish() 必须幂等：Stage 6 Skip 可能在流式中触发 finish(false)，
  // 之后 SSE 完成路径可能再触发 finish(true)。用 finished 守卫防双重 onComplete（CodeRabbit id=3602857770）。
  let finished = false;
  function finish(firstChatCompleted) {
    if (finished) return;
    finished = true;
    const config = {
      provider: state.provider || '',
      model: state.model || '',
      character_id: state.characterId || '',
      session_id: state.sessionId || '',
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
    sendInFlight = false;
    listeners.forEach(({ target, type, handler, opts }) => {
      try { target.removeEventListener(type, handler, opts); } catch {}
    });
    listeners.length = 0;
    container.innerHTML = '';
    // 焦点恢复（spec §4.2 a11y）：cleanup 时恢复 mount 前焦点元素
    try {
      if (preMountFocus && typeof preMountFocus.focus === 'function') {
        preMountFocus.focus();
      }
    } catch {}
    preMountFocus = null;
  };
}
