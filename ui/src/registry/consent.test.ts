import { describe, it, expect, beforeEach } from "vitest";
import type { WidgetDef } from "../protocol/types";
import {
  needsConsent,
  canMount,
  effectiveCapabilities,
  grant,
  revoke,
  isGranted,
  clearGrants,
  initGrants,
  initGrantsFromEngine,
  markEngineUnavailable,
  configureEngineAuthority,
  engineAuthorityState,
  type ConsentStorage,
} from "./consent";

const esm: WidgetDef = {
  type: "acme.x",
  version: "1.0.0",
  title: "X",
  entry: { kind: "esm", source: "https://cdn.example.com/x.mjs" },
  capabilities: ["read:state", "call:tool"],
};
const builtin: WidgetDef = {
  type: "core.chat",
  version: "0.1.0",
  title: "对话",
  entry: { kind: "builtin" },
  capabilities: ["read:state"],
};

/** In-memory storage mock for persistence tests. */
function mockStorage(): ConsentStorage & { dump(): Record<string, string> } {
  const store = new Map<string, string>();
  return {
    getItem: (k) => (store.has(k) ? store.get(k)! : null),
    setItem: (k, v) => { store.set(k, v); },
    removeItem: (k) => { store.delete(k); },
    dump: () => Object.fromEntries(store),
  };
}

describe("consent gate", () => {
  beforeEach(() => clearGrants());

  it("only esm widgets need consent", () => {
    expect(needsConsent(esm)).toBe(true);
    expect(needsConsent(builtin)).toBe(false);
  });

  it("builtin may always mount; esm only after grant", async () => {
    expect(canMount(builtin)).toBe(true);
    expect(canMount(esm)).toBe(false);
    await grant(esm);
    expect(canMount(esm)).toBe(true);
    await revoke(esm);
    expect(canMount(esm)).toBe(false);
  });

  it("withholds capabilities until granted", async () => {
    expect(effectiveCapabilities(esm)).toEqual([]);
    await grant(esm);
    expect(effectiveCapabilities(esm)).toEqual(["read:state", "call:tool"]);
  });

  it("builtin capabilities are available without a grant", () => {
    expect(effectiveCapabilities(builtin)).toEqual(["read:state"]);
    expect(isGranted(builtin)).toBe(false);
  });

  it("a changed source does NOT inherit the old grant", async () => {
    await grant(esm);
    expect(canMount(esm)).toBe(true);
    const moved: WidgetDef = { ...esm, entry: { kind: "esm", source: "https://evil.example.com/x.mjs" } };
    expect(canMount(moved)).toBe(false);
  });

  it("a bumped version does NOT inherit the old grant", async () => {
    await grant(esm);
    const bumped: WidgetDef = { ...esm, version: "1.1.0" };
    expect(canMount(bumped)).toBe(false);
  });
});

describe("consent persistence", () => {
  beforeEach(() => clearGrants());

  it("grant/revoke/clear persist to legacy injectable storage", async () => {
    const s = mockStorage();
    initGrants(s);
    await grant(esm);
    expect(s.dump()["airp:consent-grants"]).toContain(esm.type);
    await revoke(esm);
    expect(s.dump()["airp:consent-grants"]).toBe("[]");
    await grant(esm);
    clearGrants();
    expect(s.dump()["airp:consent-grants"]).toBe("[]");
  });

  it("initGrants restores previously saved grants across a legacy reload", async () => {
    const s = mockStorage();
    initGrants(s);
    await grant(esm);
    expect(s.dump()["airp:consent-grants"]).toContain(esm.type);
    // Simulate a reload: a fresh app instance would call initGrants again with
    // the same storage. We cannot truly reset the in-memory Set without
    // clearGrants (which would also wipe storage), so verify the round-trip
    // directly: the persisted blob contains the grant key, and re-running
    // initGrants on a fresh Set (simulated by a second mock reading the same
    // persisted string) restores it.
    const persisted = s.dump()["airp:consent-grants"];
    expect(persisted).toContain(esm.type);
    // A second storage backed by the same persisted string restores the grant
    // into a clean consent state (clearGrants first to empty memory, then
    // re-seed storage with the persisted blob before re-init).
    clearGrants(); // empties memory + storage
    s.setItem("airp:consent-grants", persisted); // restore the persisted blob
    initGrants(s);
    expect(isGranted(esm)).toBe(true);
    expect(canMount(esm)).toBe(true);
  });

  it("without initGrants, legacy consent is in-memory only", async () => {
    // no initGrants call: grant works but does not touch storage
    await grant(esm);
    expect(canMount(esm)).toBe(true);
    // clearGrants with no storage is a no-op save (does not throw)
    clearGrants();
    expect(canMount(esm)).toBe(false);
  });

  it("corrupted legacy storage data is ignored, starts fresh", async () => {
    const s = mockStorage();
    s.setItem("airp:consent-grants", "{not json");
    initGrants(s);
    expect(isGranted(esm)).toBe(false);
    // subsequent grants still work + persist
    await grant(esm);
    expect(s.dump()["airp:consent-grants"]).toContain(esm.type);
  });

  it("non-array stored data is ignored", () => {
    const s = mockStorage();
    s.setItem("airp:consent-grants", JSON.stringify({ not: "an array" }));
    initGrants(s);
    expect(isGranted(esm)).toBe(false);
  });
});

describe("engine-authoritative consent", () => {
  beforeEach(() => {
    clearGrants();
    configureEngineAuthority(null);
  });

  const digest = "a".repeat(64);
  const engineEsm: WidgetDef = {
    ...esm,
    entry: { kind: "esm", source: `/extensions/${digest}/index.js` },
  };
  const record = (overrides: Partial<{
    id: string;
    type: string;
    version: string;
    source: string | null;
    digest: string;
    enabled: boolean;
    granted_capabilities: string[];
    granted_at: number | null;
  }> = {}) => ({
    id: "ext-1",
    type: engineEsm.type,
    version: engineEsm.version,
    source: engineEsm.entry?.kind === "esm" ? engineEsm.entry.source ?? null : null,
    digest,
    enabled: true,
    granted_capabilities: ["read:state"],
    granted_at: 1,
    ...overrides,
  });

  it("uses a successful empty engine snapshot and ignores localStorage", () => {
    const s = mockStorage();
    initGrants(s);
    return grant(esm).then(() => {
      expect(canMount(esm)).toBe(true);
      expect(initGrantsFromEngine({ grants: [] })).toBe(true);
      expect(engineAuthorityState()).toBe("ready");
      expect(canMount(esm)).toBe(false);
    });
  });

  it("fails closed for unavailable or malformed engine snapshots", () => {
    expect(initGrantsFromEngine({ grants: [record({ digest: "bad" })] })).toBe(false);
    // The malformed case is the missing identity field, not an old local cache.
    expect(initGrantsFromEngine({ grants: [{ id: "ext-1", type: esm.type }] })).toBe(false);
    expect(engineAuthorityState()).toBe("unavailable");
    expect(canMount(esm)).toBe(false);
    markEngineUnavailable();
    expect(canMount(esm)).toBe(false);
  });

  it("matches exact identity and enabled engine records", () => {
    const otherDigest = "b".repeat(64);
    const otherVersion = {
      ...engineEsm,
      version: "0.9.0",
      entry: { kind: "esm" as const, source: `/extensions/${otherDigest}/index.js` },
    };
    initGrantsFromEngine({ grants: [
      record(),
      record({ id: "ext-old", version: otherVersion.version, source: otherVersion.entry.source, digest: otherDigest }),
    ] });
    expect(canMount(engineEsm)).toBe(true);
    expect(effectiveCapabilities(engineEsm)).toEqual(["read:state"]);
    expect(canMount(otherVersion)).toBe(true);
    expect(canMount({ ...engineEsm, version: "1.1.0" })).toBe(false);
    expect(canMount({ ...engineEsm, entry: { kind: "esm", source: `/extensions/${"c".repeat(64)}/index.js` } })).toBe(false);
    expect(canMount({ ...engineEsm, entry: { kind: "esm", source: "https://evil.example/x.mjs" } })).toBe(false);
    expect(canMount({ ...engineEsm, type: "other.widget" })).toBe(false);
    initGrantsFromEngine({ grants: [record({ enabled: false })] });
    expect(canMount(engineEsm)).toBe(false);
  });

  it("updates the mirror only after engine grant mutation succeeds", async () => {
    const calls: unknown[][] = [];
    configureEngineAuthority({
      listGrants: async () => ({ grants: [] }),
      updateGrant: async (id, action, capabilities) => {
        calls.push([id, action, capabilities]);
        return record({
          granted_capabilities: action === "grant" ? ["read:state"] : [],
          granted_at: action === "grant" ? 2 : null,
        });
      },
    });
    initGrantsFromEngine({ grants: [record({ granted_capabilities: [] , granted_at: null })] });
    expect(canMount(engineEsm)).toBe(false);
    await grant(engineEsm);
    expect(calls[0]).toEqual(["ext-1", "grant", ["read:state", "call:tool"]]);
    expect(canMount(engineEsm)).toBe(true);
    await revoke(engineEsm);
    expect(calls[1]).toEqual(["ext-1", "revoke", undefined]);
    expect(canMount(engineEsm)).toBe(false);
  });
});
