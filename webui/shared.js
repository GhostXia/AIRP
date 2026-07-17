// AIRP WebUI shared helpers — extracted from app.js for onboarding wizard Port injection.
// Loaded as a classic script; exposes window.AIRPShared. app.js and onboarding.js both
// depend on it, but onboarding.js receives these functions via the hostPort (no direct
// import of app.js). This module is the single source of truth for auth + error formatting.

(function () {
  'use strict';

  // makeFetcher(mode) → fetcher(path, opts)
  //
  // Encapsulates base URL + bearer auth so the onboarding wizard never touches
  // credentials directly. Behaviour matches app.js:202-216 (connect) and app.js:147-171
  // (api/headers), but returns a raw fetch Response instead of the {ok,status,data,text}
  // envelope — callers parse JSON/text themselves. This keeps the contract standard.
  //
  // mode === 'production': same-origin, gateway injects Authorization. Browser never
  //   holds the access key.
  // mode === 'dev': each call reads sessionStorage('airp_engine_url'/'airp_bearer')
  //   so the wizard's Stage 1 write takes effect immediately without a Port setter.
  //   Falls back to http://127.0.0.1:8000 if unset.
  //
  // CodeRabbit id=3602743403：fetcher 缺少超时保护。已采纳：调用方未提供 signal 时
  //   注入 30s 默认超时（AbortController），避免向导或主界面挂在 unreachable engine。
  //   SSE 调用方（Stage 6）自带 sseAbort.signal → 跳过默认超时，不影响流式长响应。
  //
  // CodeRabbit id=3602743403：!res.ok → throw 归一化未采纳。原因：
  //   1) 现有契约是"raw Response，调用方解析"；onboarding.js callApi 与 app.js api()
  //      都需要读取非 200 响应体来 formatError。throw 会丢失 body 上下文。
  //   2) SSE 端点 200 + 流式 body——!res.ok throw 对它无意义。
  //   3) 强制 try/catch 在所有调用点扩散，违反"最小修改"原则。
  //   若未来统一错误模型，应在 callApi 层包装而非 fetcher 层。
  const DEFAULT_TIMEOUT_MS = 30000;
  function makeFetcher(mode) {
    return async function fetcher(path, opts) {
      opts = opts || {};
      let base, bearer;
      if (mode === 'production') {
        base = window.location.origin;
        bearer = '';
      } else {
        base = (sessionStorage.getItem('airp_engine_url') || 'http://127.0.0.1:8000').replace(/\/+$/, '');
        bearer = sessionStorage.getItem('airp_bearer') || '';
      }
      const headers = Object.assign({}, opts.headers || {});
      if (bearer) headers['Authorization'] = 'Bearer ' + bearer;
      // 超时策略：调用方自带 signal（如 SSE sseAbort）→ 完全交给调用方管理；
      //   否则注入 30s 默认超时（CodeRabbit id=3602743403 采纳）。
      let signal = opts.signal || null;
      let timeoutCtl = null;
      let timeoutId = null;
      if (!signal) {
        try {
          timeoutCtl = new AbortController();
          signal = timeoutCtl.signal;
          timeoutId = setTimeout(() => { try { timeoutCtl.abort(); } catch {} }, DEFAULT_TIMEOUT_MS);
        } catch { signal = null; }  // AbortController 不支持时降级为无超时
      }
      const finalOpts = Object.assign({}, opts, { headers: headers });
      if (signal) finalOpts.signal = signal;
      try {
        const res = await fetch(base + path, finalOpts);
        return res;
      } finally {
        // 清理超时定时器，避免 30s 内残留回调持有 AbortController 闭包（CodeRabbit id=3602857801）
        if (timeoutId !== null) { clearTimeout(timeoutId); timeoutId = null; }
      }
    };
  }

  // formatError(data, text) — lifted verbatim from app.js:176-200.
  // Whitelist-expands known engine error fields, folds unknown fields into raw JSON.
  // Used by both app.js and onboarding.js so error UI stays consistent.
  function formatError(data, text) {
    if (data && typeof data === 'object' && data.error) {
      const err = data.error;
      const KNOWN_FIELDS = ['code', 'message', 'upstream_status', 'upstream_body', 'detail'];
      const lines = [err.code || 'error'];
      if (err.message) lines.push(err.message);
      if (err.upstream_status) lines.push('upstream_status=' + err.upstream_status);
      if (err.upstream_body) lines.push('upstream_body=' + err.upstream_body);
      if (err.detail) lines.push('detail=' + err.detail);
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

  // readJson(res) — convenience for Port consumers: parse JSON body, fall back to text.
  // Mirrors app.js:163-164 try/catch pattern.
  async function readJson(res) {
    const text = await res.text();
    let data;
    try { data = JSON.parse(text); } catch { data = text; }
    return { ok: res.ok, status: res.status, data: data, text: text };
  }

  window.AIRPShared = Object.freeze({
    makeFetcher: makeFetcher,
    formatError: formatError,
    readJson: readJson,
  });
})();
