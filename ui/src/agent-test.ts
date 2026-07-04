import type { Blueprint, Json } from "./protocol/types";

type DispatchIntent = (name: string, params?: Json) => void;

export interface AgentTestHarness {
  readonly version: 1;
  dispatchIntent(name: string, params?: Json): void;
  selectCharacter(characterId: string): void;
  sendChat(text: string, characterId?: string): void;
  refreshCharacters(): void;
  getSnapshot(): {
    blueprint: Blueprint | null;
    state: Record<string, Json>;
    selectedCharacterId: string;
    busError: string | null;
  };
  getState(scope?: string): Json | Record<string, Json> | undefined;
  getText(selector?: string): string;
  waitForText(text: string, timeoutMs?: number): Promise<boolean>;
}

export interface AgentTestContext {
  dispatchIntent: DispatchIntent;
  getBlueprint: () => Blueprint | null;
  getState: () => Record<string, Json>;
  getSelectedCharacterId: () => string;
  getBusError: () => string | null;
}

declare global {
  interface Window {
    __AIRP_AGENT_TEST__?: AgentTestHarness;
  }
}

export function shouldInstallAgentTestHarness(): boolean {
  if (!import.meta.env.DEV && import.meta.env.VITE_AIRP_AGENT_TEST !== "1") {
    return false;
  }
  if (import.meta.env.VITE_AIRP_AGENT_TEST === "1") return true;

  try {
    const params = new URLSearchParams(window.location.search);
    if (params.get("airp_agent_test") === "1") return true;
    return window.localStorage.getItem("AIRP_AGENT_TEST") === "1";
  } catch {
    return false;
  }
}

export function installAgentTestHarness(ctx: AgentTestContext): AgentTestHarness | null {
  if (!shouldInstallAgentTestHarness()) return null;

  const harness: AgentTestHarness = {
    version: 1,
    dispatchIntent(name, params) {
      ctx.dispatchIntent(name, params);
    },
    selectCharacter(characterId) {
      ctx.dispatchIntent("characters.select", { character_id: characterId });
    },
    sendChat(text, characterId) {
      if (characterId) this.selectCharacter(characterId);
      ctx.dispatchIntent("chat.send", { text });
    },
    refreshCharacters() {
      ctx.dispatchIntent("characters.list", {});
    },
    getSnapshot() {
      return {
        blueprint: clone(ctx.getBlueprint()),
        state: clone(ctx.getState()),
        selectedCharacterId: ctx.getSelectedCharacterId(),
        busError: ctx.getBusError(),
      };
    },
    getState(scope) {
      const state = ctx.getState();
      return scope ? clone(state[scope]) : clone(state);
    },
    getText(selector = "body") {
      return document.querySelector(selector)?.textContent ?? "";
    },
    async waitForText(text, timeoutMs = 5000) {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (document.body.textContent?.includes(text)) return true;
        await new Promise((resolve) => window.setTimeout(resolve, 50));
      }
      return false;
    },
  };

  window.__AIRP_AGENT_TEST__ = harness;
  console.info("[AIRP] agent UI test harness enabled");
  return harness;
}

function clone<T>(value: T): T {
  if (value == null) return value;
  return structuredClone(value);
}
