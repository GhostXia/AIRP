import { describe, it, expect, beforeEach } from "vitest";
import type { WidgetModule } from "./widget-module";
import {
  registerEsmWidgetsFromManifests,
  applyManifestMessage,
  getManifest,
  allManifests,
  clearManifests,
} from "./manifests";
import { resolveWidget } from "./registry";

function esmManifest(type: string, source: string) {
  return {
    type,
    version: "1.0.0",
    title: type,
    entry: { kind: "esm" as const, source },
  };
}

describe("manifest registry", () => {
  beforeEach(() => {
    clearManifests();
  });

  it("records manifests and wires their esm widgets", async () => {
    const mod: WidgetModule = { mount() {} };
    registerEsmWidgetsFromManifests(
      [{ type: "acme.graph", version: "1.0.0", title: "关系图", entry: { kind: "esm", source: "https://cdn.example.com/graph.mjs" } }],
      async () => ({ default: () => mod }),
    );

    expect(getManifest("acme.graph")?.title).toBe("关系图");

    const reg = resolveWidget("acme.graph");
    expect(reg?.kind).toBe("module");
    if (reg?.kind === "module") {
      expect(await reg.load()).toBe(mod);
    }
  });

  it("records a builtin manifest without registering a loader", () => {
    registerEsmWidgetsFromManifests([
      { type: "core.sample", version: "0.1.0", title: "Sample", entry: { kind: "builtin" } },
    ]);
    expect(getManifest("core.sample")?.type).toBe("core.sample");
    // builtin manifests don't auto-register an esm loader
    expect(resolveWidget("core.sample")).toBeUndefined();
  });

  it("applyManifestMessage op:set clears the whole set then registers", async () => {
    const modA: WidgetModule = { mount() {} };
    registerEsmWidgetsFromManifests([esmManifest("acme.a", "s-a")], async () => ({ default: () => modA }));
    expect(allManifests().map((m) => m.type)).toEqual(["acme.a"]);

    // A full `set` must wipe the prior manifest before registering the new one.
    const modB: WidgetModule = { mount() {} };
    applyManifestMessage("set", [esmManifest("acme.b", "s-b")], async () => ({ default: () => modB }));

    expect(allManifests().map((m) => m.type)).toEqual(["acme.b"]);
    expect(getManifest("acme.a")).toBeUndefined();
    expect(resolveWidget("acme.a")).toBeUndefined();
    const reg = resolveWidget("acme.b");
    if (reg?.kind === "module") expect(await reg.load()).toBe(modB);
  });

  it("applyManifestMessage op:patch upserts by type without dropping others", async () => {
    const modA: WidgetModule = { mount() {} };
    applyManifestMessage("set", [esmManifest("acme.a", "s-a")], async () => ({ default: () => modA }));

    // An incremental upsert keeps acme.a and adds acme.b.
    const modB: WidgetModule = { mount() {} };
    applyManifestMessage("patch", [esmManifest("acme.b", "s-b")], async () => ({ default: () => modB }));

    expect(allManifests().map((m) => m.type).sort()).toEqual(["acme.a", "acme.b"]);
    expect(resolveWidget("acme.a")?.kind).toBe("module");
    expect(resolveWidget("acme.b")?.kind).toBe("module");
  });
});

