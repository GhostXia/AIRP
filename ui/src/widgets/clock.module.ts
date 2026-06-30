/**
 * A framework-agnostic sample widget: plain DOM, no Vue.
 *
 * Demonstrates the {@link WidgetModule} contract — an author can build a widget
 * with any technology and it plugs into the same host.
 */

import type { WidgetModule, WidgetContext } from "../registry/widget-module";

export function createClockWidget(): WidgetModule {
  let timer: ReturnType<typeof setInterval> | undefined;
  let unsubscribe: (() => void) | undefined;

  return {
    mount(el: HTMLElement, ctx: WidgetContext) {
      const title = document.createElement("div");
      title.className = "w-title";
      title.textContent = "时钟 (vanilla)";

      const time = document.createElement("div");
      time.style.fontSize = "20px";

      const note = document.createElement("div");
      note.style.opacity = "0.7";
      note.style.fontSize = "12px";

      el.append(title, time, note);

      const tick = () => {
        time.textContent = new Date().toLocaleTimeString();
      };
      tick();
      timer = setInterval(tick, 1000);

      const showState = (state: unknown) => {
        const label = (state as { label?: string } | null)?.label;
        note.textContent = label ? `state.label = ${label}` : "";
      };
      showState(ctx.getState());
      unsubscribe = ctx.onState(showState);
    },

    unmount() {
      if (timer) clearInterval(timer);
      unsubscribe?.();
    },
  };
}
