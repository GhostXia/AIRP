import { describe, expect, it } from "vitest";
import type { WidgetDef } from "../protocol/types";
import memoryManifest from "../../widgets/core/memory.json";
import characterStateManifest from "../../widgets/core/character-state.json";

function manifest(name: string): WidgetDef {
  return (name === "memory" ? memoryManifest : characterStateManifest) as unknown as WidgetDef;
}

describe("editable builtin manifests", () => {
  it("declares resident memory CAS without legacy fake intents", () => {
    const memory = manifest("memory");
    expect(memory.title).toContain("未分类");
    expect(memory.intents).toEqual(["memory.replace"]);
    expect(memory.capabilities).toEqual(["read:memory", "write:memory"]);
    expect(JSON.stringify(memory)).not.toMatch(/memory\.(pin|delete)/);
  });

  it("declares revisioned character state patching", () => {
    const characterState = manifest("character-state");
    expect(characterState.intents).toEqual(["characterState.patch"]);
    expect(characterState.capabilities).toEqual(["read:state", "write:state"]);
    expect(characterStateManifest.stateSchema.properties.revision).toEqual({
      type: "integer", minimum: 0,
    });
  });
});
