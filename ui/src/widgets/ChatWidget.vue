<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";
import { computeWindow } from "./virtual-window";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Msg {
  id: string;
  role: string;
  text: string;
}

// Fixed row height for the virtualized window (performance contract: only the
// viewport slice is rendered, so a 100k-message log stays bounded).
const ITEM_H = 48;

const title = computed(() => {
  const p = props.instance.props as unknown as { title?: string } | null;
  return p?.title ?? "对话";
});
const messages = computed<Msg[]>(
  () => (props.state as { messages?: Msg[] } | null)?.messages ?? [],
);

const scrollEl = ref<HTMLElement | null>(null);
const scrollTop = ref(0);
const viewportH = ref(0);

const vwin = computed(() =>
  computeWindow({
    scrollTop: scrollTop.value,
    viewportHeight: viewportH.value,
    itemHeight: ITEM_H,
    total: messages.value.length,
    overscan: 8,
  }),
);
const visible = computed(() => messages.value.slice(vwin.value.start, vwin.value.end));

function onScroll(): void {
  const el = scrollEl.value;
  if (!el) return;
  scrollTop.value = el.scrollTop;
  viewportH.value = el.clientHeight;
  // Near the top → ask the Gateway for an older history window.
  if (el.scrollTop < ITEM_H * 2) emit("intent", "chat.loadMore");
}

onMounted(() => {
  if (scrollEl.value) viewportH.value = scrollEl.value.clientHeight;
});

const draft = ref("");

function send(): void {
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
      <input v-model="draft" placeholder="说点什么…" />
      <button type="submit">发送</button>
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
.msg .role {
  opacity: 0.6;
  font-size: 12px;
}
.w-chat-composer {
  display: flex;
  gap: 6px;
  padding: 8px;
}
.w-chat-composer input {
  flex: 1;
}
</style>
