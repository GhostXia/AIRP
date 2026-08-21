import { describe, expect, it } from "vitest";
import { nextWorkspaceIndex, WORKSPACE_PRESETS } from "./shell-model";

describe("desktop shell model", () => {
  it("keeps the four bounded workspace presets in product order", () => {
    expect(WORKSPACE_PRESETS.map((workspace) => workspace.id)).toEqual(["story", "world", "director", "debug"]);
  });

  it("supports looping arrow navigation and direct Home/End movement", () => {
    expect(nextWorkspaceIndex(0, "ArrowUp")).toBe(3);
    expect(nextWorkspaceIndex(3, "ArrowDown")).toBe(0);
    expect(nextWorkspaceIndex(2, "Home")).toBe(0);
    expect(nextWorkspaceIndex(1, "End")).toBe(3);
    expect(nextWorkspaceIndex(1, "Enter")).toBe(1);
  });
});
