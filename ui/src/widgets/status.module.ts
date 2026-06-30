/**
 * A framework-agnostic sample widget: plain DOM, no Vue.
 *
 * Doubles as a real end-to-end esm example: the MockBus advertises it to the UI
 * as a third-party widget (`acme.status-pill`, `entry: { kind: "esm" }`), and the
 * UI loads it exactly as it would any remote third-party module. Keeping the
 * source in-repo means the demo needs no network or CDN.
 */

import type { WidgetModule, WidgetContext } from "../registry/widget-module";

export function createStatusPillWidget(): WidgetModule {
  let unsubscribe: (() => void) | undefined;

  return {
    mount(el: HTMLElement, ctx: WidgetContext) {
      const title = document.createElement("div");
      title.className = "w-title";
      title.textContent = "状态胶囊 (esm)";

      const pill = document.createElement("button");
      pill.className = "status-pill";
      pill.style.cssText = "margin-top:6px;padding:6px 10px;border-radius:999px;cursor:pointer;";

      const render = (state: unknown) => {
        const label = (state as { label?: string } | null)?.label;
        const on = Boolean((state as { on?: boolean } | null)?.on);
        pill.textContent = label ? `${label}${on ? " · ON" : ""}` : "—";
        pill.style.opacity = label ? "1" : "0.6";
      };
      render(ctx.getState());
      unsubscribe = ctx.onState(render);

      pill.addEventListener("click", () => ctx.emit("status.toggle", { id: ctx.instance.id }));

      el.append(title, pill);
    },

    unmount() {
      unsubscribe?.();
    },
  };
}

/** Default-export factory — the shape an esm widget module exposes. */
export default createStatusPillWidget;
