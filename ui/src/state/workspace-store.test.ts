import { isReactive, watch } from "vue";
import { describe, expect, it } from "vitest";
import type { WorkspaceDocument } from "../protocol/http-engine-bus";
import { WorkspaceRequestGate, WorkspaceStateStore } from "./workspace-store";

function document(revision = "7", ratioBasisPoints = 6_500): WorkspaceDocument {
  return {
    schema: 1,
    id: "default",
    revision,
    updatedAt: "2026-08-28T00:00:00Z",
    layout: {
      version: 1,
      root: {
        type: "split",
        id: "workspace-root",
        orientation: "horizontal",
        ratioBasisPoints,
        children: [
          {
            type: "tabs",
            id: "workspace-primary",
            active: "chat-node",
            children: [{ type: "widget", id: "chat-node", instanceId: "chat" }],
          },
          {
            type: "stack",
            id: "workspace-context",
            children: [{ type: "widget", id: "activity-node", instanceId: "activity" }],
          },
        ],
      },
      widgets: [
        { id: "chat", type: "core.chat" },
        { id: "activity", type: "core.activity" },
      ],
    },
  };
}

describe("WorkspaceStateStore", () => {
  it("keeps one cloned accepted document and derives split ratios", () => {
    const store = new WorkspaceStateStore();
    const accepted = document();
    store.accept(accepted);

    expect(isReactive(store.state)).toBe(true);
    expect(store.acceptedDocument).not.toBe(accepted);
    expect(store.acceptedDocument).toEqual(accepted);
    expect(store.revision).toBe("7");
    expect(store.splitRatioByNodeId).toEqual({ "workspace-root": 6_500 });

    accepted.layout.root = { type: "widget", id: "foreign", instanceId: "foreign" };
    const exposed = store.acceptedDocument!;
    exposed.revision = "999";
    expect(store.revision).toBe("7");
    expect(store.splitRatioByNodeId).toEqual({ "workspace-root": 6_500 });
  });

  it("serializes commands and only replaces accepted state on success", () => {
    const store = new WorkspaceStateStore();
    store.accept(document());
    const pending: Array<string | null> = [];
    watch(() => store.pendingCommand, (value) => pending.push(value), { flush: "sync" });

    const command = { type: "resize_split", split_id: "workspace-root", ratio_basis_points: 7_000 } as const;
    expect(store.begin(command)).toBe(true);
    expect(store.begin(command)).toBe(false);
    expect(store.revision).toBe("7");
    store.finish(document("8", 7_000));

    expect(pending).toEqual(["resize_split", null]);
    expect(store.revision).toBe("8");
    expect(store.splitRatioByNodeId).toEqual({ "workspace-root": 7_000 });
  });

  it("keeps the accepted document after failure and clears cross-scope state", () => {
    const store = new WorkspaceStateStore();
    store.accept(document());
    expect(store.begin({ type: "reset_layout" })).toBe(true);
    store.fail("revision conflict");

    expect(store.revision).toBe("7");
    expect(store.lastError).toBe("revision conflict");
    store.clear();
    expect(store.acceptedDocument).toBeNull();
    expect(store.splitRatioByNodeId).toEqual({});
    expect(store.begin({ type: "reset_layout" })).toBe(false);
  });

  it("rejects a late read that would roll back the accepted revision", () => {
    const store = new WorkspaceStateStore();
    expect(store.accept(document("8", 7_000))).toBe(true);
    expect(store.accept(document("7", 6_500))).toBe(false);

    expect(store.revision).toBe("8");
    expect(store.splitRatioByNodeId).toEqual({ "workspace-root": 7_000 });
  });
});

describe("WorkspaceRequestGate", () => {
  it("invalidates older reads when a mutation or scope transition begins", () => {
    const gate = new WorkspaceRequestGate();
    const slowRead = gate.begin();
    const mutation = gate.begin();

    expect(gate.isCurrent(slowRead)).toBe(false);
    expect(gate.isCurrent(mutation)).toBe(true);
    gate.invalidate();
    expect(gate.isCurrent(mutation)).toBe(false);
  });
});
