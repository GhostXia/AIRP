/**
 * Bus factory — picks the right {@link AgentBus} for the current environment
 * (PLAN task B, ②).
 *
 * Inside the Tauri shell the UI talks to the Rust core (and onward to
 * AIRP-Gateway) via {@link TauriBus}; everywhere else (web preview, vitest,
 * `vite dev` outside Tauri) it falls back to {@link MockBus} so the scaffold
 * renders with no backend.
 *
 * Detection uses the `__TAURI_INTERNALS__` global Tauri injects into the
 * webview. `MockBus` is imported statically (it is the zero-backend fallback
 * every non-Tauri target needs); `TauriBus` + `createTauriTransport` are
 * dynamically imported **only on the Tauri branch** so the bundler can split
 * the Tauri transport (and its `@tauri-apps/api` dynamic import) out of the
 * web/vitest bundle. `@tauri-apps/api` is therefore never loaded outside the
 * shell, keeping the mock path dependency-free.
 */

import type { AgentBus } from "./bus";
import { MockBus } from "./bus";

/** True when running inside a Tauri webview (the shell injects this global). */
export function isTauriEnvironment(): boolean {
  return typeof globalThis !== "undefined"
    && "__TAURI_INTERNALS__" in globalThis;
}

/**
 * Build the bus for this environment. Tauri shell → {@link TauriBus} over the
 * real IPC transport (dynamically imported so it stays out of the web bundle);
 * otherwise → {@link MockBus}.
 */
export async function createBus(): Promise<AgentBus> {
  if (isTauriEnvironment()) {
    const { TauriBus, createTauriTransport } = await import("./tauri-bus");
    const transport = await createTauriTransport();
    return new TauriBus(transport);
  }
  return new MockBus();
}
