<script setup lang="ts">
import { computed } from "vue";
import type { BlueprintV2, Json } from "../protocol/types";
import WidgetHost from "./WidgetHost.vue";
import BlueprintNode from "./layout/BlueprintNode.vue";

const props = defineProps<{
  blueprint: BlueprintV2;
  state: Record<string, Json>;
  stateRevisions: Record<string, number>;
  activeTabs: Record<string, string>;
  authoritativeProps?: boolean;
}>();

const emit = defineEmits<{
  (event: "intent", name: string, params?: Json, instanceId?: string): void;
  (event: "activate-tab", tabsId: string, childId: string): void;
  (event: "focus-widget", instanceId: string): void;
}>();

function collectPlacedWidgets(node: BlueprintV2["root"], ids: Set<string>): void {
  if (node.type === "widget") {
    ids.add(node.instanceId);
    return;
  }
  for (const child of node.children) collectPlacedWidgets(child, ids);
}

function collectOutletIds(
  node: BlueprintV2["root"],
  path: number[],
  outlets: Record<string, string>,
): void {
  if (node.type === "widget") {
    outlets[node.instanceId] = `surface-widget-outlet-${path.length === 0 ? "root" : path.join("-")}`;
    return;
  }
  node.children.forEach((child, index) => collectOutletIds(child, [...path, index], outlets));
}

const placedWidgetIds = computed(() => {
  const ids = new Set<string>();
  collectPlacedWidgets(props.blueprint.root, ids);
  return ids;
});

const placedWidgets = computed(() =>
  props.blueprint.widgets.filter((instance) => placedWidgetIds.value.has(instance.id)),
);

const outletIds = computed<Record<string, string>>(() => {
  const outlets: Record<string, string> = {};
  collectOutletIds(props.blueprint.root, [], outlets);
  return outlets;
});

function widgetState(instanceId: string): Json {
  const projected = props.blueprint.widgets.find((widget) => widget.id === instanceId)?.props;
  if (props.authoritativeProps) return projected ?? null;
  const value = props.state[instanceId];
  return value === undefined ? null : value;
}
</script>

<template>
  <div class="blueprint" data-blueprint-version="2">
    <BlueprintNode
      :node="blueprint.root"
      :active-tabs="activeTabs"
      :outlet-ids="outletIds"
      @activate-tab="(tabsId, childId) => emit('activate-tab', tabsId, childId)"
      @focus-widget="emit('focus-widget', $event)"
    />

    <Teleport
      v-for="instance in placedWidgets"
      :key="instance.id"
      defer
      :to="`#${outletIds[instance.id]}`"
    >
      <WidgetHost
        :key="instance.type"
        :instance="instance"
        :state="widgetState(instance.id)"
        :state-revision="stateRevisions[instance.id] ?? 0"
        :read-only="authoritativeProps"
        @intent="(name, params) => emit('intent', name, params, instance.id)"
      />
    </Teleport>
  </div>
</template>

<style scoped>
.blueprint {
  width: 100%;
  height: 100%;
  min-width: 0;
  min-height: 0;
  padding: var(--space-3);
  overflow: auto;
  overscroll-behavior: contain;
}
</style>
