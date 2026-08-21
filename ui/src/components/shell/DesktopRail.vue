<script setup lang="ts">
import { nextTick } from "vue";
import type { WorkspaceId, WorkspacePreset } from "../../shell-model";
import { nextWorkspaceIndex } from "../../shell-model";

const props = defineProps<{ workspaces: readonly WorkspacePreset[]; active: WorkspaceId; compact: boolean }>();
const emit = defineEmits<{
  (event: "select", id: WorkspaceId): void;
  (event: "toggle-focus"): void;
}>();

async function navigate(event: KeyboardEvent, index: number): Promise<void> {
  const target = nextWorkspaceIndex(index, event.key, props.workspaces.length);
  if (target === index) return;
  event.preventDefault();
  emit("select", props.workspaces[target].id);
  await nextTick();
  document.querySelector<HTMLElement>(`[data-workspace-index="${target}"]`)?.focus();
}
</script>

<template>
  <aside class="rail" aria-label="工作区导航">
    <div class="rail__brand" aria-label="AIRP">
      <span class="rail__mark" aria-hidden="true">A</span>
      <span v-if="!compact" class="rail__wordmark">AIRP</span>
    </div>
    <nav class="rail__nav" aria-label="工作区">
      <button
        v-for="(workspace, index) in workspaces"
        :key="workspace.id"
        type="button"
        class="rail__item"
        :class="{ 'rail__item--active': workspace.id === active }"
        :aria-current="workspace.id === active ? 'page' : undefined"
        :aria-label="`${workspace.label}：${workspace.description}`"
        :tabindex="workspace.id === active ? 0 : -1"
        :data-workspace-index="index"
        @click="emit('select', workspace.id)"
        @keydown="navigate($event, index)"
      >
        <span class="rail__glyph" aria-hidden="true">{{ workspace.shortLabel }}</span>
        <span v-if="!compact" class="rail__copy">
          <strong>{{ workspace.label }}</strong>
          <small>{{ workspace.description }}</small>
        </span>
      </button>
    </nav>
    <button type="button" class="rail__focus" :aria-pressed="compact" @click="emit('toggle-focus')">
      <span aria-hidden="true">◐</span>
      <span v-if="!compact">专注模式</span>
    </button>
  </aside>
</template>

<style scoped>
.rail { display: flex; flex-direction: column; min-width: 0; height: 100%; background: var(--bg-surface); border-right: 1px solid var(--border-default); }
.rail__brand { height: var(--desktop-topbar-h); display: flex; align-items: center; gap: 10px; padding: 0 14px; border-bottom: 1px solid var(--border-default); }
.rail__mark { display: grid; place-items: center; width: 28px; height: 28px; flex: 0 0 28px; border-radius: 9px 9px 9px 2px; background: var(--ink); color: var(--text-inverse); font: 700 15px/1 var(--font-utility); }
.rail__wordmark { font: 700 15px/1 var(--font-utility); letter-spacing: .16em; }
.rail__nav { display: grid; gap: 4px; padding: 12px 8px; }
.rail__item { position: relative; display: flex; align-items: center; gap: 11px; width: 100%; min-height: 52px; padding: 7px 8px; border: 0; border-radius: 8px; background: transparent; color: var(--text-secondary); text-align: left; cursor: pointer; }
.rail__item::before { content: ""; position: absolute; left: -8px; width: 3px; height: 0; border-radius: 0 3px 3px 0; background: var(--primary); transition: height 160ms ease; }
.rail__item:hover { background: var(--bg-subtle); color: var(--text-primary); }
.rail__item--active { background: var(--primary-tint); color: var(--primary-strong); }
.rail__item--active::before { height: 30px; }
.rail__item:focus-visible, .rail__focus:focus-visible { outline: 3px solid color-mix(in srgb, var(--primary) 30%, transparent); outline-offset: 1px; }
.rail__glyph { display: grid; place-items: center; width: 30px; height: 30px; flex: 0 0 30px; border: 1px solid currentColor; border-radius: 50%; font: 700 11px/1 var(--font-utility); }
.rail__copy { display: grid; gap: 3px; min-width: 0; }
.rail__copy strong { font-size: 13px; color: inherit; }
.rail__copy small { overflow: hidden; color: var(--text-tertiary); font-size: 10px; text-overflow: ellipsis; white-space: nowrap; }
.rail__focus { display: flex; align-items: center; justify-content: center; gap: 8px; min-height: 42px; margin: auto 8px 10px; border: 1px solid var(--border-default); border-radius: 8px; background: transparent; color: var(--text-secondary); cursor: pointer; font: 600 12px/1 var(--font-body); }
@media (prefers-reduced-motion: reduce) { .rail__item::before { transition: none; } }
</style>
