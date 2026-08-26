import type { Json, SurfaceMessage, SurfaceSnapshot } from "./types";
import type { SurfaceApplyResult } from "./surface-v2";

const MAX_SSE_BUFFER_BYTES = 512 * 1024;
const JSON_REQUEST_TIMEOUT_MS = 10_000;
const RETRY_DELAYS_MS = [250, 500, 1_000, 2_000, 5_000] as const;
const WORKSPACE_WIDGET_TYPES = new Set([
  "core.activity", "core.card", "core.character-state", "core.characters", "core.chat",
  "core.clock", "core.emotion", "core.inventory", "core.map", "core.memory", "core.quest",
]);

export interface EngineSurfaceScope {
  characterId: string;
  sessionId: string;
  userId?: string;
}

/** Durable Workspace revisions are canonical decimal u64 strings. */
export type WorkspaceRevision = string;

export interface WorkspaceScope {
  userId?: string;
}

export interface WorkspaceWidget {
  id: string;
  type: string;
}

export type WorkspaceNode =
  | {
      type: "split";
      id: string;
      orientation: "horizontal" | "vertical";
      ratioBasisPoints: number;
      children: [WorkspaceNode, WorkspaceNode];
    }
  | { type: "tabs"; id: string; active: string; children: WorkspaceNode[] }
  | { type: "stack"; id: string; children: WorkspaceNode[] }
  | { type: "widget"; id: string; instanceId: string };

export interface WorkspaceLayout {
  version: number;
  root: WorkspaceNode;
  widgets: WorkspaceWidget[];
}

export interface WorkspaceDocument {
  schema: number;
  id: string;
  revision: WorkspaceRevision;
  updatedAt: string;
  layout: WorkspaceLayout;
}

export interface WorkspaceResizeSplitCommand {
  type: "resize_split";
  split_id: string;
  ratio_basis_points: number;
}

/** Additive command union; the initial HTTP contract exposes split resizing. */
export type WorkspaceCommand = WorkspaceResizeSplitCommand;

export interface WorkspaceCommandRequest {
  expected_revision: WorkspaceRevision;
  command: WorkspaceCommand;
}

export interface WorkspaceHistoryEntry {
  revision: WorkspaceRevision;
  updated_at: string;
  source_kind: string;
  parent_revision: WorkspaceRevision | null;
}

export interface WorkspaceHistoryResponse {
  entries: WorkspaceHistoryEntry[];
}

export interface WorkspaceRollbackRequest {
  expected_revision: WorkspaceRevision;
  target_revision: WorkspaceRevision;
}

/** Raw export remains available even when a future workspace schema is unknown. */
export interface WorkspaceRawExport {
  text: string;
  schema: string | null;
  sha256: string | null;
  contentDisposition: string | null;
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

export class IntentStreamInterrupted extends Error {}

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
export class HttpFailure extends Error {
  constructor(
    readonly status: number,
    message: string,
    readonly data?: Record<string, unknown>,
  ) {
    super(message);
    this.name = "HttpFailure";
  }

  get isConflict(): boolean {
    return this.status === 409;
  }
}

function messageOf(value: unknown): string {
  return value instanceof Error ? value.message : String(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function hasOnlyKeys(value: Record<string, unknown>, allowed: readonly string[]): boolean {
  return Object.keys(value).every((key) => allowed.includes(key));
}

function isRevision(value: unknown): value is WorkspaceRevision {
  return typeof value === "string"
    && /^(0|[1-9][0-9]*)$/.test(value)
    && BigInt(value) <= 18_446_744_073_709_551_615n;
}

function isIdentifier(value: unknown): value is string {
  return typeof value === "string"
    && new TextEncoder().encode(value).byteLength <= 128
    && /^[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value);
}

interface WorkspaceParseState {
  nodes: number;
  nodeIds: Set<string>;
  widgetIds: Set<string>;
  widgetRefs: Set<string>;
}

function nodeId(node: WorkspaceNode): string {
  return node.id;
}

function parseWorkspaceNode(value: unknown, state: WorkspaceParseState, depth = 1): WorkspaceNode | null {
  if (!isRecord(value) || !isIdentifier(value.type) || !isIdentifier(value.id)) return null;
  state.nodes += 1;
  if (depth > 16 || state.nodes > 512 || state.nodeIds.has(value.id)) return null;
  state.nodeIds.add(value.id);
  if (value.type === "split") {
    if (!hasOnlyKeys(value, ["type", "id", "orientation", "ratioBasisPoints", "children"])
      || (value.orientation !== "horizontal" && value.orientation !== "vertical")
      || typeof value.ratioBasisPoints !== "number"
      || !Number.isInteger(value.ratioBasisPoints)
      || value.ratioBasisPoints < 1_000
      || value.ratioBasisPoints > 9_000
      || !Array.isArray(value.children)
      || value.children.length !== 2) return null;
    const left = parseWorkspaceNode(value.children[0], state, depth + 1);
    const right = parseWorkspaceNode(value.children[1], state, depth + 1);
    return left && right ? {
      type: "split",
      id: value.id,
      orientation: value.orientation,
      ratioBasisPoints: value.ratioBasisPoints,
      children: [left, right],
    } : null;
  }
  if (value.type === "tabs") {
    if (!hasOnlyKeys(value, ["type", "id", "active", "children"])
      || !isIdentifier(value.active)
      || !Array.isArray(value.children)
      || value.children.length < 1
      || value.children.length > 32) return null;
    const children = value.children.map((child) => parseWorkspaceNode(child, state, depth + 1));
    return children.every((child): child is WorkspaceNode => child !== null)
      && children.some((child) => nodeId(child) === value.active)
      ? { type: "tabs", id: value.id, active: value.active, children }
      : null;
  }
  if (value.type === "stack") {
    if (!hasOnlyKeys(value, ["type", "id", "children"])
      || !Array.isArray(value.children)
      || value.children.length < 1
      || value.children.length > 32) return null;
    const children = value.children.map((child) => parseWorkspaceNode(child, state, depth + 1));
    return children.every((child): child is WorkspaceNode => child !== null)
      ? { type: "stack", id: value.id, children }
      : null;
  }
  if (value.type === "widget") {
    if (!hasOnlyKeys(value, ["type", "id", "instanceId"])
      || !isIdentifier(value.instanceId)
      || !state.widgetIds.has(value.instanceId)
      || state.widgetRefs.has(value.instanceId)) return null;
    state.widgetRefs.add(value.instanceId);
    return isIdentifier(value.instanceId)
      ? { type: "widget", id: value.id, instanceId: value.instanceId }
      : null;
  }
  return null;
}

function parseWorkspaceDocument(value: unknown): WorkspaceDocument {
  if (!isRecord(value)
    || !hasOnlyKeys(value, ["schema", "id", "revision", "updatedAt", "layout"])
    || value.schema !== 1
    || !isIdentifier(value.id)
    || !isRevision(value.revision)
    || typeof value.updatedAt !== "string"
    || value.updatedAt.length < 1
    || new TextEncoder().encode(value.updatedAt).byteLength > 64
    || !isRecord(value.layout)
    || !hasOnlyKeys(value.layout, ["version", "root", "widgets"])
    || value.layout.version !== 1
    || !Array.isArray(value.layout.widgets)
    || value.layout.widgets.length > 128) {
    throw new Error("Engine returned an invalid workspace document");
  }
  const widgetIds = new Set<string>();
  const widgets: WorkspaceWidget[] = [];
  for (const widget of value.layout.widgets) {
    if (!isRecord(widget)
      || !hasOnlyKeys(widget, ["id", "type"])
      || !isIdentifier(widget.id)
      || typeof widget.type !== "string"
      || !WORKSPACE_WIDGET_TYPES.has(widget.type)
      || widgetIds.has(widget.id)) {
      throw new Error("Engine returned an invalid workspace document");
    }
    widgetIds.add(widget.id);
    widgets.push({ id: widget.id, type: widget.type });
  }
  const state: WorkspaceParseState = {
    nodes: 0,
    nodeIds: new Set(),
    widgetIds,
    widgetRefs: new Set(),
  };
  const root = parseWorkspaceNode(value.layout.root, state);
  if (root === null) {
    throw new Error("Engine returned an invalid workspace document");
  }
  if (state.widgetRefs.size !== widgetIds.size) {
    throw new Error("Engine returned an invalid workspace document");
  }
  return {
    schema: value.schema,
    id: value.id,
    revision: value.revision,
    updatedAt: value.updatedAt,
    layout: { version: value.layout.version, root, widgets },
  };
}

function parseWorkspaceHistory(value: unknown): WorkspaceHistoryResponse {
  if (!isRecord(value)
    || !hasOnlyKeys(value, ["entries"])
    || !Array.isArray(value.entries)
    || value.entries.length > 256) {
    throw new Error("Engine returned an invalid workspace history");
  }
  const entries = value.entries.map((entry): WorkspaceHistoryEntry | null => (
    isRecord(entry)
      && hasOnlyKeys(entry, ["revision", "updated_at", "source_kind", "parent_revision"])
      && isRevision(entry.revision)
      && isIdentifier(entry.updated_at)
      && isIdentifier(entry.source_kind)
      && (entry.parent_revision === null || isRevision(entry.parent_revision))
      ? {
          revision: entry.revision,
          updated_at: entry.updated_at,
          source_kind: entry.source_kind,
          parent_revision: entry.parent_revision,
        }
      : null
  ));
  if (!entries.every((entry): entry is WorkspaceHistoryEntry => entry !== null)) {
    throw new Error("Engine returned an invalid workspace history");
  }
  return { entries };
}

function assertWorkspaceCommand(request: WorkspaceCommandRequest): void {
  if (!isRevision(request.expected_revision)
    || !isIdentifier(request.command.split_id)
    || !Number.isSafeInteger(request.command.ratio_basis_points)
    || request.command.ratio_basis_points < 1_000
    || request.command.ratio_basis_points > 9_000) {
    throw new Error("invalid workspace command request");
  }
}

function assertWorkspaceRollback(request: WorkspaceRollbackRequest): void {
  if (!isRevision(request.expected_revision) || !isRevision(request.target_revision)) {
    throw new Error("invalid workspace rollback request");
  }
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

  async getWorkspace(scope: WorkspaceScope = {}): Promise<WorkspaceDocument> {
    return parseWorkspaceDocument(await this.requestJson(this.workspacePath(scope)));
  }

  /** Sends exactly one mutation request; a 409 is surfaced as HttpFailure and is never replayed. */
  async sendWorkspaceCommand(
    request: WorkspaceCommandRequest,
    scope: WorkspaceScope = {},
  ): Promise<WorkspaceDocument> {
    assertWorkspaceCommand(request);
    return parseWorkspaceDocument(await this.requestJson(this.workspacePath(scope, "/commands"), {
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "application/json" },
      body: JSON.stringify(request),
    }));
  }

  async history(scope: WorkspaceScope = {}, limit?: number): Promise<WorkspaceHistoryResponse> {
    if (limit !== undefined && (!Number.isSafeInteger(limit) || limit < 1 || limit > 256)) {
      throw new Error("workspace history limit must be between 1 and 256");
    }
    const query = this.workspaceQuery(scope);
    if (limit !== undefined) query.set("limit", String(limit));
    const suffix = query.size ? `?${query}` : "";
    return parseWorkspaceHistory(await this.requestJson(`/v1/ui/workspace/history${suffix}`));
  }

  /** Reads the export without parsing or normalizing its future-compatible JSON text. */
  async exportWorkspace(scope: WorkspaceScope = {}): Promise<WorkspaceRawExport> {
    const response = await this.requestResponse(this.workspacePath(scope, "/export"), {
      headers: { Accept: "application/json" },
    });
    return {
      text: await response.text(),
      schema: response.headers.get("x-airp-workspace-schema"),
      sha256: response.headers.get("x-airp-workspace-sha256"),
      contentDisposition: response.headers.get("content-disposition"),
    };
  }

  async rollback(
    request: WorkspaceRollbackRequest,
    scope: WorkspaceScope = {},
  ): Promise<WorkspaceDocument> {
    assertWorkspaceRollback(request);
    return parseWorkspaceDocument(await this.requestJson(this.workspacePath(scope, "/rollback"), {
      method: "POST",
      headers: { "Content-Type": "application/json", Accept: "application/json" },
      body: JSON.stringify(request),
    }));
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
        throw new IntentStreamInterrupted("Intent stream ended without a terminal frame");
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

  private workspaceQuery(scope: WorkspaceScope): URLSearchParams {
    const query = new URLSearchParams();
    if (scope.userId) query.set("user_id", scope.userId);
    return query;
  }

  private workspacePath(scope: WorkspaceScope, suffix = ""): string {
    const query = this.workspaceQuery(scope);
    return `/v1/ui/workspace${suffix}${query.size ? `?${query}` : ""}`;
  }

  private async requestJson(path: string, init: RequestInit = {}): Promise<unknown> {
    const response = await this.requestResponse(path, init);
    return response.json();
  }

  private async requestResponse(path: string, init: RequestInit = {}): Promise<Response> {
    const timeout = AbortSignal.timeout(JSON_REQUEST_TIMEOUT_MS);
    const signal = init.signal ? AbortSignal.any([init.signal, timeout]) : timeout;
    const response = await this.authorizedFetch(path, { ...init, signal });
    if (!response.ok) throw await this.httpError(response);
    return response;
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
    let data: Record<string, unknown> | undefined;
    try {
      const body = await response.json() as unknown;
      const bodyRecord = isRecord(body) ? body : undefined;
      data = bodyRecord && isRecord(bodyRecord.error) ? bodyRecord.error : undefined;
      const message = bodyRecord?.message ?? data?.message;
      if (typeof message === "string") detail = message;
    } catch { /* retain HTTP status */ }
    return new HttpFailure(response.status, detail, data);
  }
}
