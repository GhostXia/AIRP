<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { Json } from "../protocol/types";

const props = defineProps<{
  state: unknown;
  visible: boolean;
}>();
const emit = defineEmits<{
  (e: "intent", name: string, params?: Json): void;
  (e: "close"): void;
}>();

interface SettingsState {
  loaded?: boolean;
  saving?: boolean;
  error?: string;
  settings?: EngineSettings;
}

interface EngineSettings {
  endpoint?: string;
  api_key_set?: boolean;
  model?: string;
  provider?: string;
}

const state = computed<SettingsState>(
  () => (props.state as SettingsState | null) ?? {},
);
const settings = computed<EngineSettings>(() => state.value.settings ?? {});
const saving = computed<boolean>(() => state.value.saving === true);
const error = computed<string | null>(() => state.value.error ?? null);

// 本地表单副本 — 用户编辑时不直接改 engine state，点保存才提交
const endpoint = ref("");
const apiKey = ref("");
const model = ref("");
const hasExistingKey = computed<boolean>(() => settings.value.api_key_set === true);

// 当 engine settings 更新时，同步到本地表单（仅非空字段）
watch(
  () => state.value.settings,
  (s) => {
    if (s) {
      if (s.endpoint) endpoint.value = s.endpoint;
      if (s.model) model.value = s.model;
      // api_key 是脱敏的（api_key_set bool），不回填实际 key
      // 用户不重新输入则保留空 = 不修改
    }
  },
  { immediate: true },
);

function save(): void {
  // 只传非空字段（空 apiKey = 不修改现有 key）
  const params: Record<string, string> = {};
  if (endpoint.value) params.endpoint = endpoint.value;
  if (model.value) params.model = model.value;
  if (apiKey.value) params.api_key = apiKey.value;
  emit("intent", "settings.update", params as unknown as Json);
}

function refresh(): void {
  emit("intent", "settings.get", {});
}

// modal 打开时自动拉取最新 settings
watch(
  () => props.visible,
  (v) => {
    if (v) refresh();
  },
  { immediate: true },
);
</script>

<template>
  <div v-if="visible" class="modal-overlay" @click.self="emit('close')">
    <div class="modal">
      <div class="modal-head">
        <strong>引擎设置</strong>
        <button class="close-btn" @click="emit('close')">×</button>
      </div>
      <div v-if="error" class="err">{{ error }}</div>
      <div class="form">
        <label>
          <span>API Endpoint</span>
          <input
            v-model="endpoint"
            type="text"
            placeholder="https://api.openai.com"
            :disabled="saving"
          />
        </label>
        <label>
          <span>API Key</span>
          <input
            v-model="apiKey"
            type="password"
            :placeholder="hasExistingKey ? '已设置（留空则不修改）' : 'sk-...'"
            :disabled="saving"
          />
        </label>
        <label>
          <span>Model</span>
          <input
            v-model="model"
            type="text"
            placeholder="gpt-4o"
            :disabled="saving"
          />
        </label>
      </div>
      <div class="modal-foot">
        <button class="refresh-btn" @click="refresh" :disabled="saving">刷新</button>
        <button class="save-btn" @click="save" :disabled="saving">
          {{ saving ? "保存中…" : "保存" }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.modal-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 100;
}
.modal {
  background: #161a23;
  border: 1px solid rgba(255, 255, 255, 0.12);
  border-radius: 10px;
  width: 440px;
  max-width: 90vw;
  padding: 18px 20px;
}
.modal-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 14px;
}
.close-btn {
  background: transparent;
  border: 0;
  color: inherit;
  font-size: 20px;
  cursor: pointer;
  padding: 0 4px;
  opacity: 0.6;
}
.close-btn:hover {
  opacity: 1;
}
.form {
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.form label {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.form span {
  font-size: 12px;
  opacity: 0.6;
}
.form input {
  background: rgba(255, 255, 255, 0.06);
  color: inherit;
  border: 1px solid rgba(255, 255, 255, 0.15);
  border-radius: 6px;
  padding: 8px 10px;
  font-size: 13px;
}
.form input:focus-visible {
  outline: 2px solid var(--accent, #00e5ff);
  outline-offset: -2px;
}
.err {
  font-size: 12px;
  color: #ffb4b4;
  padding: 6px 0;
  margin-bottom: 8px;
}
.modal-foot {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 16px;
}
.save-btn {
  background: rgba(0, 229, 255, 0.12);
  border-color: rgba(0, 229, 255, 0.4);
  color: var(--accent, #00e5ff);
}
.save-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.refresh-btn {
  font-size: 12px;
  opacity: 0.7;
}
</style>
