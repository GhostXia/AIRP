import { isReactive, watch } from "vue";
import { describe, expect, it } from "vitest";
import type { SurfacePatchEvent, SurfaceSnapshot } from "../protocol/types";
import { SurfaceStateStore } from "./surface-store";

function snapshot(): SurfaceSnapshot {
  return {
    kind: "snapshot",
    protocol: { major: 2, minor: 0 },
    surfaceId: "story",
    revision: "1",
    blueprint: {
      version: 2,
      root: {
        type: "split",
        id: "root",
        orientation: "horizontal",
        children: [
          {
            type: "tabs",
            id: "main-tabs",
            active: "chat-node",
            children: [
              { type: "widget", id: "chat-node", instanceId: "chat-1" },
              { type: "widget", id: "notes-node", instanceId: "notes-1" },
            ],
          },
          { type: "widget", id: "tools-node", instanceId: "tools-1" },
        ],
      },
      widgets: [
        { id: "chat-1", type: "core.chat", props: { title: "Chat" } },
        { id: "notes-1", type: "core.notes" },
        { id: "tools-1", type: "core.tools" },
      ],
    },
  };
}

function patch(revision: string, operations: SurfacePatchEvent["patch"]): SurfacePatchEvent {
  return {
    kind: "patch",
    protocol: { major: 2, minor: 0 },
    surfaceId: "story",
    baseRevision: String(Number(revision) - 1),
    revision,
    patch: operations,
  };
}

describe("SurfaceStateStore", () => {
  it("keeps accepted, pending, and ephemeral state separate and reactive", () => {
    const store = new SurfaceStateStore();
    const initial = snapshot();
    const pending: unknown[] = [];
    watch(() => store.pendingUpdate, (value) => pending.push(value), { flush: "sync" });

    expect(isReactive(store.state)).toBe(true);
    expect(store.applySnapshot(initial)).toMatchObject({ status: "applied" });
    expect(pending).toEqual([initial, null]);
    expect(store.acceptedSnapshot).not.toBe(initial);
    expect(store.acceptedSnapshot).toEqual(initial);
    const exposed = store.acceptedSnapshot!;
    exposed.revision = "999";
    expect(store.acceptedSnapshot?.revision).toBe("1");
    expect(store.pendingUpdate).toBeNull();
    expect(store.activeTabByNodeId).toEqual({ "main-tabs": "chat-node" });
    expect(store.focusedWidgetInstanceId).toBeNull();
  });

  it("preserves accepted and ephemeral state when an atomic candidate fails", () => {
    const store = new SurfaceStateStore();
    store.applySnapshot(snapshot());
    expect(store.activateTab("main-tabs", "notes-node")).toBe(true);
    expect(store.focusWidget("notes-1")).toBe(true);
    const accepted = JSON.parse(JSON.stringify(store.acceptedSnapshot)) as SurfaceSnapshot;

    const result = store.applyPatch(patch("2", [
      { op: "replace", path: "/blueprint/widgets/0/props/title", value: "Leaked candidate" },
      { op: "test", path: "/blueprint/widgets/0/type", value: "wrong.type" },
    ]));

    expect(result).toMatchObject({ status: "resync", error: { code: "invalid_patch" } });
    expect(store.acceptedSnapshot).toEqual(accepted);
    expect(store.acceptedSnapshot?.blueprint.widgets[0].props).toEqual({ title: "Chat" });
    expect(store.activeTabByNodeId["main-tabs"]).toBe("notes-node");
    expect(store.focusedWidgetInstanceId).toBe("notes-1");
    expect(store.lastResync).toMatchObject({ surfaceId: "story", expectedRevision: "1" });
    expect(store.lastError?.code).toBe("invalid_patch");
    expect(store.pendingUpdate).toBeNull();
  });

  it("preserves local selection and focus across an unrelated accepted patch", () => {
    const store = new SurfaceStateStore();
    store.applySnapshot(snapshot());
    store.activateTab("main-tabs", "notes-node");
    store.focusWidget("notes-1");

    expect(store.applyPatch(patch("2", [
      { op: "replace", path: "/blueprint/widgets/0/props/title", value: "Updated" },
    ]))).toMatchObject({ status: "applied" });

    expect(store.acceptedSnapshot?.revision).toBe("2");
    expect(store.activeTabByNodeId["main-tabs"]).toBe("notes-node");
    expect(store.focusedWidgetInstanceId).toBe("notes-1");
    expect(store.lastResync).toBeNull();
    expect(store.lastError).toBeNull();
  });

  it("prunes local references removed by an accepted blueprint", () => {
    const store = new SurfaceStateStore();
    store.applySnapshot(snapshot());
    store.activateTab("main-tabs", "notes-node");
    store.focusWidget("notes-1");

    expect(store.applyPatch(patch("2", [
      {
        op: "replace",
        path: "/blueprint/root",
        value: { type: "widget", id: "chat-node", instanceId: "chat-1" },
      },
      { op: "remove", path: "/blueprint/widgets/2" },
      { op: "remove", path: "/blueprint/widgets/1" },
    ]))).toMatchObject({ status: "applied" });

    expect(store.activeTabByNodeId).toEqual({});
    expect(store.focusedWidgetInstanceId).toBeNull();
  });

  it("lets authoritative tabs.active supersede local selection only when it changes", () => {
    const store = new SurfaceStateStore();
    store.applySnapshot(snapshot());
    store.activateTab("main-tabs", "notes-node");

    expect(store.applyPatch(patch("2", [
      { op: "add", path: "/blueprint/widgets/-", value: { id: "map-1", type: "core.map" } },
      {
        op: "add",
        path: "/blueprint/root/children/0/children/-",
        value: { type: "widget", id: "map-node", instanceId: "map-1" },
      },
      { op: "replace", path: "/blueprint/root/children/0/active", value: "map-node" },
    ]))).toMatchObject({ status: "applied" });
    expect(store.activeTabByNodeId["main-tabs"]).toBe("map-node");

    store.activateTab("main-tabs", "chat-node");
    expect(store.applyPatch(patch("3", [
      { op: "replace", path: "/blueprint/widgets/0/props/title", value: "Still unrelated" },
    ]))).toMatchObject({ status: "applied" });
    expect(store.activeTabByNodeId["main-tabs"]).toBe("chat-node");
  });

  it("validates local tab activation and widget focus without disturbing valid state", () => {
    const store = new SurfaceStateStore();
    expect(store.activateTab("main-tabs", "chat-node")).toBe(false);
    expect(store.focusWidget("chat-1")).toBe(false);
    store.applySnapshot(snapshot());

    expect(store.activateTab("missing-tabs", "chat-node")).toBe(false);
    expect(store.activateTab("main-tabs", "tools-node")).toBe(false);
    expect(store.activeTabByNodeId["main-tabs"]).toBe("chat-node");
    expect(store.focusWidget("missing-widget")).toBe(false);
    expect(store.focusedWidgetInstanceId).toBeNull();
    expect(store.focusWidget("tools-1")).toBe(true);
    expect(store.focusWidget(null)).toBe(true);
    expect(store.focusedWidgetInstanceId).toBeNull();
  });

  it("clears focus hidden by a local tab switch but preserves focus outside that tab set", () => {
    const store = new SurfaceStateStore();
    store.applySnapshot(snapshot());

    store.focusWidget("chat-1");
    store.activateTab("main-tabs", "notes-node");
    expect(store.focusedWidgetInstanceId).toBeNull();

    store.focusWidget("tools-1");
    store.activateTab("main-tabs", "chat-node");
    expect(store.focusedWidgetInstanceId).toBe("tools-1");
  });
});
