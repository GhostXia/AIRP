import type {
  Blueprint,
  BlueprintV2,
  Json,
  SurfaceLayoutNode,
  SurfaceMessage,
  SurfacePatchEvent,
  SurfacePatchOp,
  SurfaceRevision,
  SurfaceSnapshot,
  SurfaceWidgetInstance,
} from "./types";
import {
  validateSurfacePatchEventResult,
  validateSurfaceSnapshotResult,
  type SurfaceGuardResult,
} from "./guard";

export const SURFACE_RESYNC_KIND = "resync" as const;

export interface SurfaceResyncRequest {
  kind: typeof SURFACE_RESYNC_KIND;
  surfaceId: string | null;
  expectedRevision: SurfaceRevision | null;
}

export interface SurfaceAppliedResult {
  status: "applied";
  snapshot: SurfaceSnapshot;
}

export interface SurfaceResyncResult {
  status: "resync";
  error: {
    code: string;
    message: string;
  };
  request: SurfaceResyncRequest;
}

export type SurfaceApplyResult = SurfaceAppliedResult | SurfaceResyncResult;

type JsonObject = { [key: string]: Json };
type JsonContainer = JsonObject | Json[];

function isJsonObject(value: Json): value is JsonObject {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function clone<T>(value: T): T {
  return JSON.parse(JSON.stringify(value)) as T;
}

function resultSnapshot(snapshot: SurfaceSnapshot): SurfaceSnapshot {
  return clone(snapshot) as SurfaceSnapshot;
}

function asJson(value: unknown): Json {
  return clone(value as Json);
}

function pointerSegments(pointer: string): string[] {
  if (pointer === "") return [];
  if (!pointer.startsWith("/") || pointer.includes("\0") || /~(?![01])/.test(pointer)) {
    throw new Error("invalid JSON Pointer");
  }
  const segments = pointer
    .slice(1)
    .split("/")
    .map((segment) => segment.replace(/~1/g, "/").replace(/~0/g, "~"));
  if (segments.some((segment) => ["__proto__", "prototype", "constructor"].includes(segment))) {
    throw new Error("prototype-pollution path segment is forbidden");
  }
  return segments;
}

function arrayIndex(key: string, length: number, allowEnd: boolean): number {
  if (!/^(0|[1-9][0-9]*)$/.test(key)) throw new Error("array pointer index is invalid");
  const index = Number(key);
  if (!Number.isSafeInteger(index) || index < 0 || index > length || (!allowEnd && index === length)) {
    throw new Error("array pointer index is outside the target");
  }
  return index;
}

function child(container: JsonContainer, key: string): Json {
  if (Array.isArray(container)) {
    const index = arrayIndex(key, container.length, false);
    return container[index];
  }
  if (!Object.prototype.hasOwnProperty.call(container, key)) throw new Error("pointer target is missing");
  return container[key];
}

function parentOf(root: Json, segments: string[]): { parent: JsonContainer; key: string } {
  if (segments.length === 0) throw new Error("root has no parent");
  let current: Json = root;
  for (const segment of segments.slice(0, -1)) {
    if (!isJsonObject(current) && !Array.isArray(current)) throw new Error("pointer parent is not a container");
    current = child(current, segment);
  }
  if (!isJsonObject(current) && !Array.isArray(current)) throw new Error("pointer parent is not a container");
  return { parent: current, key: segments[segments.length - 1] };
}

function readAt(root: Json, pointer: string): Json {
  let current = root;
  for (const segment of pointerSegments(pointer)) {
    if (!isJsonObject(current) && !Array.isArray(current)) throw new Error("pointer target is not a container");
    current = child(current, segment);
  }
  return current;
}

function addAt(root: Json, pointer: string, value: Json): Json {
  const segments = pointerSegments(pointer);
  if (segments.length === 0) return asJson(value);
  const { parent, key } = parentOf(root, segments);
  if (Array.isArray(parent)) {
    if (key === "-") {
      parent.push(asJson(value));
    } else {
      parent.splice(arrayIndex(key, parent.length, true), 0, asJson(value));
    }
  } else {
    parent[key] = asJson(value);
  }
  return root;
}

function replaceAt(root: Json, pointer: string, value: Json): Json {
  const segments = pointerSegments(pointer);
  if (segments.length === 0) return asJson(value);
  const { parent, key } = parentOf(root, segments);
  if (Array.isArray(parent)) {
    parent[arrayIndex(key, parent.length, false)] = asJson(value);
  } else {
    if (!Object.prototype.hasOwnProperty.call(parent, key)) throw new Error("replace target is missing");
    parent[key] = asJson(value);
  }
  return root;
}

function removeAt(root: Json, pointer: string): Json {
  const segments = pointerSegments(pointer);
  if (segments.length === 0) throw new Error("removing the snapshot root is not allowed");
  const { parent, key } = parentOf(root, segments);
  if (Array.isArray(parent)) {
    parent.splice(arrayIndex(key, parent.length, false), 1);
  } else {
    if (!Object.prototype.hasOwnProperty.call(parent, key)) throw new Error("remove target is missing");
    delete parent[key];
  }
  return root;
}

function equalJson(left: Json, right: Json): boolean {
  if (left === right) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
    return left.every((value, index) => equalJson(value, right[index]));
  }
  if (typeof left !== "object" || left === null || typeof right !== "object" || right === null) {
    return false;
  }
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) return false;
  return leftKeys.every(
    (key) => Object.prototype.hasOwnProperty.call(right, key) && equalJson(left[key], right[key]),
  );
}

function applyOperation(root: Json, operation: SurfacePatchOp): Json {
  switch (operation.op) {
    case "add":
      if (!("value" in operation)) throw new Error("add requires value");
      return addAt(root, operation.path, operation.value as Json);
    case "remove":
      return removeAt(root, operation.path);
    case "replace":
      if (!("value" in operation)) throw new Error("replace requires value");
      return replaceAt(root, operation.path, operation.value as Json);
    case "move": {
      if (typeof operation.from !== "string") throw new Error("move requires from");
      const moved = clone(readAt(root, operation.from));
      removeAt(root, operation.from);
      return addAt(root, operation.path, moved);
    }
    case "copy": {
      if (typeof operation.from !== "string") throw new Error("copy requires from");
      return addAt(root, operation.path, clone(readAt(root, operation.from)));
    }
    case "test":
      if (!("value" in operation)) throw new Error("test requires value");
      if (!equalJson(readAt(root, operation.path), operation.value as Json)) throw new Error("test operation failed");
      return root;
    default:
      throw new Error("unsupported patch operation");
  }
}

function guardError(result: Exclude<SurfaceGuardResult, { ok: true }>): SurfaceResyncResult["error"] {
  return { code: result.code, message: result.error };
}

function failure(
  result: Exclude<SurfaceGuardResult, { ok: true }>,
  current: SurfaceSnapshot | null,
): SurfaceResyncResult {
  return {
    status: "resync",
    error: guardError(result),
    request: {
      kind: SURFACE_RESYNC_KIND,
      surfaceId: current?.surfaceId ?? null,
      expectedRevision: current?.revision ?? null,
    },
  };
}

function syntheticFailure(
  code: string,
  message: string,
  current: SurfaceSnapshot | null,
): SurfaceResyncResult {
  return {
    status: "resync",
    error: { code, message },
    request: {
      kind: SURFACE_RESYNC_KIND,
      surfaceId: current?.surfaceId ?? null,
      expectedRevision: current?.revision ?? null,
    },
  };
}

/**
 * Atomic client-side projection of a Surface.  Invalid snapshots and failed
 * patch operations never mutate `current` or `lastKnownGood`.
 */
export class SurfaceStore {
  private current: SurfaceSnapshot | null = null;
  private knownGood: SurfaceSnapshot | null = null;

  get snapshot(): SurfaceSnapshot | null {
    return this.current === null ? null : resultSnapshot(this.current);
  }

  get lastKnownGood(): SurfaceSnapshot | null {
    return this.knownGood === null ? null : resultSnapshot(this.knownGood);
  }

  applySnapshot(value: unknown): SurfaceApplyResult {
    const result = validateSurfaceSnapshotResult(value);
    if (!result.ok) return failure(result, this.current);
    const next = resultSnapshot(value as SurfaceSnapshot);
    this.current = next;
    this.knownGood = resultSnapshot(next);
    return { status: "applied", snapshot: resultSnapshot(next) };
  }

  applyPatch(value: unknown): SurfaceApplyResult {
    const result = validateSurfacePatchEventResult(value);
    if (!result.ok) return failure(result, this.current);
    const event = value as SurfacePatchEvent;
    if (this.current === null) {
      return syntheticFailure("resync_required", "a snapshot is required before a patch", this.current);
    }
    if (event.surfaceId !== this.current.surfaceId || event.baseRevision !== this.current.revision) {
      return syntheticFailure("revision_mismatch", "patch base revision does not match the last-known-good snapshot", this.current);
    }

    let candidate: Json;
    try {
      candidate = clone(this.current) as unknown as Json;
      for (const operation of event.patch) candidate = applyOperation(candidate, operation);
      if (!isJsonObject(candidate)) throw new Error("patched snapshot root must be an object");
      candidate.revision = event.revision;
    } catch (error) {
      return syntheticFailure("invalid_patch", `patch was rejected atomically: ${String(error)}`, this.current);
    }

    const checked = validateSurfaceSnapshotResult(candidate);
    if (!checked.ok) return failure(checked, this.current);
    const next = resultSnapshot(candidate as unknown as SurfaceSnapshot);
    this.current = next;
    this.knownGood = resultSnapshot(next);
    return { status: "applied", snapshot: resultSnapshot(next) };
  }

  apply(message: SurfaceMessage | unknown): SurfaceApplyResult {
    if (isJsonObject(message as Json) && (message as JsonObject).kind === "snapshot") return this.applySnapshot(message);
    return this.applyPatch(message);
  }
}

/**
 * Explicit v1 migration boundary.  The v2 guard itself rejects the v1 shape;
 * callers must opt into this deterministic default layout conversion.
 */
export function migrateV1Blueprint(blueprint: Blueprint): BlueprintV2 {
  const widgets: SurfaceWidgetInstance[] = blueprint.widgets.map((widget) => ({
    id: widget.id,
    type: widget.type,
    ...(widget.props === undefined ? {} : { props: clone(widget.props) }),
  }));

  const areas: SurfaceLayoutNode[] = [];
  blueprint.layout.areas.forEach((area, areaIndex) => {
    const children: SurfaceLayoutNode[] = area.widgets.map((instanceId, widgetIndex) => ({
      type: "widget",
      id: `legacy-area-${areaIndex}-widget-${widgetIndex}`,
      instanceId,
    }));
    if (children.length === 0) return;
    areas.push(
      children.length === 1
        ? children[0]
        : { type: "stack", id: `legacy-area-${areaIndex}`, children },
    );
  });

  if (areas.length === 0) throw new Error("v1 blueprint has no non-empty area to migrate");
  const root: SurfaceLayoutNode =
    areas.length === 1 ? areas[0] : { type: "stack", id: "legacy-root", children: areas };
  return { version: 2, root, widgets };
}
