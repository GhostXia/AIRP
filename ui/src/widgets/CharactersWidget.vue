<script setup lang="ts">
import { computed, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { WidgetInstance } from "../protocol/types";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: unknown): void }>();

interface CharactersState {
  ids?: string[];
  loaded?: boolean;
  error?: string;
  importing?: boolean;
  last_imported?: string;
}

const state = computed<CharactersState>(
  () => (props.state as CharactersState | null) ?? {},
);
const ids = computed<string[]>(() => state.value.ids ?? []);
const loaded = computed<boolean>(() => state.value.loaded === true);
const error = computed<string | null>(() => state.value.error ?? null);
const importing = computed<boolean>(() => state.value.importing === true);
const lastImported = computed<string | null>(() => state.value.last_imported ?? null);

const title = computed(() => {
  const p = props.instance.props as unknown as { title?: string } | null;
  return p?.title ?? "角色";
});

// 导入失败提示（dialog 取消/IO 错误等），独立于引擎返回的 state.error。
const localError = ref<string | null>(null);

function select(id: string): void {
  emit("intent", "characters.select", { character_id: id });
}

// Q4 主入口：plugin-dialog open 拿绝对路径 → 只发 path 给引擎（守不变式6，
// 大 blob 不进 store/渲染树）。character_id 不传，引擎 slugify 卡名派生。
async function onImport(): Promise<void> {
  localError.value = null;
  try {
    const selected = await open({
      multiple: false,
      // 酒馆卡：PNG（含嵌 JSON）或 .json/png/.card 二进制。directory=false 默认。
      filters: [
        { name: "角色卡", extensions: ["png", "json", "card"] },
      ],
    });
    if (selected === null) return; // 用户取消
    // open(multiple:false) 返回 string | null。
    const cardPath = typeof selected === "string" ? selected : null;
    if (!cardPath) return;
    emit("intent", "characters.import", { card_path: cardPath });
  } catch (e) {
    localError.value = e instanceof Error ? e.message : String(e);
  }
}
</script>

<template>
  <div class="w-characters">
    <div class="w-head">
      <div class="w-title">{{ title }}</div>
      <button
        type="button"
        class="import-btn"
        :disabled="importing"
        @click="onImport"
      >
        {{ importing ? "导入中…" : "导入卡" }}
      </button>
    </div>
    <div v-if="error || localError" class="err">{{ error || localError }}</div>
    <div v-else-if="lastImported && !error" class="ok">已导入：{{ lastImported }}</div>
    <div v-else-if="!loaded" class="hint">加载中…</div>
    <div v-else-if="ids.length === 0" class="hint">无角色。点上方"导入卡"开始。</div>
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
.w-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-bottom: 6px;
}
.w-title {
  font-size: 13px;
  opacity: 0.6;
}
.import-btn {
  background: transparent;
  border: 1px solid rgba(255, 255, 255, 0.2);
  color: inherit;
  padding: 3px 10px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
}
.import-btn:hover:not(:disabled) {
  border-color: var(--accent, #00e5ff);
  color: var(--accent, #00e5ff);
}
.import-btn:focus-visible {
  outline: 2px solid var(--accent, #00e5ff);
  outline-offset: -2px;
}
.import-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.hint, .ok {
  font-size: 12px;
  opacity: 0.6;
  padding: 4px 0;
}
.err {
  font-size: 12px;
  color: #ffb4b4;
  padding: 4px 0;
}
.ok { color: #b4ffb4; opacity: 1; }
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
