<script setup lang="ts">
import { onMounted, onUnmounted, shallowRef, ref } from "vue";
import type { Blueprint, Envelope, Json } from "./protocol/types";
import type { AgentBus } from "./protocol/bus";
import { createBus } from "./protocol/bus-factory";
import { validateEnvelope } from "./protocol/guard";
import { stateStore, setState, patchState, applyJsonPatch } from "./state/store";
import { registerBuiltins, applyManifestMessage } from "./registry";
import BlueprintRenderer from "./components/BlueprintRenderer.vue";

// Register first-party widgets into the open registry.
registerBuiltins();

const blueprint = shallowRef<Blueprint | null>(null);

// The bus is picked per environment: Tauri shell → TauriBus over IPC to the
// Rust core (→ AIRP-Gateway); everywhere else → MockBus (no backend). Built in
// onMounted because the Tauri transport is async to construct.
//
// `disposed` guards the async completion: if the component unmounts before
// `createBus()` resolves, the late continuation exits without subscribing,
// so no listener is left attached to a dead instance.
let bus: AgentBus | null = null;
let unsubscribe: (() => void) | null = null;
let disposed = false;

function onEnvelope(e: Envelope): void {
  // Boundary guard: a real IPC/Gateway feed is untyped JSON. Validate the wire
  // shape BEFORE letting it touch the registry/store — a malformed envelope
  // (missing scope, a patch that isn't an array, an unknown body kind) would
  // otherwise corrupt state silently. Rejected envelopes are surfaced as an
  // `error` upstream and recorded in `busError` so the user sees the boundary
  // failure instead of an empty/half-applied UI. See docs/PLAN.md §2.6 #4.
  const guard = validateEnvelope(e);
  if (!guard.ok) {
    console.error("[App] rejected envelope:", guard.error, e);
    busError.value = `envelope: ${guard.error}`;
    reportError(e, guard.error);
    return;
  }
  const body = e.body;
  // Manifests are processed BEFORE blueprint: the renderer resolves a widget
  // type once at mount, so a third-party esm widget must be registered before
  // the blueprint that references it arrives.
  if (body.kind === "manifest") {
    applyManifestMessage(body.op, body.manifests);
  } else if (body.kind === "blueprint") {
    if (body.op === "set" && body.blueprint) {
      blueprint.value = body.blueprint;
    } else if (body.op === "patch" && body.patch && blueprint.value) {
      // shallowRef: patch a clone then reassign so the renderer updates.
      const next = structuredClone(blueprint.value);
      applyJsonPatch(next as unknown as Json, body.patch);
      blueprint.value = next;
    }
  } else if (body.kind === "state") {
    if (body.op === "set") setState(body.scope, body.state ?? null);
    else if (body.op === "patch" && body.patch) patchState(body.scope, body.patch);
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
      body: {
        kind: "error",
        code: "ENVELOPE_INVALID",
        message: reason,
        detail: { ref: rejected.id },
      },
    }),
  ).catch((err: unknown) => {
    console.error("[App] reportError dispatch failed:", err);
  });
}

// Surfaced in the template so a backend failure isn't a silent empty shell.
const busError = ref<string | null>(null);

function onIntent(name: string, params?: Json): void {
  if (!bus) return;
  // `dispatch` is async on TauriBus (awaits the IPC round-trip) and sync on
  // MockBus (`void`). Normalize to a promise so a rejection — e.g. the Rust
  // `airp_dispatch` command returns Err on version mismatch or a serde/IPC
  // failure — is caught instead of becoming an unhandled promise rejection
  // and silently dropping the user's intent.
  Promise.resolve(
    bus.dispatch({
      v: 1,
      id: `ui-${Date.now()}`,
      ts: Date.now(),
      src: "ui",
      body: { kind: "intent", name, params },
    }),
  ).catch((err: unknown) => {
    console.error("[App] dispatch failed:", err);
    busError.value = String(err ?? "dispatch failed");
  });
}

onMounted(async () => {
  // `createBus()` is async because the Tauri transport dynamically imports
  // `@tauri-apps/api`. That import (or the IPC handshake) can reject inside the
  // shell — without a catch Vue surfaces an unhandled promise rejection and the
  // user gets an empty shell with no indication of why. Wrap and record it.
  try {
    const built = await createBus();
    // If the component unmounted while the bus was being built, drop the bus
    // without subscribing — otherwise the listener would outlive the instance.
    if (disposed) return;
    bus = built;
    unsubscribe = bus.subscribe(onEnvelope);
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
      <small>scaffold · {{ blueprint?.theme?.name ?? "—" }}</small>
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
