<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Location {
  id: string;
  name: string;
  visited?: boolean;
}

const slice = computed(() => props.state as { current?: string; locations?: Location[] } | null);
const current = computed(() => slice.value?.current ?? "");
const locations = computed<Location[]>(() => slice.value?.locations ?? []);
</script>

<template>
  <div class="w-map">
    <div class="w-title">地图</div>
    <ul class="list">
      <li v-for="loc in locations" :key="loc.id" :class="{ current: loc.id === current }">
        <button class="row" @click="emit('intent', 'map.travel', { id: loc.id })">
          <span class="name">{{ loc.name }}</span>
          <span v-if="loc.id === current" class="here">在此</span>
          <span v-else-if="loc.visited" class="seen">去过</span>
        </button>
      </li>
      <li v-if="locations.length === 0" class="empty">（无地点）</li>
    </ul>
  </div>
</template>

<style scoped>
.w-map {
  padding: 8px;
}
.list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.row {
  display: flex;
  gap: 8px;
  width: 100%;
  padding: 4px 6px;
  text-align: left;
}
.current .name {
  color: var(--accent, #00e5ff);
}
.here,
.seen {
  margin-left: auto;
  font-size: 11px;
  opacity: 0.7;
}
.empty {
  opacity: 0.5;
}
</style>
