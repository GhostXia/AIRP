// C-P1 首方示范 widget：时钟（对译 ui/src/widgets/clock.module.ts）。
// framework-agnostic：纯 DOM，无框架。展示 WidgetModule 契约——
// 作者可用任何技术实现，只要符合 mount(el, ctx) / unmount()。
// 作为 builtin（module kind）进程内挂载，无需 consent。

export function createClockWidget() {
  let timer;
  let unsubscribe;

  return {
    mount(el, ctx) {
      const title = document.createElement('div');
      title.className = 'w-title';
      title.textContent = '时钟 (vanilla)';

      const time = document.createElement('div');
      time.className = 'widget-clock-time';

      const note = document.createElement('div');
      note.className = 'widget-clock-note';

      el.append(title, time, note);

      const tick = () => {
        time.textContent = new Date().toLocaleTimeString();
      };
      tick();
      timer = setInterval(tick, 1000);

      const showState = (state) => {
        const label = state && state.label;
        note.textContent = label ? 'state.label = ' + label : '';
      };
      showState(ctx.getState());
      unsubscribe = ctx.onState(showState);
    },

    unmount() {
      if (timer) clearInterval(timer);
      if (unsubscribe) unsubscribe();
    },
  };
}
