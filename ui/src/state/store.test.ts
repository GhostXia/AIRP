import { describe, it, expect, beforeEach } from "vitest";
import { stateStore, setState, patchState } from "./store";

beforeEach(() => {
  for (const key of Object.keys(stateStore)) delete stateStore[key];
});

describe("state store", () => {
  it("set replaces a scope", () => {
    setState("a", { x: 1 });
    expect(stateStore.a).toEqual({ x: 1 });
  });

  it("patch replace updates a value", () => {
    setState("e", { emotion: 60, label: "平静" });
    patchState("e", [{ op: "replace", path: "/emotion", value: 80 }]);
    expect((stateStore.e as { emotion: number }).emotion).toBe(80);
  });

  it("patch add appends to an array with '-'", () => {
    setState("c", { messages: [] });
    patchState("c", [{ op: "add", path: "/messages/-", value: { id: "1" } }]);
    expect((stateStore.c as { messages: unknown[] }).messages).toHaveLength(1);
  });

  it("patch replace updates an id-keyed chat message", () => {
    setState("chat", {
      messages: { a1: { id: "a1", role: "assistant", text: "" } },
      order: ["a1"],
    });
    patchState("chat", [{ op: "replace", path: "/messages/a1/text", value: "partial" }]);
    const chat = stateStore.chat as { messages: Record<string, { text: string }>; order: string[] };
    expect(chat.messages.a1.text).toBe("partial");
    expect(chat.order).toEqual(["a1"]);
  });

  it("patch remove deletes a key", () => {
    setState("o", { a: 1, b: 2 });
    patchState("o", [{ op: "remove", path: "/b" }]);
    expect((stateStore.o as { b?: number }).b).toBeUndefined();
  });

  it("patch on a fresh scope initializes an object", () => {
    patchState("new", [{ op: "add", path: "/k", value: 1 }]);
    expect((stateStore.new as { k: number }).k).toBe(1);
  });

  it("patch copy duplicates a value", () => {
    setState("cp", { a: 1 });
    patchState("cp", [{ op: "copy", from: "/a", path: "/b" }]);
    expect(stateStore.cp).toEqual({ a: 1, b: 1 });
  });

  it("patch move relocates a value", () => {
    setState("mv", { a: 1 });
    patchState("mv", [{ op: "move", from: "/a", path: "/b" }]);
    expect(stateStore.mv).toEqual({ b: 1 });
  });

  it("patch test passes when the value matches, then later ops apply", () => {
    setState("t", { x: 5 });
    patchState("t", [
      { op: "test", path: "/x", value: 5 },
      { op: "replace", path: "/x", value: 9 },
    ]);
    expect((stateStore.t as { x: number }).x).toBe(9);
  });

  it("patch test throws when the value does not match", () => {
    setState("tf", { x: 5 });
    expect(() => patchState("tf", [{ op: "test", path: "/x", value: 1 }])).toThrow();
  });

  it("rolls back earlier ops when a later op fails", () => {
    setState("tx", { x: 5, y: 1 });
    expect(() =>
      patchState("tx", [
        { op: "replace", path: "/x", value: 9 },
        { op: "test", path: "/y", value: 2 },
      ]),
    ).toThrow();
    expect(stateStore.tx).toEqual({ x: 5, y: 1 });
  });
});
