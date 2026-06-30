/**
 * Real AgentBus over Tauri IPC (PLAN task B).
 *
 * UI → core: `invoke("airp_dispatch", { env })` forwards an upstream envelope to
 * the Rust core, which relays it to the Gateway.
 * core → UI: the core emits downstream envelopes on the `airp:envelope` event.
 *
 * The IPC is injected as a {@link TauriTransport} so the logic is unit-testable
 * without a running Tauri shell. `createTauriTransport()` builds the real one by
 * dynamically importing `@tauri-apps/api` (only loaded inside the Tauri app).
 *
 * NOTE: the real end-to-end link (core ↔ Gateway) is runtime-only; it is on the
 * "unverified — debug phase" ledger in docs/PLAN.md. CI covers the logic here.
 */

import type { AgentBus, EnvelopeHandler } from "./bus";
import type { Envelope } from "./types";

const DISPATCH_CMD = "airp_dispatch";
const ENVELOPE_EVENT = "airp:envelope";

/** Minimal slice of the Tauri IPC this bus needs (injectable for tests). */
export interface TauriTransport {
  invoke(cmd: string, args?: Record<string, unknown>): Promise<unknown>;
  /** Subscribe to a Tauri event; resolves to an unlisten fn. */
  listen(event: string, cb: (payload: Envelope) => void): Promise<() => void>;
}

export class TauriBus implements AgentBus {
  constructor(private readonly transport: TauriTransport) {}

  async dispatch(env: Envelope): Promise<void> {
    await this.transport.invoke(DISPATCH_CMD, { env });
  }

  subscribe(handler: EnvelopeHandler): () => void {
    let cancelled = false;
    const ready = this.transport.listen(ENVELOPE_EVENT, (env) => {
      if (!cancelled) handler(env);
    });
    // If unsubscribed before `listen` resolved, tear down as soon as it does.
    void ready.then((unlisten) => {
      if (cancelled) unlisten();
    });
    return () => {
      cancelled = true;
      void ready.then((unlisten) => unlisten());
    };
  }
}

/** Build the real Tauri transport (only call inside a Tauri window). */
export async function createTauriTransport(): Promise<TauriTransport> {
  const { invoke } = await import("@tauri-apps/api/core");
  const { listen } = await import("@tauri-apps/api/event");
  return {
    invoke: (cmd, args) => invoke(cmd, args),
    listen: (event, cb) => listen<Envelope>(event, (e) => cb(e.payload)),
  };
}
