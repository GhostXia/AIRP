<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from "vue";
import type { BlueprintV2, Json, JsonPatch, SurfaceSnapshot } from "./protocol/types";
import type { SurfaceApplyResult } from "./protocol/surface-v2";
import {
  HttpEngineBus, IntentStreamFailure, IntentStreamInterrupted, type EngineSurfaceScope,
} from "./protocol/http-engine-bus";
import { currentBearer, renewDesktopSession } from "./protocol/desktop-session";
import { isTauriEnvironment } from "./protocol/bus-factory";
import { stateRevisionStore, stateStore, setState, patchState } from "./state/store";
import { SurfaceStateStore } from "./state/surface-store";
import { registerBuiltins } from "./registry";
import BlueprintRenderer from "./components/BlueprintRenderer.vue";
import SettingsModal from "./widgets/SettingsModal.vue";
import DesktopRail from "./components/shell/DesktopRail.vue";
import ContextInspector from "./components/shell/ContextInspector.vue";
import UiButton from "./components/primitives/UiButton.vue";
import { WORKSPACE_PRESETS, type WorkspaceId } from "./shell-model";
import {
  cancellationIsExplicitlyNotCommitted, captureChatBaseline, chatProjectionChanged,
  classifyChatRecoveryProjection,
  type ChatOperationBaseline,
} from "./chat-recovery";

// Register first-party widgets into the open registry.
registerBuiltins();

const SHELL_PREVIEW_SURFACE: SurfaceSnapshot = {
  kind: "snapshot",
  protocol: { major: 2, minor: 0 },
  surfaceId: "story.preview",
  revision: "1",
  blueprint: {
    version: 2,
    root: {
      type: "split",
      id: "story-root",
      orientation: "horizontal",
      children: [
        {
          type: "tabs",
          id: "story-tabs",
          active: "chat-node",
          children: [
            { type: "widget", id: "chat-node", instanceId: "w-chat" },
            { type: "widget", id: "characters-node", instanceId: "w-characters" },
          ],
        },
        {
          type: "stack",
          id: "story-tools",
          children: [{ type: "widget", id: "clock-node", instanceId: "w-clock" }],
        },
      ],
    },
    widgets: [
      { id: "w-chat", type: "core.chat", props: { title: "故事预览" } },
      { id: "w-characters", type: "core.characters" },
      { id: "w-clock", type: "core.clock" },
    ],
  },
};

const surface = new SurfaceStateStore();
const fixturePreview = import.meta.env.DEV && new URLSearchParams(location.search).get("airp_fixture") === "1";
const productionSurface = !fixturePreview
  && (location.pathname === "/desktop" || location.pathname.startsWith("/desktop/"));
if (!productionSurface) {
  const fixtureResult = surface.applySnapshot(SHELL_PREVIEW_SURFACE);
  if (fixtureResult.status !== "applied") console.warn("[App] preview Surface fixture rejected", fixtureResult);
}
const acceptedSurface = computed(() => surface.acceptedSnapshot);

// Outside the Engine-hosted /desktop/ entry, development stays on one clearly
// labelled fixture so a preview can never masquerade as a live connection.
const isTauri = isTauriEnvironment();
const connectionState = ref<"preview" | "connecting" | "connected" | "failed">(
  productionSurface ? "connecting" : "preview",
);
const connectionLabel = computed(() => {
  if (connectionState.value === "connected") return "Engine 已连接";
  if (connectionState.value === "connecting") return "Engine 连接中";
  if (connectionState.value === "failed") return "Engine 未连接";
  return "固定协议预览";
});
const selectedCharacterId = ref<string>("");
const selectedSessionId = ref<string>("");
const sessionIds = ref<string[]>([]);
const creatingSession = ref(false);
const showSettings = ref(false);

const activeWorkspace = ref<WorkspaceId>("story");
const focusMode = ref(false);
const inspectorCollapsed = ref(false);
const activeWorkspacePreset = computed(
  () => WORKSPACE_PRESETS.find((workspace) => workspace.id === activeWorkspace.value) ?? WORKSPACE_PRESETS[0],
);

setState("w-chat", {
  context: {
    character_id: "preview-character-stable-id",
    session_id: "00000000-0000-4000-8000-000000000001",
    persona_id: "preview-persona-stable-id",
    persona_source: "fixture",
    scene_id: "preview-scene-stable-id",
    worldbook_source_ids: ["character:preview-character-stable-id"],
  },
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

let bus: HttpEngineBus | null = null;
let scope: EngineSurfaceScope | null = null;
let disconnect: (() => void) | null = null;
let disposed = false;
let compactInspectorMedia: MediaQueryList | null = null;
let busAttempt = 0;

function collapseForCompactViewport(event: MediaQueryListEvent | MediaQueryList): void {
  if (event.matches) inspectorCollapsed.value = true;
}

function activateSurfaceTab(tabsId: string, childId: string): void {
  surface.activateTab(tabsId, childId);
}

function focusSurfaceWidget(instanceId: string): void {
  surface.focusWidget(instanceId);
}

// Surfaced in the template so a backend failure isn't a silent empty shell.
const busError = ref<string | null>(null);
const widgetOperations = ref<Record<string, Json>>({});

type Operation = {
  status?: string;
  mode?: string;
  body?: string;
  thinking?: string;
  userText?: string;
  error?: string;
  startedRevision?: string;
  history?: Record<string, unknown>;
  targetMessageId?: string;
  targetIndex?: number;
  baseline?: ChatOperationBaseline;
  idleObservations?: number;
  retryName?: string;
  retryParams?: Json;
};

function operationFor(instanceId: string): Operation {
  return (widgetOperations.value[instanceId] as Operation | undefined) ?? {};
}

function setOperation(instanceId: string, operation: Operation): void {
  widgetOperations.value = { ...widgetOperations.value, [instanceId]: operation as Json };
}

function mergeHistory(previous: Record<string, unknown> | undefined, next: Record<string, unknown>): Record<string, unknown> {
  if (!previous || !Array.isArray(previous.message_ids) || !Array.isArray(next.message_ids)) return next;
  const vectorFields = [
    "messages", "message_timestamps", "message_candidates", "message_swipe_index", "message_parents",
  ] as const;
  const rows = new Map<string, Record<string, unknown>>();
  for (const source of [next, previous]) {
    const ids = source.message_ids as unknown[];
    ids.forEach((id, index) => {
      if (typeof id !== "string" || rows.has(id)) return;
      const row: Record<string, unknown> = {};
      for (const field of vectorFields) {
        const values = source[field];
        if (Array.isArray(values)) row[field] = values[index];
      }
      rows.set(id, row);
    });
  }
  return {
    ...previous,
    ...next,
    message_ids: [...rows.keys()],
    ...Object.fromEntries(vectorFields.map((field) => [field, [...rows.values()].map((row) => row[field])])),
  };
}

function reconcileAcceptedOperations(freshIdleObservation = false): void {
  const snapshot = acceptedSurface.value;
  if (!snapshot) return;
  for (const [instanceId, raw] of Object.entries(widgetOperations.value)) {
    const operation = raw as Operation;
    if (!operation.status || !["awaiting_surface", "reconciling", "recovery_required"].includes(operation.status)) {
      continue;
    }
    const projection = classifyChatRecoveryProjection(snapshot, instanceId, operation);
    if (projection === "recovery_required") {
      setOperation(instanceId, {
        ...operation,
        status: "recovery_required",
        error: "Engine 检测到未完成提交，需要先完成会话恢复；请勿重复提交。",
      });
      continue;
    }
    if (projection === "committed") {
      setOperation(instanceId, { history: operation.history });
      continue;
    }
    if (operation.status === "awaiting_surface") continue;
    if (projection === "busy") {
      setOperation(instanceId, { ...operation, status: "reconciling", idleObservations: 0 });
    } else if (projection === "unchanged" && operation.status === "reconciling" && !freshIdleObservation) {
      queueMicrotask(() => {
        if (bus && scope) void probeRecoveringOperation(bus, scope, instanceId);
      });
    } else if ((projection === "changed" || projection === "unchanged") && freshIdleObservation) {
      if (projection === "changed" || chatProjectionChanged(snapshot, instanceId, operation)) {
        setOperation(instanceId, {
          ...operation,
          status: "recovery_required",
          error: "权威历史已变化但无法确认完整提交；请检查历史，切勿直接重试。",
        });
      } else {
        const observations = (operation.idleObservations ?? 0) + 1;
        setOperation(instanceId, observations >= 2 ? {
          ...operation,
          status: "retryable",
          idleObservations: observations,
          error: "Engine 已连续确认权威历史未变化；本次未提交，可以安全重试。",
        } : { ...operation, status: "reconciling", idleObservations: observations });
      }
    }
  }
}

watch(() => acceptedSurface.value?.revision, (revision) => {
  if (revision) reconcileAcceptedOperations();
});

function invalidateBus(message: string): void {
  busAttempt += 1;
  const stop = disconnect;
  const activeBus = bus;
  disconnect = null;
  bus = null;
  scope = null;
  if (stop) stop();
  else activeBus?.disconnect();
  connectionState.value = "failed";
  busError.value = message;
}

function applySurface(message: unknown): SurfaceApplyResult {
  const result = surface.apply(message);
  if (result.status === "applied") {
    busError.value = null;
    queueMicrotask(() => reconcileAcceptedOperations());
  }
  return result;
}

const recoveryProbes = new Map<string, HttpEngineBus>();

async function refreshSurfaceForBus(
  activeBus: HttpEngineBus,
  activeScope: EngineSurfaceScope,
): Promise<void> {
  await activeBus.refreshSurface(activeScope, {
    apply(message) {
      if (bus !== activeBus) throw new Error("Discarding a stale Surface refresh");
      return applySurface(message);
    },
  });
}

async function probeRecoveringOperation(
  activeBus: HttpEngineBus,
  activeScope: EngineSurfaceScope,
  instanceId: string,
  attempts = 2,
): Promise<void> {
  if (recoveryProbes.get(instanceId) === activeBus) return;
  recoveryProbes.set(instanceId, activeBus);
  try {
    for (let attempt = 0; attempt < attempts; attempt += 1) {
      await refreshSurfaceForBus(activeBus, activeScope);
      if (bus !== activeBus) return;
      reconcileAcceptedOperations(true);
      if (operationFor(instanceId).status !== "reconciling") return;
    }
  } catch (err: unknown) {
    if (bus === activeBus) busError.value = String(err ?? "Surface refresh failed");
  } finally {
    if (recoveryProbes.get(instanceId) === activeBus) recoveryProbes.delete(instanceId);
  }
}

function probeAllRecoveringOperations(): void {
  if (!bus || !scope) return;
  for (const [instanceId, raw] of Object.entries(widgetOperations.value)) {
    if ((raw as Operation).status === "reconciling") {
      void probeRecoveringOperation(bus, scope, instanceId);
    }
  }
}

async function reconcileOperation(
  activeBus: HttpEngineBus,
  activeScope: EngineSurfaceScope,
  instanceId: string,
): Promise<boolean> {
  try {
    await refreshSurfaceForBus(activeBus, activeScope);
  } catch (err: unknown) {
    if (bus === activeBus) busError.value = String(err ?? "Surface refresh failed");
    return false;
  }
  if (bus !== activeBus) return false;
  reconcileAcceptedOperations(true);
  return operationFor(instanceId).status === undefined;
}

async function onIntent(name: string, params?: Json, instanceId = "desktop-shell"): Promise<void> {
  const surfaceId = acceptedSurface.value?.surfaceId;
  if (!bus || !scope || !surfaceId) return;
  const activeBus = bus;
  const activeScope = scope;
  const current = operationFor(instanceId);
  const streams = ["chat.send", "chat.regen", "chat.continue"];
  const recoverableMutations = [...streams, "chat.swipe"];
  if (streams.includes(name)) {
    const raw = params as { text?: unknown } | undefined;
    setOperation(instanceId, {
      history: current.history,
      status: "streaming",
      mode: name.slice(5),
      body: "",
      thinking: "",
      userText: name === "chat.send" && typeof raw?.text === "string" ? raw.text : undefined,
      startedRevision: acceptedSurface.value?.revision,
      baseline: captureChatBaseline(acceptedSurface.value, instanceId),
      retryName: name,
      retryParams: params,
    });
  } else if (name === "chat.stop") {
    setOperation(instanceId, { ...current, status: "stopping" });
  } else if (name === "chat.loadMore") {
    setOperation(instanceId, { ...current, status: "loading_history" });
  } else if (name === "chat.swipe") {
    const raw = params as { message_id?: unknown; index?: unknown } | undefined;
    setOperation(instanceId, {
      ...current,
      status: "submitting",
      mode: "swipe",
      targetMessageId: typeof raw?.message_id === "string" ? raw.message_id : undefined,
      targetIndex: typeof raw?.index === "number" ? raw.index : undefined,
      startedRevision: acceptedSurface.value?.revision,
      baseline: captureChatBaseline(acceptedSurface.value, instanceId),
      idleObservations: 0,
      retryName: name,
      retryParams: params,
    });
  }
  try {
    const result = await activeBus.dispatchIntent(surfaceId, instanceId, name, params, ({ data }) => {
      if (bus !== activeBus) return;
      const live = operationFor(instanceId);
      if (data.type === "body_chunk" && typeof data.text === "string") {
        setOperation(instanceId, { ...live, status: "streaming", body: `${live.body ?? ""}${data.text}` });
      } else if (data.type === "think_chunk" && typeof data.text === "string") {
        setOperation(instanceId, { ...live, status: "streaming", thinking: `${live.thinking ?? ""}${data.text}` });
      } else if (data.type === "done") {
        setOperation(instanceId, { ...live, status: "awaiting_surface" });
      }
    });
    if (bus !== activeBus) return;
    if (streams.includes(name)) {
      await reconcileOperation(activeBus, activeScope, instanceId);
    } else if (name === "chat.loadMore" && result && typeof result === "object") {
      const live = operationFor(instanceId);
      setOperation(instanceId, {
        ...live,
        status: undefined,
        history: mergeHistory(live.history, result as Record<string, unknown>),
      });
    } else if (name === "chat.swipe") {
      setOperation(instanceId, { ...operationFor(instanceId), status: "awaiting_surface" });
      await reconcileOperation(activeBus, activeScope, instanceId);
    }
  } catch (err: unknown) {
    if (bus === activeBus) {
      const typed = err instanceof IntentStreamFailure
        ? err.data.error as { code?: unknown; commit_state?: unknown } | undefined
        : undefined;
      if (typed?.code === "cancelled") {
        const live = operationFor(instanceId);
        const commitState = err instanceof IntentStreamFailure
          ? (err.data.error as { commit_state?: unknown } | undefined)?.commit_state
          : undefined;
        if (cancellationIsExplicitlyNotCommitted(commitState)) {
          if (bus === activeBus) setOperation(instanceId, { history: live.history });
          return;
        }
        setOperation(instanceId, {
          ...live,
          status: "reconciling",
          error: commitState === "partially_committed"
            ? "取消结果正在与 Engine 权威历史核对；请勿重复提交。"
            : "取消回执缺少明确的未提交状态，正在核对 Engine 权威历史；请勿重复提交。",
        });
        await probeRecoveringOperation(activeBus, activeScope, instanceId);
        return;
      }
      if (recoverableMutations.includes(name) && (
        err instanceof IntentStreamInterrupted
        || !(err instanceof IntentStreamFailure)
        || typed?.commit_state !== "not_committed"
      )) {
        const live = operationFor(instanceId);
        setOperation(instanceId, {
          ...live,
          status: "reconciling",
          idleObservations: 0,
          error: "连接在终态前中断，正在核对 Engine 权威历史；不会自动重放。",
        });
        await probeRecoveringOperation(activeBus, activeScope, instanceId);
        return;
      }
      if (recoverableMutations.includes(name)) {
        const live = operationFor(instanceId);
        setOperation(instanceId, {
          ...live,
          status: "retryable",
          error: "Engine 已确认本次未提交，可以安全重试。",
        });
        return;
      }
      console.error("[App] dispatch failed:", err);
      const message = String(err ?? "intent dispatch failed");
      setOperation(instanceId, { ...operationFor(instanceId), status: "error", error: message });
      busError.value = message;
    }
  }
}

async function initializeBus(): Promise<void> {
  if (!productionSurface || disposed) return;
  const attempt = ++busAttempt;
  const previousScope = scope;
  connectionState.value = "connecting";
  busError.value = null;
  const previousStop = disconnect;
  const previousBus = bus;
  if (previousStop) previousStop();
  else previousBus?.disconnect();
  disconnect = null;
  bus = null;
  try {
    const built = new HttpEngineBus({
      bearer: currentBearer,
      renew: () => renewDesktopSession(),
      onConnection: (connected) => {
        if (attempt !== busAttempt) return;
        connectionState.value = connected ? "connected" : "connecting";
        if (connected) busError.value = null;
        if (connected) queueMicrotask(probeAllRecoveringOperations);
      },
      onError: (error) => {
        if (attempt !== busAttempt) return;
        busError.value = error.message;
      },
    });
    bus = built;
    const selected = await built.resolveScope();
    if (disposed || attempt !== busAttempt) {
      built.disconnect();
      if (bus === built) bus = null;
      return;
    }
    scope = selected;
    if (previousScope && (
      previousScope.characterId !== selected.characterId
      || previousScope.sessionId !== selected.sessionId
      || previousScope.userId !== selected.userId
    )) {
      widgetOperations.value = {};
    }
    selectedCharacterId.value = selected.characterId;
    selectedSessionId.value = selected.sessionId;
    sessionIds.value = await built.listSessions(selected.characterId, selected.userId);
    const stop = await built.connect(selected, { apply: applySurface });
    if (disposed || attempt !== busAttempt) {
      stop();
      return;
    }
    disconnect = stop;
    bus = built;
    connectionState.value = "connected";
  } catch (err) {
    if (attempt !== busAttempt) return;
    console.error("[App] createBus failed:", err);
    invalidateBus(String(err ?? "createBus failed"));
  }
}

async function refreshCharacters(): Promise<void> {
  await initializeBus();
}

async function stopActiveChatBeforeScopeChange(): Promise<boolean> {
  const operation = operationFor("chat");
  if (operation.status !== "streaming" && operation.status !== "stopping") return true;
  const surfaceId = acceptedSurface.value?.surfaceId;
  if (!bus || !surfaceId) return false;
  try {
    await bus.dispatchIntent(surfaceId, "chat", "chat.stop");
    setOperation("chat", {});
    return true;
  } catch (error) {
    const message = String(error ?? "active generation could not be stopped");
    setOperation("chat", { ...operation, status: "error", error: message });
    busError.value = message;
    return false;
  }
}

async function selectSession(sessionId: string, control?: HTMLSelectElement): Promise<void> {
  if (!sessionId || sessionId === selectedSessionId.value) return;
  if (!await stopActiveChatBeforeScopeChange()) {
    if (control) control.value = selectedSessionId.value;
    return;
  }
  sessionStorage.setItem("airp_session_id", sessionId);
  await initializeBus();
}

async function createSession(): Promise<void> {
  if (!bus || !scope || creatingSession.value) return;
  creatingSession.value = true;
  try {
    if (!await stopActiveChatBeforeScopeChange()) return;
    const sessionId = await bus.createSession(scope.characterId, scope.userId);
    sessionStorage.setItem("airp_session_id", sessionId);
    await initializeBus();
  } catch (error) {
    busError.value = String(error ?? "session creation failed");
  } finally {
    creatingSession.value = false;
  }
}

type AgentTestInstaller = {
  installAgentTestHarness: (ctx: {
    dispatchIntent: (name: string, params?: Json) => void;
    getBlueprint: () => BlueprintV2 | null;
    getSurface: () => SurfaceSnapshot | null;
    applySurface: (message: unknown) => SurfaceApplyResult;
    setWidgetState: (scope: string, state: Json) => void;
    setWidgetOperation: (instanceId: string, operation: Json) => void;
    patchWidgetState: (scope: string, patch: JsonPatch) => void;
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
    getBlueprint: () => acceptedSurface.value?.blueprint ?? null,
    getSurface: () => acceptedSurface.value,
    applySurface,
    setWidgetState: setState,
    setWidgetOperation: (instanceId, operation) => setOperation(instanceId, operation as Operation),
    patchWidgetState: patchState,
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
  if (productionSurface) await initializeBus();
  try {
    await installOptionalAgentTestHarness();
  } catch (err) {
    console.error("[App] agent test harness failed:", err);
  }
});
onUnmounted(() => {
  disposed = true;
  busAttempt += 1;
  const stop = disconnect;
  const activeBus = bus;
  disconnect = null;
  bus = null;
  if (stop) stop();
  else activeBus?.disconnect();
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
          <label v-if="productionSurface && sessionIds.length" class="session-picker">
            <span>会话</span>
            <select :value="selectedSessionId" @change="selectSession(($event.target as HTMLSelectElement).value, $event.target as HTMLSelectElement)">
              <option v-for="sessionId in sessionIds" :key="sessionId" :value="sessionId">{{ sessionId.slice(0, 8) }}</option>
            </select>
          </label>
          <UiButton v-if="productionSurface" label="创建会话" :disabled="creatingSession || !bus" @click="createSession">新会话</UiButton>
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
      <div v-else class="status-banner" role="status">
        <strong>{{ productionSurface ? "Engine Surface" : "Blueprint runtime preview" }}</strong>
        <span v-if="productionSurface">当前 Surface 来自 Engine snapshot + SSE；断流或 patch 失败会自动重同步。</span>
        <span v-else>当前 Surface 是本地 v2 fixture；没有使用 MockBus 或 legacy Engine 消息制造真实 Surface 状态。</span>
      </div>

      <section class="surface" aria-label="动态 Surface 容器">
        <header class="surface__heading">
          <div>
          <span class="surface__kicker">Surface / {{ acceptedSurface?.surfaceId ?? "unavailable" }}</span>
            <h2>{{ activeWorkspacePreset.description }}</h2>
          </div>
          <span class="surface__revision">rev {{ acceptedSurface?.revision ?? "—" }}</span>
        </header>
        <div class="surface__body">
          <BlueprintRenderer
            v-if="acceptedSurface"
            :blueprint="acceptedSurface.blueprint"
            :state="stateStore"
            :state-revisions="stateRevisionStore"
            :active-tabs="surface.activeTabByNodeId"
            :authoritative-props="productionSurface"
            :writable-widget-types="productionSurface ? ['core.chat'] : []"
            :operations="widgetOperations"
            @activate-tab="activateSurfaceTab"
            @focus-widget="focusSurfaceWidget"
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
      :surface-id="acceptedSurface?.surfaceId ?? null"
      :revision="acceptedSurface?.revision ?? null"
      :focused-widget-id="surface.focusedWidgetInstanceId"
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
.session-picker { display: flex; align-items: center; gap: 5px; color: var(--text-tertiary); font-size: 10px; }
.session-picker select { max-width: 110px; border: 1px solid var(--border-default); border-radius: var(--radius-input); background: var(--bg-surface); color: var(--text-primary); padding: 5px; }
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
@media (max-height: 420px) { .workspace { grid-template-rows: 44px auto minmax(0, 1fr); } .workspace__topbar { padding-inline: 14px; } .workspace__identity::before { top: -12px; height: 44px; } .status-banner:not(.status-banner--error), .surface__heading, .workspace__footer { display: none; } .surface { margin: 4px 8px; } .surface__body :deep(.blueprint) { padding-block: 0; } }
</style>
