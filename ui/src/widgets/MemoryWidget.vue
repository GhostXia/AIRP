<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Entry {
  id: string;
  text: string;
  pinned?: boolean;
}

const entries = computed<Entry[]>(
  () => (props.state as { entries?: Entry[] } | null)?.entries ?? [],
);
</script>

<template>
  <div class="w-memory">
    <div class="w-title">记忆</div>
    <ul class="list">
      <li v-for="e in entries" :key="e.id" :class="{ pinned: e.pinned }">
        <button class="pin" title="pin" @click="emit('intent', 'memory.pin', { id: e.id })">★</button>
        <span class="text">{{ e.text }}</span>
      </li>
      <li v-if="entries.length === 0" class="empty">（暂无记忆）</li>
    </ul>
  </div>
</template>

<style scoped>
.w-memory {
  padding: 8px;
}
.list {
  list-style: none;
  margin: 0;
  padding: 0;
}
.list li {
  display: flex;
  gap: 6px;
  align-items: baseline;
  margin: 4px 0;
}
.pin {
  padding: 0 4px;
  opacity: 0.6;
}
.pinned .pin {
  opacity: 1;
  color: var(--accent, #00e5ff);
}
.empty {
  opacity: 0.5;
}
</style>
