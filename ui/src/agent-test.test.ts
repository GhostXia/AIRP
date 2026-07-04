import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { installAgentTestHarness, shouldInstallAgentTestHarness } from "./agent-test";
import type { Json } from "./protocol/types";

const originalWindow = globalThis.window;
const originalDocument = globalThis.document;

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

  it("is gated by explicit dev query flag", () => {
    installDom("http://localhost:1420/");
    expect(shouldInstallAgentTestHarness()).toBe(false);

    installDom("http://localhost:1420/?airp_agent_test=1");
    expect(shouldInstallAgentTestHarness()).toBe(true);
  });

  it("exposes safe UI actions and snapshots", async () => {
    const { body } = installDom();
    const calls: Array<[string, Json | undefined]> = [];
    const state = { "w-chat": { messages: {}, order: [] } } satisfies Record<string, Json>;
    const harness = installAgentTestHarness({
      dispatchIntent(name, params) {
        calls.push([name, params]);
      },
      getBlueprint: () => ({ version: "bp", layout: { type: "dock", areas: [] }, widgets: [] }),
      getState: () => state,
      getSelectedCharacterId: () => "alice",
      getBusError: () => null,
    });

    expect(harness).not.toBeNull();
    expect(window.__AIRP_AGENT_TEST__).toBe(harness);

    harness!.selectCharacter("bob");
    harness!.sendChat("hello", "bob");
    harness!.refreshCharacters();

    expect(calls).toEqual([
      ["characters.select", { character_id: "bob" }],
      ["characters.select", { character_id: "bob" }],
      ["chat.send", { text: "hello" }],
      ["characters.list", {}],
    ]);
    expect(harness!.getSnapshot().selectedCharacterId).toBe("alice");
    expect(harness!.getState("w-chat")).toEqual(state["w-chat"]);
    expect(harness!.getText()).toContain("AIRP ready");
    expect(await harness!.waitForText("AIRP")).toBe(true);

    body.textContent = "changed";
    expect(await harness!.waitForText("missing", 20)).toBe(false);
  });
});
