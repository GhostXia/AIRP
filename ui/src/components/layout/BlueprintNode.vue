<script setup lang="ts">
import { ref } from "vue";
import type { SurfaceLayoutNode } from "../../protocol/types";

const props = defineProps<{
  node: SurfaceLayoutNode;
  activeTabs: Record<string, string>;
  outletIds: Record<string, string>;
  splitRatios: Record<string, number>;
  resizeDisabled?: boolean;
}>();

const emit = defineEmits<{
  (event: "activate-tab", tabsId: string, childId: string): void;
  (event: "resize-split", splitId: string, ratioBasisPoints: number): void;
  (event: "focus-widget", instanceId: string): void;
}>();
const focusedTabId = ref<string | null>(null);

function activeChildId(): string {
  if (props.node.type !== "tabs") return "";
  const local = props.activeTabs[props.node.id];
  return props.node.children.some((child) => child.id === local) ? local : props.node.active;
}

function tabStopChildId(): string {
  if (props.node.type !== "tabs") return "";
  return props.node.children.some((child) => child.id === focusedTabId.value)
    ? focusedTabId.value ?? activeChildId()
    : activeChildId();
}

function activateTab(childId: string): void {
  if (props.node.type !== "tabs" || props.resizeDisabled) return;
  focusedTabId.value = childId;
  emit("activate-tab", props.node.id, childId);
}

function panelId(tabsId: string, index: number): string {
  return `surface-tab-${encodeURIComponent(tabsId)}-${index}`;
}

function splitRatio(): number {
  if (props.node.type !== "split") return 5_000;
  return Math.min(9_000, Math.max(1_000, props.splitRatios[props.node.id] ?? 5_000));
}

function splitStyle(): Record<string, string> {
  const ratio = splitRatio();
  return {
    "--split-leading": `${ratio}fr`,
    "--split-trailing": `${10_000 - ratio}fr`,
    "--split-position": `${ratio / 100}%`,
  };
}

function resizeSplit(delta: number): void {
  if (props.node.type !== "split" || props.resizeDisabled) return;
  const next = Math.min(9_000, Math.max(1_000, splitRatio() + delta));
  if (next !== splitRatio()) emit("resize-split", props.node.id, next);
}

function activateFromKeyboard(event: KeyboardEvent, index: number): void {
  if (props.node.type !== "tabs" || props.resizeDisabled) return;
  const count = props.node.children.length;
  if (count === 0) return;
  let next = index;
  if (event.key === "ArrowRight" || event.key === "ArrowDown") next = (index + 1) % count;
  else if (event.key === "ArrowLeft" || event.key === "ArrowUp") next = (index - 1 + count) % count;
  else if (event.key === "Home") next = 0;
  else if (event.key === "End") next = count - 1;
  else return;
  event.preventDefault();
  const child = props.node.children[next];
  focusedTabId.value = child.id;
  emit("activate-tab", props.node.id, child.id);
  const buttons = (event.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>('[role="tab"]');
  buttons?.[next]?.focus();
}
</script>

<template>
  <div
    v-if="node.type === 'split'"
    class="layout-node layout-split"
    :class="`layout-split--${node.orientation}`"
    :data-node-id="node.id"
    :style="splitStyle()"
  >
    <BlueprintNode
      v-for="child in node.children"
      :key="child.id"
      :node="child"
      :active-tabs="activeTabs"
      :outlet-ids="outletIds"
      :split-ratios="splitRatios"
      :resize-disabled="resizeDisabled"
      @activate-tab="(tabsId, childId) => emit('activate-tab', tabsId, childId)"
      @resize-split="(splitId, ratio) => emit('resize-split', splitId, ratio)"
      @focus-widget="emit('focus-widget', $event)"
    />
    <div
      v-if="splitRatios[node.id] !== undefined"
      class="split-ridge"
      role="group"
      :aria-label="`调整分区 ${node.id}`"
    >
      <button
        type="button"
        :disabled="resizeDisabled || splitRatio() <= 1_000"
        aria-label="缩小主分区"
        @click="resizeSplit(-500)"
      >−</button>
      <output :aria-label="`主分区占比 ${splitRatio() / 100}%`">{{ splitRatio() / 100 }}%</output>
      <button
        type="button"
        :disabled="resizeDisabled || splitRatio() >= 9_000"
        aria-label="扩大主分区"
        @click="resizeSplit(500)"
      >+</button>
    </div>
  </div>

  <section v-else-if="node.type === 'tabs'" class="layout-node layout-tabs" :data-node-id="node.id">
    <div
      class="layout-tabs__list"
      role="tablist"
      :aria-label="`布局页签 ${node.id}`"
      :aria-busy="resizeDisabled || undefined"
    >
      <button
        v-for="(child, index) in node.children"
        :id="`${panelId(node.id, index)}-tab`"
        :key="child.id"
        type="button"
        role="tab"
        :aria-selected="activeChildId() === child.id"
        :aria-controls="panelId(node.id, index)"
        :aria-disabled="resizeDisabled || undefined"
        :tabindex="tabStopChildId() === child.id ? 0 : -1"
        @click="activateTab(child.id)"
        @keydown="activateFromKeyboard($event, index)"
      >
        {{ child.id }}
      </button>
    </div>
    <div class="layout-tabs__panels">
      <div
        v-for="(child, index) in node.children"
        v-show="activeChildId() === child.id"
        :id="panelId(node.id, index)"
        :key="child.id"
        class="layout-tabs__panel"
        role="tabpanel"
        :aria-labelledby="`${panelId(node.id, index)}-tab`"
        :inert="activeChildId() !== child.id"
      >
        <BlueprintNode
          :node="child"
          :active-tabs="activeTabs"
          :outlet-ids="outletIds"
          :split-ratios="splitRatios"
          :resize-disabled="resizeDisabled"
          @activate-tab="(tabsId, childId) => emit('activate-tab', tabsId, childId)"
          @resize-split="(splitId, ratio) => emit('resize-split', splitId, ratio)"
          @focus-widget="emit('focus-widget', $event)"
        />
      </div>
    </div>
  </section>

  <div v-else-if="node.type === 'stack'" class="layout-node layout-stack" :data-node-id="node.id">
    <BlueprintNode
      v-for="child in node.children"
      :key="child.id"
      :node="child"
      :active-tabs="activeTabs"
      :outlet-ids="outletIds"
      :split-ratios="splitRatios"
      :resize-disabled="resizeDisabled"
      @activate-tab="(tabsId, childId) => emit('activate-tab', tabsId, childId)"
      @resize-split="(splitId, ratio) => emit('resize-split', splitId, ratio)"
      @focus-widget="emit('focus-widget', $event)"
    />
  </div>

  <div
    v-else
    :id="outletIds[node.instanceId]"
    class="layout-node layout-widget"
    :data-node-id="node.id"
    :data-widget-instance="node.instanceId"
    @focusin="emit('focus-widget', node.instanceId)"
  ></div>
</template>

<style scoped>
.layout-node { min-width: 0; min-height: 0; }
.layout-split { position: relative; display: grid; gap: var(--space-3); height: 100%; }
.layout-split--horizontal { grid-template-columns: minmax(0, var(--split-leading)) minmax(0, var(--split-trailing)); }
.layout-split--vertical { grid-template-rows: minmax(0, var(--split-leading)) minmax(0, var(--split-trailing)); }
.split-ridge { position: absolute; z-index: 3; display: flex; align-items: center; gap: 3px; padding: 3px; border: 1px solid var(--border-default); border-radius: 999px; background: color-mix(in srgb, var(--bg-surface) 92%, transparent); box-shadow: 0 4px 14px color-mix(in srgb, var(--ink) 12%, transparent); }
.layout-split--horizontal > .split-ridge { left: var(--split-position); top: 50%; transform: translate(-50%, -50%); }
.layout-split--vertical > .split-ridge { left: 50%; top: var(--split-position); transform: translate(-50%, -50%); }
.split-ridge::before { content: ""; position: absolute; z-index: -1; background: color-mix(in srgb, var(--primary) 52%, var(--border-default)); }
.layout-split--horizontal > .split-ridge::before { left: 50%; top: -34px; bottom: -34px; width: 1px; }
.layout-split--vertical > .split-ridge::before { left: -34px; right: -34px; top: 50%; height: 1px; }
.split-ridge button { display: grid; place-items: center; width: 24px; height: 24px; border: 0; border-radius: 50%; color: var(--text-primary); background: var(--bg-subtle); font: 700 14px/1 var(--font-utility); }
.split-ridge button:hover:not(:disabled) { background: var(--primary-tint); color: var(--primary); }
.split-ridge button:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.split-ridge button:disabled { opacity: .4; }
.split-ridge output { min-width: 34px; color: var(--text-secondary); font: 650 9px/1 var(--font-utility); text-align: center; }
.layout-stack { display: flex; flex-direction: column; gap: var(--space-3); min-height: 100%; }
.layout-stack > .layout-node { flex: 1 1 0; }
.layout-tabs { display: grid; grid-template-rows: auto minmax(0, 1fr); height: 100%; overflow: hidden; }
.layout-tabs__list { display: flex; gap: var(--space-1); padding: var(--space-2); border-bottom: 1px solid var(--border-default); }
.layout-tabs__list button { border: 0; border-radius: var(--radius-input); padding: var(--space-1) var(--space-2); color: var(--text-secondary); background: transparent; }
.layout-tabs__list button[aria-selected="true"] { color: var(--text-primary); background: var(--bg-subtle); }
.layout-tabs__list button:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
.layout-tabs__panels, .layout-tabs__panel, .layout-widget { min-height: 0; height: 100%; overflow: hidden; }
.layout-widget { border: 1px solid var(--border-default); border-radius: var(--radius-card); background: var(--bg-surface); }
@media (max-width: 760px) {
  .layout-split--horizontal { grid-template-columns: minmax(0, 1fr); grid-auto-rows: minmax(220px, auto); overflow-y: auto; }
  .layout-split--horizontal > .split-ridge { display: none; }
}
@media (prefers-reduced-motion: reduce) { .split-ridge button { transition: none; } }
</style>
