<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Json, WidgetInstance } from "../protocol/types";
import {
  editorIsReadOnly, formatObjectDraft, isAuthoritativeRevision, parseObjectDraft,
  preservesDraftOnAuthorityRefresh, topLevelObjectPatch, type JsonObject,
} from "./editable-state";

const props = defineProps<{
  instance: WidgetInstance;
  state: unknown;
  readOnly?: boolean;
  operation?: unknown;
}>();
const emit = defineEmits<{ (event: "intent", name: string, params?: Json): void }>();

interface CharacterStateSource {
  kind?: unknown;
  scope?: unknown;
  character_id?: unknown;
}

interface CharacterStateProjection {
  revision?: unknown;
  timestamp?: unknown;
  state?: unknown;
  source?: CharacterStateSource;
}

const projection = computed(() => (
  props.state && typeof props.state === "object" && !Array.isArray(props.state)
    ? props.state as CharacterStateProjection
    : {}
));
const authoritativeState = computed<JsonObject>(() => {
  const value = projection.value.state;
  return value && typeof value === "object" && !Array.isArray(value) ? value as JsonObject : {};
});
const authoritativeRevision = computed<number | null>(() => {
  const value = projection.value.revision;
  return isAuthoritativeRevision(value) ? value : null;
});
const source = computed(() => projection.value.source ?? {});
const operation = computed(() => (
  props.operation && typeof props.operation === "object" && !Array.isArray(props.operation)
    ? props.operation as { status?: unknown; error?: unknown }
    : {}
));
const operationStatus = computed(() => typeof operation.value.status === "string" ? operation.value.status : "");

const baselineState = ref<JsonObject>({});
const baselineRevision = ref<number | null>(null);
const baselineDraft = ref("{}");
const draft = ref("{}");
const parsed = computed(() => parseObjectDraft(draft.value));
const dirty = computed(() => draft.value !== baselineDraft.value);
const patch = computed(() => parsed.value.ok ? topLevelObjectPatch(baselineState.value, parsed.value.value) : []);
const saving = computed(() => operationStatus.value === "saving");
const editorReadOnly = computed(() => editorIsReadOnly(props.readOnly, operationStatus.value));
const canSave = computed(() => (
  !props.readOnly && dirty.value && parsed.value.ok && patch.value.length > 0
  && baselineRevision.value !== null && !saving.value
));

function adoptAuthority(preserveDraft: boolean): void {
  baselineState.value = structuredClone(authoritativeState.value);
  baselineRevision.value = authoritativeRevision.value;
  baselineDraft.value = formatObjectDraft(authoritativeState.value);
  if (!preserveDraft) draft.value = baselineDraft.value;
}

watch([authoritativeState, authoritativeRevision], () => {
  if (!dirty.value) adoptAuthority(false);
}, { immediate: true });

watch(operationStatus, (status, previous) => {
  if (status === previous) return;
  if (status === "saved") adoptAuthority(false);
  else if (preservesDraftOnAuthorityRefresh(status)) adoptAuthority(true);
});

function save(): void {
  if (!canSave.value || baselineRevision.value === null) return;
  emit("intent", "characterState.patch", {
    expected_revision: baselineRevision.value,
    patch: patch.value,
  });
}

const timestampText = computed(() => {
  if (projection.value.timestamp === null) return "尚无更新时间";
  if (typeof projection.value.timestamp !== "string") return "—";
  const timestamp = new Date(projection.value.timestamp);
  return Number.isNaN(timestamp.getTime()) ? projection.value.timestamp : timestamp.toLocaleString();
});
const statusText = computed(() => {
  if (saving.value) return "正在应用角色状态 patch…";
  if (operationStatus.value === "conflict") return String(operation.value.error ?? "版本冲突；草稿已保留。");
  if (operationStatus.value === "error") return String(operation.value.error ?? "保存失败；草稿已保留。");
  if (operationStatus.value === "saved" && !dirty.value) return "已保存并刷新权威 Surface。";
  return "";
});
</script>

<template>
  <form class="w-character-state editable-widget" aria-labelledby="character-state-title" @submit.prevent="save">
    <header>
      <div id="character-state-title" class="w-title">角色状态</div>
      <span class="scope-badge">{{ source.scope === "character" ? "角色范围" : "范围未知" }}</span>
    </header>
    <dl class="metadata" aria-label="角色状态来源与版本">
      <div><dt>source</dt><dd>{{ source.kind ?? "—" }}</dd></div>
      <div><dt>character</dt><dd>{{ source.character_id ?? "—" }}</dd></div>
      <div><dt>revision</dt><dd>{{ projection.revision ?? "—" }}</dd></div>
      <div><dt>time</dt><dd>{{ timestampText }}</dd></div>
    </dl>

    <label class="editor-label" :for="`${instance.id}-state-editor`">编辑顶层 JSON object</label>
    <textarea
      :id="`${instance.id}-state-editor`"
      v-model="draft"
      rows="12"
      spellcheck="false"
      :readonly="editorReadOnly"
      :aria-invalid="!parsed.ok"
      :aria-describedby="`${instance.id}-state-help ${instance.id}-state-error ${instance.id}-state-status`"
    ></textarea>
    <p :id="`${instance.id}-state-help`" class="editor-help">
      仅提交顶层字段的 add / replace / remove；冲突时保留这份草稿。
    </p>
    <p :id="`${instance.id}-state-error`" class="parse-error" role="alert">{{ parsed.ok ? "" : parsed.error }}</p>
    <div class="editor-actions">
      <button type="submit" :disabled="!canSave">{{ saving ? "应用中…" : "应用字段变更" }}</button>
      <span v-if="dirty" class="dirty-indicator">{{ parsed.ok ? `${patch.length} 个字段变更` : "草稿未保存" }}</span>
    </div>
    <p
      :id="`${instance.id}-state-status`"
      class="operation-status"
      :class="`operation-status--${operationStatus}`"
      aria-live="polite"
      :role="operationStatus === 'error' || operationStatus === 'conflict' ? 'alert' : 'status'"
    >{{ statusText }}</p>
  </form>
</template>

<style scoped>
.editable-widget { height: 100%; overflow: auto; padding: 12px; }
header { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 10px; }
.w-title { color: var(--text-secondary); font: 700 11px/1.2 var(--font-utility); letter-spacing: .06em; }
.scope-badge { border: 1px solid var(--border-default); border-radius: 999px; padding: 3px 7px; color: var(--text-secondary); font-size: 11px; white-space: nowrap; }
.metadata { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 5px 12px; margin: 0 0 12px; }
.metadata div { min-width: 0; }
dt { color: var(--text-tertiary); font-size: 10px; text-transform: uppercase; }
dd { margin: 1px 0 0; overflow-wrap: anywhere; color: var(--text-secondary); font-size: 12px; }
.editor-label { display: block; margin-bottom: 5px; color: var(--text-secondary); font-size: 12px; font-weight: 650; }
textarea { box-sizing: border-box; width: 100%; resize: vertical; border: 1px solid var(--border-default); border-radius: var(--radius-input); padding: 9px; color: var(--text-primary); background: var(--bg-subtle); font: 12px/1.5 ui-monospace, SFMono-Regular, Consolas, monospace; }
textarea:focus-visible { outline: 2px solid var(--primary); outline-offset: 2px; }
textarea[aria-invalid="true"] { border-color: var(--danger); }
.editor-help { margin: 5px 0 0; color: var(--text-tertiary); font-size: 11px; }
.parse-error { min-height: 1.4em; margin: 3px 0 8px; color: var(--danger); font-size: 11px; }
.editor-actions { display: flex; align-items: center; gap: 10px; }
button { border: 0; border-radius: var(--radius-input); padding: 6px 12px; color: var(--bg-canvas); background: var(--primary); font-weight: 700; }
button:disabled { cursor: not-allowed; opacity: .45; }
.dirty-indicator { color: var(--warning); font-size: 11px; }
.operation-status { min-height: 1.4em; margin: 8px 0 0; font-size: 12px; }
.operation-status--saved { color: var(--success); }
.operation-status--conflict, .operation-status--error { color: var(--danger); }
@media (max-width: 760px) { .metadata { grid-template-columns: minmax(0, 1fr); } }
</style>
