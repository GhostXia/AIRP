<script setup lang="ts">
import { onMounted, onUnmounted, shallowRef, ref } from "vue";
import type { Blueprint, Envelope, Json } from "./protocol/types";
import type { AgentBus } from "./protocol/bus";
import { createBus, isTauriEnvironment } from "./protocol/bus-factory";
import { validateEnvelope } from "./protocol/guard";
import { stateStore, setState, patchState, applyJsonPatch } from "./state/store";
import { registerBuiltins, applyManifestMessage } from "./registry";
import BlueprintRenderer from "./components/BlueprintRenderer.vue";

// Register first-party widgets into the open registry.
registerBuiltins();

const blueprint = shallowRef<Blueprint | null>(null);

// Phase 0: in the Tauri shell the engine does not push a blueprint, so the UI
// self-builds a minimal one (chat + a character picker sidebar). Outside Tauri
// the MockBus still primes its own sample blueprint, which we honor as-is.
const isTauri = isTauriEnvironment();
const selectedCharacterId = ref<string>("");

const MINIMAL_BLUEPRINT: Blueprint = {
  version: "bp-phase0",
  profile: "rp:phase0",
  theme: { name: "phase0", tokens: { accent: "#00e5ff" } },
  layout: {
    type: "dock",
    areas: [
      { id: "main", widgets: ["w-chat"] },
      { id: "sidebar", widgets: ["w-characters"], props: { side: "right" } },
    ],
  },
  widgets: [
    { id: "w-chat", type: "core.chat", props: { title: "对话" }, state: "w-chat" },
    { id: "w-characters", type: "core.characters", state: "w-characters", capabilities: ["read:state"] },
  ],
};

// The bus is picked per environment: Tauri shell → TauriBus over IPC to the
// Rust core (→ AIRP engine); everywhere else → MockBus (no backend). Built in
// onMounted because the Tauri transport is async to construct.
let bus: AgentBus | null = null;
let unsubscribe: (() => void) | null = null;
let disposed = false;

function onEnvelope(e: Envelope): void {
  const guard = validateEnvelope(e);
  if (!guard.ok) {
    console.error("[App] rejected envelope:", guard.error, e);
    busError.value = `envelope: ${guard.error}`;
    reportError(e, guard.error);
    return;
  }
  const body = e.body;
  // Clear a stale backend-error banner once a good (non-error) envelope arrives,
  // so a successful retry doesn't leave the old error visible (coderabbit finding).
  if (body.kind !== "error" && busError.value) busError.value = null;
  if (body.kind === "manifest") {
    applyManifestMessage(body.op, body.manifests);
  } else if (body.kind === "blueprint") {
    if (body.op === "set" && body.blueprint) {
      blueprint.value = body.blueprint;
    } else if (body.op === "patch" && body.patch && blueprint.value) {
      const next = structuredClone(blueprint.value);
      applyJsonPatch(next as unknown as Json, body.patch);
      blueprint.value = next;
    }
  } else if (body.kind === "state") {
    if (body.op === "set") setState(body.scope, body.state ?? null);
    else if (body.op === "patch" && body.patch) patchState(body.scope, body.patch);
  } else if (body.kind === "error") {
    busError.value = `${body.code}: ${body.message}`;
  }
}

/** Report a rejected envelope upstream as an `error` body (best-effort). */
function reportError(rejected: Envelope, reason: string): void {
  if (!bus) return;
  Promise.resolve(
    bus.dispatch({
      v: 1,
      id: `err-${Date.now()}`,
      ts: Date.now(),
      src: "ui",
      body: { kind: "error", code: "ENVELOPE_INVALID", message: reason, detail: { ref: rejected.id } },
    }),
  ).catch((err: unknown) => {
    console.error("[App] reportError dispatch failed:", err);
  });
}

// Surfaced in the template so a backend failure isn't a silent empty shell.
const busError = ref<string | null>(null);

function onIntent(name: string, params?: Json): void {
  if (!bus) return;
  // Phase 0: characters.select just records the selection locally (the engine
  // is stateless per-call; the chosen id rides on each chat.send). chat.send is
  // tagged with the current selection so the engine knows which card to assemble.
  if (name === "characters.select") {
    const id = (params as { character_id?: string } | undefined)?.character_id;
    if (id) selectedCharacterId.value = id;
    return;
  }
  let finalParams = params;
  if (name === "chat.send" && selectedCharacterId.value) {
    const obj = (params ?? {}) as Record<string, Json>;
    finalParams = { ...obj, character_id: selectedCharacterId.value } as Json;
  }
  Promise.resolve(
    bus.dispatch({
      v: 1,
      id: `ui-${Date.now()}`,
      ts: Date.now(),
      src: "ui",
      body: { kind: "intent", name, params: finalParams },
    }),
  ).catch((err: unknown) => {
    console.error("[App] dispatch failed:", err);
    busError.value = String(err ?? "dispatch failed");
  });
}

function refreshCharacters(): void {
  onIntent("characters.list", {});
}

type AgentTestInstaller = {
  installAgentTestHarness: (ctx: {
    dispatchIntent: (name: string, params?: Json) => void;
    getBlueprint: () => Blueprint | null;
    getState: () => typeof stateStore;
    getSelectedCharacterId: () => string;
    getBusError: () => string | null;
  }) => unknown;
};

const agentTestModules = import.meta.glob<AgentTestInstaller>("./agent-test.ts");

async function installOptionalAgentTestHarness(): Promise<void> {
  const load = Object.values(agentTestModules)[0];
  if (!load) return;
  const mod = await load();
  mod.installAgentTestHarness({
    dispatchIntent: onIntent,
    getBlueprint: () => blueprint.value,
    getState: () => stateStore,
    getSelectedCharacterId: () => selectedCharacterId.value,
    getBusError: () => busError.value,
  });
}

onMounted(async () => {
  try {
    const built = await createBus();
    if (disposed) return;
    bus = built;
    unsubscribe = bus.subscribe(onEnvelope);
    // In the Tauri shell the engine does not push a blueprint — self-build the
    // minimal one and ask the engine for the character list. MockBus (non-Tauri)
    // keeps its own sample blueprint via its subscribe priming.
    if (isTauri) {
      blueprint.value = MINIMAL_BLUEPRINT;
      // Prime an empty chat scope so the first patch (add /messages/{id} and
      // /order/-) applies. messages is id-keyed, order holds render order.
      setState("w-chat", { messages: {}, order: [] });
      setState("w-characters", { ids: [], loaded: false });
      refreshCharacters();
    }
    await installOptionalAgentTestHarness();
  } catch (err) {
    console.error("[App] createBus failed:", err);
    busError.value = String(err ?? "createBus failed");
  }
});
onUnmounted(() => {
  disposed = true;
  unsubscribe?.();
});
</script>

<template>
  <main class="app">
    <header class="topbar">
      <strong>AIRP&nbsp;UI</strong>
      <small>{{ isTauri ? "phase0 · engine live" : "scaffold · mock" }}</small>
      <small v-if="isTauri && selectedCharacterId" class="char-badge">角色: {{ selectedCharacterId }}</small>
      <button v-if="isTauri" class="refresh" @click="refreshCharacters">刷新角色</button>
    </header>
    <div v-if="busError" class="bus-error">bus: {{ busError }}</div>
    <BlueprintRenderer
      v-if="blueprint"
      :blueprint="blueprint"
      :state="stateStore"
      @intent="onIntent"
    />
    <div v-else class="loading">等待 Blueprint…</div>
  </main>
</template>

<style>
:root {
  --accent: #00e5ff;
}
* {
  box-sizing: border-box;
}
body {
  margin: 0;
  font-family: system-ui, -apple-system, "Segoe UI", sans-serif;
  background: #0b0e14;
  color: #e6e6e6;
}
.app {
  display: flex;
  flex-direction: column;
  height: 100vh;
}
.topbar {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding: 10px 14px;
  border-bottom: 1px solid rgba(255, 255, 255, 0.08);
}
.topbar small {
  opacity: 0.6;
}
.topbar .char-badge {
  color: var(--accent);
  opacity: 0.9;
}
.topbar .refresh {
  margin-left: auto;
  font-size: 12px;
  padding: 4px 8px;
}
.loading {
  margin: auto;
  opacity: 0.6;
}
.bus-error {
  margin: 10px 14px;
  padding: 6px 10px;
  color: #ffb4b4;
  background: rgba(255, 80, 80, 0.08);
  border: 1px solid rgba(255, 80, 80, 0.25);
  border-radius: 6px;
  font-size: 13px;
}
input,
button {
  background: rgba(255, 255, 255, 0.06);
  color: inherit;
  border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 6px;
  padding: 6px 10px;
}
button {
  cursor: pointer;
}
</style>
