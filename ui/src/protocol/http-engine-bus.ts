import type { Json, SurfaceMessage, SurfaceSnapshot } from "./types";
import type { SurfaceApplyResult } from "./surface-v2";

const MAX_SSE_BUFFER_BYTES = 512 * 1024;
const JSON_REQUEST_TIMEOUT_MS = 10_000;
const RETRY_DELAYS_MS = [250, 500, 1_000, 2_000, 5_000] as const;

export interface EngineSurfaceScope {
  characterId: string;
  sessionId: string;
  userId?: string;
}

export interface SurfaceTarget {
  apply(message: SurfaceMessage | unknown): SurfaceApplyResult;
}

export interface EngineBusError {
  code: string;
  message: string;
  recoverable: boolean;
}

export interface IntentStreamEvent {
  event: string;
  data: Record<string, unknown>;
}

export class IntentStreamFailure extends Error {
  constructor(readonly data: Record<string, unknown>, message: string) { super(message); }
}

export interface HttpEngineBusOptions {
  base?: string;
  fetchImpl?: typeof fetch;
  bearer: () => string;
  renew?: () => Promise<boolean>;
  sleep?: (milliseconds: number) => Promise<void>;
  onError?: (error: EngineBusError) => void;
  onConnection?: (connected: boolean) => void;
}

interface SnapshotResponse {
  cursor: string;
  snapshot: SurfaceSnapshot;
}

class ResyncRequired extends Error {}
class HttpFailure extends Error {
  constructor(readonly status: number, message: string) { super(message); }
}

function messageOf(value: unknown): string {
  return value instanceof Error ? value.message : String(value);
}

export class HttpEngineBus {
  private readonly base: string;
  private readonly fetchImpl: typeof fetch;
  private readonly bearer: () => string;
  private readonly renew?: () => Promise<boolean>;
  private readonly sleep: (milliseconds: number) => Promise<void>;
  private readonly onError?: (error: EngineBusError) => void;
  private readonly onConnection?: (connected: boolean) => void;
  private controller: AbortController | null = null;
  private generation = 0;

  constructor(options: HttpEngineBusOptions) {
    this.base = (options.base ?? location.origin).replace(/\/+$/, "");
    this.fetchImpl = options.fetchImpl ?? fetch;
    this.bearer = options.bearer;
    this.renew = options.renew;
    this.sleep = options.sleep ?? ((milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)));
    this.onError = options.onError;
    this.onConnection = options.onConnection;
  }

  async hello(): Promise<unknown> {
    return this.requestJson("/version");
  }

  async resolveScope(): Promise<EngineSurfaceScope> {
    const query = new URLSearchParams(location.search);
    const userId = query.get("user_id") ?? undefined;
    const rememberedCharacter = query.get("character_id")
      ?? query.get("character")
      ?? sessionStorage.getItem("airp_character_id");
    const characterQuery = new URLSearchParams();
    if (userId) characterQuery.set("user_id", userId);
    const characters = await this.requestJson(`/v1/characters${characterQuery.size ? `?${characterQuery}` : ""}`) as unknown;
    if (!Array.isArray(characters) || !characters.every((value) => typeof value === "string")) {
      throw new Error("Engine returned an invalid character list");
    }
    const characterId = rememberedCharacter && characters.includes(rememberedCharacter)
      ? rememberedCharacter
      : characters[0];
    if (!characterId) throw new Error("没有可用角色；请先在 WebUI 导入角色");

    const sessions = await this.listSessions(characterId, userId);
    const rememberedSession = query.get("session_id")
      ?? query.get("session")
      ?? sessionStorage.getItem("airp_session_id");
    let sessionId = rememberedSession && sessions.includes(rememberedSession)
      ? rememberedSession
      : sessions[0];
    if (!sessionId) sessionId = await this.createSession(characterId, userId);
    sessionStorage.setItem("airp_character_id", characterId);
    sessionStorage.setItem("airp_session_id", sessionId);
    return { characterId, sessionId, ...(userId ? { userId } : {}) };
  }

  async listSessions(characterId: string, userId?: string): Promise<string[]> {
    const query = new URLSearchParams();
    if (userId) query.set("user_id", userId);
    const suffix = query.size ? `?${query}` : "";
    const sessions = await this.requestJson(`/v1/sessions/${encodeURIComponent(characterId)}${suffix}`) as unknown;
    if (!Array.isArray(sessions) || !sessions.every((value) => typeof value === "string")) {
      throw new Error("Engine returned an invalid session list");
    }
    return sessions;
  }

  async createSession(characterId: string, userId?: string): Promise<string> {
    const query = new URLSearchParams();
    if (userId) query.set("user_id", userId);
    const suffix = query.size ? `?${query}` : "";
    const sessionId = await this.requestJson(`/v1/sessions/${encodeURIComponent(characterId)}${suffix}`, {
      method: "POST",
    });
    if (typeof sessionId !== "string") throw new Error("Engine returned an invalid session id");
    return sessionId;
  }

  async connect(scope: EngineSurfaceScope, target: SurfaceTarget): Promise<() => void> {
    this.disconnect();
    const generation = ++this.generation;
    const controller = new AbortController();
    this.controller = controller;
    await this.hello();
    let cursor = await this.loadSnapshot(scope, target, controller.signal);
    if (generation !== this.generation || controller.signal.aborted) return () => {};
    this.onConnection?.(true);
    void this.run(scope, target, cursor, generation, controller).catch((error) => {
      if (!controller.signal.aborted && generation === this.generation) {
        this.onConnection?.(false);
        this.onError?.({ code: "surface_stream_failed", message: messageOf(error), recoverable: true });
      }
    });
    return () => {
      if (generation === this.generation) this.disconnect();
    };
  }

  disconnect(): void {
    this.generation += 1;
    this.controller?.abort();
    this.controller = null;
    this.onConnection?.(false);
  }

  async dispatchIntent(
    surfaceId: string,
    instanceId: string,
    name: string,
    params?: Json,
    onEvent?: (event: IntentStreamEvent) => void,
  ): Promise<unknown> {
    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort("Intent response headers timed out"), JSON_REQUEST_TIMEOUT_MS);
    let response: Response;
    try {
      response = await this.authorizedFetch("/v1/ui/intents", {
        method: "POST",
        signal: controller.signal,
        headers: { "Content-Type": "application/json", Accept: "application/json, text/event-stream" },
        body: JSON.stringify({
          surface_id: surfaceId,
          instance_id: instanceId,
          name,
          ...(params === undefined ? {} : { params }),
        }),
      });
    } finally {
      clearTimeout(timeout);
    }
    if (!response.ok) throw await this.httpError(response);
    if ((response.headers.get("content-type") ?? "").includes("text/event-stream")) {
      return this.consumeIntentStream(response, onEvent);
    }
    return response.json();
  }

  async refreshSurface(scope: EngineSurfaceScope, target: SurfaceTarget): Promise<void> {
    const controller = new AbortController();
    await this.loadSnapshot(scope, target, controller.signal);
  }

  private async consumeIntentStream(
    response: Response,
    onEvent?: (event: IntentStreamEvent) => void,
  ): Promise<IntentStreamEvent> {
    if (!response.body) throw new Error("Intent stream has no body");
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    const encoder = new TextEncoder();
    let buffer = "";
    let terminal: IntentStreamEvent | null = null;
    try {
      while (true) {
        const { value, done } = await reader.read();
        buffer += decoder.decode(value, { stream: !done });
        buffer = buffer.replace(/\r\n/g, "\n");
        let boundary: number;
        while ((boundary = buffer.indexOf("\n\n")) >= 0) {
          const frame = buffer.slice(0, boundary);
          buffer = buffer.slice(boundary + 2);
          if (!frame || frame.startsWith(":")) continue;
          if (encoder.encode(frame).byteLength > MAX_SSE_BUFFER_BYTES) {
            throw new Error("Intent SSE frame exceeds limit");
          }
          let event = "message";
          const dataLines: string[] = [];
          for (const line of frame.split("\n")) {
            if (line.startsWith("event:")) event = line.slice(6).trimStart();
            else if (line.startsWith("data:")) dataLines.push(line.slice(5).trimStart());
          }
          if (dataLines.length === 0) continue;
          let data: Record<string, unknown>;
          try { data = JSON.parse(dataLines.join("\n")) as Record<string, unknown>; }
          catch { throw new Error("Invalid intent SSE JSON"); }
          if (terminal) throw new Error("Intent stream emitted a frame after its terminal frame");
          if (event !== "message" && event !== "error") throw new Error(`Unknown intent SSE event: ${event}`);
          if ((event === "error") !== (data.type === "error")) {
            throw new Error("Intent SSE event name does not match its data type");
          }
          const parsed = { event, data };
          onEvent?.(parsed);
          if (event === "error" || data.type === "done") terminal = parsed;
          if (event === "error") {
            const detail = typeof data.text === "string" ? data.text : "Chat generation failed";
            throw new IntentStreamFailure(data, detail);
          }
        }
        if (encoder.encode(buffer).byteLength > MAX_SSE_BUFFER_BYTES) {
          throw new Error("Intent SSE frame exceeds limit");
        }
        if (done) break;
      }
      if (!terminal || terminal.data.type !== "done") {
        throw new Error("Intent stream ended without a terminal frame");
      }
      return terminal;
    } finally {
      await reader.cancel().catch(() => {});
      reader.releaseLock();
    }
  }

  private async run(
    scope: EngineSurfaceScope,
    target: SurfaceTarget,
    initialCursor: string,
    generation: number,
    controller: AbortController,
  ): Promise<void> {
    let cursor = initialCursor;
    let failures = 0;
    let needsSnapshot = false;
    while (!controller.signal.aborted && generation === this.generation) {
      const acceptedCursor = { value: cursor };
      try {
        if (needsSnapshot) {
          cursor = await this.loadSnapshot(scope, target, controller.signal);
          acceptedCursor.value = cursor;
          needsSnapshot = false;
          this.onConnection?.(true);
        }
        await this.stream(scope, target, acceptedCursor, controller.signal, () => { failures = 0; });
        cursor = acceptedCursor.value;
        failures = 0;
      } catch (error) {
        cursor = acceptedCursor.value;
        if (controller.signal.aborted || generation !== this.generation) return;
        this.onConnection?.(false);
        if (error instanceof ResyncRequired) {
          needsSnapshot = true;
          this.onError?.({ code: "surface_resyncing", message: messageOf(error), recoverable: true });
          await this.sleep(RETRY_DELAYS_MS[Math.min(failures, RETRY_DELAYS_MS.length - 1)]);
          failures += 1;
          continue;
        }
        if (error instanceof HttpFailure
          && error.status !== 408
          && error.status !== 429
          && error.status < 500) {
          throw error;
        }
        this.onError?.({ code: "surface_reconnecting", message: messageOf(error), recoverable: true });
        await this.sleep(RETRY_DELAYS_MS[Math.min(failures, RETRY_DELAYS_MS.length - 1)]);
        failures += 1;
      }
    }
  }

  private async loadSnapshot(scope: EngineSurfaceScope, target: SurfaceTarget, signal: AbortSignal): Promise<string> {
    const response = await this.requestJson(this.surfacePath(scope), { signal }) as SnapshotResponse;
    if (!response || typeof response.cursor !== "string") throw new Error("Surface snapshot response has no cursor");
    const applied = target.apply(response.snapshot);
    if (applied.status !== "applied") throw new Error(`Surface snapshot rejected: ${applied.error.message}`);
    return response.cursor;
  }

  private async stream(
    scope: EngineSurfaceScope,
    target: SurfaceTarget,
    cursor: { value: string },
    signal: AbortSignal,
    onAccepted: () => void,
  ): Promise<void> {
    const response = await this.authorizedFetch(this.surfacePath(scope, true), {
      signal,
      headers: { Accept: "text/event-stream", "Last-Event-ID": cursor.value },
    });
    if (!response.ok) throw await this.httpError(response);
    if (!response.body) throw new Error("Surface event stream has no body");
    this.onConnection?.(true);
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    const encoder = new TextEncoder();
    let buffer = "";
    while (true) {
      const { value, done } = await reader.read();
      buffer += decoder.decode(value, { stream: !done });
      buffer = buffer.replace(/\r\n/g, "\n");
      let boundary: number;
      while ((boundary = buffer.indexOf("\n\n")) >= 0) {
        const frame = buffer.slice(0, boundary);
        buffer = buffer.slice(boundary + 2);
        if (encoder.encode(frame).byteLength > MAX_SSE_BUFFER_BYTES) {
          throw new ResyncRequired("Surface SSE frame exceeds limit");
        }
        const parsed = this.parseFrame(frame);
        if (parsed === null) continue;
        if (parsed.event === "error") throw new ResyncRequired("Engine requested Surface resync");
        if (parsed.event !== "snapshot" && parsed.event !== "patch") {
          throw new ResyncRequired(`Unknown Surface event: ${parsed.event}`);
        }
        let message: unknown;
        try { message = JSON.parse(parsed.data); } catch { throw new ResyncRequired("Invalid Surface SSE JSON"); }
        if (!message || typeof message !== "object" || (message as { kind?: unknown }).kind !== parsed.event) {
          throw new ResyncRequired("Surface SSE event name does not match payload kind");
        }
        const applied = target.apply(message);
        if (applied.status !== "applied") throw new ResyncRequired(applied.error.message);
        cursor.value = parsed.id;
        onAccepted();
      }
      if (encoder.encode(buffer).byteLength > MAX_SSE_BUFFER_BYTES) {
        throw new ResyncRequired("Surface SSE frame exceeds limit");
      }
      if (done) throw new Error(`Surface event stream ended after ${cursor.value}`);
    }
  }

  private parseFrame(frame: string): { id: string; event: string; data: string } | null {
    if (!frame || frame.startsWith(":")) return null;
    let id = "";
    let event = "";
    const data: string[] = [];
    for (const line of frame.split("\n")) {
      if (line.startsWith("id:")) id = line.slice(3).trimStart();
      else if (line.startsWith("event:")) event = line.slice(6).trimStart();
      else if (line.startsWith("data:")) data.push(line.slice(5).trimStart());
    }
    if (!id || !event || data.length === 0) throw new ResyncRequired("Incomplete Surface SSE frame");
    return { id, event, data: data.join("\n") };
  }

  private surfacePath(scope: EngineSurfaceScope, events = false): string {
    const query = new URLSearchParams({ character_id: scope.characterId });
    if (scope.userId) query.set("user_id", scope.userId);
    const suffix = events ? "/events" : "";
    return `/v1/ui/surfaces/session/${encodeURIComponent(scope.sessionId)}${suffix}?${query}`;
  }

  private async requestJson(path: string, init: RequestInit = {}): Promise<unknown> {
    const timeout = AbortSignal.timeout(JSON_REQUEST_TIMEOUT_MS);
    const signal = init.signal ? AbortSignal.any([init.signal, timeout]) : timeout;
    const response = await this.authorizedFetch(path, { ...init, signal });
    if (!response.ok) throw await this.httpError(response);
    return response.json();
  }

  private async authorizedFetch(path: string, init: RequestInit): Promise<Response> {
    const execute = () => {
      const headers = new Headers(init.headers);
      const bearer = this.bearer();
      if (bearer) headers.set("Authorization", `Bearer ${bearer}`);
      const fetchImpl = this.fetchImpl;
      return fetchImpl(new URL(path, `${this.base}/`), { ...init, headers });
    };
    let response = await execute();
    if (response.status === 401 && await this.renew?.()) response = await execute();
    return response;
  }

  private async httpError(response: Response): Promise<Error> {
    let detail = `${response.status} ${response.statusText}`.trim();
    try {
      const body = await response.json() as { message?: unknown; error?: { message?: unknown } };
      const message = body.message ?? body.error?.message;
      if (typeof message === "string") detail = message;
    } catch { /* retain HTTP status */ }
    return new HttpFailure(response.status, detail);
  }
}
