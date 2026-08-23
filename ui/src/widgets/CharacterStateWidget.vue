<script setup lang="ts">
import { computed } from "vue";
const props = defineProps<{ state: unknown }>();
const entries = computed(() => {
  if (!props.state || typeof props.state !== "object" || Array.isArray(props.state)) return [];
  return Object.entries(props.state as Record<string, unknown>);
});
</script>

<template>
  <div class="projection-widget">
    <div class="w-title">角色状态</div>
    <dl v-if="entries.length">
      <template v-for="([key, value]) in entries" :key="key">
        <dt>{{ key }}</dt><dd>{{ typeof value === "string" ? value : JSON.stringify(value) }}</dd>
      </template>
    </dl>
    <p v-else>暂无角色状态</p>
  </div>
</template>

<style scoped>
.projection-widget { height: 100%; overflow: auto; padding: 8px; }
.w-title { margin-bottom: 8px; color: var(--text-secondary); font: 700 10px/1 var(--font-utility); letter-spacing: .08em; text-transform: uppercase; }
dl { display: grid; grid-template-columns: minmax(90px, auto) 1fr; gap: 6px 10px; margin: 0; }
dt { color: var(--text-tertiary); } dd { margin: 0; overflow-wrap: anywhere; } p { color: var(--text-tertiary); }
</style>

