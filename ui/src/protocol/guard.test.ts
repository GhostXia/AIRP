/**
 * Tests for the runtime envelope guard.
 *
 * Covers: valid envelopes for every kind, and rejection of every class of
 * structural defect the guard is designed to catch. The guard is intentionally
 * lightweight — it checks skeleton shape, not full schema compliance — so these
 * tests exercise the boundary conditions, not exhaustive JSON Schema coverage.
 */

import { describe, it, expect } from "vitest";
import contract from "../../../protocol/wire-discriminants.json";
import authority from "../../../protocol/surface-protocol-v2.json";
import rustSnapshot from "../../../protocol/fixtures/surface-v2/rust-to-ts.json";
import negativeFixtures from "../../../protocol/fixtures/surface-v2/negative.json";
import {
  BODY_KINDS,
  CAPABILITIES,
  ENTRY_KINDS,
  LAYOUT_KINDS,
  PATCH_OPS,
  SET_OR_PATCH,
  SURFACE_ERROR_CODES,
  SURFACE_FORBIDDEN_FIELDS,
  SURFACE_LIMITS,
  validateBlueprintV2,
  validateSurfacePatchEventResult,
  validateSurfaceSnapshot,
  validateSurfaceSnapshotResult,
  validateEnvelope,
} from "./guard";

describe("wire discriminant contract", () => {
  it("matches the shared Rust/UI fixture", () => {
    expect(BODY_KINDS).toEqual(contract.bodyKinds);
    expect(SET_OR_PATCH).toEqual(contract.setOrPatch);
    expect(PATCH_OPS).toEqual(contract.patchOps);
    expect(CAPABILITIES).toEqual(contract.capabilities);
    expect(ENTRY_KINDS).toEqual(contract.entryKinds);
    expect(LAYOUT_KINDS).toEqual(contract.layoutKinds);
  });
});
import type { Envelope, Body, Blueprint, WidgetDef, JsonPatch } from "./types";

function env(body: Body, overrides?: Partial<Envelope>): Envelope {
  return { v: 1, id: "t1", ts: 1000, src: "engine", body, ...overrides };
}

const VALID_BP: Blueprint = {
  version: "bp-1",
  layout: { type: "dock", areas: [{ id: "main", widgets: ["w1"] }] },
  widgets: [{ id: "w1", type: "core.chat" }],
};

const VALID_MANIFEST: WidgetDef = {
  type: "acme.pill",
  version: "1.0.0",
  title: "Pill",
  entry: { kind: "esm", source: "https://acme.dev/pill.js" },
};

describe("validateEnvelope", () => {
  // --- valid envelopes for every kind ---

  it("accepts a valid blueprint op:set", () => {
    expect(validateEnvelope(env({ kind: "blueprint", op: "set", blueprint: VALID_BP }))).toEqual({ ok: true });
  });

  it("accepts a valid blueprint op:patch", () => {
    const patch: JsonPatch = [{ op: "add", path: "/widgets/-", value: { id: "w2", type: "core.emotion" } }];
    expect(validateEnvelope(env({ kind: "blueprint", op: "patch", patch }))).toEqual({ ok: true });
  });

  it("accepts a valid state op:set", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "set", state: { x: 1 } }))).toEqual({ ok: true });
  });

  it("accepts a valid state op:patch", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "patch", patch: [{ op: "replace", path: "/x", value: 2 }] }))).toEqual({ ok: true });
  });

  it("accepts a valid manifest op:set", () => {
    expect(validateEnvelope(env({ kind: "manifest", op: "set", manifests: [VALID_MANIFEST] }))).toEqual({ ok: true });
  });

  it("accepts a valid event", () => {
    expect(validateEnvelope(env({ kind: "event", topic: "toast", data: "hi" }))).toEqual({ ok: true });
  });

  it("accepts a valid error", () => {
    expect(validateEnvelope(env({ kind: "error", code: "E1", message: "fail" }))).toEqual({ ok: true });
  });

  it("accepts a valid intent", () => {
    expect(validateEnvelope(env({ kind: "intent", name: "chat.send", params: { text: "hi" } }))).toEqual({ ok: true });
  });

  it("accepts a valid subscribe", () => {
    expect(validateEnvelope(env({ kind: "subscribe", scopes: ["w1"] }))).toEqual({ ok: true });
  });

  it("accepts a valid hello", () => {
    expect(validateEnvelope(env({ kind: "hello", client: "airp-ui", version: "0.1" }))).toEqual({ ok: true });
  });

  it("accepts a valid ack", () => {
    expect(validateEnvelope(env({ kind: "ack", ref: "t1" }))).toEqual({ ok: true });
  });

  // --- envelope-level rejections ---

  it("rejects non-object", () => {
    expect(validateEnvelope(null).ok).toBe(false);
    expect(validateEnvelope("x").ok).toBe(false);
    expect(validateEnvelope(42).ok).toBe(false);
  });

  it("rejects wrong version", () => {
    expect(validateEnvelope(env({ kind: "ack", ref: "t1" }, { v: 2 as 1 })).ok).toBe(false);
  });

  it("rejects missing id", () => {
    expect(validateEnvelope({ v: 1, id: "", ts: 1000, src: "g", body: { kind: "ack", ref: "t1" } }).ok).toBe(false);
  });

  it("rejects non-finite ts", () => {
    expect(validateEnvelope(env({ kind: "ack", ref: "t1" }, { ts: NaN })).ok).toBe(false);
    expect(validateEnvelope(env({ kind: "ack", ref: "t1" }, { ts: Infinity })).ok).toBe(false);
  });

  it("rejects missing src", () => {
    expect(validateEnvelope({ v: 1, id: "t1", ts: 1000, src: "", body: { kind: "ack", ref: "t1" } }).ok).toBe(false);
  });

  // --- body-level rejections ---

  it("rejects unknown kind", () => {
    expect(validateEnvelope({ v: 1, id: "t1", ts: 1000, src: "g", body: { kind: "bogus" } as unknown as Body }).ok).toBe(false);
  });

  it("rejects blueprint op:set without blueprint", () => {
    expect(validateEnvelope(env({ kind: "blueprint", op: "set" } as unknown as Body)).ok).toBe(false);
  });

  it("rejects blueprint op:patch without patch", () => {
    expect(validateEnvelope(env({ kind: "blueprint", op: "patch" } as unknown as Body)).ok).toBe(false);
  });

  it("rejects blueprint with missing layout", () => {
    const bad = { ...VALID_BP, layout: undefined } as unknown as Blueprint;
    expect(validateEnvelope(env({ kind: "blueprint", op: "set", blueprint: bad })).ok).toBe(false);
  });

  it("rejects state without scope", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "", op: "set", state: 1 } as unknown as Body)).ok).toBe(false);
  });

  it("rejects state op:patch without patch", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "patch" } as unknown as Body)).ok).toBe(false);
  });

  it("rejects manifest with invalid entry", () => {
    const bad: WidgetDef = { ...VALID_MANIFEST, entry: { kind: "bad" as unknown as "esm", source: "x" } };
    expect(validateEnvelope(env({ kind: "manifest", op: "set", manifests: [bad] })).ok).toBe(false);
  });

  it("rejects manifest esm without source", () => {
    const bad = { ...VALID_MANIFEST, entry: { kind: "esm" } } as unknown as WidgetDef;
    expect(validateEnvelope(env({ kind: "manifest", op: "set", manifests: [bad] })).ok).toBe(false);
  });

  it("rejects unknown capability", () => {
    const bad = { ...VALID_MANIFEST, capabilities: ["read:secrets" as unknown as "read:state"] };
    expect(validateEnvelope(env({ kind: "manifest", op: "set", manifests: [bad] })).ok).toBe(false);
  });

  it("rejects event without topic", () => {
    expect(validateEnvelope(env({ kind: "event", topic: "" } as unknown as Body)).ok).toBe(false);
  });

  it("rejects intent without name", () => {
    expect(validateEnvelope(env({ kind: "intent", name: "" } as unknown as Body)).ok).toBe(false);
  });

  it("rejects ack without ref", () => {
    expect(validateEnvelope(env({ kind: "ack", ref: "" } as unknown as Body)).ok).toBe(false);
  });

  // --- patch-level rejections ---

  it("rejects patch with unknown op", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "patch", patch: [{ op: "bogus", path: "/x" }] as unknown as JsonPatch })).ok).toBe(false);
  });

  it("rejects patch op missing path", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "patch", patch: [{ op: "add", path: "" }] as unknown as JsonPatch })).ok).toBe(false);
  });

  it("rejects move without from", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "patch", patch: [{ op: "move", path: "/a" }] as unknown as JsonPatch })).ok).toBe(false);
  });

  it("rejects add without value", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "patch", patch: [{ op: "add", path: "/a" }] as unknown as JsonPatch })).ok).toBe(false);
  });

  // --- blueprint widget instance rejections ---

  it("rejects widget instance without id", () => {
    const bad = { ...VALID_BP, widgets: [{ type: "core.chat" }] as unknown as Blueprint["widgets"] };
    expect(validateEnvelope(env({ kind: "blueprint", op: "set", blueprint: bad })).ok).toBe(false);
  });

  it("rejects unknown layout type", () => {
    const bad = { ...VALID_BP, layout: { type: "circular", areas: [] } } as unknown as Blueprint;
    expect(validateEnvelope(env({ kind: "blueprint", op: "set", blueprint: bad })).ok).toBe(false);
  });

  // --- state null is valid JSON ---

  it("accepts state op:set with null state", () => {
    expect(validateEnvelope(env({ kind: "state", scope: "w1", op: "set", state: null }))).toEqual({ ok: true });
  });
});

describe("Surface Protocol v2 guard", () => {
  const snapshot = rustSnapshot as unknown as Record<string, unknown>;

  it("binds the runtime limits, error codes, and forbidden fields to the authority", () => {
    expect(SURFACE_LIMITS).toMatchObject(authority.resourceLimits);
    expect([...SURFACE_ERROR_CODES]).toEqual(authority.errors.map((entry) => entry.code));
    expect([...SURFACE_FORBIDDEN_FIELDS]).toEqual(authority.unknownFields.forbiddenFields);
  });

  it("accepts the Rust-to-TS snapshot fixture and opaque additive fields", () => {
    expect(validateSurfaceSnapshot(snapshot)).toBe(true);
    const additive = {
      ...snapshot,
      additiveMetadata: { provider: "fixture", ignored: true },
      blueprint: {
        ...(snapshot.blueprint as Record<string, unknown>),
        futureBlueprintField: "opaque",
        root: {
          ...((snapshot.blueprint as Record<string, unknown>).root as Record<string, unknown>),
          futureNodeField: { preservedByCaller: true },
        },
      },
    };
    expect(validateSurfaceSnapshot(additive)).toBe(true);
  });

  it("rejects unknown major versions but accepts a newer additive minor", () => {
    expect(validateSurfaceSnapshot(negativeFixtures.unknownMajor)).toBe(false);
    const newerMinor = { ...snapshot, protocol: { major: 2, minor: 99 }, future: "opaque" };
    expect(validateSurfaceSnapshot(newerMinor)).toBe(true);
  });

  it("rejects v1 shape, duplicate instance placement, invalid references, and executable fields", () => {
    expect(validateBlueprintV2(VALID_BP)).toMatchObject({ ok: false, code: "invalid_version" });
    expect(validateSurfaceSnapshot(negativeFixtures.duplicateInstance)).toBe(false);
    expect(validateSurfaceSnapshot(negativeFixtures.invalidReference)).toBe(false);
    expect(validateSurfaceSnapshot(negativeFixtures.forbiddenExecutableField)).toBe(false);
    expect(validateSurfaceSnapshotResult(negativeFixtures.forbiddenExecutableField)).toMatchObject({
      ok: false,
      code: "forbidden_executable_field",
    });
  });

  it("rejects depth, node, document, and patch resource limit violations", () => {
    let deepNode: Record<string, unknown> = { type: "widget", id: "leaf", instanceId: "w-1" };
    for (let depth = 0; depth < SURFACE_LIMITS.maxBlueprintDepth; depth += 1) {
      deepNode = { type: "stack", id: `deep-${depth}`, children: [deepNode] };
    }
    const deep = {
      ...snapshot,
      blueprint: { version: 2, root: deepNode, widgets: [{ id: "w-1", type: "core.chat" }] },
    };
    expect(validateSurfaceSnapshotResult(deep)).toMatchObject({ ok: false, code: "resource_limit" });

    let nextWidget = 0;
    const widgets = Array.from({ length: 128 }, () => ({ id: `w-${nextWidget++}`, type: "core.chat" }));
    let nodeCounter = 0;
    let leafWidget = 0;
    const makeGroup = (count: number): Record<string, unknown> => ({
      type: "stack",
      id: `group-${nodeCounter++}`,
      children: Array.from({ length: count }, () => {
        const widgetIndex = leafWidget++;
        let chain: Record<string, unknown> = {
          type: "widget",
          id: `node-${nodeCounter++}`,
          instanceId: `w-${widgetIndex}`,
        };
        for (let level = 0; level < 8; level += 1) {
          chain = { type: "stack", id: `chain-${nodeCounter++}`, children: [chain] };
        }
        return chain;
      }),
    });
    const tooManyNodes = {
      ...snapshot,
      blueprint: {
        version: 2,
        root: {
          type: "stack",
          id: "node-limit-root",
          children: [makeGroup(32), makeGroup(32), makeGroup(32), makeGroup(32)],
        },
        widgets,
      },
    };
    expect(validateSurfaceSnapshotResult(tooManyNodes)).toMatchObject({ ok: false, code: "resource_limit" });

    const tooLarge = { ...snapshot, opaque: "x".repeat(SURFACE_LIMITS.maxDocumentBytes) };
    expect(validateSurfaceSnapshotResult(tooLarge)).toMatchObject({ ok: false, code: "document_too_large" });

    const tooManyOps = {
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story",
      baseRevision: "42",
      revision: "43",
      patch: Array.from({ length: SURFACE_LIMITS.maxPatchOperations + 1 }, () => ({
        op: "test",
        path: "/surfaceId",
        value: "story",
      })),
    };
    expect(validateSurfacePatchEventResult(tooManyOps)).toMatchObject({ ok: false, code: "resource_limit" });
  });

  it("validates the decimal u64 boundary and rejects malformed patch operations", () => {
    const maxRevision = {
      ...snapshot,
      revision: "18446744073709551615",
    };
    expect(validateSurfaceSnapshot(maxRevision)).toBe(true);
    expect(validateSurfaceSnapshot({ ...snapshot, revision: "18446744073709551616" })).toBe(false);
    expect(validateSurfacePatchEventResult(negativeFixtures.revisionGap)).toMatchObject({ ok: false, code: "revision_gap" });
    expect(validateSurfacePatchEventResult(negativeFixtures.badPatch)).toMatchObject({ ok: false, code: "invalid_patch" });
    expect(validateSurfacePatchEventResult(negativeFixtures.invalidPointerEscape)).toMatchObject({ ok: false, code: "invalid_patch" });
    expect(validateSurfacePatchEventResult({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story",
      baseRevision: "42",
      revision: "43",
      patch: [{ op: "replace", path: "/blueprint/root", value: "not-a-node" }],
    })).toMatchObject({ ok: true });
    expect(validateSurfacePatchEventResult({
      kind: "patch",
      protocol: { major: 2, minor: 0 },
      surfaceId: "story",
      baseRevision: "42",
      revision: "43",
      patch: [{ op: "move", path: "/blueprint/root", from: "/revision" }],
    })).toMatchObject({ ok: false, code: "invalid_patch" });
  });
});
