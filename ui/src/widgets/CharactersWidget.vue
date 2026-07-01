<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance } from "../protocol/types";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: unknown): void }>();

interface CharactersState {
  ids?: string[];
  loaded?: boolean;
  error?: string;
}

const state = computed<CharactersState>(
  () => (props.state as CharactersState | null) ?? {},
);
const ids = computed<string[]>(() => state.value.ids ?? []);
const loaded = computed<boolean>(() => state.value.loaded === true);
const error = computed<string | null>(() => state.value.error ?? null);

const title = computed(() => {
  const p = props.instance.props as unknown as { title?: string } | null;
  return p?.title ?? "角色";
});

function select(id: string): void {
  emit("intent", "characters.select", { character_id: id });
}
</script>

<template>
  <div class="w-characters">
    <div class="w-title">{{ title }}</div>
    <div v-if="error" class="err">{{ error }}</div>
    <div v-else-if="!loaded" class="hint">加载中…</div>
    <div v-else-if="ids.length === 0" class="hint">无角色。请先导入角色卡。</div>
    <ul v-else class="list">
      <li v-for="id in ids" :key="id">
        <button type="button" class="item" @click="select(id)">{{ id }}</button>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.w-characters {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  padding: 8px;
  overflow-y: auto;
}
.w-title {
  font-size: 13px;
  opacity: 0.6;
  margin-bottom: 6px;
}
.hint, .err {
  font-size: 12px;
  opacity: 0.6;
  padding: 4px 0;
}
.err { color: #ffb4b4; opacity: 1; }
.list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.item {
  display: block;
  width: 100%;
  text-align: left;
  background: transparent;
  border: 0;
  color: inherit;
  padding: 6px 8px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 13px;
}
.item:hover {
  background: rgba(255, 255, 255, 0.06);
}
.item:focus-visible {
  outline: 2px solid var(--accent, #00e5ff);
  outline-offset: -2px;
}
</style>
