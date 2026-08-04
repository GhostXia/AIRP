// C-P1：widget 宿主（对译 ui/src/components/WidgetHost.vue 四态，~150 行核心）。
//
// 四态（与 Vue 基准一致）：
//   failed    —— widget 出错（错误信息可见）
//   gated     —— 第三方 esm 未获 consent（授权 UI：来源/申请权限/授权按钮）
//   sandboxed —— 已同意的 esm widget，opaque-origin iframe + postMessage 桥
//   module    —— 进程内 framework-agnostic module widget
//   missing   —— 未注册的 type
//
// BUG-6 修复（审计基线 2026-08-04）：Vue 基准里 sandboxed 是 opt-in
// （entry.sandbox === true 才进沙箱），第三方 esm 未声明 sandbox 即与宿主
// 同进程运行。本宿主反转默认：**esm entry 缺 sandbox:true 一律拒载**并进入
// failed 态——缺失即拒绝，不存在进程内第三方代码路径。
//
// 容错：misbehaving widget 的 unmount/destroy 抛错不破坏宿主 teardown。
(function (root, factory) {
  const api = factory(root);
  if (typeof module === 'object' && module.exports) module.exports = api;
  root.AIRPWidgetHost = api;
})(typeof globalThis !== 'undefined' ? globalThis : this, function (root) {
  'use strict';

  function deps(options) {
    const o = options || {};
    return {
      registry: o.registry || root.AIRPWidgetRegistry,
      manifests: o.manifests || root.AIRPWidgetManifests,
      consent: o.consent || root.AIRPWidgetConsent,
      sandbox: o.sandbox || root.AIRPSandboxBridge,
      doc: o.doc || (typeof document !== 'undefined' ? document : null),
      // 可注入 transport 工厂（测试用假 transport，浏览器用真 iframe）。
      transportFactory: o.transportFactory || null,
    };
  }

  function errMsg(e) {
    return e instanceof Error ? e.message : String(e);
  }

  function el(doc, tag, className, text) {
    const node = doc.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  }

  /** BUG-6：第三方 esm widget 必须声明 entry.sandbox === true 才允许加载。 */
  function sandboxEnforced(manifest) {
    return Boolean(
      manifest
      && manifest.entry
      && manifest.entry.kind === 'esm'
      && manifest.entry.sandbox !== true,
    );
  }

  /**
   * 在一个容器元素内挂载一个 widget 实例，返回宿主句柄：
   *   pushState(state) / destroy() / approve() / state()。
   * @param {HTMLElement} container slot 挂载容器
   * @param {object} instance WidgetInstance { id, type, props?, state?, capabilities? }
   * @param {unknown} initialState 该 widget 作用域的初始 state 切片
   * @param {object} [options] onIntent(name, params)、注入依赖（测试）
   */
  function mountWidget(container, instance, initialState, options) {
    const d = deps(options);
    // doc 优先注入值，其次容器所属 document（node 单测无全局 document 时的兜底）。
    const doc = d.doc || (container && container.ownerDocument) || null;
    const onIntent = options && typeof options.onIntent === 'function' ? options.onIntent : () => {};
    let currentState = initialState;
    let failed = null;
    let mod = null;
    let stateCb = null;
    let bridge = null;
    let mounted = false;
    let destroyed = false;

    const reg = d.registry.resolveWidget(instance.type);
    const manifest = d.manifests.getManifest(instance.type);

    function effectiveCaps() {
      return manifest ? d.consent.effectiveCapabilities(manifest) : (instance.capabilities || []);
    }

    function makeContext() {
      return {
        instance,
        getState: () => currentState,
        onState: (cb) => {
          stateCb = cb;
          return () => { if (stateCb === cb) stateCb = null; };
        },
        emit: (name, params) => onIntent(name, params),
        // 只有经同意的 capabilities 到达 widget（宿主强制）。
        capabilities: effectiveCaps(),
      };
    }

    async function mountModule(target) {
      if (mounted) return;
      mounted = true;
      try {
        const loaded = reg.load();
        mod = loaded && typeof loaded.then === 'function' ? await loaded : loaded;
        await mod.mount(target, makeContext());
        if (stateCb) stateCb(currentState);
      } catch (e) {
        mounted = false;
        failed = errMsg(e);
        render();
      }
    }

    async function mountSandbox(target) {
      if (bridge) return;
      const source = manifest && manifest.entry ? manifest.entry.source : null;
      if (!source) {
        failed = 'sandboxed esm widget missing entry.source';
        render();
        return;
      }
      try {
        const transport = d.transportFactory
          ? d.transportFactory(target, source)
          : d.sandbox.createIframeTransport(target, source, doc);
        bridge = new d.sandbox.SandboxBridge(
          transport,
          (name, params) => onIntent(name, params),
          (message) => {
            failed = message;
            render();
          },
        );
        await bridge.mount(instance, effectiveCaps());
        bridge.pushState(currentState);
      } catch (e) {
        failed = errMsg(e);
        render();
      }
    }

    function approve() {
      if (manifest) {
        d.consent.grant(manifest);
        render(); // 授权后重挂载（等价 Vue 基准的 watch(el) immediate 语义）
      }
    }

    function render() {
      container.replaceChildren();
      if (destroyed) return;

      if (failed) {
        container.appendChild(el(doc, 'div', 'widget-error', 'widget 出错：' + instance.type + ' — ' + failed));
        return;
      }

      // BUG-6 硬门禁先于 consent：未声明 sandbox 的第三方 esm 直接拒载，
      // 不暴露「授权进程内加载」的假选择。
      if (sandboxEnforced(manifest)) {
        failed = '第三方 esm widget 未声明 entry.sandbox:true，宿主拒绝加载（BUG-6 安全门禁）';
        container.appendChild(el(doc, 'div', 'widget-error', 'widget 出错：' + instance.type + ' — ' + failed));
        return;
      }

      const gated = Boolean(manifest && d.consent.needsConsent(manifest) && !d.consent.isGranted(manifest));
      if (gated) {
        const box = el(doc, 'div', 'widget-consent');
        box.appendChild(el(doc, 'div', 'w-title', '第三方 widget：' + instance.type));
        box.appendChild(el(doc, 'div', 'widget-consent-source', '来源：' + ((manifest.entry && manifest.entry.source) || '—')));
        const caps = manifest.capabilities || [];
        const capsBox = el(doc, 'div', 'widget-consent-caps');
        capsBox.appendChild(doc.createTextNode('申请权限：'));
        if (!caps.length) capsBox.appendChild(el(doc, 'span', '', '无'));
        for (const c of caps) capsBox.appendChild(el(doc, 'code', '', c));
        box.appendChild(capsBox);
        const btn = el(doc, 'button', 'btn btn-secondary widget-consent-approve', '授权并加载');
        btn.type = 'button';
        btn.addEventListener('click', approve);
        box.appendChild(btn);
        box.appendChild(el(doc, 'div', 'widget-consent-note', '未授权前不会加载、不获得任何权限。我们不审核其代码，风险自担。'));
        container.appendChild(box);
        return;
      }

      const sandboxed = Boolean(manifest && manifest.entry && manifest.entry.kind === 'esm' && manifest.entry.sandbox === true);
      if (sandboxed) {
        const target = el(doc, 'div', 'widget-sandbox');
        container.appendChild(target);
        void mountSandbox(target);
        return;
      }

      if (reg && reg.kind === 'module') {
        const target = el(doc, 'div', 'widget-mount');
        container.appendChild(target);
        void mountModule(target);
        return;
      }

      container.appendChild(el(doc, 'div', 'widget-missing', '未注册的 widget：' + instance.type));
    }

    render();

    return {
      pushState(state) {
        currentState = state;
        try {
          if (stateCb) stateCb(state); // 进程内 module widget
          if (bridge) bridge.pushState(state); // 沙箱 widget
        } catch (e) {
          failed = errMsg(e);
          render();
        }
      },
      approve,
      state() { return failed ? 'failed' : 'mounted'; },
      destroy() {
        if (destroyed) return;
        destroyed = true;
        try { if (mod && mod.unmount) mod.unmount(); } catch { /* misbehaving widget 不破坏 teardown */ }
        try { if (bridge) bridge.destroy(); } catch { /* 同上：containment */ }
        container.replaceChildren();
      },
    };
  }

  return { mountWidget, sandboxEnforced };
});
