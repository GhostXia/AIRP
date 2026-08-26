import { describe, expect, it } from "vitest";
import {
  characterCount, editorIsReadOnly, isAuthoritativeRevision, parseObjectDraft,
  preservesDraftOnAuthorityRefresh, topLevelObjectPatch,
} from "./editable-state";

describe("editable state contracts", () => {
  it("builds only top-level add, replace, and remove operations", () => {
    expect(topLevelObjectPatch(
      { mood: "calm", obsolete: true, nested: { score: 1 }, "a/b~c": 1 },
      { mood: "alert", added: [1, 2], nested: { score: 2 }, "a/b~c": 1 },
    )).toEqual([
      { op: "remove", path: "/obsolete" },
      { op: "replace", path: "/mood", value: "alert" },
      { op: "add", path: "/added", value: [1, 2] },
      { op: "replace", path: "/nested", value: { score: 2 } },
    ]);
  });

  it("escapes top-level JSON pointer keys", () => {
    expect(topLevelObjectPatch({}, { "a/b~c": true })).toEqual([
      { op: "add", path: "/a~1b~0c", value: true },
    ]);
  });

  it("rejects invalid JSON and non-object top levels locally", () => {
    expect(parseObjectDraft("{")).toMatchObject({ ok: false, error: expect.stringContaining("JSON 解析错误") });
    expect(parseObjectDraft("[]")).toEqual({ ok: false, error: "角色状态必须是顶层 JSON object。" });
    expect(parseObjectDraft("null")).toEqual({ ok: false, error: "角色状态必须是顶层 JSON object。" });
  });

  it("treats object key order as unchanged and counts Unicode characters", () => {
    expect(topLevelObjectPatch({ nested: { a: 1, b: 2 } }, { nested: { b: 2, a: 1 } })).toEqual([]);
    expect(characterCount("A😀文")).toBe(3);
  });

  it("accepts only non-negative safe integer authority revisions", () => {
    expect(isAuthoritativeRevision(0)).toBe(true);
    expect(isAuthoritativeRevision(42)).toBe(true);
    expect(isAuthoritativeRevision("42")).toBe(false);
    expect(isAuthoritativeRevision(-1)).toBe(false);
    expect(isAuthoritativeRevision(Number.MAX_SAFE_INTEGER + 1)).toBe(false);
  });

  it("preserves drafts when a refreshed mutation result is conflict or uncertain", () => {
    expect(preservesDraftOnAuthorityRefresh("conflict")).toBe(true);
    expect(preservesDraftOnAuthorityRefresh("error")).toBe(true);
    expect(preservesDraftOnAuthorityRefresh("saved")).toBe(false);
  });

  it("locks editors while a mutation is saving", () => {
    expect(editorIsReadOnly(false, "saving")).toBe(true);
    expect(editorIsReadOnly(true, "saved")).toBe(true);
    expect(editorIsReadOnly(false, "saved")).toBe(false);
  });
});
