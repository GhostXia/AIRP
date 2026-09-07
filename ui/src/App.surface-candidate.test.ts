import appSource from "./App.vue?raw";
import { parse } from "@vue/compiler-sfc";
import ts from "typescript";
import { describe, expect, it, vi } from "vitest";
import { SurfaceStore } from "./protocol/surface-v2";
import type { SurfaceTarget } from "./protocol/http-engine-bus";
import type { SurfaceSnapshot } from "./protocol/types";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}
function snapshot(revision: string): SurfaceSnapshot {
  return {
    kind: "snapshot", protocol: { major: 2, minor: 0 }, surfaceId: "session:test", revision,
    blueprint: { version: 2, root: { type: "widget", id: "chat-node", instanceId: "chat" },
      widgets: [{ id: "chat", type: "core.chat" }] },
  };
}

// Run the real orchestration function with controlled async Bus boundaries,
// without mirroring its guard or needing a browser/component mount.
function harness() {
  const script = parse(appSource).descriptor.scriptSetup!.content;
  const ast = ts.createSourceFile("App.ts", script, ts.ScriptTarget.Latest, true);
  const declaration = ast.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "initializeBus");
  if (!declaration) throw new Error("Production initializeBus function missing");
  const code = ts.transpileModule(declaration.getText(ast), { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  const store = new SurfaceStore();
  const instances: FakeBus[] = [];
  class FakeBus {
    target?: SurfaceTarget;
    connectGate?: ReturnType<typeof deferred>;
    sessionsGate?: ReturnType<typeof deferred>;
    listed = false;
    disconnect = vi.fn();
    constructor() { instances.push(this); }
    async resolveScope() { return { characterId: "c", sessionId: "s" }; }
    async getWorkspace() { return { revision: "1" }; }
    async listSessions() { this.listed = true; await this.sessionsGate?.promise; return ["s"]; }
    async connect(_scope: unknown, target: SurfaceTarget) {
      this.target = target;
      await this.connectGate?.promise;
      target.apply(snapshot(String(instances.indexOf(this) + 1)));
      return () => this.disconnect();
    }
  }
  const state = {
    productionSurface: true, disposed: false, busAttempt: 0,
    bus: null as FakeBus | null, connectingBus: null as FakeBus | null,
    scope: null, disconnect: null, workspaceBusReady: { value: false },
    connectionState: { value: "" }, busError: { value: null },
    workspace: { pendingCommand: null, accept: vi.fn(), clear: vi.fn(), fail: vi.fn() },
    workspaceRequests: { invalidate: vi.fn() }, widgetOperations: { value: {} },
    selectedCharacterId: { value: "" }, selectedSessionId: { value: "" }, sessionIds: { value: [] },
    currentBearer: () => "test", renewDesktopSession: vi.fn(), probeAllRecoveringOperations: vi.fn(),
    applySurface: vi.fn((message: unknown) => store.apply(message)), invalidateBus: vi.fn(),
  };
  const initialize = new Function("state", "HttpEngineBus", `with (state) { ${code}; return initializeBus; }`)(state, FakeBus) as () => Promise<void>;
  return { initialize, state, store, instances };
}
async function until(predicate: () => boolean) {
  for (let i = 0; i < 30; i++) { if (predicate()) return; await Promise.resolve(); }
  throw new Error("Controlled Bus boundary was not reached");
}

describe("candidate Surface publication authority (#613)", () => {
  it("rejects a superseded initial snapshot without altering the accepted Surface", async () => {
    const h = harness();
    const oldRun = h.initialize();
    const old = h.instances[0];
    old.connectGate = deferred();
    await until(() => !!old.target);
    await h.initialize();
    const accepted = h.store.snapshot;
    expect(h.state.bus).toBe(h.instances[1]);
    old.connectGate.resolve();
    await oldRun;
    expect(h.store.snapshot).toEqual(accepted);
    h.state.connectingBus = old; // Identity alone must not revive an old attempt.
    expect(() => old.target!.apply({ kind: "patch" })).toThrow("stale candidate");
    expect(h.state.applySurface).toHaveBeenCalledTimes(1);
    expect(h.state.invalidateBus).not.toHaveBeenCalled();
  });

  it("permits the current candidate and published Bus but checks identity and disposal", async () => {
    const h = harness();
    await h.initialize();
    const target = h.instances[0].target!;
    expect(h.state.applySurface).toHaveBeenCalledTimes(1);
    expect(target.apply(snapshot("2")).status).toBe("applied");
    h.state.bus = null; // Same attempt, but this Bus no longer owns either slot.
    expect(() => target.apply(snapshot("3"))).toThrow("stale candidate");
    h.state.bus = h.instances[0];
    h.state.disposed = true;
    expect(() => target.apply(snapshot("4"))).toThrow("stale candidate");
    expect(h.state.applySurface).toHaveBeenCalledTimes(2);
  });

  it("does not reconnect a superseded candidate after delayed session listing", async () => {
    const h = harness();
    const oldRun = h.initialize();
    const old = h.instances[0];
    old.sessionsGate = deferred();
    await until(() => old.listed);
    await h.initialize();
    old.sessionsGate.resolve();
    await oldRun;
    expect(old.target).toBeUndefined();
    expect(old.disconnect).toHaveBeenCalled();
    expect(h.state.applySurface).toHaveBeenCalledTimes(1);
  });
});
