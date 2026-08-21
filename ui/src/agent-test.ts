import { defineComponent } from "vue";
import type { BlueprintV2, Json, JsonPatch, SurfaceSnapshot } from "./protocol/types";
import type { SurfaceApplyResult } from "./protocol/surface-v2";
import { registerModuleWidget, registerVueWidget, resolveWidget } from "./registry";

type DispatchIntent = (name: string, params?: Json) => void;

interface WidgetLifecycleEvidence {
  mounts: number;
  unmounts: number;
  lastProps: Json | null;
  lastState: Json | null;
}

const widgetLifecycleEvidence: WidgetLifecycleEvidence = {
  mounts: 0,
  unmounts: 0,
  lastProps: null,
  lastState: null,
};

export interface AgentTestHarness {
  readonly version: 1;
  dispatchIntent(name: string, params?: Json): void;
  selectCharacter(characterId: string): void;
  sendChat(text: string, characterId?: string): void;
  refreshCharacters(): void;
  applySurface(message: unknown): SurfaceApplyResult;
  setWidgetState(scope: string, state: Json): void;
  patchWidgetState(scope: string, patch: JsonPatch): void;
  getWidgetLifecycle(): WidgetLifecycleEvidence;
  setBusError(message: string | null): void;
  getSnapshot(): {
    blueprint: BlueprintV2 | null;
    surface: SurfaceSnapshot | null;
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
  getBlueprint: () => BlueprintV2 | null;
  getSurface: () => SurfaceSnapshot | null;
  applySurface: (message: unknown) => SurfaceApplyResult;
  setWidgetState: (scope: string, state: Json) => void;
  patchWidgetState: (scope: string, patch: JsonPatch) => void;
  getState: () => Record<string, Json>;
  getSelectedCharacterId: () => string;
  getBusError: () => string | null;
  setBusError: (message: string | null) => void;
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

  if (!resolveWidget("agent-test.throw")) {
    registerVueWidget("agent-test.throw", () => defineComponent({
      name: "AgentTestThrowingWidget",
      setup() {
        throw new Error("agent-test throwing widget");
      },
    }));
  }
  if (!resolveWidget("agent-test.lifecycle")) {
    registerModuleWidget("agent-test.lifecycle", () => {
      let unsubscribe: (() => void) | null = null;
      return {
        mount(element, widgetContext) {
          widgetLifecycleEvidence.mounts += 1;
          widgetLifecycleEvidence.lastProps = clone(widgetContext.instance.props ?? null);
          element.textContent = "agent-test lifecycle widget";
          unsubscribe = widgetContext.onState((state) => {
            widgetLifecycleEvidence.lastState = clone(state as Json);
            widgetLifecycleEvidence.lastProps = clone(widgetContext.instance.props ?? null);
          });
        },
        unmount() {
          widgetLifecycleEvidence.unmounts += 1;
          unsubscribe?.();
          unsubscribe = null;
        },
      };
    });
  }
  widgetLifecycleEvidence.mounts = 0;
  widgetLifecycleEvidence.unmounts = 0;
  widgetLifecycleEvidence.lastProps = null;
  widgetLifecycleEvidence.lastState = null;

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
    applySurface(message) {
      return ctx.applySurface(message);
    },
    setWidgetState(scope, state) {
      ctx.setWidgetState(scope, state);
    },
    patchWidgetState(scope, patch) {
      ctx.patchWidgetState(scope, patch);
    },
    getWidgetLifecycle() {
      return clone(widgetLifecycleEvidence);
    },
    setBusError(message) {
      ctx.setBusError(message);
    },
    getSnapshot() {
      return {
        blueprint: clone(ctx.getBlueprint()),
        surface: clone(ctx.getSurface()),
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
  // Harness payloads are JSON; this also unwraps Vue proxies that structuredClone rejects.
  return JSON.parse(JSON.stringify(value)) as T;
}
