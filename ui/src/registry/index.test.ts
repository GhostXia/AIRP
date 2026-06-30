import { describe, it, expect } from "vitest";
import { registerBuiltins, registeredTypes } from "./index";

describe("builtins", () => {
  it("registers every core widget type", () => {
    registerBuiltins();
    const types = registeredTypes();
    for (const t of [
      "core.chat",
      "core.emotion",
      "core.memory",
      "core.inventory",
      "core.quest",
      "core.map",
      "core.card",
      "core.clock",
    ]) {
      expect(types).toContain(t);
    }
  });
});
