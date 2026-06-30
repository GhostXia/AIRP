/**
 * Framework-agnostic widget contract.
 *
 * This is the interface AIRP exposes to widget authors. A widget author may use
 * ANY technology (Vue, React, Svelte, Lit, vanilla DOM, a compiled Web
 * Component) — the host only requires that the module conforms to `WidgetModule`.
 *
 * Responsibility boundary (see docs/SECURITY.md): the host secures itself
 * (enforces capabilities, isolates its own secrets, obtains consent, contains
 * errors). It does NOT audit widget code. Installing a widget is the user's
 * choice and risk.
 */

import type { WidgetInstance, Json, Capability } from "../protocol/types";

/** What the host hands a widget at mount time. */
export interface WidgetContext {
  /** This widget instance (id, type, props, requested capabilities). */
  instance: WidgetInstance;
  /** Read the current state slice for this widget's scope. */
  getState(): unknown;
  /** Subscribe to state changes for this scope; returns an unsubscribe fn. */
  onState(cb: (state: unknown) => void): () => void;
  /** Emit an intent upstream (user action). */
  emit(intent: string, params?: Json): void;
  /** Capabilities the host granted this widget (host-enforced). */
  capabilities: Capability[];
}

/** A widget implementation. Build the DOM inside `mount` with any technology. */
export interface WidgetModule {
  mount(el: HTMLElement, ctx: WidgetContext): void | Promise<void>;
  unmount?(): void;
}

/** Default export shape for an esm widget module. */
export type WidgetFactory = () => WidgetModule;
