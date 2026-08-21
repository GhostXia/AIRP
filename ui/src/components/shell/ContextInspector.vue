<script setup lang="ts">
defineProps<{
  collapsed: boolean;
  workspaceLabel: string;
  surfaceId: string | null;
  revision: string | null;
  focusedWidgetId: string | null;
}>();
const emit = defineEmits<{ (event: "toggle"): void }>();
</script>

<template>
  <aside class="inspector" :class="{ 'inspector--collapsed': collapsed }" aria-label="上下文检查器">
    <button class="inspector__toggle" type="button" :aria-expanded="!collapsed" aria-controls="context-inspector-body" @click="emit('toggle')">
      <span aria-hidden="true">{{ collapsed ? "‹" : "›" }}</span>
      <span class="sr-only">{{ collapsed ? "展开上下文检查器" : "收起上下文检查器" }}</span>
    </button>
    <div v-show="!collapsed" id="context-inspector-body" class="inspector__body">
      <header>
        <span class="eyebrow">当前上下文</span>
        <h2>{{ workspaceLabel }}</h2>
      </header>
      <section aria-labelledby="scene-heading">
        <h3 id="scene-heading">场景</h3>
        <div class="context-card">
          <strong>{{ surfaceId ?? "Surface 不可用" }}</strong>
          <span>revision {{ revision ?? "—" }}</span>
          <span>{{ focusedWidgetId ? `Widget ${focusedWidgetId}` : "尚未聚焦 Widget" }}</span>
          <small>真实 Engine 投影将在 PR 5 接入</small>
        </div>
      </section>
      <section aria-labelledby="activity-heading">
        <div class="section-heading">
          <h3 id="activity-heading">活动</h3>
          <span class="count">1</span>
        </div>
        <div class="activity"><span class="activity__dot" aria-hidden="true"></span><div><strong>Surface ready</strong><small>本地 fixture 已通过 guard</small></div></div>
      </section>
    </div>
  </aside>
</template>

<style scoped>
.inspector { position: relative; min-width: 0; background: var(--bg-surface); border-left: 1px solid var(--border-default); }
.inspector--collapsed { width: 38px; }
.inspector__toggle { position: absolute; top: 12px; left: -15px; z-index: 2; display: grid; place-items: center; width: 28px; height: 28px; border: 1px solid var(--border-default); border-radius: 50%; background: var(--bg-surface); color: var(--text-secondary); cursor: pointer; }
.inspector__toggle:focus-visible { outline: 3px solid color-mix(in srgb, var(--primary) 30%, transparent); }
.inspector__body { height: 100%; overflow: auto; padding: 22px 18px; }
.eyebrow { color: var(--primary-strong); font: 700 10px/1 var(--font-utility); letter-spacing: .12em; text-transform: uppercase; }
h2 { margin: 7px 0 26px; font: 650 22px/1.15 var(--font-display); }
section + section { margin-top: 26px; padding-top: 22px; border-top: 1px solid var(--border-default); }
h3 { margin: 0 0 10px; color: var(--text-secondary); font: 700 11px/1 var(--font-utility); letter-spacing: .08em; text-transform: uppercase; }
.context-card { display: grid; gap: 5px; padding: 12px; border-left: 3px solid var(--primary); background: var(--bg-subtle); }
.context-card strong, .activity strong { font-size: 13px; }
.context-card span { color: var(--text-secondary); font-size: 12px; }
.context-card small, .activity small { color: var(--text-tertiary); font-size: 11px; }
.section-heading { display: flex; justify-content: space-between; }
.count { display: grid; place-items: center; min-width: 20px; height: 20px; border-radius: var(--radius-pill); background: var(--primary-tint); color: var(--primary-strong); font: 700 10px/1 var(--font-utility); }
.activity { display: flex; gap: 9px; align-items: flex-start; }
.activity__dot { width: 8px; height: 8px; margin-top: 3px; border-radius: 50%; background: var(--success); box-shadow: 0 0 0 4px var(--success-tint); }
.activity div { display: grid; gap: 4px; }
</style>
