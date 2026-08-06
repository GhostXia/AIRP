// C-P1：沙箱桥（对译 ui/src/registry/sandbox-bridge.ts 行为基准）。
//
// 把第三方（esm）widget 承载进 sandboxed iframe，并以 postMessage 桥接
// WidgetContext。安全模型（docs/SECURITY.md）：iframe 以 sandbox="allow-scripts"
// 且**不带** allow-same-origin 创建，运行于 opaque origin，无法触及宿主 DOM、
// window、localStorage、cookie 与同源网络。widget 的 WidgetContext 被代理：
// 宿主监听 iframe 的 intent 消息并向内推送 state；widget 永远拿不到宿主对象引用。
//
// 承载方式（实证 2026-08-05，srcdoc 方案被 Chrome 实证否决）：iframe 的 src
// 指向同源静态页 assets/widgets/sandbox-frame.html（外链 sandbox-frame.css +
// sandbox-frame.js，全部 'self' 资源，父页 CSP 天然放行——srcdoc 的内联
// script/style 会被父页 CSP 拦截，故不用）。widget source 由宿主解析为绝对
// URL 后经 ?src= 传入；frame 引导脚本（经典脚本）动态 import() 该 URL 并
// 调用其 mount(iframe.document.body, ctxProxy)。module import 自 opaque origin
// 属 CORS 请求，engine 对 /assets/widgets/ 响应附 Access-Control-Allow-Origin:*
// （静态公开资产，无敏感面）。跨源 esm 的 CORS 行为留给 C-P2 R0 硬门禁。
//
// 消息协议（host ↔ iframe；宿主以 event.source === iframe.contentWindow 门控）：
//  - host → iframe：targetOrigin 只能 '*'（实证 2026-08-05：Chrome 不接受
//    'null' 作 targetOrigin 且 opaque origin 不可精准匹配；'*' 仅投递给本
//    iframe 窗口对象，frame 导航面为本地静态页）；{ kind:"mount", instance,
//    capabilities }；其后每次 state 变化 { kind:"state", state }。
//  - iframe → host：宿主 origin 由宿主经 frame URL 的 ?origin= 传入（opaque
//    origin 读不到父源，实证 2026-08-05：对真实 origin 的父窗发
//    targetOrigin "null" 会被浏览器静默丢弃），{ kind:"ready" }（引导已加载，
//    等待 mount）、{ kind:"intent", name, params }、{ kind:"error", message }。
//
// transport（SandboxTransport）可注入，桥逻辑可无真实 iframe 单测。
(function (root, factory) {
  const api = factory();
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPSandboxBridge = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function () {
  'use strict';

  /** 沙箱引导页相对屏页面（screens/*.html）的位置。 */
  const FRAME_RELATIVE = '../assets/widgets/sandbox-frame.html';

  /**
   * 沙箱 widget 的宿主侧桥。持有 transport、向内转发 state、向外暴露
   * intent/error。宿主（widget-host.js）在 manifest 的 esm entry 通过
   * consent + sandbox 门控后构造它：mount() 一次、每次 state 变化 pushState()、
   * 卸载时 destroy()。
   */
  class SandboxBridge {
    constructor(transport, onIntent, onError) {
      this.transport = transport;
      this.onIntent = onIntent;
      this.onError = onError;
      this.destroyed = false;
      this.ready = false;
      /** mount() 的等待者，停泊到 iframe 发出 ready 为止。 */
      this.readyWaiters = [];
      this.off = transport.onMessage((msg) => {
        if (this.destroyed) return;
        if (msg.kind === 'ready') {
          // 在常开监听（而非 mount() 内部）捕获 ready：ready 可能先于 mount()
          // 到达——iframe 引导脚本一执行就发 ready，会抢跑在宿主 mount() 之前，
          // 这里捕获后不让它丢失。
          if (!this.ready) {
            this.ready = true;
            const waiters = this.readyWaiters;
            this.readyWaiters = [];
            for (const w of waiters) w();
          }
        } else if (msg.kind === 'intent') this.onIntent(msg.name, msg.params);
        else if (msg.kind === 'error') this.onError(msg.message);
      });
    }

    /**
     * 通知 iframe 挂载 widget。在 iframe 发出 ready（引导已加载）后 resolve——
     * ready 已到达则立即发送——随后发出 mount 消息。iframe 始终不 ready 时，
     * readyTimeoutMs（默认 5s）后 reject，让宿主暴露加载失败而非悬挂。
     */
    mount(instance, capabilities, readyTimeoutMs) {
      if (readyTimeoutMs === undefined) readyTimeoutMs = 5000;
      return new Promise((resolve, reject) => {
        if (this.destroyed) return reject(new Error('sandbox destroyed'));
        const sendMount = () => {
          this.transport.postMessage({ kind: 'mount', instance, capabilities });
          resolve();
        };
        // 已就绪（可能早于本次调用）：立即 mount，无竞态窗口。
        if (this.ready) {
          sendMount();
          return;
        }
        let done = false;
        const timer = setTimeout(() => {
          if (done) return;
          done = true;
          // 摘掉 waiter，晚到的 ready 不会触发已拒绝的 mount。
          this.readyWaiters = this.readyWaiters.filter((w) => w !== waiter);
          reject(new Error('sandbox iframe did not signal ready in time'));
        }, readyTimeoutMs);
        const waiter = () => {
          if (done) return;
          done = true;
          clearTimeout(timer);
          sendMount();
        };
        waiter.timer = timer; // 测试钩子：拒绝后可清理未触发定时器
        this.readyWaiters.push(waiter);
      });
    }

    /** 向 iframe 推送新的 state 切片。 */
    pushState(state) {
      if (this.destroyed) return;
      this.transport.postMessage({ kind: 'state', state });
    }

    /** 拆除：停止转发、销毁 iframe。 */
    destroy() {
      if (this.destroyed) return;
      this.destroyed = true;
      this.off();
      this.transport.destroy();
    }
  }

  /**
   * 构建真实 iframe transport：创建 <iframe sandbox="allow-scripts"
   * src="sandbox-frame.html?src=...">，接通 postMessage（以
   * event.source === iframe.contentWindow 门控），返回 SandboxTransport。
   * 宿主把 iframe 追加进 container。
   *
   * source 相对宿主页面（doc.baseURI）解析为绝对 URL 后传入 frame——frame
   * 内是独立文档，不能继承宿主的相对基准。
   */
  function createIframeTransport(container, source, doc) {
    const d = doc || document;
    const win = d.defaultView || (typeof window !== 'undefined' ? window : null);
    const base = d.baseURI || (win && win.location ? win.location.href : undefined);
    const absoluteSource = new URL(source, base).href;
    // frame 读不到父源（opaque origin + no-referrer），由宿主把自己的 origin
    // 经 ?origin= 传入，供 iframe 回邮时做精准 targetOrigin。
    const hostOrigin = new URL(base).origin;
    const frameUrl = new URL(
      FRAME_RELATIVE + '?src=' + encodeURIComponent(absoluteSource) + '&origin=' + encodeURIComponent(hostOrigin),
      base,
    ).href;

    const iframe = d.createElement('iframe');
    // allow-scripts 让 widget 运行；刻意不带 allow-same-origin，
    // iframe 为 opaque origin，无法读取宿主 DOM/存储/cookie。
    iframe.setAttribute('sandbox', 'allow-scripts');
    iframe.setAttribute('src', frameUrl);
    // 透明 + 填满：widget 在 iframe 内自渲染。
    iframe.style.border = '0';
    iframe.style.width = '100%';
    iframe.style.height = '100%';
    iframe.style.background = 'transparent';
    container.appendChild(iframe);

    const listeners = new Set();
    function onWindow(ev) {
      // 门控：只接受来自本 iframe 窗口的消息。恶意兄弟 frame 无法冒充——
      // 它的 source 不同。
      if (ev.source !== iframe.contentWindow) return;
      const msg = ev.data;
      if (!msg || typeof msg.kind !== 'string') return;
      for (const cb of listeners) cb(msg);
    }
    win.addEventListener('message', onWindow);

    return {
      postMessage: (msg) => {
        // Chrome 不接受 'null' 作 targetOrigin（抛 SyntaxError），opaque origin
        // 也无法精准匹配；'*' 只投递给本 iframe 窗口对象，无旁路。
        if (iframe.contentWindow) iframe.contentWindow.postMessage(msg, '*');
      },
      onMessage: (cb) => {
        listeners.add(cb);
        return () => listeners.delete(cb);
      },
      destroy: () => {
        win.removeEventListener('message', onWindow);
        listeners.clear();
        iframe.remove();
      },
    };
  }

  return { SandboxBridge, createIframeTransport, FRAME_RELATIVE };
});
