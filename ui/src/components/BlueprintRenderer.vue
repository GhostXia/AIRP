<script setup lang="ts">
import { computed } from "vue";
import type { Blueprint, Json, WidgetInstance } from "../protocol/types";
import WidgetHost from "./WidgetHost.vue";

const props = defineProps<{ blueprint: Blueprint; state: Record<string, Json> }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface ResolvedItem {
  instance: WidgetInstance;
  scope: string;
}
interface ResolvedArea {
  id: string;
  items: ResolvedItem[];
}

// Flatten layout areas into resolved widget instances + their state scope.
const areas = computed<ResolvedArea[]>(() =>
  props.blueprint.layout.areas.map((area) => ({
    id: area.id,
    items: area.widgets
      .map((wid) => props.blueprint.widgets.find((w) => w.id === wid))
      .filter((w): w is WidgetInstance => Boolean(w))
      .map((w) => ({ instance: w, scope: w.state ?? w.id })),
  })),
);

function onIntent(name: string, params?: Json): void {
  emit("intent", name, params);
}
</script>

<template>
  <div class="blueprint" :data-theme="blueprint.theme?.name" :data-layout="blueprint.layout.type">
    <section v-for="area in areas" :key="area.id" :class="['area', `area-${area.id}`]">
      <WidgetHost
        v-for="item in area.items"
        :key="item.instance.id"
        :instance="item.instance"
        :state="state[item.scope] ?? null"
        @intent="onIntent"
      />
    </section>
  </div>
</template>

<style scoped>
.blueprint {
  display: flex;
  gap: 12px;
  height: 100%;
  min-height: 0;
  padding: 12px;
}
.area {
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.area-main {
  flex: 1;
}
.area-sidebar {
  width: 320px;
}
.area-tools {
  width: 260px;
}
</style>
