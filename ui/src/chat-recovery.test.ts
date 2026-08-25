import { describe, expect, it } from "vitest";
import type { SurfaceSnapshot } from "./protocol/types";
import {
  captureChatBaseline,
  cancellationIsExplicitlyNotCommitted,
  chatOperationIsCommitted,
  chatProjectionChanged,
  classifyChatRecoveryProjection,
  projectedSessionPhase,
} from "./chat-recovery";

function snapshot(
  messages: Array<{ role: string; content: string }>,
  ids: string[],
  phase = "idle",
  candidates: string[][] = messages.map(() => []),
  swipe: number[] = messages.map(() => 0),
): SurfaceSnapshot {
  return {
    kind: "snapshot", protocol: { major: 2, minor: 0 }, surfaceId: "session:s1", revision: "1",
    blueprint: {
      version: 2,
      root: { type: "widget", id: "chat-node", instanceId: "chat" },
      widgets: [
        { id: "chat", type: "core.chat", props: {
          messages, message_ids: ids, message_candidates: candidates, message_swipe_index: swipe,
        } },
        { id: "activity", type: "core.activity", props: { live: { phase } } },
      ],
    },
  };
}

describe("chat recovery projection", () => {
  it("uses stable IDs so an older equal-text message cannot satisfy a new send", () => {
    const before = snapshot([
      { role: "user", content: "same" }, { role: "assistant", content: "reply" },
    ], ["u-old", "a-old"]);
    const baseline = captureChatBaseline(before, "chat");
    expect(chatOperationIsCommitted(before, "chat", { mode: "send", baseline })).toBe(false);
    const after = snapshot([
      { role: "user", content: "same" }, { role: "assistant", content: "reply" },
      { role: "user", content: "same" }, { role: "assistant", content: "reply" },
    ], ["u-old", "a-old", "u-new", "a-new"]);
    expect(chatOperationIsCommitted(after, "chat", { mode: "send", baseline })).toBe(true);
  });

  it("distinguishes committed continuation, regeneration, and swipe", () => {
    const before = snapshot([{ role: "assistant", content: "one" }], ["a1"], "generating", [["one"]], [0]);
    const baseline = captureChatBaseline(before, "chat");
    expect(projectedSessionPhase(before)).toBe("generating");
    const continued = snapshot([{ role: "assistant", content: "one two" }], ["a1"]);
    expect(chatOperationIsCommitted(continued, "chat", { mode: "continue", baseline })).toBe(true);
    const regenerated = snapshot([{ role: "assistant", content: "other" }], ["a1"], "idle", [["one", "other"]], [1]);
    expect(chatOperationIsCommitted(regenerated, "chat", { mode: "regen", baseline })).toBe(true);
    expect(chatOperationIsCommitted(regenerated, "chat", {
      mode: "swipe", baseline, targetMessageId: "a1", targetIndex: 1,
    })).toBe(true);
  });

  it("detects an incomplete canonical change without calling it committed", () => {
    const before = snapshot([{ role: "assistant", content: "old" }], ["a1"]);
    const baseline = captureChatBaseline(before, "chat");
    const halfTurn = snapshot([
      { role: "assistant", content: "old" }, { role: "user", content: "new" },
    ], ["a1", "u2"]);
    expect(chatOperationIsCommitted(halfTurn, "chat", { mode: "send", baseline })).toBe(false);
    expect(chatProjectionChanged(halfTurn, "chat", { mode: "send", baseline })).toBe(true);
  });

  it("keeps the recovery lock even when history looks committed", () => {
    const before = snapshot([{ role: "assistant", content: "old" }], ["a1"]);
    const baseline = captureChatBaseline(before, "chat");
    const locked = snapshot([
      { role: "assistant", content: "old" }, { role: "assistant", content: "new" },
    ], ["a1", "a2"], "recovering");
    expect(classifyChatRecoveryProjection(locked, "chat", { mode: "send", baseline }))
      .toBe("recovery_required");
  });

  it("clears cancellation only for the explicit not_committed state", () => {
    expect(cancellationIsExplicitlyNotCommitted("not_committed")).toBe(true);
    for (const state of [undefined, null, "partially_committed", "completed", "future_state"]) {
      expect(cancellationIsExplicitlyNotCommitted(state)).toBe(false);
    }
  });
});
