<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance } from "../protocol/types";
import { emotionView, projectionTimestamp } from "./emotion-inventory-view";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();

const view = computed(() => emotionView(props.state));
const metadata = computed(() => view.value.metadata);
const emotion = computed(() => view.value.status === "available" ? view.value.value.emotion : null);
const label = computed(() => view.value.status === "available" ? view.value.value.label : undefined);
const statusText = computed(() => view.value.status === "unconfigured" ? "未配置" : "不可用");
const timestampText = computed(() => projectionTimestamp(metadata.value));
</script>

<template>
  <section class="w-emotion" :aria-labelledby="`${instance.id}-emotion-title`">
    <header>
      <div :id="`${instance.id}-emotion-title`" class="w-title">情绪</div>
      <span class="readonly-badge">只读</span>
    </header>
    <template v-if="view.status === 'available'">
      <div class="gauge" role="meter" aria-label="情绪值" aria-valuemin="0" aria-valuemax="100" :aria-valuenow="emotion ?? undefined">
        <div class="fill" :style="{ width: `${emotion}%` }"></div>
      </div>
      <div class="value">{{ emotion }} / 100</div>
      <div v-if="label" class="label">{{ label }}</div>
    </template>
    <p v-else class="empty">{{ statusText }}</p>
    <dl class="metadata" aria-label="情绪来源与版本">
      <div><dt>来源</dt><dd>{{ metadata ? `${metadata.source.kind} · ${metadata.source.scope} · ${metadata.source.characterId}` : "—" }}</dd></div>
      <div><dt>版本</dt><dd>{{ metadata?.revision ?? "—" }}</dd></div>
      <div><dt>时间</dt><dd>{{ timestampText }}</dd></div>
    </dl>
  </section>
</template>

<style scoped>
.w-emotion {
  height: 100%;
  overflow: auto;
  min-width: 0;
  padding: min(12px, 8%);
}
header { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 4px; margin-bottom: 12px; }
.readonly-badge { border: 1px solid var(--border-default); border-radius: 999px; padding: 3px 7px; color: var(--text-secondary); font-size: 11px; }
.gauge {
  height: 10px;
  background: var(--bg-subtle);
  border-radius: 5px;
  overflow: hidden;
}
.fill {
  height: 100%;
  background: var(--accent, #00e5ff);
  transition: width 0.4s ease;
}
.value { margin-top: 7px; color: var(--text-primary); font-size: 13px; font-weight: 650; }
.label { margin-top: 3px; color: var(--text-secondary); font-size: 12px; overflow-wrap: anywhere; }
.empty { margin: 0; color: var(--text-secondary); font-size: 13px; }
.metadata { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 5px 12px; margin: 14px 0 0; padding-top: 10px; border-top: 1px solid var(--border-default); }
.metadata div:last-child { grid-column: 1 / -1; }
dt { color: var(--text-tertiary); font-size: 10px; }
dd { margin: 1px 0 0; color: var(--text-secondary); font-size: 11px; overflow-wrap: anywhere; }
@media (prefers-reduced-motion: reduce) { .fill { transition: none; } }
</style>
