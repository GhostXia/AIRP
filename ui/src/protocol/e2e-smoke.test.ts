/**
 * End-to-end smoke for the State Protocol round-trip — the loop the audit
 * (docs/PLAN.md §2.6 #4) asked to be covered:
 *
 *   manifest down → blueprint down → intent up → state patch back down.
 *
 * Driven through the real `MockBus` and the SAME boundary guard App.vue uses,
 * against the real reactive store. No Vue rendering (the component layer is
 * covered by App.manifest.test.ts); this asserts the *protocol contract*
 * closes cleanly end-to-end, and that the guard rejects malformed traffic
 * instead of letting it half-apply.
 */
import { describe, it, expect, beforeEach } from "vitest";
import type { Envelope, Blueprint, WidgetDef, JsonPatch } from "./types";
import { MockBus } from "./bus";
import { validateEnvelope } from "./guard";
import {
  applyManifestMessage,
  clearManifests,
  getManifest,
} from "../registry/index";
import { registerBuiltins } from "../registry/index";
import { resolveWidget } from "../registry/index";
import { stateStore, setState, patchState, applyJsonPatch } from "../state/store";
import type { WidgetModule } from "../registry/widget-module";

// The exact processing App.vue does in onEnvelope, minus the Vue refs — so this
// smoke drives the same dispatch against the same guard + registry + store.
function makeDispatcher(
  blueprintRef: { bp: Blueprint | null },
  importer?: (s: string) => Promise<unknown>,
) {
  return (e: Envelope): void => {
    const guard = validateEnvelope(e);
    if (!guard.ok) return; // rejected at boundary — caller tracks this separately
    const body = e.body;
    if (body.kind === "manifest") {
      applyManifestMessage(body.op, body.manifests, importer);
    } else if (body.kind === "blueprint") {
      if (body.op === "set" && body.blueprint) blueprintRef.bp = body.blueprint;
      else if (body.op === "patch" && body.patch && blueprintRef.bp) {
        const next = structuredClone(blueprintRef.bp);
        applyJsonPatch(next as unknown as import("./types").Json, body.patch);
        blueprintRef.bp = next;
      }
    } else if (body.kind === "state") {
      if (body.op === "set") setState(body.scope, body.state ?? null);
      else if (body.op === "patch" && body.patch) patchState(body.scope, body.patch);
    }
    // intents are produced by the UI upstream, not consumed here.
  };
}

const THIRD_PARTY: WidgetDef = {
  type: "acme.status-pill",
  version: "1.0.0",
  title: "状态胶囊",
  entry: { kind: "esm", source: "demo:acme/status-pill" },
  intents: ["status.toggle"],
  capabilities: ["read:state"],
};

describe("protocol round-trip e2e smoke (audit §2.6 #4)", () => {
  beforeEach(() => {
    clearManifests();
    for (const k of Object.keys(stateStore)) delete stateStore[k];
    registerBuiltins();
  });

  it("manifest → blueprint → intent → state patch closes the loop", async () => {
    const bus = new MockBus();
    const blueprintRef = { bp: null as Blueprint | null };
    const seenKinds: string[] = [];

    // Register the third-party importer the demo uses (local map, no network).
    const mod: WidgetModule = { mount() {} };
    const importer = async (s: string) => {
      if (s === "demo:acme/status-pill") return { default: () => mod };
      throw new Error(`unknown source ${s}`);
    };

    bus.subscribe((e: Envelope) => {
      seenKinds.push(e.body.kind);
      makeDispatcher(blueprintRef, importer)(e);
    });

    // Let the MockBus prime its session (manifest + blueprint + seed state).
    await Promise.resolve();

    // 1. manifest arrived and was registered → type resolvable as esm module.
    expect(getManifest("acme.status-pill")?.version).toBe("1.0.0");
    const reg = resolveWidget("acme.status-pill");
    expect(reg?.kind).toBe("module");
    if (reg?.kind === "module") expect(await reg.load()).toBe(mod);

    // 2. blueprint arrived and was stored.
    expect(blueprintRef.bp).not.toBeNull();
    expect(blueprintRef.bp!.widgets.some((w) => w.type === "acme.status-pill")).toBe(true);

    // 3. ordering: manifest precedes blueprint (the contract App.vue relies on).
    expect(seenKinds.indexOf("manifest")).toBeLessThan(seenKinds.indexOf("blueprint"));

    // 4. seed state arrived for the chat scope.
    expect(stateStore["w-chat"]).toBeDefined();
    const messagesBefore = (stateStore["w-chat"] as { messages: unknown[] }).messages.length;

    // 5. intent up: a user sends a chat line. MockBus echoes it back as a patch.
    bus.dispatch({
      v: 1,
      id: "ui-1",
      ts: Date.now(),
      src: "ui",
      body: { kind: "intent", name: "chat.send", params: { text: "hello" } },
    });

    // 6. the patch回流 added a user + assistant line to the chat scope.
    const messagesAfter = (stateStore["w-chat"] as { messages: unknown[] }).messages.length;
    expect(messagesAfter).toBe(messagesBefore + 2);
    const last = (stateStore["w-chat"] as { messages: { text: string }[] }).messages.slice(-2);
    expect(last[0].text).toBe("hello");
    expect(last[1].text).toBe("（示例回应）");
  });

  it("the guard rejects malformed envelopes so they cannot half-apply", () => {
    // A state patch without a `patch` array would otherwise hit patchState with
    // undefined and corrupt the scope. The guard stops it at the boundary.
    const bad: Envelope = {
      v: 1,
      id: "x",
      ts: 1,
      src: "gateway",
      body: { kind: "state", scope: "w-chat", op: "patch" } as unknown as Envelope["body"],
    };
    expect(validateEnvelope(bad).ok).toBe(false);

    // And a manifest with an unknown capability is rejected (consent binds to
    // the known cap set; an unknown one must not slip into the registry).
    const badManifest: Envelope = {
      v: 1,
      id: "y",
      ts: 1,
      src: "gateway",
      body: {
        kind: "manifest",
        op: "set",
        manifests: [
          { ...THIRD_PARTY, capabilities: ["read:secrets" as unknown as "read:state"] },
        ],
      },
    };
    expect(validateEnvelope(badManifest).ok).toBe(false);
  });

  it("blueprint op:patch mutates the live blueprint (drift recovery)", () => {
    const bus = new MockBus();
    const blueprintRef = { bp: null as Blueprint | null };
    const dispatch = makeDispatcher(blueprintRef);
    bus.subscribe((e) => dispatch(e));
    // flush microtask to receive the seed blueprint.
    return Promise.resolve().then(() => {
      expect(blueprintRef.bp).not.toBeNull();
      const widgetCountBefore = blueprintRef.bp!.widgets.length;

      // Gateway pushes a downstream blueprint patch adding a widget. This is a
      // bus→UI delivery (not an upstream intent), so simulate it by feeding the
      // dispatcher directly the way App.vue's subscribe handler would receive it.
      const patch: JsonPatch = [
        { op: "add", path: "/widgets/-", value: { id: "w-extra", type: "core.card" } },
      ];
      dispatch({
        v: 1,
        id: "bp-patch",
        ts: Date.now(),
        src: "gateway",
        body: { kind: "blueprint", op: "patch", patch },
      });
      // The dispatcher validates + applies the blueprint patch in place.
      expect(blueprintRef.bp!.widgets.length).toBe(widgetCountBefore + 1);
      expect(blueprintRef.bp!.widgets[blueprintRef.bp!.widgets.length - 1].id).toBe("w-extra");
    });
  });
});
