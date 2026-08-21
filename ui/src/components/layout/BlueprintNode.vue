<script setup lang="ts">
import type { SurfaceLayoutNode } from "../../protocol/types";

const props = defineProps<{
  node: SurfaceLayoutNode;
  activeTabs: Record<string, string>;
  outletIds: Record<string, string>;
}>();

const emit = defineEmits<{
  (event: "activate-tab", tabsId: string, childId: string): void;
  (event: "focus-widget", instanceId: string): void;
}>();

function activeChildId(): string {
  if (props.node.type !== "tabs") return "";
  const local = props.activeTabs[props.node.id];
  return props.node.children.some((child) => child.id === local) ? local : props.node.active;
}

function panelId(tabsId: string, index: number): string {
  return `surface-tab-${encodeURIComponent(tabsId)}-${index}`;
}

function activateFromKeyboard(event: KeyboardEvent, index: number): void {
  if (props.node.type !== "tabs") return;
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
  >
    <BlueprintNode
      v-for="child in node.children"
      :key="child.id"
      :node="child"
      :active-tabs="activeTabs"
      :outlet-ids="outletIds"
      @activate-tab="(tabsId, childId) => emit('activate-tab', tabsId, childId)"
      @focus-widget="emit('focus-widget', $event)"
    />
  </div>

  <section v-else-if="node.type === 'tabs'" class="layout-node layout-tabs" :data-node-id="node.id">
    <div class="layout-tabs__list" role="tablist" :aria-label="`布局页签 ${node.id}`">
      <button
        v-for="(child, index) in node.children"
        :id="`${panelId(node.id, index)}-tab`"
        :key="child.id"
        type="button"
        role="tab"
        :aria-selected="activeChildId() === child.id"
        :aria-controls="panelId(node.id, index)"
        :tabindex="activeChildId() === child.id ? 0 : -1"
        @click="emit('activate-tab', node.id, child.id)"
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
          @activate-tab="(tabsId, childId) => emit('activate-tab', tabsId, childId)"
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
      @activate-tab="(tabsId, childId) => emit('activate-tab', tabsId, childId)"
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
.layout-split { display: grid; gap: var(--space-3); height: 100%; }
.layout-split--horizontal { grid-template-columns: repeat(2, minmax(0, 1fr)); }
.layout-split--vertical { grid-template-rows: repeat(2, minmax(0, 1fr)); }
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
}
</style>
