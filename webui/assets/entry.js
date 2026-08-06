(function () {
  'use strict';
  // C-P0：桌面壳 bearer 注入通道承接点。
  // Tauri 壳用进程互信（access key）向 engine 换取短时效 desktop session
  // token 后，以 URL fragment（#airp-token=...）导航首屏。fragment 不发送到
  // 服务端、不进日志与 Referer；这里写入 sessionStorage.airp_bearer 后立即
  // 清理 URL，后续屏由 console-runtime.js / chat-space.js 读取并经
  // api-client.js 的同源 bearer 防护发出。
  function applyDesktopTokenFromFragment() {
    var PREFIX = '#airp-token=';
    var hash = location.hash || '';
    if (hash.indexOf(PREFIX) !== 0) return;
    var token = '';
    try { token = decodeURIComponent(hash.slice(PREFIX.length)); } catch (e) { token = hash.slice(PREFIX.length); }
    if (token) {
      try { sessionStorage.setItem('airp_bearer', token); } catch (e) { /* noop */ }
    }
    // 清理 fragment，保留 search（entry 跳转会把 search 透传给目标屏）。
    history.replaceState(null, '', location.pathname + location.search);
  }
  applyDesktopTokenFromFragment();

  // #303: onboarding 状态从 Engine data_root 读取，不再依赖浏览器 localStorage
  fetch('health').then(function (r) { return r.json(); }).then(function (h) {
    redirect(h && h.onboarded === true);
  }).catch(function () {
    // Engine 不可达时回退 localStorage（兼容离线/旧版）
    let fallback = false;
    try { fallback = localStorage.getItem('airp_onboarded') === 'true'; } catch (e) { /* noop */ }
    redirect(fallback);
  });
  function redirect(onboarded) {
    const target = new URL(onboarded ? 'screens/01-role-list.html' : 'screens/16-onboarding.html', location.href);
    target.search = location.search;
    location.replace(target.href);
  }
})();
