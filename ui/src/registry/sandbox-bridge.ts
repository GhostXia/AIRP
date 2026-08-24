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
 * The iframe loads a shared, CSP-compatible static bootstrap page, which
 * dynamically `import()`s the widget's same-origin digest-pinned `source` and calls its
 * `mount(iframe.document.body, ctxProxy)` where `ctxProxy` translates calls
 * into `postMessage` to the host. `allow-scripts` permits the import + mount.
 *
 * Every message carries a random bridge session and stable instance id. The
 * iframe is opaque, so the host also gates on its exact `contentWindow`.
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
 * Browser smoke covers the real opaque-frame path in addition to these unit
 * tests for bridge lifecycle and host-side gating.
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
  /** Pending mount operations parked until the iframe signals `ready`. */
  private readyWaiters: Array<{ ready: () => void; cancel: () => void }> = [];

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
          for (const waiter of waiters) waiter.ready();
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
        this.readyWaiters = this.readyWaiters.filter((entry) => entry.ready !== waiter);
        reject(new Error("sandbox iframe did not signal ready in time"));
      }, readyTimeoutMs);
      const waiter = (): void => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        sendMount();
      };
      const cancel = (): void => {
        if (done) return;
        done = true;
        clearTimeout(timer);
        reject(new Error("sandbox destroyed"));
      };
      this.readyWaiters.push({ ready: waiter, cancel });
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
    const waiters = this.readyWaiters;
    this.readyWaiters = [];
    for (const waiter of waiters) waiter.cancel();
    this.off();
    this.transport.destroy();
  }
}

/**
 * Build a live transport through the shared static sandbox frame. Every wire
 * message is bound to a random bridge session and the stable Widget instance,
 * in addition to the iframe window identity check.
 */
export function createIframeTransport(
  container: HTMLElement,
  source: string,
  instanceId: string,
): SandboxTransport {
  const base = document.baseURI || location.href;
  const hostOrigin = new URL(base).origin;
  const absoluteSource = new URL(source, base);
  if (absoluteSource.origin !== hostOrigin) throw new Error("sandbox widget source must be same-origin");
  const bridgeSession = crypto.randomUUID();
  const frameUrl = new URL("/assets/widgets/sandbox-frame.html", base);
  frameUrl.searchParams.set("src", absoluteSource.href);
  frameUrl.searchParams.set("origin", hostOrigin);
  frameUrl.searchParams.set("bridge_session", bridgeSession);
  frameUrl.searchParams.set("instance_id", instanceId);
  const iframe = document.createElement("iframe");
  // `allow-scripts` lets the widget run; deliberately NO `allow-same-origin`,
  // so the iframe is opaque-origin and cannot read host DOM/storage/cookies.
  iframe.setAttribute("sandbox", "allow-scripts");
  iframe.setAttribute("src", frameUrl.href);
  iframe.setAttribute("referrerpolicy", "no-referrer");
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
    const wire = ev.data as SandboxToHost & { bridge_session?: unknown; instance_id?: unknown };
    if (!wire || wire.bridge_session !== bridgeSession || wire.instance_id !== instanceId) return;
    if (wire.kind !== "ready"
      && !(wire.kind === "intent" && typeof wire.name === "string" && wire.name.length > 0)
      && !(wire.kind === "error" && typeof wire.message === "string")) return;
    const msg = wire as SandboxToHost;
    for (const cb of listeners) cb(msg);
  }
  window.addEventListener("message", onWindow);

  return {
    postMessage: (msg) => {
      // Opaque origins cannot be named as targetOrigin; window identity plus
      // the per-mount bridge session prevents sibling/stale frame confusion.
      iframe.contentWindow?.postMessage({
        ...msg,
        bridge_session: bridgeSession,
        instance_id: instanceId,
      }, "*");
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
