import { afterEach, describe, expect, it, vi } from "vitest";
import { SurfaceStore } from "./surface-v2";
import { HttpEngineBus, HttpFailure, IntentStreamInterrupted } from "./http-engine-bus";
import type { SurfaceSnapshot } from "./types";

function snapshot(revision = "1", title = "one"): SurfaceSnapshot {
  return {
    kind: "snapshot",
    protocol: { major: 2, minor: 0 },
    surfaceId: "session:s1",
    revision,
    blueprint: {
      version: 2,
      root: { type: "widget", id: "chat-node", instanceId: "chat" },
      widgets: [{ id: "chat", type: "core.chat", props: { title } }],
    },
  };
}

function json(value: unknown, status = 200): Response {
  return new Response(JSON.stringify(value), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function stream(chunks: string[]): Response {
  const encoder = new TextEncoder();
  return new Response(new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
    },
  }), { headers: { "Content-Type": "text/event-stream" } });
}

function closedStream(chunks: string[]): Response {
  const encoder = new TextEncoder();
  return new Response(new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
      controller.close();
    },
  }), { headers: { "Content-Type": "text/event-stream" } });
}

async function until(predicate: () => boolean): Promise<void> {
  for (let index = 0; index < 100; index += 1) {
    if (predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
  throw new Error("condition was not reached");
}

describe("HttpEngineBus", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("applies split CRLF SSE and sends the last accepted opaque cursor", async () => {
    const calls: Array<{ path: string; cursor: string | null }> = [];
    const store = new SurfaceStore();
    const patch = JSON.stringify({
      kind: "patch", protocol: { major: 2, minor: 0 }, surfaceId: "session:s1",
      baseRevision: "1", revision: "2",
      patch: [{ op: "replace", path: "/blueprint/widgets/0/props/title", value: "two" }],
    });
    const fetchImpl = async (input: URL | RequestInfo, init?: RequestInit) => {
      const url = new URL(String(input));
      calls.push({ path: url.pathname, cursor: new Headers(init?.headers).get("Last-Event-ID") });
      if (url.pathname === "/version") return json({ version: "test" });
      if (url.pathname.endsWith("/events")) {
        return stream([`id: cursor-2\r`, `\nevent: patch\r\ndata: ${patch}\r\n\r\n`]);
      }
      return json({ cursor: "cursor-1", snapshot: snapshot() });
    };
    const bus = new HttpEngineBus({ base: "http://engine.test", bearer: () => "secret", fetchImpl });
    const stop = await bus.connect({ characterId: "alice", sessionId: "s1" }, store);
    await until(() => store.snapshot?.revision === "2");
    stop();
    expect(store.snapshot?.blueprint.widgets[0].props).toEqual({ title: "two" });
    expect(calls.find((call) => call.path.endsWith("/events"))?.cursor).toBe("cursor-1");
    expect(calls.every((call) => !String(call.path).includes("secret"))).toBe(true);
  });

  it("retries a transient resync failure and then installs the fresh snapshot", async () => {
    let snapshots = 0;
    let streams = 0;
    const delays: number[] = [];
    const store = new SurfaceStore();
    const fetchImpl = async (input: URL | RequestInfo) => {
      const url = new URL(String(input));
      if (url.pathname === "/version") return json({ version: "test" });
      if (url.pathname.endsWith("/events")) {
        streams += 1;
        if (streams === 1) {
          return stream([`id: bad\nevent: patch\ndata: ${JSON.stringify(snapshot())}\n\n`]);
        }
        return stream([]);
      }
      snapshots += 1;
      if (snapshots === 2) return json({ message: "temporarily unavailable" }, 503);
      return json({ cursor: `cursor-${snapshots}`, snapshot: snapshot(String(snapshots), `snapshot-${snapshots}`) });
    };
    const bus = new HttpEngineBus({
      base: "http://engine.test", bearer: () => "secret", fetchImpl,
      sleep: async (milliseconds) => { delays.push(milliseconds); },
    });
    const stop = await bus.connect({ characterId: "alice", sessionId: "s1" }, store);
    await until(() => snapshots >= 3);
    stop();
    expect(store.snapshot?.blueprint.widgets[0].props).toEqual({ title: "snapshot-3" });
    expect(delays).toEqual([250, 500]);
  });

  it("reconnects a broken stream from the last successfully applied cursor", async () => {
    const cursors: Array<string | null> = [];
    let streams = 0;
    const store = new SurfaceStore();
    const event = JSON.stringify({
      kind: "patch", protocol: { major: 2, minor: 0 }, surfaceId: "session:s1",
      baseRevision: "1", revision: "2",
      patch: [{ op: "replace", path: "/blueprint/widgets/0/props/title", value: "accepted" }],
    });
    const fetchImpl = async (input: URL | RequestInfo, init?: RequestInit) => {
      const url = new URL(String(input));
      if (url.pathname === "/version") return json({ version: "test" });
      if (!url.pathname.endsWith("/events")) return json({ cursor: "cursor-1", snapshot: snapshot() });
      streams += 1;
      cursors.push(new Headers(init?.headers).get("Last-Event-ID"));
      return streams === 1
        ? closedStream([`id: cursor-2\nevent: patch\ndata: ${event}\n\n`])
        : stream([]);
    };
    const bus = new HttpEngineBus({
      base: "http://engine.test", bearer: () => "secret", fetchImpl,
      sleep: async () => {},
    });
    const stop = await bus.connect({ characterId: "alice", sessionId: "s1" }, store);
    await until(() => streams >= 2);
    stop();
    expect(cursors).toEqual(["cursor-1", "cursor-2"]);
    expect(store.snapshot?.revision).toBe("2");
  });

  it("renews once on 401 and resolves the bearer again for the retry", async () => {
    let bearer = "stale";
    const auth: string[] = [];
    const fetchImpl = async (_input: URL | RequestInfo, init?: RequestInit) => {
      const value = new Headers(init?.headers).get("Authorization") ?? "";
      auth.push(value);
      return value === "Bearer fresh" ? json({ ok: true }) : json({ message: "expired" }, 401);
    };
    const bus = new HttpEngineBus({
      base: "http://engine.test",
      bearer: () => bearer,
      renew: async () => { bearer = "fresh"; return true; },
      fetchImpl,
    });
    await expect(bus.dispatchIntent(
      "session:s1", "chat", "chat.send", { text: "hi" },
    )).resolves.toEqual({ ok: true });
    expect(auth).toEqual(["Bearer stale", "Bearer fresh"]);
  });

  it("exposes HTTP 409 as a typed conflict without replaying the intent", async () => {
    let requests = 0;
    const bus = new HttpEngineBus({
      base: "http://engine.test",
      bearer: () => "secret",
      fetchImpl: async () => {
        requests += 1;
        return json({ message: "revision mismatch" }, 409);
      },
    });

    const failure = await bus.dispatchIntent(
      "session:s1", "character-state", "characterState.patch", { expected_revision: 4, patch: [] },
    ).catch((error: unknown) => error);
    expect(failure).toBeInstanceOf(HttpFailure);
    expect(failure).toMatchObject({ status: 409, isConflict: true, message: "revision mismatch" });
    expect(requests).toBe(1);
  });

  it("parses chat intent SSE across chunk boundaries and requires done", async () => {
    const frames: unknown[] = [];
    const fetchImpl = async () => closedStream([
      "event: message\r\ndata: {\"type\":\"body_chunk\",\"text\":\"hel",
      "lo\"}\r\n\r\nevent: message\r\ndata: {\"type\":\"done\"}\r\n\r\n",
    ]);
    const bus = new HttpEngineBus({ base: "http://engine.test", bearer: () => "secret", fetchImpl });
    await expect(bus.dispatchIntent(
      "session:s1", "chat", "chat.send", { text: "hi" }, (event) => frames.push(event.data),
    )).resolves.toMatchObject({ data: { type: "done" } });
    expect(frames).toEqual([
      { type: "body_chunk", text: "hello" },
      { type: "done" },
    ]);
  });

  it("classifies terminal-free intent EOF without replaying the mutation", async () => {
    let requests = 0;
    const bus = new HttpEngineBus({
      base: "http://engine.test", bearer: () => "secret",
      fetchImpl: async () => {
        requests += 1;
        return closedStream(["event: message\ndata: {\"type\":\"body_chunk\",\"text\":\"partial\"}\n\n"]);
      },
    });
    await expect(bus.dispatchIntent("session:s1", "chat", "chat.send", { text: "hi" }))
      .rejects.toBeInstanceOf(IntentStreamInterrupted);
    expect(requests).toBe(1);
  });

  it("surfaces typed chat stream errors", async () => {
    let cancelled = false;
    const encoder = new TextEncoder();
    const fetchImpl = async () => new Response(new ReadableStream({
      start(controller) {
        controller.enqueue(encoder.encode(
          "event: error\ndata: {\"type\":\"error\",\"text\":\"provider unavailable\"}\n\n",
        ));
      },
      cancel() { cancelled = true; },
    }), { headers: { "Content-Type": "text/event-stream" } });
    const bus = new HttpEngineBus({ base: "http://engine.test", bearer: () => "secret", fetchImpl });
    await expect(bus.dispatchIntent(
      "session:s1", "chat", "chat.send", { text: "hi" },
    )).rejects.toThrow("provider unavailable");
    expect(cancelled).toBe(true);
  });

  it("rejects unknown and post-terminal intent frames", async () => {
    const unknownBus = new HttpEngineBus({
      base: "http://engine.test", bearer: () => "secret",
      fetchImpl: async () => closedStream(["event: mystery\ndata: {\"type\":\"body_chunk\",\"text\":\"x\"}\n\n"]),
    });
    await expect(unknownBus.dispatchIntent("session:s1", "chat", "chat.send", { text: "hi" }))
      .rejects.toThrow("Unknown intent SSE event");

    const trailingBus = new HttpEngineBus({
      base: "http://engine.test", bearer: () => "secret",
      fetchImpl: async () => closedStream([
        "event: message\ndata: {\"type\":\"done\"}\n\n",
        "event: message\ndata: {\"type\":\"body_chunk\",\"text\":\"late\"}\n\n",
      ]),
    });
    await expect(trailingBus.dispatchIntent("session:s1", "chat", "chat.send", { text: "hi" }))
      .rejects.toThrow("after its terminal frame");
  });

  it("bootstraps a first session when the selected character has none", async () => {
    const stored = new Map<string, string>();
    vi.stubGlobal("location", { search: "?user_id=tenant-a" });
    vi.stubGlobal("sessionStorage", {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => stored.set(key, value),
    });
    const methods: string[] = [];
    const fetchImpl = async (input: URL | RequestInfo, init?: RequestInit) => {
      const url = new URL(String(input));
      methods.push(`${init?.method ?? "GET"} ${url.pathname}${url.search}`);
      if (url.pathname === "/v1/characters") return json(["alice"]);
      if (url.pathname === "/v1/sessions/alice" && init?.method === "POST") return json("new-session");
      if (url.pathname === "/v1/sessions/alice") return json([]);
      throw new Error(`unexpected request ${url}`);
    };
    const bus = new HttpEngineBus({ base: "http://engine.test", bearer: () => "secret", fetchImpl });
    await expect(bus.resolveScope()).resolves.toMatchObject({
      characterId: "alice", sessionId: "new-session", userId: "tenant-a",
    });
    expect(methods).toEqual([
      "GET /v1/characters?user_id=tenant-a",
      "GET /v1/sessions/alice?user_id=tenant-a",
      "POST /v1/sessions/alice?user_id=tenant-a",
    ]);
  });
});
