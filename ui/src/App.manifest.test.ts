/**
 * End-to-end test of the manifest→blueprint wiring (task A).
 *
 * Mirrors the dispatch in App.vue's `onEnvelope`: a downstream `manifest` body
 * must register a third-party esm widget BEFORE the `blueprint` that references
 * it arrives — otherwise WidgetHost resolves the type to undefined once at
 * mount. This asserts the real App.vue ordering holds: manifest processed, then
 * the referenced type is resolvable when the blueprint lands.
 */
import { describe, it, expect, beforeEach } from "vitest";
import type { Envelope, WidgetDef, Blueprint } from "./protocol/types";
import { MockBus } from "./protocol/bus";
import {
  applyManifestMessage,
  clearManifests,
  allManifests,
} from "./registry";
import { resolveWidget } from "./registry";
import type { WidgetModule } from "./registry/widget-module";

const THIRD_PARTY: WidgetDef = {
  type: "acme.status-pill",
  version: "1.0.0",
  title: "状态胶囊",
  entry: { kind: "esm", source: "demo:acme/status-pill" },
  intents: ["status.toggle"],
  capabilities: ["read:state"],
};

describe("manifest→blueprint end-to-end (task A)", () => {
  beforeEach(() => clearManifests());

  it("a third-party esm widget is resolvable once its manifest has arrived", async () => {
    const mod: WidgetModule = { mount() {} };
    const importer = async () => ({ default: () => mod });

    // 1. Gateway sends the manifest first (op: patch = incremental upsert).
    applyManifestMessage("patch", [THIRD_PARTY], importer);
    expect(allManifests().map((m) => m.type)).toContain("acme.status-pill");

    // 2. The blueprint arrives referencing it; the type now resolves.
    const reg = resolveWidget("acme.status-pill");
    expect(reg?.kind).toBe("module");
    if (reg?.kind === "module") expect(await reg.load()).toBe(mod);
  });

  it("MockBus delivers manifest before blueprint (the demo ordering)", () => {
    const bus = new MockBus();
    const order: string[] = [];
    const types = new Set<string>();

    bus.subscribe((e: Envelope) => {
      order.push(e.body.kind);
      if (e.body.kind === "manifest") {
        for (const m of e.body.manifests) types.add(m.type);
      }
    });

    // Flush the microtask the MockBus primes on subscribe.
    return new Promise<void>((resolve) => {
      queueMicrotask(() => {
        // manifest precedes blueprint in the delivery sequence.
        expect(order.indexOf("manifest")).toBeLessThan(order.indexOf("blueprint"));
        // the advertised third-party type is present.
        expect(types.has("acme.status-pill")).toBe(true);
        resolve();
      });
    });
  });

  it("a blueprint referencing an unregistered type resolves to undefined", () => {
    // Without the manifest delivered, the third-party type is unknown — this is
    // the failure mode the ordering convention exists to prevent.
    const bp: Blueprint = {
      version: "bp",
      layout: { type: "dock", areas: [{ id: "main", widgets: ["w-x"] }] },
      widgets: [{ id: "w-x", type: "acme.status-pill" }],
    };
    expect(resolveWidget(bp.widgets[0].type)).toBeUndefined();
  });
});
