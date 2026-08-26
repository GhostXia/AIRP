<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";
import {
  characterCount, editorIsReadOnly, preservesDraftOnAuthorityRefresh,
} from "./editable-state";

const props = defineProps<{
  instance: WidgetInstance;
  state: unknown;
  readOnly?: boolean;
  operation?: unknown;
}>();
const emit = defineEmits<{ (event: "intent", name: string, params?: Json): void }>();

interface MemorySource {
  kind?: unknown;
  scope?: unknown;
  character_id?: unknown;
  session_id?: unknown;
}

interface MemoryProjection {
  content?: unknown;
  content_hash?: unknown;
  char_count?: unknown;
  capacity_chars?: unknown;
  source?: MemorySource;
}

const projection = computed(() => (
  props.state && typeof props.state === "object" && !Array.isArray(props.state)
    ? props.state as MemoryProjection
    : {}
));
const authoritativeContent = computed(() => (
  typeof projection.value.content === "string" ? projection.value.content : ""
));
const authoritativeHash = computed(() => (
  typeof projection.value.content_hash === "string" ? projection.value.content_hash : ""
));
const capacity = computed(() => (
  typeof projection.value.capacity_chars === "number" ? projection.value.capacity_chars : null
));
const source = computed(() => projection.value.source ?? {});
const operation = computed(() => (
  props.operation && typeof props.operation === "object" && !Array.isArray(props.operation)
    ? props.operation as { status?: unknown; error?: unknown }
    : {}
));
const operationStatus = computed(() => typeof operation.value.status === "string" ? operation.value.status : "");

const draft = ref("");
const baselineContent = ref("");
const baselineHash = ref("");
const dirty = computed(() => draft.value !== baselineContent.value);
const draftChars = computed(() => characterCount(draft.value));
const overCapacity = computed(() => capacity.value !== null && draftChars.value > capacity.value);
const saving = computed(() => operationStatus.value === "saving");
const editorReadOnly = computed(() => editorIsReadOnly(props.readOnly, operationStatus.value));
const canSave = computed(() => (
  !props.readOnly && dirty.value && !saving.value && !overCapacity.value && baselineHash.value.length > 0
));

function adoptAuthority(preserveDraft: boolean): void {
  baselineContent.value = authoritativeContent.value;
  baselineHash.value = authoritativeHash.value;
  if (!preserveDraft) draft.value = authoritativeContent.value;
}

watch([authoritativeContent, authoritativeHash], () => {
  if (!dirty.value) adoptAuthority(false);
}, { immediate: true });

watch(operationStatus, (status, previous) => {
  if (status === previous) return;
  if (status === "saved") adoptAuthority(false);
  else if (preservesDraftOnAuthorityRefresh(status)) adoptAuthority(true);
});

function save(): void {
  if (!canSave.value) return;
  emit("intent", "memory.replace", {
    content: draft.value,
    expected_content_hash: baselineHash.value,
  });
}

const statusText = computed(() => {
  if (saving.value) return "正在保存 resident memory…";
  if (operationStatus.value === "conflict") return String(operation.value.error ?? "版本冲突；草稿已保留。");
  if (operationStatus.value === "error") return String(operation.value.error ?? "保存失败；草稿已保留。");
  if (operationStatus.value === "saved" && !dirty.value) return "已保存并刷新权威 Surface。";
  return "";
});
</script>

<template>
  <form class="w-memory editable-widget" aria-labelledby="memory-title" @submit.prevent="save">
    <header>
      <div>
        <div id="memory-title" class="w-title">未分类 resident memory</div>
        <p class="classification-note">这是会话级原始记忆文本，不代表已核实事实，也不是建议。</p>
      </div>
      <span class="scope-badge">{{ source.scope === "session" ? "会话范围" : "范围未知" }}</span>
    </header>

    <dl class="metadata" aria-label="Resident memory 来源与容量">
      <div><dt>source</dt><dd>{{ source.kind ?? "—" }}</dd></div>
      <div><dt>character</dt><dd>{{ source.character_id ?? "—" }}</dd></div>
      <div><dt>session</dt><dd>{{ source.session_id ?? "—" }}</dd></div>
      <div><dt>容量</dt><dd>{{ draftChars }} / {{ capacity ?? "—" }} 字符</dd></div>
      <div><dt>权威计数</dt><dd>{{ projection.char_count ?? "—" }}</dd></div>
    </dl>

    <label class="editor-label" :for="`${instance.id}-memory-editor`">编辑未分类 resident memory</label>
    <textarea
      :id="`${instance.id}-memory-editor`"
      v-model="draft"
      rows="10"
      :readonly="editorReadOnly"
      :aria-invalid="overCapacity"
      :aria-describedby="`${instance.id}-memory-help ${instance.id}-memory-status`"
    ></textarea>
    <p :id="`${instance.id}-memory-help`" class="editor-help">
      保存使用内容 hash 做并发检查。{{ overCapacity ? "草稿超过容量上限，无法保存。" : "冲突时不会清除草稿。" }}
    </p>
    <div class="editor-actions">
      <button type="submit" :disabled="!canSave">{{ saving ? "保存中…" : "保存" }}</button>
      <span v-if="dirty" class="dirty-indicator">有未保存修改</span>
    </div>
    <p
      :id="`${instance.id}-memory-status`"
      class="operation-status"
      :class="`operation-status--${operationStatus}`"
      aria-live="polite"
      :role="operationStatus === 'error' || operationStatus === 'conflict' ? 'alert' : 'status'"
    >{{ statusText }}</p>
  </form>
</template>

<style scoped>
.editable-widget { height: 100%; overflow: auto; padding: 12px; }
header { display: flex; justify-content: space-between; gap: 12px; }
.w-title { color: var(--text-secondary); font: 700 11px/1.2 var(--font-utility); letter-spacing: .06em; }
.classification-note, .editor-help { margin: 5px 0 10px; color: var(--text-tertiary); font-size: 12px; }
.scope-badge { align-self: flex-start; border: 1px solid var(--border-default); border-radius: 999px; padding: 3px 7px; color: var(--text-secondary); font-size: 11px; white-space: nowrap; }
.metadata { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 5px 12px; margin: 0 0 12px; }
.metadata div { min-width: 0; }
dt { color: var(--text-tertiary); font-size: 10px; text-transform: uppercase; }
dd { margin: 1px 0 0; overflow-wrap: anywhere; color: var(--text-secondary); font-size: 12px; }
.editor-label { display: block; margin-bottom: 5px; color: var(--text-secondary); font-size: 12px; font-weight: 650; }
textarea { box-sizing: border-box; width: 100%; resize: vertical; border: 1px solid var(--border-default); border-radius: var(--radius-input); padding: 9px; color: var(--text-primary); background: var(--bg-subtle); font: 12px/1.55 ui-monospace, SFMono-Regular, Consolas, monospace; }
textarea:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
textarea[aria-invalid="true"] { border-color: var(--danger); }
.editor-actions { display: flex; align-items: center; gap: 10px; }
button { border: 0; border-radius: var(--radius-input); padding: 6px 12px; color: var(--bg-canvas); background: var(--primary); font-weight: 700; }
button:disabled { cursor: not-allowed; opacity: .45; }
.dirty-indicator { color: var(--warning); font-size: 11px; }
.operation-status { min-height: 1.4em; margin: 8px 0 0; font-size: 12px; }
.operation-status--saved { color: var(--success); }
.operation-status--conflict, .operation-status--error { color: var(--danger); }
@media (max-width: 760px) { .metadata { grid-template-columns: minmax(0, 1fr); } }
</style>
