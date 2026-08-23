<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";
import { computeWindow } from "./virtual-window";

const props = defineProps<{ instance: WidgetInstance; state: unknown; readOnly?: boolean }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Msg {
  id: string;
  role: string;
  text: string;
  // #81 W-03: 历史消息的时间戳（ISO 8601，来自 ChatLog.message_timestamps）。
  // 新消息（chat.send 流式 turn）当前无 ts，留 undefined。未来 ChatWidget
  // 想显示时间戳时可直接读此字段。
  ts?: string;
}

// Task 1.2: chat state is `{ messages: {id: Msg}, order: id[] }`. We render
// in `order` sequence, looking each id up in `messages` (O(1)). Virtual scroll
// windows over `order` so 100k logs stay bounded (perf contract).
type ChatState = {
  messages?: Record<string, Msg> | Array<{ role?: unknown; content?: unknown }>;
  order?: string[];
  message_ids?: string[];
  message_timestamps?: Array<string | null>;
};

// Fixed row height for the virtualized window (performance contract: only the
// viewport slice is rendered, so a 100k-message log stays bounded).
const ITEM_H = 72;

const title = computed(() => {
  const p = props.instance.props as unknown as { title?: string } | null;
  return p?.title ?? "对话";
});
const chatState = computed<ChatState>(
  () => (props.state as ChatState | null) ?? {},
);
const projected = computed(() => Array.isArray(chatState.value.messages));
const projectedMessages = computed<Msg[]>(() => {
  if (!Array.isArray(chatState.value.messages)) return [];
  return chatState.value.messages.map((message, index) => ({
    id: chatState.value.message_ids?.[index] ?? `projected-${index}`,
    role: typeof message.role === "string" ? message.role : "unknown",
    text: typeof message.content === "string" ? message.content : "",
    ts: chatState.value.message_timestamps?.[index] ?? undefined,
  }));
});
const messagesById = computed<Record<string, Msg>>(() => projected.value
  ? Object.fromEntries(projectedMessages.value.map((message) => [message.id, message]))
  : chatState.value.messages as Record<string, Msg> ?? {});
const order = computed<string[]>(() => projected.value
  ? projectedMessages.value.map((message) => message.id)
  : chatState.value.order ?? []);

const scrollEl = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportH = ref(0);

const vwin = computed(() =>
  computeWindow({
    scrollTop: scrollTop.value,
    viewportHeight: viewportH.value,
    itemHeight: ITEM_H,
    total: order.value.length,
    overscan: 8,
  }),
);
// Render the viewport slice of `order`, resolving each id to its message.
// Skip ids missing from `messages` (shouldn't happen, but fail soft).
const visible = computed<Msg[]>(() =>
  order.value
    .slice(vwin.value.start, vwin.value.end)
    .map((id) => messagesById.value[id])
    .filter((m): m is Msg => m != null),
);

function onScroll(): void {
  const el = scrollEl.value;
  if (!el) return;
  scrollTop.value = el.scrollTop;
  viewportH.value = el.clientHeight;
  // Near the top → ask the Gateway for an older history window.
  if (!props.readOnly && !projected.value && el.scrollTop < ITEM_H * 2) emit("intent", "chat.loadMore");
}

onMounted(() => {
  if (scrollEl.value) viewportH.value = scrollEl.value.clientHeight;
});

const draft = ref("");

function send(): void {
  if (props.readOnly) return;
  const text = draft.value.trim();
  if (!text) return;
  emit("intent", "chat.send", { text });
  draft.value = "";
}
</script>

<template>
  <div class="w-chat">
    <div class="w-title">{{ title }}</div>
    <div ref="scrollEl" class="w-chat-log" @scroll="onScroll">
      <div class="spacer" :style="{ height: vwin.padTop + 'px' }"></div>
      <div
        v-for="m in visible"
        :key="m.id"
        :class="['msg', m.role]"
        :style="{ height: ITEM_H + 'px' }"
      >
        <span class="role">{{ m.role }}</span>
        <span class="text">{{ m.text }}</span>
      </div>
      <div class="spacer" :style="{ height: vwin.padBottom + 'px' }"></div>
    </div>
    <form class="w-chat-composer" @submit.prevent="send">
      <input v-model="draft" :disabled="readOnly" :placeholder="readOnly ? 'PR6 只读 Surface；发送将在 Chat 纵切开放' : '说点什么…'" />
      <button type="submit" :disabled="readOnly">发送</button>
    </form>
  </div>
</template>

<style scoped>
.w-chat {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}
.w-title { padding: 12px 14px 9px; border-bottom: 1px solid var(--border-default); color: var(--text-secondary); font: 700 10px/1 var(--font-utility); letter-spacing: .08em; text-transform: uppercase; }
.w-chat-log {
  flex: 1;
  overflow-y: auto;
  padding: 8px;
}
.msg {
  display: flex;
  align-items: center;
  gap: 6px;
}
.msg + .msg { border-top: 1px solid color-mix(in srgb, var(--border-default) 62%, transparent); }
.msg .role {
  width: 62px;
  flex: 0 0 62px;
  color: var(--text-tertiary);
  font-size: 12px;
}
.msg .text { min-width: 0; max-height: 58px; overflow: auto; color: var(--text-primary); font-size: 13px; line-height: 1.45; overflow-wrap: anywhere; overscroll-behavior: contain; }
.w-chat-composer {
  display: flex;
  gap: 6px;
  padding: 8px;
  border-top: 1px solid var(--border-default);
  background: var(--bg-subtle);
}
.w-chat-composer input {
  flex: 1;
  min-width: 0;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-input);
  background: var(--bg-surface);
  color: var(--text-primary);
  padding: 8px 10px;
}
.w-chat-composer button { border: 1px solid var(--primary-action); border-radius: var(--radius-input); background: var(--primary-action); color: var(--text-inverse); padding: 0 14px; cursor: pointer; font-weight: 650; }
.w-chat-composer input:focus-visible, .w-chat-composer button:focus-visible { outline: 3px solid color-mix(in srgb, var(--primary) 30%, transparent); outline-offset: 1px; }
</style>
