// C-P4-4：示例第三方 widget，演示 SDK 用法。
//
// 这是一个完整可跑的第三方 widget 模板：声明 manifest、用 createWidget 包装
// 工厂、用 h 构建 DOM、消费 WidgetContext 的全部能力（getState / onState /
// emit / capabilities）。作者可复制本文件作为起点，修改 type / capabilities /
// 渲染逻辑即可。
//
// 部署：本文件作为 esm 被 sandbox-frame.js 动态 import()。作者发布时把
// manifest + 本文件（+ 依赖资产）打包成 digest-pinned 安装包，POST 到
// /v1/extensions/install。engine 校验摘要、强制改写 entry.source 为
// /extensions/<digest>/index.js、签发 capability grant（需用户 consent）。
//
// 安全：本文件运行在 opaque-origin sandboxed iframe 内（sandbox="allow-scripts"，
// 无 allow-same-origin）。它读不到宿主 DOM / localStorage / cookie / 同源网络。
// WidgetContext 由宿主经 postMessage 代理，widget 拿不到宿主对象引用。

import { createWidget, defineManifest, h } from './widget-sdk.js';

// 模块级 manifest 声明。defineManifest 在模块加载即校验，让作者在本地 import
// 时立刻发现 host_api / sandbox / type 缺失等问题，而不是装到 engine 才被拒。
export const manifest = defineManifest({
  type: 'acme.sdk-example',
  version: '1.0.0',
  title: 'SDK 示例 widget',
  author: 'AIRP',
  // 申请 read:state capability。engine 在用户 consent 后签发 grant；
  // widget 通过 ctx.capabilities 读到已授予的子集（可能小于声明）。
  capabilities: ['read:state'],
  // C-P4-3：声明宿主合同 major 版本。engine 当前支持 HOST_API_MAJOR = 1。
  // 当 engine 升级到 major 2 时，本 widget 安装即被拒（前向兼容铁律），
  // 强迫作者显式声明兼容性。
  host_api: '1',
  entry: {
    kind: 'esm',
    // 安装时被 engine 强制改写为 /extensions/<digest>/index.js。
    // 本字段在打包安装包时可填占位符，engine 不信任作者声明的 source。
    source: './example-widget.js',
    sandbox: true, // BUG-6：第三方 esm 必须显式 true
  },
});

// 默认导出的 WidgetFactory。createWidget 包装它，注入错误捕获 + 生命周期日志。
// 作者也可不包装，直接 `export default function createExampleWidget() { ... }`；
// createWidget 是可选的渐进式辅助。
export default createWidget(function createExampleWidget() {
  let unsubscribe;

  return {
    mount(el, ctx) {
      // ctx.capabilities 是 engine 签发的 grant 子集（C-P3）。
      // 即使 manifest 声明了 read:state，若用户未 consent，ctx.capabilities 为空。
      const hasReadState = ctx.capabilities.includes('read:state');

      const title = h('div', { className: 'w-title' }, 'SDK 示例 widget');

      const caps = h(
        'div',
        { className: 'widget-sdk-caps' },
        '已授予 capability：',
        ctx.capabilities.length ? ctx.capabilities.join(', ') : '无',
      );

      // 读 state 切片。capability 是 engine 强制的：未授予 read:state 时，
      // 宿主不应下发敏感 state（C-P3 逐调用强制）。widget 仍可调用 ctx.getState()，
      // 但拿到的可能是 undefined 或脱敏后的切片。
      const stateLine = h('div', { className: 'widget-sdk-state' });
      const renderState = (state) => {
        const label = state && hasReadState ? state.label : null;
        stateLine.textContent = 'state.label = ' + (label != null ? String(label) : '—');
      };
      renderState(ctx.getState());
      unsubscribe = ctx.onState(renderState);

      // 发 intent。widget-intents.json 是机器可读合同源；intent 名必须在
      // 合同里声明，否则 engine 拒绝（C-P2 拒绝默认）。
      const ping = h(
        'button',
        {
          type: 'button',
          onClick: () => ctx.emit('acme.sdk-example.ping', { id: ctx.instance.id }),
        },
        '发出 intent (ping)',
      );

      el.append(title, caps, stateLine, ping);
    },

    unmount() {
      if (unsubscribe) unsubscribe();
    },
  };
}, {
  // onError：mount/unmount 抛错时回调。作者可在此上报错误或展示降级 UI。
  // SDK 已捕获错误不炸宿主；onError 只是让作者自己能看到。
  onError: (e) => {
    // eslint-disable-next-line no-console
    console.error('[acme.sdk-example] widget error:', e);
  },
  // debug：开启生命周期日志。作者本地调试时设为 true；发布时去掉或用
  // globalThis.__AIRP_WIDGET_DEBUG = true 统一开启。
  debug: false,
});
