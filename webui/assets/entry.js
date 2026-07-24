(function () {
  'use strict';
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
