<script setup lang="ts">
import {
  ref,
  shallowRef,
  computed,
  watch,
  onBeforeUnmount,
  onErrorCaptured,
  defineAsyncComponent,
} from "vue";
import type { Component } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";
import type { WidgetModule, WidgetContext } from "../registry/widget-module";
import { resolveWidget } from "../registry/registry";
import { getManifest } from "../registry/manifests";
import { needsConsent, isGranted, grant, effectiveCapabilities } from "../registry/consent";
import { SandboxBridge, createIframeTransport } from "../registry/sandbox-bridge";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

const reg = resolveWidget(props.instance.type);
const manifest = getManifest(props.instance.type);
const failed = ref<string | null>(null);

// esm (third-party) widget that the user hasn't approved yet → gate it.
const gated = computed(() => !!manifest && needsConsent(manifest) && !isGranted(manifest));

// A sandboxed esm widget runs inside an opaque-origin iframe (no
// allow-same-origin); the WidgetContext is bridged over postMessage so it
// cannot touch host DOM/global/same-origin resources (PLAN task D, SECURITY.md).
// Only esm widgets whose manifest opts in (`entry.sandbox: true`) take this
// path; in-process esm stays the default.
const sandboxed = computed(
  () => !!manifest && manifest.entry?.kind === "esm" && manifest.entry.sandbox === true,
);

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
function onIntent(name: string, params?: Json): void {
  emit("intent", name, params);
}
function approve(): void {
  if (manifest) grant(manifest);
}

// --- kind: vue (native component) ---
const vueComponent = shallowRef<Component | null>(null);
if (reg?.kind === "vue") {
  try {
    const r = reg.load();
    vueComponent.value = r instanceof Promise ? defineAsyncComponent(() => r) : r;
  } catch (e) {
    failed.value = errMsg(e);
  }
}

// --- kind: module (framework-agnostic) ---
const moduleEl = ref<HTMLElement | null>(null);
let mod: WidgetModule | null = null;
let stateCb: ((state: unknown) => void) | null = null;
let mounted = false;

// --- sandboxed esm: bridged over postMessage to an opaque-origin iframe ---
const sandboxEl = ref<HTMLElement | null>(null);
let sandboxBridge: SandboxBridge | null = null;

function makeContext(): WidgetContext {
  return {
    instance: props.instance,
    getState: () => props.state,
    onState: (cb) => {
      stateCb = cb;
      return () => {
        if (stateCb === cb) stateCb = null;
      };
    },
    emit: (name, params) => emit("intent", name, params),
    // Only the consented capabilities reach the widget (host-enforced).
    capabilities: manifest ? effectiveCapabilities(manifest) : props.instance.capabilities ?? [],
  };
}

async function mountModule(): Promise<void> {
  if (reg?.kind !== "module" || !moduleEl.value || mounted) return;
  mounted = true;
  try {
    const loaded = reg.load();
    mod = loaded instanceof Promise ? await loaded : loaded;
    await mod.mount(moduleEl.value, makeContext());
    stateCb?.(props.state);
  } catch (e) {
    mounted = false;
    failed.value = errMsg(e);
  }
}

// Mount the sandboxed widget inside an iframe. The transport is built from
// the manifest's esm `source`; the bridge proxies WidgetContext over
// postMessage. Intents/errors surface through the same handlers as in-process.
async function mountSandbox(): Promise<void> {
  if (!sandboxed.value || !sandboxEl.value || sandboxBridge) return;
  const source = manifest?.entry?.source;
  if (!source) {
    failed.value = "sandboxed esm widget missing entry.source";
    return;
  }
  try {
    const transport = createIframeTransport(sandboxEl.value, source);
    sandboxBridge = new SandboxBridge(
      transport,
      (name, params) => emit("intent", name, params),
      (message) => {
        failed.value = message;
      },
    );
    await sandboxBridge.mount(props.instance, effectiveCapabilities(manifest!));
    sandboxBridge.pushState(props.state);
  } catch (e) {
    failed.value = errMsg(e);
  }
}

// Mount once the module container exists (it only renders after any consent gate
// passes), so approving a gated widget triggers its mount.
watch(moduleEl, (el) => {
  if (el) void mountModule();
}, { immediate: true });

// Same for the sandbox container: only renders when sandboxed + consented.
watch(sandboxEl, (el) => {
  if (el) void mountSandbox();
}, { immediate: true });

watch(
  () => props.state,
  (s) => {
    try {
      // in-process module widget
      stateCb?.(s);
      // sandboxed widget: push the new state over the bridge
      sandboxBridge?.pushState(s);
    } catch (e) {
      failed.value = errMsg(e);
    }
  },
);

onBeforeUnmount(() => {
  try {
    mod?.unmount?.();
  } catch {
    /* a misbehaving widget must not break teardown */
  }
  try {
    sandboxBridge?.destroy();
    sandboxBridge = null;
  } catch {
    /* same: containment */
  }
});

onErrorCaptured((e) => {
  failed.value = errMsg(e);
  return false;
});
</script>

<template>
  <div class="widget-host">
    <div v-if="failed" class="widget-error">widget 出错：{{ instance.type }} — {{ failed }}</div>

    <div v-else-if="gated" class="widget-consent">
      <div class="w-title">第三方 widget：{{ instance.type }}</div>
      <div class="source">来源：{{ manifest?.entry?.source ?? "—" }}</div>
      <div class="caps">
        申请权限：
        <span v-if="(manifest?.capabilities ?? []).length === 0">无</span>
        <code v-for="c in manifest?.capabilities ?? []" :key="c">{{ c }}</code>
      </div>
      <button type="button" @click="approve">授权并加载</button>
      <div class="note">未授权前不会加载、不获得任何权限。我们不审核其代码，风险自担。</div>
    </div>

    <component
      :is="vueComponent"
      v-else-if="reg?.kind === 'vue' && vueComponent"
      :instance="instance"
      :state="state"
      @intent="onIntent"
    />
    <div v-else-if="sandboxed" ref="sandboxEl" class="widget-sandbox"></div>
    <div v-else-if="reg?.kind === 'module'" ref="moduleEl" class="widget-mount"></div>
    <div v-else class="widget-missing">未注册的 widget：{{ instance.type }}</div>
  </div>
</template>

<style scoped>
.widget-host {
  height: 100%;
  min-height: 0;
}
.widget-mount {
  height: 100%;
  padding: 8px;
}
.widget-sandbox {
  height: 100%;
  /* the iframe fills this; transparent so host theme shows through */
}
.widget-sandbox iframe {
  border: 0;
  width: 100%;
  height: 100%;
}
.widget-missing,
.widget-error {
  padding: 8px;
  font-size: 13px;
  color: #e0668a;
}
.widget-consent {
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 13px;
}
.widget-consent .source {
  opacity: 0.8;
  word-break: break-all;
}
.widget-consent .caps code {
  margin-right: 4px;
  background: rgba(255, 255, 255, 0.08);
  padding: 0 4px;
  border-radius: 4px;
}
.widget-consent .note {
  opacity: 0.6;
  font-size: 12px;
}
</style>
