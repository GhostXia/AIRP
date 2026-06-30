<script setup lang="ts">
import { computed } from "vue";
import type { WidgetInstance, Json } from "../protocol/types";

const props = defineProps<{ instance: WidgetInstance; state: unknown }>();
const emit = defineEmits<{ (e: "intent", name: string, params?: Json): void }>();

interface Card {
  id: string;
  name: string;
  zone?: "hand" | "deck" | "discard" | "played";
  image?: string;
}

const cards = computed<Card[]>(() => (props.state as { cards?: Card[] } | null)?.cards ?? []);
const hand = computed(() => cards.value.filter((c) => (c.zone ?? "hand") === "hand"));
</script>

<template>
  <div class="w-card">
    <div class="w-title">卡牌（手牌 {{ hand.length }}）</div>
    <div class="hand">
      <button v-for="c in hand" :key="c.id" class="card" @click="emit('intent', 'card.play', { id: c.id })">
        {{ c.name }}
      </button>
      <span v-if="hand.length === 0" class="empty">（无手牌）</span>
    </div>
  </div>
</template>

<style scoped>
.w-card {
  padding: 8px;
}
.hand {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
}
.card {
  min-width: 60px;
  min-height: 84px;
  border-radius: 6px;
  display: flex;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 6px;
}
.empty {
  opacity: 0.5;
}
</style>
