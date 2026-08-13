import { describe, expect, it } from "vitest";
import rustSnapshot from "../../../protocol/fixtures/surface-v2/rust-to-ts.json";
import tsPatch from "../../../protocol/fixtures/surface-v2/ts-to-rust.json";
import migrationFixture from "../../../protocol/fixtures/surface-v2/v1-migration.json";
import type { Blueprint, SurfacePatchEvent, SurfaceSnapshot } from "./types";
import {
  migrateV1Blueprint,
  SurfaceStore,
} from "./surface-v2";
import {
  validateBlueprintV2,
  validateSurfacePatchEventResult,
  validateSurfaceSnapshot,
} from "./guard";

describe("Surface Protocol v2 fixtures and atomic store", () => {
  it("accepts both direction fixtures at their protocol boundary", () => {
    expect(validateSurfaceSnapshot(rustSnapshot)).toBe(true);
    const typedPatch: SurfacePatchEvent = {
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story",
      baseRevision: "42",
      revision: "43",
      patch: [
        {
          op: "add",
          path: "/blueprint/widgets/0/props",
          value: { tone: "warm" },
        },
      ],
    };
    expect(typedPatch).toEqual(tsPatch);
    expect(validateSurfacePatchEventResult(typedPatch)).toEqual({ ok: true });
  });

  it("applies a valid patch and advances the decimal revision", () => {
    const store = new SurfaceStore();
    expect(store.applySnapshot(rustSnapshot)).toMatchObject({ status: "applied" });
    const result = store.applyPatch(tsPatch);
    expect(result).toMatchObject({ status: "applied", snapshot: { revision: "43" } });
    expect(store.snapshot?.blueprint.widgets[0].props).toEqual({ tone: "warm" });
    expect(store.lastKnownGood?.revision).toBe("43");
  });

  it("resyncs on revision mismatch without attempting the patch", () => {
    const store = new SurfaceStore();
    store.applySnapshot(rustSnapshot);
    const result = store.applyPatch({
      ...(tsPatch as unknown as SurfacePatchEvent),
      baseRevision: "41",
      revision: "42",
      patch: [{ op: "replace", path: "/blueprint/root", value: "should-not-apply" }],
    });
    expect(result).toMatchObject({ status: "resync", error: { code: "revision_mismatch" } });
    expect(store.snapshot?.revision).toBe("42");
    expect(store.snapshot?.blueprint.root).toEqual((rustSnapshot as unknown as SurfaceSnapshot).blueprint.root);
  });

  it("keeps the last-known-good snapshot when one operation fails", () => {
    const store = new SurfaceStore();
    store.applySnapshot(rustSnapshot);
    const before = store.lastKnownGood;
    const result = store.applyPatch({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story",
      baseRevision: "42",
      revision: "43",
      patch: [
        { op: "add", path: "/blueprint/widgets/0/props/transient", value: true },
        { op: "test", path: "/blueprint/widgets/0/type", value: "wrong-widget" },
      ],
    });
    expect(result).toMatchObject({ status: "resync", error: { code: "invalid_patch" } });
    expect(store.snapshot).toEqual(before);
    expect(store.lastKnownGood).toEqual(before);
  });

  it("treats object key order as irrelevant for a JSON test operation", () => {
    const store = new SurfaceStore();
    store.applySnapshot(rustSnapshot);
    const result = store.applyPatch({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story",
      baseRevision: "42",
      revision: "43",
      patch: [{
        op: "test",
        path: "/blueprint/root/children/0",
        value: {
          children: [
            { type: "widget", id: "chat-node", instanceId: "chat-1" },
            { type: "widget", id: "tools-node", instanceId: "tools-1" },
          ],
          active: "chat-node",
          id: "tabs",
          type: "tabs",
        },
      }],
    });
    expect(result).toMatchObject({ status: "applied", snapshot: { revision: "43" } });
  });

  it("requires a snapshot before accepting a patch and rejects invalid snapshots", () => {
    const store = new SurfaceStore();
    expect(store.applyPatch(tsPatch)).toMatchObject({ status: "resync", error: { code: "resync_required" } });
    expect(store.applySnapshot({ ...(rustSnapshot as object), protocol: { major: 9, minor: 0 } })).toMatchObject({
      status: "resync",
      error: { code: "unsupported_major" },
    });
    expect(store.snapshot).toBeNull();
  });

  it("uses an explicit deterministic v1 migration boundary", () => {
    const v1 = migrationFixture.v1 as unknown as Blueprint;
    expect(validateBlueprintV2(v1)).toMatchObject({ ok: false, code: "invalid_version" });
    const migrated = migrateV1Blueprint(v1);
    expect(migrated).toEqual(migrationFixture.expectedV2);
    expect(validateBlueprintV2(migrated)).toEqual({ ok: true });
  });
});
