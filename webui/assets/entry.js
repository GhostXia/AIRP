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

  const MAIN_UI = 'screens/01-role-list.html';
  const WIZARD = 'screens/16-onboarding.html';

  function targetUrl(path) {
    const target = new URL(path, location.href);
    // entry 跳转要保留 engine/desktop 等 query 参数，但绝不把 token 放入 URL。
    target.search = location.search;
    return target;
  }

  function redirect(onboarded, health) {
    if (!onboarded) return showFirstRunChoice(health);
    location.replace(targetUrl(MAIN_UI).href);
  }

  function persistLocalMarker() {
    try { localStorage.setItem('airp_onboarded', 'true'); } catch (e) { /* noop */ }
  }

  function bearerHeaders() {
    let bearer = '';
    try { bearer = sessionStorage.getItem('airp_bearer') || ''; } catch (e) { /* noop */ }
    return bearer ? { Authorization: 'Bearer ' + bearer } : {};
  }

  function completeAndEnterMain(button, status) {
    if (button && button.disabled) return;
    if (button) {
      button.disabled = true;
      if (button.setAttribute) button.setAttribute('aria-disabled', 'true');
    }
    if (status) status.textContent = '正在进入主界面…';
    let request;
    try {
      // keepalive lets the marker request finish while the browser navigates away.
      request = fetch('/v1/onboarding/complete', {
        method: 'POST',
        headers: bearerHeaders(),
        keepalive: true,
      });
    } catch (error) {
      request = Promise.reject(error);
    }
    Promise.resolve(request).then(function (response) {
      if (response && response.ok === false) throw new Error('onboarding marker request failed');
      // The Engine marker is authoritative; localStorage remains only an offline fallback.
      persistLocalMarker();
      redirect(true);
    }).catch(function () {
      // Do not leave the entry page until the Engine has acknowledged its marker.
      if (status) status.textContent = '无法保存首次启动状态；请检查 Engine 后重试。';
      if (button) {
        button.disabled = false;
        if (button.removeAttribute) button.removeAttribute('aria-disabled');
      }
    });
  }

  // Keep the legacy fallback safe even if another layer makes it visible while
  // health is still pending: entering the main UI always records the Engine marker.
  const fallbackLink = document.querySelector('#entry-fallback');
  const entryStatus = document.querySelector('#entry-status');
  if (fallbackLink) fallbackLink.addEventListener('click', function (event) {
    event.preventDefault();
    completeAndEnterMain(fallbackLink, entryStatus);
  });

  function showFirstRunChoice(health) {
    const title = document.querySelector('#entry-title');
    const description = document.querySelector('#entry-description');
    const status = document.querySelector('#entry-status');
    const actions = document.querySelector('#entry-actions');
    const start = document.querySelector('#entry-start-wizard');
    const enter = document.querySelector('#entry-enter-main');
    const fallback = document.querySelector('#entry-fallback');
    if (!actions || !start || !enter) {
      // Keep older bundled entry pages usable if they have not yet received the gate markup.
      location.replace(targetUrl(WIZARD).href);
      return;
    }
    if (title) title.textContent = '欢迎使用 AIRP';
    if (description) description.textContent = '首次运行可以先完成启动向导，也可以直接进入主界面。';
    if (status) {
      status.textContent = health && health.provider_configured === false
        ? 'Provider 尚未配置；可以稍后在设置中完成，不影响进入主界面。'
        : '请选择下一步。';
    }
    start.href = targetUrl(WIZARD).href;
    actions.hidden = false;
    if (fallback) fallback.hidden = true;
    enter.addEventListener('click', function () { completeAndEnterMain(enter, status); });
  }

  // #303: onboarding 状态从 Engine data_root 读取，不再依赖浏览器 localStorage
  fetch('health').then(function (r) {
    if (r.ok === false) throw new Error('health request failed');
    return r.json();
  }).then(function (h) {
    redirect(h && h.onboarded === true, h);
  }).catch(function () {
    // Engine 不可达时回退 localStorage（兼容离线/旧版）
    let fallback = false;
    try { fallback = localStorage.getItem('airp_onboarded') === 'true'; } catch (e) { /* noop */ }
    redirect(fallback);
  });
})();
