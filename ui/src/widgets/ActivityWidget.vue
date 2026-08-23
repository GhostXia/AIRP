<script setup lang="ts">
import { computed } from "vue";
const props = defineProps<{ state: unknown }>();
const view = computed(() => props.state && typeof props.state === "object"
  ? JSON.stringify(props.state, null, 2)
  : "暂无活动");
</script>

<template>
  <div class="activity-widget">
    <div class="w-title">活动</div>
    <pre>{{ view }}</pre>
  </div>
</template>

<style scoped>
.activity-widget { height: 100%; overflow: auto; padding: 8px; }
.w-title { margin-bottom: 8px; color: var(--text-secondary); font: 700 10px/1 var(--font-utility); letter-spacing: .08em; text-transform: uppercase; }
pre { margin: 0; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--text-secondary); font: 11px/1.45 var(--font-utility); }
</style>

