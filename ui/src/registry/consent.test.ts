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

  it("builtin may always mount; esm only after grant", () => {
    expect(canMount(builtin)).toBe(true);
    expect(canMount(esm)).toBe(false);
    grant(esm);
    expect(canMount(esm)).toBe(true);
    revoke(esm);
    expect(canMount(esm)).toBe(false);
  });

  it("withholds capabilities until granted", () => {
    expect(effectiveCapabilities(esm)).toEqual([]);
    grant(esm);
    expect(effectiveCapabilities(esm)).toEqual(["read:state", "call:tool"]);
  });

  it("builtin capabilities are available without a grant", () => {
    expect(effectiveCapabilities(builtin)).toEqual(["read:state"]);
    expect(isGranted(builtin)).toBe(false);
  });

  it("a changed source does NOT inherit the old grant", () => {
    grant(esm);
    expect(canMount(esm)).toBe(true);
    const moved: WidgetDef = { ...esm, entry: { kind: "esm", source: "https://evil.example.com/x.mjs" } };
    expect(canMount(moved)).toBe(false);
  });

  it("a bumped version does NOT inherit the old grant", () => {
    grant(esm);
    const bumped: WidgetDef = { ...esm, version: "1.1.0" };
    expect(canMount(bumped)).toBe(false);
  });
});

describe("consent persistence", () => {
  beforeEach(() => clearGrants());

  it("grant/revoke/clear persist to storage", () => {
    const s = mockStorage();
    initGrants(s);
    grant(esm);
    expect(s.dump()["airp:consent-grants"]).toContain(esm.type);
    revoke(esm);
    expect(s.dump()["airp:consent-grants"]).toBe("[]");
    grant(esm);
    clearGrants();
    expect(s.dump()["airp:consent-grants"]).toBe("[]");
  });

  it("initGrants restores previously saved grants across a reload", () => {
    const s = mockStorage();
    initGrants(s);
    grant(esm);
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

  it("without initGrants, consent is in-memory only (backward compatible)", () => {
    // no initGrants call: grant works but does not touch storage
    grant(esm);
    expect(canMount(esm)).toBe(true);
    // clearGrants with no storage is a no-op save (does not throw)
    clearGrants();
    expect(canMount(esm)).toBe(false);
  });

  it("corrupted storage data is ignored, starts fresh", () => {
    const s = mockStorage();
    s.setItem("airp:consent-grants", "{not json");
    initGrants(s);
    expect(isGranted(esm)).toBe(false);
    // subsequent grants still work + persist
    grant(esm);
    expect(s.dump()["airp:consent-grants"]).toContain(esm.type);
  });

  it("non-array stored data is ignored", () => {
    const s = mockStorage();
    s.setItem("airp:consent-grants", JSON.stringify({ not: "an array" }));
    initGrants(s);
    expect(isGranted(esm)).toBe(false);
  });
});
