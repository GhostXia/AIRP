<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Item {
  id: string;
  name: string;
  qty?: number;
  icon?: string;
}

const items = computed<Item[]>(() => (props.state as { items?: Item[] } | null)?.items ?? []);
</script>

<template>
  <div class="w-inventory">
    <div class="w-title">物品栏</div>
    <ul class="grid">
      <li v-for="it in items" :key="it.id">
        <button class="cell" @click="emit('intent', 'inventory.use', { id: it.id })">
          <span class="icon">{{ it.icon ?? "▣" }}</span>
          <span class="name">{{ it.name }}</span>
          <span v-if="it.qty != null" class="qty">×{{ it.qty }}</span>
        </button>
      </li>
      <li v-if="items.length === 0" class="empty">（空）</li>
    </ul>
  </div>
</template>

<style scoped>
.w-inventory {
  padding: 8px;
}
.grid {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.cell {
  display: flex;
  flex-direction: column;
  align-items: center;
  min-width: 56px;
  padding: 6px;
}
.qty {
  font-size: 11px;
  opacity: 0.7;
}
.empty {
  opacity: 0.5;
}
</style>
