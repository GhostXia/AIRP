/**
 * Environment detection plus an explicitly named demo-only MockBus factory.
 * Production `/desktop/` uses HttpEngineBus in browsers and Tauri alike.
 */

import type { AgentBus } from "./bus";
import { MockBus } from "./bus";

/** True when running inside a Tauri webview (the shell injects this global). */
export function isTauriEnvironment(): boolean {
  return typeof globalThis !== "undefined"
    && "__TAURI_INTERNALS__" in globalThis;
}

/**
 * Build an explicit fixture bus for tests/demos. It is never selected from
 * environment detection, so a browser cannot silently look connected.
 */
export function createMockBus(): AgentBus {
  return new MockBus();
}
