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
  flex-wrap: wrap;
  gap: 12px;
  height: 100%;
  width: 100%;
  min-width: 0;
  min-height: 0;
  padding: 12px;
  overflow: auto;
  overscroll-behavior: contain;
}
.area {
  background: rgba(255, 255, 255, 0.04);
  border: 1px solid rgba(255, 255, 255, 0.08);
  border-radius: 8px;
  min-width: 0;
  min-height: 0;
  display: flex;
  flex-direction: column;
  overflow: auto;
  overscroll-behavior: contain;
}
.area-main {
  flex: 1 1 0;
  min-width: 0;
}
.area-sidebar {
  width: min(320px, 100%);
  flex: 0 1 320px;
}
.area-tools {
  width: min(260px, 100%);
  flex: 0 1 260px;
}
@media (max-width: 900px) {
  .blueprint {
    flex-direction: column;
    flex-wrap: nowrap;
    overflow-x: hidden;
    overflow-y: auto;
  }
  .area-main,
  .area-sidebar,
  .area-tools {
    width: 100%;
    max-width: 100%;
  }
  .area-main {
    flex: 0 0 clamp(220px, 62dvh, 560px);
  }
  .area-sidebar,
  .area-tools {
    flex: 0 0 clamp(140px, 28dvh, 240px);
  }
}
</style>
