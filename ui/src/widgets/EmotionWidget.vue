<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance } from "../protocol/types";

// `instance` is part of the uniform widget prop contract; unused here.
const props = defineProps<{ instance: WidgetInstance; state: unknown }>();

const emotion = computed(() => (props.state as { emotion?: number } | null)?.emotion ?? 0);
const label = computed(() => (props.state as { label?: string } | null)?.label ?? "");
</script>

<template>
  <div class="w-emotion">
    <div class="w-title">情绪</div>
    <div class="gauge"><div class="fill" :style="{ width: emotion + '%' }"></div></div>
    <div class="meta">{{ emotion }} · {{ label }}</div>
  </div>
</template>

<style scoped>
.w-emotion {
  padding: 8px;
}
.gauge {
  height: 10px;
  background: rgba(255, 255, 255, 0.15);
  border-radius: 5px;
  overflow: hidden;
}
.fill {
  height: 100%;
  background: var(--accent, #00e5ff);
  transition: width 0.4s ease;
}
.meta {
  margin-top: 6px;
  font-size: 13px;
  opacity: 0.8;
}
</style>
