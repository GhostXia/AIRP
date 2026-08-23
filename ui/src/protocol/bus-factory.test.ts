import { describe, it, expect } from "vitest";
import { isTauriEnvironment, createMockBus } from "./bus-factory";
import { MockBus } from "./bus";
import type { AgentBus } from "./bus";

describe("bus-factory", () => {
  it("isTauriEnvironment is false in vitest (no __TAURI_INTERNALS__)", () => {
    expect(isTauriEnvironment()).toBe(false);
  });

  it("MockBus requires an explicit demo/test factory call", () => {
    const bus: AgentBus = createMockBus();
    expect(bus).toBeInstanceOf(MockBus);
  });

  it("isTauriEnvironment flips when the sentinel global is set/cleared", () => {
    const g = globalThis as Record<string, unknown>;
    // try/finally so a failing assertion cannot leak the sentinel into later
    // tests — cleanup always runs.
    try {
      expect(isTauriEnvironment()).toBe(false);
      g.__TAURI_INTERNALS__ = {};
      expect(isTauriEnvironment()).toBe(true);
    } finally {
      delete g.__TAURI_INTERNALS__;
    }
    expect(isTauriEnvironment()).toBe(false);
  });

});
