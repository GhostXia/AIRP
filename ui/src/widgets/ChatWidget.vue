<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";
import { computeWindow } from "./virtual-window";

const props = defineProps<{ instance: WidgetInstance; state: unknown; readOnly?: boolean; operation?: Json }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Msg {
  id: string;
  role: string;
  text: string;
  // #81 W-03: 历史消息的时间戳（ISO 8601，来自 ChatLog.message_timestamps）。
  // 新消息（chat.send 流式 turn）当前无 ts，留 undefined。未来 ChatWidget
  // 想显示时间戳时可直接读此字段。
  ts?: string;
  candidateIndex?: number;
  candidateCount?: number;
}

// Task 1.2: chat state is `{ messages: {id: Msg}, order: id[] }`. We render
// in `order` sequence, looking each id up in `messages` (O(1)). Virtual scroll
// windows over `order` so 100k logs stay bounded (perf contract).
type ChatState = {
  messages?: Record<string, Msg> | Array<{ role?: unknown; content?: unknown }>;
  order?: string[];
  message_ids?: string[];
  message_timestamps?: Array<string | null>;
  message_candidates?: string[][];
  message_swipe_index?: number[];
  has_more?: boolean;
  oldest_id?: string | null;
  context?: { character_id?: string; session_id?: string };
};

type ChatOperation = {
  status?: string;
  mode?: string;
  body?: string;
  thinking?: string;
  userText?: string;
  error?: string;
  history?: ChatState;
  retryName?: string;
  retryParams?: Json;
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
const operation = computed<ChatOperation>(() => (props.operation as ChatOperation | null) ?? {});
const busy = computed(() => [
  "streaming", "stopping", "submitting", "awaiting_surface", "reconciling", "recovery_required",
].includes(operation.value.status ?? ""));
const projected = computed(() => Array.isArray(chatState.value.messages));
function decodeProjected(state: ChatState): Msg[] {
  if (!Array.isArray(state.messages)) return [];
  return state.messages.map((message, index) => ({
    id: state.message_ids?.[index] ?? `projected-${index}`,
    role: typeof message.role === "string" ? message.role : "unknown",
    text: typeof message.content === "string" ? message.content : "",
    ts: state.message_timestamps?.[index] ?? undefined,
    candidateIndex: state.message_swipe_index?.[index] ?? 0,
    candidateCount: Math.max(1, state.message_candidates?.[index]?.length ?? 0),
  }));
}
const projectedMessages = computed<Msg[]>(() => {
  const older = decodeProjected(operation.value.history ?? {});
  const current = decodeProjected(chatState.value);
  const seen = new Set<string>();
  const durable = [...older, ...current].filter((message) => {
    if (seen.has(message.id)) return false;
    seen.add(message.id);
    return true;
  });
  const transient: Msg[] = [];
  const op = operation.value;
  const last = durable[durable.length - 1];
  if (op.userText && !(last?.role === "user" && last.text === op.userText)) {
    transient.push({ id: "transient-user", role: "user", text: op.userText });
  }
  const bodyAlreadyProjected = last?.role === "assistant"
    && !!op.body
    && (op.mode === "continue" ? last.text.endsWith(op.body) : last.text === op.body);
  if (["streaming", "stopping", "awaiting_surface", "reconciling", "recovery_required", "retryable"].includes(op.status ?? "") && op.body && !bodyAlreadyProjected) {
    transient.push({ id: "transient-assistant", role: "assistant", text: op.body });
  }
  return [...durable, ...transient];
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
let followLatest = true;

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
  followLatest = el.scrollHeight - el.scrollTop - el.clientHeight < ITEM_H * 2;
  // Near the top → ask the Gateway for an older history window.
  if (!props.readOnly && el.scrollTop < ITEM_H * 2 && operation.value.status !== "loading_history") {
    const history = operation.value.history;
    const hasMore = history ? history.has_more : chatState.value.has_more;
    const before = history?.oldest_id ?? chatState.value.oldest_id ?? order.value[0];
    if (hasMore && before) emit("intent", "chat.loadMore", { before, limit: 50 });
  }
}

function scrollToLatest(): void {
  const el = scrollEl.value;
  if (!el) return;
  el.scrollTop = el.scrollHeight;
  scrollTop.value = el.scrollTop;
}

watch(() => order.value.length, () => {
  if (followLatest) void nextTick(scrollToLatest);
});

let resizeObserver: ResizeObserver | null = null;
onMounted(() => {
  if (scrollEl.value) viewportH.value = scrollEl.value.clientHeight;
  resizeObserver = new ResizeObserver(([entry]) => { viewportH.value = entry.contentRect.height; });
  if (scrollEl.value) resizeObserver.observe(scrollEl.value);
  void nextTick(scrollToLatest);
});
onBeforeUnmount(() => resizeObserver?.disconnect());

const draft = ref("");

function send(): void {
  if (props.readOnly || busy.value) return;
  const text = draft.value.trim();
  if (!text) return;
  followLatest = true;
  emit("intent", "chat.send", { text });
  draft.value = "";
  void nextTick(scrollToLatest);
}

function generate(name: "chat.regen" | "chat.continue"): void {
  if (props.readOnly || busy.value) return;
  followLatest = true;
  emit("intent", name);
  void nextTick(scrollToLatest);
}

function swipe(message: Msg, direction: -1 | 1): void {
  if (props.readOnly || busy.value || !message.candidateCount || message.candidateCount < 2) return;
  const next = ((message.candidateIndex ?? 0) + direction + message.candidateCount) % message.candidateCount;
  emit("intent", "chat.swipe", { message_id: message.id, index: next });
}
</script>

<template>
  <div class="w-chat">
    <div class="w-title">
      <span>{{ title }}</span>
      <span v-if="chatState.context?.character_id" class="context-chip">角色 {{ chatState.context.character_id }}</span>
      <span v-if="chatState.context?.session_id" class="context-chip">会话 {{ chatState.context.session_id.slice(0, 8) }}</span>
    </div>
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
        <span v-if="(m.candidateCount ?? 0) > 1" class="swipe-controls">
          <button type="button" :disabled="readOnly || busy" aria-label="上一个候选" @click="swipe(m, -1)">‹</button>
          {{ (m.candidateIndex ?? 0) + 1 }}/{{ m.candidateCount }}
          <button type="button" :disabled="readOnly || busy" aria-label="下一个候选" @click="swipe(m, 1)">›</button>
        </span>
      </div>
      <div class="spacer" :style="{ height: vwin.padBottom + 'px' }"></div>
    </div>
    <div v-if="operation?.thinking" class="generation-note">思考中：{{ operation.thinking }}</div>
    <div v-if="operation?.error" class="generation-error" role="alert">{{ operation.error }}</div>
    <div class="generation-actions">
      <button
        v-if="operation?.status === 'retryable' && operation.retryName"
        type="button"
        :disabled="readOnly"
        @click="emit('intent', operation.retryName, operation.retryParams)"
      >重试未提交操作</button>
      <button type="button" :disabled="readOnly || busy" @click="generate('chat.regen')">重新生成</button>
      <button type="button" :disabled="readOnly || busy" @click="generate('chat.continue')">继续</button>
      <button v-if="operation?.status === 'streaming' || operation?.status === 'stopping'" type="button" @click="emit('intent', 'chat.stop')">停止</button>
    </div>
    <form class="w-chat-composer" @submit.prevent="send">
      <input v-model="draft" :disabled="readOnly || busy" :placeholder="readOnly ? '当前为只读 Surface；发送功能尚未开放' : busy ? '等待当前操作完成…' : '说点什么…'" />
      <button type="submit" :disabled="readOnly || busy">发送</button>
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
.w-title { display: flex; align-items: center; gap: 6px; padding: 12px 14px 9px; border-bottom: 1px solid var(--border-default); color: var(--text-secondary); font: 700 10px/1 var(--font-utility); letter-spacing: .08em; text-transform: uppercase; }
.context-chip { overflow: hidden; max-width: 150px; padding: 3px 6px; border: 1px solid var(--border-default); border-radius: 999px; color: var(--text-tertiary); font-weight: 550; letter-spacing: 0; text-overflow: ellipsis; text-transform: none; white-space: nowrap; }
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
.generation-actions { display: flex; gap: 6px; padding: 6px 8px 0; border-top: 1px solid var(--border-default); }
.generation-actions button, .swipe-controls button { border: 1px solid var(--border-default); border-radius: var(--radius-input); background: var(--bg-surface); color: var(--text-secondary); cursor: pointer; }
.generation-actions button { padding: 5px 9px; }
.generation-actions button:disabled, .swipe-controls button:disabled { cursor: not-allowed; opacity: .45; }
.swipe-controls { display: inline-flex; align-items: center; gap: 3px; margin-left: auto; color: var(--text-tertiary); font-size: 11px; }
.generation-note, .generation-error { max-height: 52px; overflow: auto; padding: 5px 9px; font-size: 11px; }
.generation-note { color: var(--text-tertiary); background: var(--bg-subtle); }
.generation-error { color: var(--danger); background: var(--danger-tint); }
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
