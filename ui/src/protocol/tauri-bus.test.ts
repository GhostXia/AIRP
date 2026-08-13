import { describe, it, expect } from "vitest";
import { TauriBus, type TauriTransport } from "./tauri-bus";
import type { Envelope } from "./types";

function helloEnvelope(): Envelope {
  return { v: 1, id: "x", ts: 0, src: "ui", body: { kind: "hello", client: "t", version: "0" } };
}

describe("TauriBus", () => {
  it("dispatch invokes the dispatch command with the envelope", async () => {
    const calls: Array<[string, Record<string, unknown> | undefined]> = [];
    const transport: TauriTransport = {
      invoke: async (cmd, args) => {
        calls.push([cmd, args]);
      },
      listen: async () => () => {},
    };
    const env = helloEnvelope();
    await new TauriBus(transport).dispatch(env);
    expect(calls[0][0]).toBe("airp_dispatch");
    expect((calls[0][1] as { env: Envelope }).env).toBe(env);
  });

  it("subscribe forwards events; unsubscribe stops and unlistens", async () => {
    let emit: (env: Envelope) => void = () => {};
    let unlistenCount = 0;
    const transport: TauriTransport = {
      invoke: async () => {},
      listen: async (_event, cb) => {
        emit = cb;
        return () => {
          unlistenCount += 1;
        };
      },
    };
    const bus = new TauriBus(transport);
    const seen: string[] = [];
    const off = await bus.subscribe((e) => seen.push(e.body.kind));

    emit(helloEnvelope());
    expect(seen).toEqual(["hello"]);

    off();
    off();
    expect(unlistenCount).toBe(1);

    // after unsubscribe, further events are ignored
    emit(helloEnvelope());
    expect(seen).toEqual(["hello"]);
  });

  it("subscribe rejects when the transport listener cannot attach", async () => {
    const transport: TauriTransport = {
      invoke: async () => {},
      listen: async () => { throw new Error("listener unavailable"); },
    };

    await expect(new TauriBus(transport).subscribe(() => {})).rejects.toThrow("listener unavailable");
  });
});
