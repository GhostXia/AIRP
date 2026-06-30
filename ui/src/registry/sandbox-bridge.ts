/**
 * Sandbox bridge — hosts a third-party (esm) widget inside a sandboxed iframe
 * and bridges the {@link WidgetContext} over `postMessage` (PLAN task D).
 *
 * Security model (SECURITY.md): the iframe is created with
 * `sandbox="allow-scripts"` and **no** `allow-same-origin`, so it runs under an
 * opaque origin and cannot reach the host's DOM, `window`, `localStorage`,
 * cookies, or same-origin network. The widget's `WidgetContext` is proxied:
 * the host listens for `intent` messages from the iframe and pushes `state`
 * messages in; the widget never gets a direct reference to host objects.
 *
 * The iframe loads a small bootstrap (`SANDBOX_BOOTSTRAP`) via `srcdoc`, which
 * dynamically `import()`s the widget's `source` and calls its
 * `mount(iframe.document.body, ctxProxy)` where `ctxProxy` translates calls
 * into `postMessage` to the host. `allow-scripts` permits the import + mount.
 *
 * Message protocol (host ↔ iframe), all `postMessage` with targetOrigin `"*"`
 * (the iframe is opaque; the host gates on `event.source === iframe.contentWindow`):
 *
 * - host → iframe: `{ kind: "mount", instance, capabilities }` then
 *   `{ kind: "state", state }` per state change.
 * - iframe → host: `{ kind: "ready" }` (bootstrap loaded, awaiting mount),
 *   `{ kind: "intent", name, params }`, `{ kind: "error", message }`.
 *
 * The transport (`SandboxTransport`) is injectable so the bridge logic is
 * unit-testable without a real iframe (see `sandbox-bridge.test.ts`). The live
 * transport (`createIframeTransport`) builds a real iframe element.
 *
 * NOTE: the real end-to-end sandboxed load (remote esm inside the iframe) is a
 * runtime verification item (PLAN §2.5 D) — CI covers the bridge protocol and
 * the host-side gating, not browser frame behavior.
 */

import type { WidgetInstance, Json, Capability } from "../protocol/types";

/** Messages the host sends into the iframe. */
export type HostToSandbox =
  | { kind: "mount"; instance: WidgetInstance; capabilities: Capability[] }
  | { kind: "state"; state: unknown };

/** Messages the iframe sends back to the host. */
export type SandboxToHost =
  | { kind: "ready" }
  | { kind: "intent"; name: string; params?: Json }
  | { kind: "error"; message: string };

/** Minimal slice of an iframe the bridge needs (injectable for tests). */
export interface SandboxTransport {
  /** Send a message into the iframe. */
  postMessage(msg: HostToSandbox): void;
  /** Register a handler for messages from the iframe; returns an unsubscribe. */
  onMessage(cb: (msg: SandboxToHost) => void): () => void;
  /** Tear down the iframe + listeners. */
  destroy(): void;
}

/**
 * Bridge the host side of a sandboxed widget. Holds the transport, forwards
 * state into the iframe, and surfaces intents/errors out. The host
 * (`WidgetHost.vue`) constructs this when a manifest has `entry.sandbox: true`,
 * calls `mount()` once, `pushState()` on each state change, and `destroy()` on
 * unmount.
 */
export class SandboxBridge {
  private destroyed = false;
  private ready = false;
  private readonly off: () => void;
  /** mount() resolvers parked until the iframe signals `ready`. */
  private readyWaiters: Array<() => void> = [];

  constructor(
    private readonly transport: SandboxTransport,
    private readonly onIntent: (name: string, params?: Json) => void,
    private readonly onError: (message: string) => void,
  ) {
    this.off = transport.onMessage((msg) => {
      if (this.destroyed) return;
      if (msg.kind === "ready") {
        // Capture `ready` on the always-on listener (not inside mount()), so a
        // `ready` arriving BEFORE mount() is called is not lost: the iframe's
        // bootstrap posts `ready` the instant its srcdoc script runs, which can
        // race ahead of the host calling mount().
        if (!this.ready) {
          this.ready = true;
          const waiters = this.readyWaiters;
          this.readyWaiters = [];
          for (const w of waiters) w();
        }
      } else if (msg.kind === "intent") this.onIntent(msg.name, msg.params);
      else if (msg.kind === "error") this.onError(msg.message);
    });
  }

  /**
   * Tell the iframe to mount the widget. Resolves once the iframe has signalled
   * `ready` (bootstrap loaded) — immediately if `ready` already arrived — then
   * sends the `mount` message. If the iframe never signals ready, this rejects
   * after `readyTimeoutMs` (default 5s) so the host surfaces a load failure
   * rather than hanging.
   */
  mount(instance: WidgetInstance, capabilities: Capability[], readyTimeoutMs = 5000): Promise<void> {
    return new Promise((resolve, reject) => {
      if (this.destroyed) return reject(new Error("sandbox destroyed"));
      const sendMount = (): void => {
        this.transport.postMessage({ kind: "mount", instance, capabilities });
        resolve();
      };
      // Already ready (possibly before this call): mount now, no race window.
      if (this.ready) {
        sendMount();
        return;
      }
      let done = false;
      const timer = setTimeout(() => {
        if (done) return;
        done = true;
        // Drop our waiter so a late `ready` can't fire a rejected mount.
        this.readyWaiters = this.readyWaiters.filter((w) => w !== waiter);
        reject(new Error("sandbox iframe did not signal ready in time"));
      }, readyTimeoutMs);
      const waiter = (): void => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        sendMount();
      };
      this.readyWaiters.push(waiter);
    });
  }

  /** Push a new state slice into the iframe. */
  pushState(state: unknown): void {
    if (this.destroyed) return;
    this.transport.postMessage({ kind: "state", state });
  }

  /** Tear down: stop forwarding, destroy the iframe. */
  destroy(): void {
    if (this.destroyed) return;
    this.destroyed = true;
    this.off();
    this.transport.destroy();
  }
}

/**
 * The HTML written into the iframe via `srcdoc`. It waits for the host's
 * `mount` message, dynamically `import()`s the widget `source`, and proxies
 * the `WidgetContext` over `postMessage`. Kept as a function so the source
 * can be interpolated without string concatenation ambiguity.
 *
 * The bootstrap uses `parent.postMessage(msg, "*")` to reach the host; the
 * host gates on `event.source === iframe.contentWindow` (see
 * `createIframeTransport`), so a hostile iframe cannot spoof messages from a
 * different frame.
 */
export function sandboxBootstrap(source: string): string {
  // The bootstrap is self-contained and runs inside the iframe. It cannot
  // reference host modules. It defines a `ctxProxy` that turns every call into
  // a postMessage to the parent, then imports the widget factory and mounts.
  const safeSource = JSON.stringify(source);
  return `<!doctype html><html><head><meta charset="utf-8"><style>html,body{margin:0;height:100%;background:transparent;color:inherit;font:inherit}</style></head>
<body><script>
(function(){
  var SRC = ${safeSource};
  function send(msg){ parent.postMessage(msg, "*"); }
  // Buffer the latest state: a state message can arrive before the widget's
  // async import has registered its onState callback. We keep the last value
  // and replay it on registration, so the first state slice is never dropped.
  var lastState; var hasState = false; var stateCb = null;
  // WidgetContext proxy: the widget calls these; we translate to messages.
  var ctx = {
    instance: null,
    getState: function(){ return hasState ? lastState : undefined; },
    onState: function(cb){
      stateCb = cb;
      if (hasState) { try { cb(lastState); } catch(e){ send({ kind: "error", message: String(e && e.message || e) }); } }
      return function(){ if (stateCb === cb) stateCb = null; };
    },
    emit: function(name, params){ send({ kind: "intent", name: name, params: params }); },
    capabilities: []
  };
  window.addEventListener("message", function(ev){
    var msg = ev.data || {};
    if (msg.kind === "mount") {
      ctx.instance = msg.instance;
      ctx.capabilities = msg.capabilities || [];
      import(SRC).then(function(mod){
        var factory = typeof mod === "function" ? mod : mod.default;
        if (typeof factory !== "function") throw new Error("esm widget default export must be a WidgetFactory");
        return factory().mount(document.body, ctx);
      }).catch(function(e){ send({ kind: "error", message: String(e && e.message || e) }); });
    } else if (msg.kind === "state") {
      lastState = msg.state; hasState = true;
      try { if (stateCb) stateCb(msg.state); } catch(e){ send({ kind: "error", message: String(e && e.message || e) }); }
    }
  });
  send({ kind: "ready" });
})();
</script></body></html>`;
}

/**
 * Build a live iframe transport: creates an `<iframe sandbox="allow-scripts">`
 * with the given widget `source` baked into its `srcdoc`, wires `postMessage`
 * (gating on `event.source === iframe.contentWindow`), and returns a
 * {@link SandboxTransport}. The host appends the iframe to `container`.
 */
export function createIframeTransport(
  container: HTMLElement,
  source: string,
): SandboxTransport {
  const iframe = document.createElement("iframe");
  // `allow-scripts` lets the widget run; deliberately NO `allow-same-origin`,
  // so the iframe is opaque-origin and cannot read host DOM/storage/cookies.
  iframe.setAttribute("sandbox", "allow-scripts");
  iframe.setAttribute("srcdoc", sandboxBootstrap(source));
  // Transparent + filling: the widget renders its own DOM inside the iframe.
  iframe.style.border = "0";
  iframe.style.width = "100%";
  iframe.style.height = "100%";
  iframe.style.background = "transparent";
  container.appendChild(iframe);

  const listeners = new Set<(msg: SandboxToHost) => void>();
  function onWindow(ev: MessageEvent): void {
    // Gate: only accept messages originating from this iframe's window. A
    // hostile sibling frame cannot spoof because its `source` differs.
    if (ev.source !== iframe.contentWindow) return;
    const msg = ev.data as SandboxToHost;
    if (!msg || typeof msg.kind !== "string") return;
    for (const cb of listeners) cb(msg);
  }
  window.addEventListener("message", onWindow);

  return {
    postMessage: (msg) => {
      // targetOrigin "*" because the iframe is opaque-origin (no real origin).
      iframe.contentWindow?.postMessage(msg, "*");
    },
    onMessage: (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    destroy: () => {
      window.removeEventListener("message", onWindow);
      listeners.clear();
      iframe.remove();
    },
  };
}
