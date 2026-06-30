import { describe, it, expect } from "vitest";
import { isTauriEnvironment, createBus } from "./bus-factory";
import { MockBus } from "./bus";
import type { AgentBus } from "./bus";

describe("bus-factory", () => {
  it("isTauriEnvironment is false in vitest (no __TAURI_INTERNALS__)", () => {
    expect(isTauriEnvironment()).toBe(false);
  });

  it("createBus returns MockBus outside the Tauri shell", async () => {
    const bus: AgentBus = await createBus();
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

  // The Tauri branch of createBus dynamically imports @tauri-apps/api and
  // builds a real IPC transport, which only works inside the shell. Exercising
  // it here would either flake on import or assert against an error — neither
  // proves the live link. The live UI→core→Gateway round-trip is on the
  // runtime verification ledger (PLAN §2.5 B), so we deliberately do NOT unit-
  // test the Tauri branch here; the branch selection above is what CI guards.
});
