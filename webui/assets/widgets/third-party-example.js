// C-P1 第三方示范 widget（模拟第三方，源码在仓库内、经 esm+sandbox 链路加载）。
//
// 作用：完整演示第三方链路四步——consent 授权 UI → opaque-origin 沙箱 iframe
// → postMessage WidgetContext 代理 → capability 只给已声明者。
// 真实第三方发布时其 source 指向独立 URL（C-P2 安装面强制改写为
// /extensions/<digest>/index.js），manifest 身份（type@version#source）决定
// 换源/升版必须重新 consent。
//
// 它声明 read:state capability 并消费 ctx.capabilities 展示宿主授予结果；
// 同时故意尝试触碰宿主的 localStorage（沙箱内为 opaque origin，读到的是
// iframe 自己的空存储，证明隔离生效）。

export default function createThirdPartyExampleWidget() {
  let unsubscribe;

  return {
    mount(el, ctx) {
      const title = document.createElement('div');
      title.className = 'w-title';
      title.textContent = '第三方示范 widget (esm·sandbox)';

      const caps = document.createElement('div');
      caps.textContent = '已授予 capability：' + (ctx.capabilities.length ? ctx.capabilities.join(', ') : '无');

      const iso = document.createElement('div');
      let isolation = '未知';
      try {
        // opaque-origin 沙箱内 localStorage 是独立空存储；若能读到宿主的
        // consent 记录则说明隔离失效（预期永远读不到）。
        isolation = window.localStorage.getItem('airp:consent-grants') == null ? '生效' : '失效';
      } catch (e) {
        isolation = '生效（存储不可达）';
      }
      iso.textContent = '沙箱隔离：' + isolation;

      const stateLine = document.createElement('div');
      const render = (state) => {
        stateLine.textContent = 'state：' + (state && state.label ? String(state.label) : '—');
      };
      render(ctx.getState());
      unsubscribe = ctx.onState(render);

      const ping = document.createElement('button');
      ping.type = 'button';
      ping.textContent = '发出 intent (ping)';
      ping.style.cssText = 'margin-top:6px;padding:4px 8px;border:1px solid currentColor;border-radius:6px;background:transparent;color:inherit;font:inherit;cursor:pointer;';
      ping.addEventListener('click', () => ctx.emit('third-party.ping', { id: ctx.instance.id }));

      el.append(title, caps, iso, stateLine, ping);
    },

    unmount() {
      if (unsubscribe) unsubscribe();
    },
  };
}
