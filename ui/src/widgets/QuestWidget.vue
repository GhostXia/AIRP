<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Quest {
  id: string;
  title: string;
  status: "active" | "done" | "failed";
  steps?: string[];
}

const quests = computed<Quest[]>(() => (props.state as { quests?: Quest[] } | null)?.quests ?? []);
</script>

<template>
  <div class="w-quest">
    <div class="w-title">任务</div>
    <ul class="list">
      <li v-for="q in quests" :key="q.id" :class="['quest', q.status]">
        <button class="row" @click="emit('intent', 'quest.select', { id: q.id })">
          <span class="dot" />
          <span class="title">{{ q.title }}</span>
          <span class="status">{{ q.status }}</span>
        </button>
      </li>
      <li v-if="quests.length === 0" class="empty">（无任务）</li>
    </ul>
  </div>
</template>

<style scoped>
.w-quest {
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
  align-items: center;
  width: 100%;
  padding: 4px 6px;
  text-align: left;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--accent, #00e5ff);
}
.done .dot {
  background: #4caf50;
}
.failed .dot {
  background: #e0668a;
}
.status {
  margin-left: auto;
  font-size: 11px;
  opacity: 0.7;
}
.empty {
  opacity: 0.5;
}
</style>
