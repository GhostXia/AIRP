import { describe, it, expect, vi } from "vitest";
import { SandboxBridge, type SandboxTransport, type HostToSandbox, type SandboxToHost } from "./sandbox-bridge";
import { sandboxBootstrap } from "./sandbox-bridge";
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

describe("sandboxBootstrap", () => {
  it("embeds the source as a JSON-quoted string (no injection / interpolation ambiguity)", () => {
    const html = sandboxBootstrap("https://cdn.acme/widget.js");
    expect(html).toContain('var SRC = "https://cdn.acme/widget.js"');
    // the sandbox attr lives on the iframe element (createIframeTransport),
    // not in the bootstrap html — the bootstrap is just the srcdoc payload
    expect(html).not.toContain('sandbox="allow-scripts"');
    // the bootstrap must post ready
    expect(html).toContain('send({ kind: "ready" })');
  });

  it("escapes a source containing quotes/backslashes so the inline script stays valid", () => {
    // A source with a double-quote would break the inline `var SRC = "..."`.
    // JSON.stringify must escape it.
    const html = sandboxBootstrap('a"b\\c');
    expect(html).toContain('var SRC = "a\\"b\\\\c"');
  });

  it("buffers state so a slice arriving before onState registration is replayed", () => {
    // Regression guard: the widget import() is async, so a `state` message can
    // land before the widget calls onState. The bootstrap must keep the last
    // value (hasState/lastState) and replay it on registration, instead of the
    // old window.__sandboxStateCb pattern that silently dropped it.
    const html = sandboxBootstrap("https://cdn.acme/widget.js");
    expect(html).toContain("hasState");
    expect(html).toContain("lastState = msg.state");
    expect(html).toContain("if (hasState)");
    expect(html).not.toContain("__sandboxStateCb");
  });
});
