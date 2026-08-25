import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import type { Json, SurfaceSnapshot } from "./protocol/types";
import type { AgentTestContext, AgentTestHarness } from "./agent-test";

const originalWindow = globalThis.window;
const originalDocument = globalThis.document;

type AgentTestModule = {
  shouldInstallAgentTestHarness(): boolean;
  installAgentTestHarness(ctx: AgentTestContext): AgentTestHarness | null;
};

const agentTestModules = import.meta.glob<AgentTestModule>("./agent-test.ts");

async function loadAgentTestModule(): Promise<AgentTestModule | null> {
  const load = Object.values(agentTestModules)[0];
  return load ? await load() : null;
}

function installDom(url = "http://localhost:1420/?airp_agent_test=1") {
  const body = { textContent: "AIRP ready" };
  const document = {
    body,
    querySelector(selector: string) {
      return selector === "body" ? body : null;
    },
  };
  const local = new Map<string, string>();
  const window = {
    location: new URL(url),
    localStorage: {
      getItem(key: string) {
        return local.get(key) ?? null;
      },
      setItem(key: string, value: string) {
        local.set(key, value);
      },
    },
    setTimeout,
  };
  vi.stubGlobal("window", window);
  vi.stubGlobal("document", document);
  return { window, body };
}

describe("agent UI test harness", () => {
  beforeEach(() => {
    vi.unstubAllGlobals();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    if (originalWindow) vi.stubGlobal("window", originalWindow);
    if (originalDocument) vi.stubGlobal("document", originalDocument);
  });

  it("is gated by explicit dev query flag", async () => {
    const mod = await loadAgentTestModule();
    if (!mod) return;

    installDom("http://localhost:1420/");
    expect(mod.shouldInstallAgentTestHarness()).toBe(false);

    installDom("http://localhost:1420/?airp_agent_test=1");
    expect(mod.shouldInstallAgentTestHarness()).toBe(true);
  });

  it("exposes safe UI actions and snapshots", async () => {
    const mod = await loadAgentTestModule();
    if (!mod) return;

    const { body } = installDom();
    const calls: Array<[string, Json | undefined]> = [];
    let busError: string | null = null;
    const state: Record<string, Json> = { "w-chat": { messages: {}, order: [] } };
    const surface: SurfaceSnapshot = {
      kind: "snapshot",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story",
      revision: "1",
      blueprint: {
        version: 2,
        root: { type: "widget", id: "chat-node", instanceId: "w-chat" },
        widgets: [{ id: "w-chat", type: "core.chat" }],
      },
    };
    const harness = mod.installAgentTestHarness({
      dispatchIntent(name, params) {
        calls.push([name, params]);
      },
      getBlueprint: () => surface.blueprint,
      getSurface: () => surface,
      applySurface: () => ({ status: "applied", snapshot: surface }),
      setWidgetState: (scope, value) => { state[scope] = value; },
      setWidgetOperation: (instanceId, operation) => { state[`operation:${instanceId}`] = operation; },
      patchWidgetState: (scope, patch) => {
        const target = state[scope] as Record<string, Json>;
        for (const op of patch) if (op.op === "replace") target[op.path.slice(1)] = op.value ?? null;
      },
      getState: () => state,
      getSelectedCharacterId: () => "alice",
      getBusError: () => busError,
      setBusError: (message) => { busError = message; },
    });

    expect(harness).not.toBeNull();
    expect((window as Window & { __AIRP_AGENT_TEST__?: unknown }).__AIRP_AGENT_TEST__).toBe(harness);

    harness!.selectCharacter("bob");
    harness!.sendChat("hello", "bob");
    harness!.refreshCharacters();
    harness!.setBusError("offline");

    expect(calls).toEqual([
      ["characters.select", { character_id: "bob" }],
      ["characters.select", { character_id: "bob" }],
      ["chat.send", { text: "hello" }],
      ["characters.list", {}],
    ]);
    expect(harness!.getSnapshot().selectedCharacterId).toBe("alice");
    expect(harness!.applySurface(surface)).toMatchObject({ status: "applied" });
    harness!.setWidgetState("w-chat", { ready: true });
    expect(state["w-chat"]).toEqual({ ready: true });
    harness!.setWidgetOperation("w-chat", { status: "streaming" });
    expect(state["operation:w-chat"]).toEqual({ status: "streaming" });
    harness!.patchWidgetState("w-chat", [{ op: "replace", path: "/ready", value: false }]);
    expect(state["w-chat"]).toEqual({ ready: false });
    expect(busError).toBe("offline");
    expect(harness!.getState("w-chat")).toEqual(state["w-chat"]);
    expect(harness!.getText()).toContain("AIRP ready");
    expect(await harness!.waitForText("AIRP")).toBe(true);

    body.textContent = "changed";
    expect(await harness!.waitForText("missing", 20)).toBe(false);
  });
});
