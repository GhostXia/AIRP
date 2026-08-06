// C-P1：沙箱引导脚本（经典脚本；运行于 sandbox="allow-scripts" 的
// opaque-origin iframe 内，见 sandbox-frame.html）。
//
// 职责：等待宿主 mount 消息 → 动态 import() widget source（绝对 URL，经
// ?src= 传入）→ 调用 factory().mount(document.body, ctxProxy)。ctxProxy 把
// WidgetContext 的每次调用翻译为对宿主的 postMessage；widget 永远拿不到
// 宿主对象引用。state 缓冲回放保证首个 state 切片不丢（对译
// ui/src/registry/sandbox-bridge.ts 的 SANDBOX_BOOTSTRAP 行为）。
//
// module import 自 opaque origin 属 CORS 请求；engine 对 /assets/widgets/
// 响应附 Access-Control-Allow-Origin:*（本地 loopback 静态资产）。
(function () {
  'use strict';

  var params = new URLSearchParams(window.location.search);
  var SRC = params.get('src');
  // opaque origin 读不到父源（且 engine 设 no-referrer）；宿主把自己的
  // origin 经 ?origin= 传入，回邮用它做精准 targetOrigin（实证：对真实
  // origin 的父窗发 targetOrigin "null" 会被浏览器静默丢弃）。
  var TARGET = params.get('origin') || '*';

  function send(msg) {
    // 宿主以 event.source === iframe.contentWindow 门控消息来源，恶意
    // frame 无法冒充；即便 TARGET 退化 '*' 也仅投递给父窗。
    parent.postMessage(msg, TARGET);
  }

  // 缓冲最新 state：state 消息可能先于 widget 的异步 import 注册 onState
  // 回调而到达。保留最后值并在注册时回放，首个 state 切片绝不丢失。
  var lastState;
  var hasState = false;
  var stateCb = null;

  // WidgetContext 代理：widget 调用这些方法，我们翻译为消息。
  var ctx = {
    instance: null,
    getState: function () { return hasState ? lastState : undefined; },
    onState: function (cb) {
      stateCb = cb;
      if (hasState) {
        try { cb(lastState); } catch (e) { send({ kind: 'error', message: String(e && e.message || e) }); }
      }
      return function () { if (stateCb === cb) stateCb = null; };
    },
    emit: function (name, params) { send({ kind: 'intent', name: name, params: params }); },
    capabilities: [],
  };

  window.addEventListener('message', function (ev) {
    var msg = ev.data || {};
    if (msg.kind === 'mount') {
      ctx.instance = msg.instance;
      ctx.capabilities = msg.capabilities || [];
      if (!SRC) {
        send({ kind: 'error', message: 'sandbox frame missing src parameter' });
        return;
      }
      import(SRC)
        .then(function (mod) {
          var factory = typeof mod === 'function' ? mod : mod.default;
          if (typeof factory !== 'function') throw new Error('esm widget default export must be a WidgetFactory');
          return factory().mount(document.body, ctx);
        })
        .catch(function (e) { send({ kind: 'error', message: String(e && e.message || e) }); });
    } else if (msg.kind === 'state') {
      lastState = msg.state;
      hasState = true;
      try { if (stateCb) stateCb(msg.state); } catch (e) { send({ kind: 'error', message: String(e && e.message || e) }); }
    }
  });

  send({ kind: 'ready' });
})();
