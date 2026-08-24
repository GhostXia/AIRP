import { describe, it, expect, vi } from "vitest";
import { SandboxBridge, createIframeTransport, type SandboxTransport, type HostToSandbox, type SandboxToHost } from "./sandbox-bridge";
import type { WidgetInstance } from "../protocol/types";

/** A mock transport: lets the test pump messages in both directions. */
function mockTransport(): SandboxTransport & {
  sent: HostToSandbox[];
  emit: (msg: SandboxToHost) => void;
} {
  const sent: HostToSandbox[] = [];
  const listeners = new Set<(msg: SandboxToHost) => void>();
  return {
    sent,
    emit: (msg) => {
      for (const cb of listeners) cb(msg);
    },
    postMessage: (msg) => {
      sent.push(msg);
    },
    onMessage: (cb) => {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    destroy: () => {
      listeners.clear();
    },
  };
}

function instance(): WidgetInstance {
  return { id: "w-status", type: "acme.status-pill", state: "w-status", capabilities: ["read:state"] };
}

describe("SandboxBridge", () => {
  it("mount waits for ready, then sends the mount message", async () => {
    const t = mockTransport();
    const intents: Array<[string, unknown]> = [];
    const bridge = new SandboxBridge(t, (n, p) => intents.push([n, p]), () => {});
    const mounting = bridge.mount(instance(), ["read:state"]);
    // iframe bootstrap would send ready:
    t.emit({ kind: "ready" });
    await mounting;
    expect(t.sent).toEqual([
      { kind: "mount", instance: instance(), capabilities: ["read:state"] },
    ]);
    bridge.destroy();
  });

  it("mount succeeds when ready arrived BEFORE mount() was called (no race)", async () => {
    const t = mockTransport();
    const bridge = new SandboxBridge(t, () => {}, () => {});
    // The iframe bootstrap fires `ready` before the host gets to mount().
    t.emit({ kind: "ready" });
    await bridge.mount(instance(), ["read:state"]);
    expect(t.sent).toEqual([
      { kind: "mount", instance: instance(), capabilities: ["read:state"] },
    ]);
    bridge.destroy();
  });

  it("mount rejects if ready does not arrive in time", async () => {
    const t = mockTransport();
    const bridge = new SandboxBridge(t, () => {}, () => {});
    await expect(bridge.mount(instance(), [], 20)).rejects.toThrow(
      /did not signal ready/,
    );
    bridge.destroy();
  });

  it("mount rejects if bridge already destroyed", async () => {
    const t = mockTransport();
    const bridge = new SandboxBridge(t, () => {}, () => {});
    bridge.destroy();
    await expect(bridge.mount(instance(), [])).rejects.toThrow(/destroyed/);
  });

  it("destroy cancels a pending mount without retaining its timeout", async () => {
    vi.useFakeTimers();
    try {
      const t = mockTransport();
      const bridge = new SandboxBridge(t, () => {}, () => {});
      const mounting = bridge.mount(instance(), [], 5_000);

      bridge.destroy();

      await expect(mounting).rejects.toThrow(/destroyed/);
      expect(vi.getTimerCount()).toBe(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it("pushState forwards state into the iframe", () => {
    const t = mockTransport();
    const bridge = new SandboxBridge(t, () => {}, () => {});
    bridge.pushState({ on: true });
    expect(t.sent).toEqual([{ kind: "state", state: { on: true } }]);
    bridge.destroy();
  });

  it("forwards intent messages from the iframe to the host", () => {
    const t = mockTransport();
    const intents: Array<[string, unknown]> = [];
    const bridge = new SandboxBridge(t, (n, p) => intents.push([n, p]), () => {});
    t.emit({ kind: "intent", name: "status.toggle", params: { id: "w-status" } });
    expect(intents).toEqual([["status.toggle", { id: "w-status" }]]);
    bridge.destroy();
  });

  it("forwards error messages from the iframe", () => {
    const t = mockTransport();
    const errors: string[] = [];
    const bridge = new SandboxBridge(t, () => {}, (m) => errors.push(m));
    t.emit({ kind: "error", message: "import failed" });
    expect(errors).toEqual(["import failed"]);
    bridge.destroy();
  });

  it("ignores messages after destroy", () => {
    const t = mockTransport();
    const intents: Array<[string, unknown]> = [];
    const bridge = new SandboxBridge(t, (n, p) => intents.push([n, p]), () => {});
    bridge.destroy();
    t.emit({ kind: "intent", name: "status.toggle" });
    expect(intents).toEqual([]);
    // pushState after destroy is a no-op too
    bridge.pushState({});
    expect(t.sent).toEqual([]);
  });

  it("destroy tears down the transport", () => {
    const t = mockTransport();
    const destroyed = vi.fn();
    // wrap destroy to observe
    const wrapped: SandboxTransport = {
      ...t,
      destroy: () => {
        destroyed();
        t.destroy();
      },
    };
    const bridge = new SandboxBridge(wrapped, () => {}, () => {});
    bridge.destroy();
    expect(destroyed).toHaveBeenCalledTimes(1);
    // second destroy is a no-op
    bridge.destroy();
    expect(destroyed).toHaveBeenCalledTimes(1);
  });
});

describe("createIframeTransport", () => {
  it("uses the static opaque frame and binds messages to window, session, and instance", () => {
    const posted = vi.fn();
    let onMessage: ((event: MessageEvent) => void) | undefined;
    const attributes: Record<string, string> = {};
    const contentWindow = { postMessage: posted };
    const iframe = {
      contentWindow,
      style: {},
      setAttribute: (name: string, value: string) => { attributes[name] = value; },
      remove: vi.fn(),
    };
    const appendChild = vi.fn();
    vi.stubGlobal("document", {
      baseURI: "http://127.0.0.1:8765/desktop/",
      createElement: () => iframe,
    });
    vi.stubGlobal("location", { href: "http://127.0.0.1:8765/desktop/" });
    vi.stubGlobal("crypto", { randomUUID: () => "bridge-123" });
    vi.stubGlobal("window", {
      addEventListener: (_name: string, listener: (event: MessageEvent) => void) => { onMessage = listener; },
      removeEventListener: vi.fn(),
    });
    try {
      const transport = createIframeTransport(
        { appendChild } as unknown as HTMLElement,
        "/extensions/aabb/index.js",
        "widget-7",
      );
      const frameUrl = new URL(attributes.src);
      expect(attributes.sandbox).toBe("allow-scripts");
      expect(attributes.referrerpolicy).toBe("no-referrer");
      expect(frameUrl.pathname).toBe("/assets/widgets/sandbox-frame.html");
      expect(frameUrl.searchParams.get("src")).toBe("http://127.0.0.1:8765/extensions/aabb/index.js");
      expect(frameUrl.searchParams.get("bridge_session")).toBe("bridge-123");
      expect(frameUrl.searchParams.get("instance_id")).toBe("widget-7");

      const received = vi.fn();
      transport.onMessage(received);
      onMessage?.({ source: {}, data: { kind: "ready", bridge_session: "bridge-123", instance_id: "widget-7" } } as MessageEvent);
      onMessage?.({ source: contentWindow, data: { kind: "ready", bridge_session: "wrong", instance_id: "widget-7" } } as unknown as MessageEvent);
      onMessage?.({ source: contentWindow, data: { kind: "ready", bridge_session: "bridge-123", instance_id: "widget-7" } } as unknown as MessageEvent);
      expect(received).toHaveBeenCalledTimes(1);

      transport.postMessage({ kind: "state", state: { ok: true } });
      expect(posted).toHaveBeenCalledWith({
        kind: "state",
        state: { ok: true },
        bridge_session: "bridge-123",
        instance_id: "widget-7",
      }, "*");
      transport.destroy();
      expect(iframe.remove).toHaveBeenCalledOnce();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("rejects cross-origin widget sources", () => {
    vi.stubGlobal("document", { baseURI: "http://127.0.0.1:8765/desktop/" });
    vi.stubGlobal("location", { href: "http://127.0.0.1:8765/desktop/" });
    try {
      expect(() => createIframeTransport({} as HTMLElement, "https://example.invalid/widget.js", "widget-7"))
        .toThrow(/same-origin/);
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("rejects an empty instance identity before creating a frame", () => {
    const createElement = vi.fn();
    vi.stubGlobal("document", {
      baseURI: "http://127.0.0.1:8765/desktop/",
      createElement,
    });
    try {
      expect(() => createIframeTransport({} as HTMLElement, "/extensions/aabb/index.js", ""))
        .toThrow(/instance id is required/);
      expect(createElement).not.toHaveBeenCalled();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
