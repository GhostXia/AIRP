// C-P1 首方示范 widget：状态胶囊（对译 ui/src/widgets/status.module.ts）。
// framework-agnostic：纯 DOM，无框架。
//
// 双重示范：它同时也是真实的 esm 端到端示例——slots.json 把
// `airp.status-pill` 以第三方 widget 身份声明（entry: { kind: "esm",
// sandbox: true }），宿主像加载任何远程第三方模块一样经 opaque-origin
// 沙箱 iframe 加载它。源码在仓库内，示范不依赖网络/CDN。
//
// 沙箱内的样式注意：iframe 是独立文档，看不到宿主 tokens.css；widget 只能
// 用 inherit（引导脚本已设 color/font: inherit）与自身内联样式表达视觉。
// C-P4 的 Widget SDK 将提供令牌注入约定（docs/WIDGET-DEVELOPMENT.md）。

export function createStatusPillWidget() {
  let unsubscribe;

  return {
    mount(el, ctx) {
      const title = document.createElement('div');
      title.className = 'w-title';
      title.textContent = '状态胶囊 (esm·sandbox)';

      const pill = document.createElement('button');
      pill.type = 'button';
      pill.style.cssText = 'margin-top:6px;padding:6px 10px;border-radius:999px;cursor:pointer;border:1px solid currentColor;background:transparent;color:inherit;font:inherit;';

      const render = (state) => {
        const label = state && state.label;
        const on = Boolean(state && state.on);
        pill.textContent = label ? label + (on ? ' · ON' : '') : '—';
        pill.style.opacity = label ? '1' : '0.6';
      };
      render(ctx.getState());
      unsubscribe = ctx.onState(render);

      pill.addEventListener('click', () => ctx.emit('status.toggle', { id: ctx.instance.id }));

      el.append(title, pill);
    },

    unmount() {
      if (unsubscribe) unsubscribe();
    },
  };
}

/** default 导出 factory —— esm widget 模块必须暴露的形状。 */
export default createStatusPillWidget;
