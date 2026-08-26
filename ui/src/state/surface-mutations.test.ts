import { describe, expect, it } from "vitest";
import {
  NON_STREAMING_SURFACE_MUTATIONS, WRITABLE_SURFACE_WIDGET_TYPES, isNonStreamingSurfaceMutation,
} from "./surface-mutations";

describe("production Surface mutations", () => {
  it("opens exactly chat, memory, and character state for writes", () => {
    expect(WRITABLE_SURFACE_WIDGET_TYPES).toEqual([
      "core.chat", "core.memory", "core.character-state",
    ]);
  });

  it("recognizes the two non-streaming state mutation contracts", () => {
    expect(NON_STREAMING_SURFACE_MUTATIONS).toEqual(["memory.replace", "characterState.patch"]);
    expect(isNonStreamingSurfaceMutation("memory.replace")).toBe(true);
    expect(isNonStreamingSurfaceMutation("characterState.patch")).toBe(true);
    expect(isNonStreamingSurfaceMutation("characterState.replace")).toBe(false);
  });
});
