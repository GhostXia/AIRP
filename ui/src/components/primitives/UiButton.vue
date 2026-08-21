<script setup lang="ts">
withDefaults(defineProps<{
  label: string;
  variant?: "quiet" | "primary";
  pressed?: boolean;
}>(), { variant: "quiet", pressed: undefined });
</script>

<template>
  <button
    type="button"
    :class="['ui-button', `ui-button--${variant}`]"
    :aria-label="label"
    :aria-pressed="pressed"
  >
    <slot>{{ label }}</slot>
  </button>
</template>

<style scoped>
.ui-button {
  min-height: 36px;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-input);
  background: var(--bg-surface);
  color: var(--text-primary);
  padding: 0 var(--space-3);
  font: 600 13px/1 var(--font-body);
  cursor: pointer;
}
.ui-button:hover { border-color: color-mix(in srgb, var(--primary) 55%, var(--border-default)); }
.ui-button--primary { background: var(--primary-action); border-color: var(--primary-action); color: var(--text-inverse); }
.ui-button--primary:hover { background: var(--primary-action-hover); }
.ui-button:focus-visible { outline: 3px solid color-mix(in srgb, var(--primary) 32%, transparent); outline-offset: 2px; }
</style>
