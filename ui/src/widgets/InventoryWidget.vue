<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance } from "../protocol/types";
import { inventoryView, projectionTimestamp } from "./emotion-inventory-view";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const view = computed(() => inventoryView(props.state));
const metadata = computed(() => view.value.metadata);
const items = computed(() => view.value.status === "available" ? view.value.value : []);
const statusText = computed(() => view.value.status === "unconfigured" ? "未配置" : "不可用");
const timestampText = computed(() => projectionTimestamp(metadata.value));
</script>

<template>
  <section class="w-inventory" :aria-labelledby="`${instance.id}-inventory-title`">
    <header>
      <div :id="`${instance.id}-inventory-title`" class="w-title">物品栏</div>
      <span class="readonly-badge">只读</span>
    </header>
    <ul v-if="view.status === 'available'" class="grid">
      <li v-for="it in items" :key="it.id">
        <div class="cell">
          <span class="icon">{{ it.icon ?? "▣" }}</span>
          <span class="name">{{ it.name }}</span>
          <span v-if="it.qty != null" class="qty">×{{ it.qty }}</span>
        </div>
      </li>
      <li v-if="items.length === 0" class="empty">暂无物品</li>
    </ul>
    <p v-else class="empty">{{ statusText }}</p>
    <dl class="metadata" aria-label="物品栏来源与版本">
      <div><dt>来源</dt><dd>{{ metadata ? `${metadata.source.kind} · ${metadata.source.scope} · ${metadata.source.characterId}` : "—" }}</dd></div>
      <div><dt>版本</dt><dd>{{ metadata?.revision ?? "—" }}</dd></div>
      <div><dt>时间</dt><dd>{{ timestampText }}</dd></div>
    </dl>
  </section>
</template>

<style scoped>
.w-inventory {
  height: 100%;
  overflow: auto;
  padding: 12px;
}
header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 12px; }
.readonly-badge { border: 1px solid var(--border-default); border-radius: 999px; padding: 3px 7px; color: var(--text-secondary); font-size: 11px; }
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
  border: 1px solid var(--border-default);
  border-radius: var(--radius-input);
  background: var(--bg-subtle);
}
.name { max-width: 120px; overflow-wrap: anywhere; color: var(--text-primary); font-size: 12px; }
.qty {
  font-size: 11px;
  color: var(--text-secondary);
}
.empty {
  margin: 0;
  color: var(--text-secondary);
  font-size: 13px;
}
.metadata { display: grid; grid-template-columns: minmax(0, 1fr) auto; gap: 5px 12px; margin: 14px 0 0; padding-top: 10px; border-top: 1px solid var(--border-default); }
.metadata div:last-child { grid-column: 1 / -1; }
dt { color: var(--text-tertiary); font-size: 10px; }
dd { margin: 1px 0 0; color: var(--text-secondary); font-size: 11px; overflow-wrap: anywhere; }
</style>
