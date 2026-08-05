// C-P2：desktop session token 续期（rotation）的 webui 侧唯一入口。
//
// 背景：桌面壳经 POST /v1/desktop-session 换取 8h UI token；token 过半时
// 壳主动续期并经 webview.eval 推送新值（见 ui/src-tauri/src/main.rs）。
// 本模块承担 webui 侧的兜底半边：任何请求撞 401（token 过期/被 rotation
// 撤销）时，api-client 的 onUnauthorized 钩子调 renewDesktopSession()——
// 有效则 rotation 换新 token 写入 sessionStorage.airp_bearer 并返回 true
// （触发单次重试），无效则返回 false（回到既有登录提示行为）。
//
// 纯浏览器 local-webui 模式（无 access key）：renew 端点 403 fail-closed，
// 本模块返回 false，行为与 C-P1 一致（不硬失败、不假交互）。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPDesktopSession = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  const STORAGE_KEY = 'airp_bearer';

  function currentBearer() {
    try { return sessionStorage.getItem(STORAGE_KEY) || ''; } catch { return ''; }
  }

  /**
   * 以当前 bearer 调 POST /v1/desktop-session/renew（rotation：撤旧发新）。
   * 成功：写入新 token、dispatch 'airp-bearer-renewed'、返回 true；
   * 失败（401 旧 token 无效 / 403 无 key / 网络错误）：返回 false。
   * 并发安全：同页多请求同时撞 401 时，以 in-flight Promise 去重（rotation
   * 语义下第二次 renew 必然失败，去重避免无谓 401 噪音）。
   */
  let inflight = null;
  function renewDesktopSession(options) {
    if (inflight) return inflight;
    const opts = options || {};
    // 别名 fetch：测试可注入 fetchImpl；下方字面 fetch 调用保持可被
    // endpoint-guard / route-contract 的直接 fetch 扫描命中。
    const fetch = typeof opts.fetchImpl === 'function' ? opts.fetchImpl : globalThis.fetch;
    const base = String(opts.base || (globalThis.location && globalThis.location.origin) || '').replace(/\/+$/, '');
    inflight = (async () => {
      const bearer = currentBearer();
      if (!bearer) return false;
      try {
        const resp = await fetch(base + '/v1/desktop-session/renew', {
          method: 'POST',
          headers: { Authorization: 'Bearer ' + bearer },
        });
        if (!resp.ok) return false;
        const body = await resp.json();
        if (!body || typeof body.token !== 'string' || !body.token) return false;
        try { sessionStorage.setItem(STORAGE_KEY, body.token); } catch { return false; }
        try {
          globalThis.dispatchEvent(new CustomEvent('airp-bearer-renewed', {
            detail: { expires_in: body.expires_in || null },
          }));
        } catch { /* 事件派发失败不影响续期结果 */ }
        return true;
      } catch {
        return false;
      } finally {
        inflight = null;
      }
    })();
    return inflight;
  }

  return { renewDesktopSession, currentBearer, STORAGE_KEY };
});
