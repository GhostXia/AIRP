<script setup lang="ts">
import { computed, onMounted, onUnmounted, shallowRef, ref } from "vue";
import type { Blueprint, Envelope, Json } from "./protocol/types";
import type { AgentBus } from "./protocol/bus";
import { createBus, isTauriEnvironment } from "./protocol/bus-factory";
import { validateEnvelope } from "./protocol/guard";
import { stateStore, setState, patchState, applyJsonPatch } from "./state/store";
import { registerBuiltins, applyManifestMessage } from "./registry";
import BlueprintRenderer from "./components/BlueprintRenderer.vue";
import SettingsModal from "./widgets/SettingsModal.vue";
import DesktopRail from "./components/shell/DesktopRail.vue";
import ContextInspector from "./components/shell/ContextInspector.vue";
import UiButton from "./components/primitives/UiButton.vue";
import { WORKSPACE_PRESETS, type WorkspaceId } from "./shell-model";

// Register first-party widgets into the open registry.
registerBuiltins();

const SHELL_PREVIEW_BLUEPRINT: Blueprint = {
  version: "shell-preview-v1",
  profile: "rp:story-preview",
  theme: { name: "airp-light" },
  layout: {
    type: "dock",
    areas: [
      { id: "main", widgets: ["w-chat"] },
      { id: "sidebar", widgets: ["w-characters"], props: { side: "right" } },
    ],
  },
  widgets: [
    { id: "w-chat", type: "core.chat", props: { title: "故事预览" }, state: "w-chat" },
    { id: "w-characters", type: "core.characters", state: "w-characters", capabilities: ["read:state"] },
  ],
};

const blueprint = shallowRef<Blueprint | null>(SHELL_PREVIEW_BLUEPRINT);

// PR 3 deliberately renders one fixed, labelled Surface fixture. The real
// Surface endpoint/runtime arrives in later #564 slices; browser preview must
// not imply a successful Engine connection.
const isTauri = isTauriEnvironment();
const connectionState = ref<"preview" | "connecting" | "connected" | "failed">(
  isTauri ? "connecting" : "preview",
);
const connectionLabel = computed(() => {
  if (connectionState.value === "connected") return "Engine 已连接";
  if (connectionState.value === "connecting") return "Engine 连接中";
  if (connectionState.value === "failed") return "Engine 未连接";
  return "固定协议预览";
});
const selectedCharacterId = ref<string>("");
const showSettings = ref(false);

const activeWorkspace = ref<WorkspaceId>("story");
const focusMode = ref(false);
const inspectorCollapsed = ref(false);
const activeWorkspacePreset = computed(
  () => WORKSPACE_PRESETS.find((workspace) => workspace.id === activeWorkspace.value) ?? WORKSPACE_PRESETS[0],
);

setState("w-chat", {
  messages: {
    preview: {
      id: "preview",
      role: "narrator",
      text: "固定 Surface 预览：这里验证桌面 shell 的布局、状态与可访问性；真实 Engine 链路将在后续纵切接入。",
    },
  },
  order: ["preview"],
});
setState("w-characters", { ids: [], loaded: true });

// The legacy Tauri transport remains available only when this Vue bundle is
// explicitly run inside Tauri. Browser preview does not construct MockBus.
let bus: AgentBus | null = null;
let unsubscribe: (() => void) | null = null;
let disposed = false;
let compactInspectorMedia: MediaQueryList | null = null;
let busAttempt = 0;

function collapseForCompactViewport(event: MediaQueryListEvent | MediaQueryList): void {
  if (event.matches) inspectorCollapsed.value = true;
}

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
  const activeBus = bus;
  Promise.resolve(
    activeBus.dispatch({
      v: 1,
      id: `err-${Date.now()}`,
      ts: Date.now(),
      src: "ui",
      body: { kind: "error", code: "ENVELOPE_INVALID", message: reason, detail: { ref: rejected.id } },
    }),
  ).catch((err: unknown) => {
    console.error("[App] reportError dispatch failed:", err);
    if (bus === activeBus) invalidateBus(String(err ?? "dispatch failed"));
  });
}

// Surfaced in the template so a backend failure isn't a silent empty shell.
const busError = ref<string | null>(null);

function invalidateBus(message: string): void {
  busAttempt += 1;
  const stop = unsubscribe;
  unsubscribe = null;
  bus = null;
  stop?.();
  connectionState.value = "failed";
  busError.value = message;
}

function onIntent(name: string, params?: Json): void {
  if (!bus) return;
  const activeBus = bus;
  // Phase 0: characters.select records the selection locally (the engine is
  // stateless per-call; the chosen id rides on each chat.send). chat.send is
  // tagged with the current selection so the engine knows which card to assemble.
  // M4: 选角色后拉 chat history（legacy 单 session），让用户重启后看到旧对话。
  if (name === "characters.select") {
    const id = (params as { character_id?: string } | undefined)?.character_id;
    if (id) {
      selectedCharacterId.value = id;
      // 拉取该角色的 chat history。此处递归调用 onIntent 函数本身，但
      // chat.history 走正常 dispatch 路径，不会回到 characters.select 分支，
      // 因此不会无限递归。
      onIntent("chat.history", { character_id: id } as Json);
    }
    return;
  }
  let finalParams = params;
  if (name === "chat.send" && selectedCharacterId.value) {
    const obj = (params ?? {}) as Record<string, Json>;
    finalParams = { ...obj, character_id: selectedCharacterId.value } as Json;
  }
  Promise.resolve(
    activeBus.dispatch({
      v: 1,
      id: `ui-${Date.now()}`,
      ts: Date.now(),
      src: "ui",
      body: { kind: "intent", name, params: finalParams },
    }),
  ).catch((err: unknown) => {
    console.error("[App] dispatch failed:", err);
    if (bus === activeBus) invalidateBus(String(err ?? "dispatch failed"));
  });
}

async function initializeBus(): Promise<void> {
  if (!isTauri || disposed) return;
  const attempt = ++busAttempt;
  connectionState.value = "connecting";
  busError.value = null;
  unsubscribe?.();
  unsubscribe = null;
  bus = null;
  try {
    const built = await createBus();
    if (disposed || attempt !== busAttempt) return;
    const stop = await built.subscribe(onEnvelope);
    if (disposed || attempt !== busAttempt) {
      stop();
      return;
    }
    unsubscribe = stop;
    bus = built;
    connectionState.value = "connected";
    setState("w-characters", { ids: [], loaded: false });
    setState("w-settings", { loaded: false });
    onIntent("characters.list", {});
  } catch (err) {
    if (attempt !== busAttempt) return;
    console.error("[App] createBus failed:", err);
    invalidateBus(String(err ?? "createBus failed"));
  }
}

async function refreshCharacters(): Promise<void> {
  if (!bus) {
    await initializeBus();
    return;
  }
  onIntent("characters.list", {});
}

type AgentTestInstaller = {
  installAgentTestHarness: (ctx: {
    dispatchIntent: (name: string, params?: Json) => void;
    getBlueprint: () => Blueprint | null;
    getState: () => typeof stateStore;
    getSelectedCharacterId: () => string;
    getBusError: () => string | null;
    setBusError: (message: string | null) => void;
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
    setBusError: (message) => { busError.value = message; },
  });
}

onMounted(async () => {
  compactInspectorMedia = window.matchMedia("(max-width: 1180px)");
  collapseForCompactViewport(compactInspectorMedia);
  compactInspectorMedia.addEventListener("change", collapseForCompactViewport);
  if (isTauri) await initializeBus();
  try {
    await installOptionalAgentTestHarness();
  } catch (err) {
    console.error("[App] agent test harness failed:", err);
  }
});
onUnmounted(() => {
  disposed = true;
  busAttempt += 1;
  const stop = unsubscribe;
  unsubscribe = null;
  bus = null;
  stop?.();
  compactInspectorMedia?.removeEventListener("change", collapseForCompactViewport);
});
</script>

<template>
  <main class="desktop-shell" :class="{ 'desktop-shell--focus': focusMode, 'desktop-shell--inspector-collapsed': inspectorCollapsed }">
    <DesktopRail
      :workspaces="WORKSPACE_PRESETS"
      :active="activeWorkspace"
      :compact="focusMode"
      @select="activeWorkspace = $event"
      @toggle-focus="focusMode = !focusMode"
    />

    <section class="workspace" aria-labelledby="workspace-title">
      <header class="workspace__topbar">
        <div class="workspace__identity">
          <span class="workspace__chapter">Workspace / {{ activeWorkspacePreset.id }}</span>
          <h1 id="workspace-title">{{ activeWorkspacePreset.label }}</h1>
        </div>
        <div class="workspace__actions">
          <span class="connection" :class="{ 'connection--live': connectionState === 'connected' }">
            <span aria-hidden="true"></span>{{ connectionLabel }}
          </span>
          <UiButton
            v-if="!focusMode"
            class="context-toggle"
            label="切换上下文检查器"
            :pressed="!inspectorCollapsed"
            @click="inspectorCollapsed = !inspectorCollapsed"
          >上下文</UiButton>
          <UiButton v-if="isTauri" label="打开设置" @click="showSettings = true">设置</UiButton>
        </div>
      </header>

      <div v-if="busError" class="status-banner status-banner--error" role="alert">
        <strong>Surface 未更新</strong>
        <span>{{ busError }}</span>
        <UiButton label="重试 Engine 连接" @click="refreshCharacters">重试</UiButton>
      </div>
      <div v-else-if="!isTauri" class="status-banner" role="status">
        <strong>Shell preview</strong>
        <span>此 PR 只验证桌面设计 shell；没有使用 MockBus 制造成功状态。</span>
      </div>

      <section class="surface" aria-label="动态 Surface 容器">
        <header class="surface__heading">
          <div>
            <span class="surface__kicker">Surface / story.preview</span>
            <h2>{{ activeWorkspacePreset.description }}</h2>
          </div>
          <span class="surface__revision">rev fixture-1</span>
        </header>
        <div class="surface__body">
          <BlueprintRenderer
            v-if="blueprint"
            :blueprint="blueprint"
            :state="stateStore"
            @intent="onIntent"
          />
          <div v-else class="surface-state" role="status">
            <strong>正在准备工作区</strong>
            <span>等待一份通过校验的 Surface snapshot。</span>
          </div>
        </div>
      </section>

      <footer class="workspace__footer">
        <span>Blueprint v2 protocol layer</span>
        <span v-if="selectedCharacterId">角色 {{ selectedCharacterId }}</span>
        <span>键盘：方向键切换工作区</span>
      </footer>
    </section>

    <ContextInspector
      v-if="!focusMode"
      :collapsed="inspectorCollapsed"
      :workspace-label="activeWorkspacePreset.label"
      @toggle="inspectorCollapsed = !inspectorCollapsed"
    />
    <SettingsModal
      v-if="isTauri"
      :state="stateStore['w-settings']"
      :visible="showSettings"
      @intent="onIntent"
      @close="showSettings = false"
    />
  </main>
</template>

<style scoped>
.desktop-shell { display: grid; grid-template-columns: var(--desktop-rail-w) minmax(0, 1fr) var(--desktop-inspector-w); width: 100%; height: 100dvh; overflow: hidden; background: var(--bg-base); }
.desktop-shell--focus { grid-template-columns: var(--desktop-rail-compact-w) minmax(0, 1fr); }
.desktop-shell--inspector-collapsed { grid-template-columns: var(--desktop-rail-w) minmax(0, 1fr) 38px; }
.workspace { display: grid; grid-template-rows: var(--desktop-topbar-h) auto minmax(0, 1fr) 28px; min-width: 0; min-height: 0; }
.workspace__topbar { display: flex; align-items: center; justify-content: space-between; gap: var(--space-4); padding: 0 18px 0 22px; border-bottom: 1px solid var(--border-default); background: color-mix(in srgb, var(--bg-surface) 92%, transparent); }
.workspace__identity { position: relative; display: flex; align-items: baseline; gap: 12px; min-width: 0; }
.workspace__identity::before { content: ""; position: absolute; left: -22px; top: -17px; width: 4px; height: var(--desktop-topbar-h); background: var(--primary); }
.workspace__chapter, .surface__kicker, .surface__revision { color: var(--text-tertiary); font: 600 10px/1 var(--font-utility); letter-spacing: .08em; text-transform: uppercase; }
h1 { margin: 0; font: 650 21px/1 var(--font-display); }
.workspace__actions { display: flex; align-items: center; gap: 8px; }
.connection { display: flex; align-items: center; gap: 7px; margin-right: 4px; color: var(--text-secondary); font-size: 11px; white-space: nowrap; }
.connection > span { width: 7px; height: 7px; border-radius: 50%; background: var(--warning); box-shadow: 0 0 0 3px var(--warning-tint); }
.connection--live > span { background: var(--success); box-shadow: 0 0 0 3px var(--success-tint); }
.status-banner { display: flex; align-items: center; gap: 12px; min-height: 42px; padding: 7px 18px 7px 22px; border-bottom: 1px solid var(--border-default); background: var(--warning-tint); color: var(--text-secondary); font-size: 12px; }
.status-banner strong { color: var(--text-primary); font: 700 10px/1 var(--font-utility); letter-spacing: .06em; text-transform: uppercase; }
.status-banner--error { background: var(--danger-tint); color: var(--danger); }
.status-banner :deep(.ui-button) { min-height: 28px; margin-left: auto; }
.surface { display: grid; grid-template-rows: auto minmax(0, 1fr); min-width: 0; min-height: 0; margin: 14px 16px 10px; overflow: hidden; border: 1px solid var(--border-default); border-radius: var(--radius-card); background: var(--bg-surface); box-shadow: 0 1px 0 rgba(0, 0, 0, .03); }
.surface__heading { display: flex; align-items: center; justify-content: space-between; gap: 16px; min-height: 58px; padding: 10px 16px 10px 18px; border-bottom: 1px solid var(--border-default); }
.surface__heading > div { display: grid; gap: 5px; }
.surface__heading h2 { margin: 0; font: 650 16px/1.2 var(--font-display); }
.surface__body { min-height: 0; overflow: hidden; }
.surface__body :deep(.blueprint) { padding: 10px; }
.surface-state { display: grid; place-content: center; gap: 8px; height: 100%; text-align: center; }
.surface-state span { color: var(--text-secondary); font-size: 13px; }
.workspace__footer { display: flex; align-items: center; gap: 18px; padding: 0 18px; overflow: hidden; border-top: 1px solid var(--border-default); color: var(--text-tertiary); font: 500 9px/1 var(--font-utility); white-space: nowrap; }
@media (max-width: 1180px) { .desktop-shell { grid-template-columns: var(--desktop-rail-compact-w) minmax(0, 1fr) 38px; } .desktop-shell--focus { grid-template-columns: var(--desktop-rail-compact-w) minmax(0, 1fr); } .desktop-shell:not(.desktop-shell--inspector-collapsed):not(.desktop-shell--focus) { grid-template-columns: var(--desktop-rail-compact-w) minmax(0, 1fr); } .desktop-shell:not(.desktop-shell--inspector-collapsed):not(.desktop-shell--focus) :deep(.inspector) { position: fixed; top: 0; right: 0; bottom: 0; z-index: 10; width: var(--desktop-inspector-w); overflow: hidden; box-shadow: -12px 0 28px color-mix(in srgb, var(--ink) 14%, transparent); } .desktop-shell:not(.desktop-shell--inspector-collapsed):not(.desktop-shell--focus) :deep(.inspector__toggle) { left: 12px; } .desktop-shell:not(.desktop-shell--inspector-collapsed):not(.desktop-shell--focus) :deep(.inspector__body) { padding-top: 52px; } .desktop-shell :deep(.rail__wordmark), .desktop-shell :deep(.rail__copy), .desktop-shell :deep(.rail__focus span:last-child) { display: none; } }
@media (max-width: 760px) { .desktop-shell, .desktop-shell--inspector-collapsed { grid-template-columns: var(--desktop-rail-compact-w) minmax(0, 1fr); } .desktop-shell :deep(.inspector), .context-toggle { display: none; } .workspace__chapter, .connection { display: none; } .surface { margin: 8px; } .workspace__footer { gap: 10px; } }
@media (max-height: 640px) { .status-banner { min-height: 34px; padding-block: 4px; } .status-banner > span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; } .surface { margin-top: 8px; } }
</style>
