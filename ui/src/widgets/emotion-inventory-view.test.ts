import { describe, expect, it } from "vitest";
import emotionManifest from "../../widgets/core/emotion.json";
import inventoryManifest from "../../widgets/core/inventory.json";
import { emotionView, inventoryView, MAX_INVENTORY_ITEMS } from "./emotion-inventory-view";

const metadata = {
  revision: 7,
  timestamp: "2026-08-30T00:00:00Z",
  source: { kind: "character_state", scope: "character", character_id: "alice" },
};

describe("emotion and inventory projection views", () => {
  it("does not turn missing or malformed emotion into zero", () => {
    expect(emotionView(null)).toEqual({ status: "unavailable", metadata: null });
    expect(emotionView({ ...metadata, available: true })).toMatchObject({ status: "unavailable" });
    expect(emotionView({ ...metadata, available: true, emotion: -1 })).toMatchObject({ status: "unavailable" });
    expect(emotionView({ ...metadata, available: true, emotion: 0 })).toMatchObject({
      status: "available", value: { emotion: 0 },
    });
  });

  it("distinguishes an unconfigured projection and preserves its provenance", () => {
    expect(emotionView({ ...metadata, available: false, reason: "missing" })).toEqual({
      status: "unconfigured",
      metadata: {
        revision: 7,
        timestamp: "2026-08-30T00:00:00Z",
        source: { kind: "character_state", scope: "character", characterId: "alice" },
      },
    });
    expect(inventoryView({ ...metadata, available: false, reason: "missing" })).toMatchObject({
      status: "unconfigured", metadata: { revision: 7 },
    });
    expect(inventoryView({ ...metadata, available: false, reason: "invalid" })).toMatchObject({
      status: "unavailable",
    });
  });

  it("accepts a valid empty inventory but rejects missing, oversized, or invalid items", () => {
    expect(inventoryView({ ...metadata, available: true, items: [] })).toMatchObject({
      status: "available", value: [],
    });
    expect(inventoryView({ ...metadata, available: true })).toMatchObject({ status: "unavailable" });
    expect(inventoryView({
      ...metadata,
      available: true,
      items: Array.from({ length: MAX_INVENTORY_ITEMS + 1 }, (_, index) => ({ id: `${index}`, name: "item" })),
    })).toMatchObject({ status: "unavailable" });
    expect(inventoryView({ ...metadata, available: true, items: [{ id: "tea", name: "Tea", qty: -1 }] }))
      .toMatchObject({ status: "unavailable" });
  });

  it("rejects malformed provenance and duplicate inventory ids", () => {
    expect(emotionView({
      ...metadata,
      available: true,
      emotion: 50,
      source: { ...metadata.source, scope: "session" },
    })).toEqual({ status: "unavailable", metadata: null });
    expect(inventoryView({
      ...metadata,
      available: true,
      items: [{ id: "same", name: "One" }, { id: "same", name: "Two" }],
    })).toMatchObject({ status: "unavailable" });
  });
});

describe("emotion and inventory manifests", () => {
  it("declare bounded revisioned read-only projections", () => {
    expect(emotionManifest.capabilities).toEqual(["read:state"]);
    expect(inventoryManifest.capabilities).toEqual(["read:state"]);
    expect("intents" in inventoryManifest).toBe(false);
    expect(inventoryManifest.stateSchema.properties.items.maxItems).toBe(MAX_INVENTORY_ITEMS);
    expect(emotionManifest.stateSchema.required).toEqual(["available", "revision", "timestamp", "source"]);
    expect(inventoryManifest.stateSchema.required).toEqual(["available", "revision", "timestamp", "source"]);
    expect(JSON.stringify(inventoryManifest)).not.toMatch(/inventory\.(use|drop)/);
  });
});
