/**
 * Runtime structural guard for inbound Envelopes.
 *
 * TS types only protect code we write ourselves — a real runtime / IPC feed is
 * untyped JSON and must not be trusted past the boundary. This module validates
 * the *skeleton* of an Envelope (required fields present, `kind` known, each
 * body shape consistent) WITHOUT pulling in ajv or the JSON Schema bundle:
 *
 *   - a full ajv compile of `schema/airp-state-protocol.schema.json` would add a
 *     heavy runtime dep to the UI bundle and duplicate the truth that already
 *     lives in `schema/`; the schema job in CI is the exhaustive check.
 *   - here we only need enough to stop a malformed envelope from silently
 *     corrupting the registry/store: missing `scope`, a `patch` that isn't an
 *     array, a `blueprint` without `layout`, etc. A rejected envelope is surfaced
 *     as an `error` (see App.vue) instead of being half-applied.
 *
 * The guard never throws — it returns `{ok:false, error}` so the caller decides
 * how to report (an `error` envelope upstream, a banner, a log). Anything it
 * cannot positively confirm as well-formed is rejected (fail-closed).
 */

import type {
  Body,
  Capability,
  Json,
  JsonPatch,
  PatchOpKind,
  SurfaceMessage,
  SurfacePatchEvent,
  SurfaceRevision,
  SurfaceSnapshot,
  SurfaceErrorCode,
} from "./types";

export type GuardResult = { ok: true } | { ok: false; error: string };

export const BODY_KINDS = [
  "blueprint",
  "state",
  "manifest",
  "event",
  "error",
  "intent",
  "subscribe",
  "unsubscribe",
  "hello",
  "ack",
] as const satisfies readonly Body["kind"][];

export const SET_OR_PATCH = ["set", "patch"] as const;
export const PATCH_OPS = [
  "add",
  "remove",
  "replace",
  "move",
  "copy",
  "test",
] as const satisfies readonly PatchOpKind[];
export const CAPABILITIES = [
  "read:memory",
  "write:memory",
  "read:worldbook",
  "read:state",
  "write:state",
  "call:tool",
] as const satisfies readonly Capability[];
export const ENTRY_KINDS = ["builtin", "esm"] as const;
export const LAYOUT_KINDS = ["dock", "grid", "stack", "tabs"] as const;

const KNOWN_KINDS = new Set<Body["kind"]>(BODY_KINDS);
const KNOWN_OPS = new Set<string>(SET_OR_PATCH);
const KNOWN_PATCH_OPS = new Set<PatchOpKind>(PATCH_OPS);
const KNOWN_CAPS = new Set<Capability>(CAPABILITIES);
const KNOWN_ENTRY_KINDS = new Set<string>(ENTRY_KINDS);
const KNOWN_LAYOUT_KINDS = new Set<string>(LAYOUT_KINDS);

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

function isJson(v: unknown): v is Json {
  if (v === null || typeof v === "boolean" || typeof v === "number" || typeof v === "string")
    return true;
  if (typeof v !== "object") return false;
  if (Array.isArray(v)) return v.every(isJson);
  return Object.values(v as Record<string, unknown>).every(isJson);
}

function fail(error: string): GuardResult {
  return { ok: false, error };
}

/** Validate a single RFC 6902 patch op (shape only — not pointer semantics). */
function checkPatchOp(op: unknown): string | null {
  if (!isObject(op)) return "patch op must be an object";
  if (typeof op.op !== "string" || !KNOWN_PATCH_OPS.has(op.op as PatchOpKind))
    return `unknown patch op "${String(op.op)}"`;
  if (typeof op.path !== "string" || op.path.length === 0) return "patch op.path missing";
  // add/replace/test need `value`; move/copy need `from`.
  if (op.op === "move" || op.op === "copy") {
    if (typeof op.from !== "string" || op.from.length === 0) return `${op.op} needs from`;
  } else {
    if (!("value" in op)) return `${op.op} needs value`;
    if (!isJson(op.value)) return `${op.op} value not JSON`;
  }
  return null;
}

function checkPatch(patch: unknown, label: string): string | null {
  if (!Array.isArray(patch)) return `${label} patch must be an array`;
  for (const op of patch) {
    const err = checkPatchOp(op);
    if (err) return `${label}: ${err}`;
  }
  return null;
}

function checkCapability(c: unknown): string | null {
  return typeof c === "string" && KNOWN_CAPS.has(c as Capability)
    ? null
    : `unknown capability "${String(c)}"`;
}

function checkWidgetInstance(w: unknown): string | null {
  if (!isObject(w)) return "widget instance must be an object";
  if (typeof w.id !== "string" || w.id.length === 0) return "widget.id missing";
  if (typeof w.type !== "string" || w.type.length === 0) return "widget.type missing";
  if ("props" in w && !isJson(w.props)) return "widget.props not JSON";
  if ("state" in w && typeof w.state !== "string") return "widget.state must be string";
  if ("capabilities" in w) {
    if (!Array.isArray(w.capabilities)) return "widget.capabilities must be array";
    for (const c of w.capabilities) {
      const err = checkCapability(c);
      if (err) return err;
    }
  }
  return null;
}

function checkBlueprint(bp: unknown): string | null {
  if (!isObject(bp)) return "blueprint must be an object";
  if (typeof bp.version !== "string" || bp.version.length === 0) return "blueprint.version missing";
  if ("profile" in bp && typeof bp.profile !== "string") return "blueprint.profile must be string";
  if ("theme" in bp && bp.theme != null) {
    if (!isObject(bp.theme)) return "theme must be an object";
    if (typeof bp.theme.name !== "string") return "theme.name missing";
    if ("tokens" in bp.theme && !isObject(bp.theme.tokens)) return "theme.tokens must be object";
  }
  const layout = bp.layout;
  if (!isObject(layout)) return "blueprint.layout missing";
  if (typeof layout.type !== "string" || !KNOWN_LAYOUT_KINDS.has(layout.type))
    return `unknown layout type "${String(layout.type)}"`;
  if (!Array.isArray(layout.areas)) return "layout.areas must be array";
  for (const area of layout.areas) {
    if (!isObject(area)) return "area must be an object";
    if (typeof area.id !== "string" || area.id.length === 0) return "area.id missing";
    if (!Array.isArray(area.widgets)) return "area.widgets must be array";
    if (!area.widgets.every((w) => typeof w === "string")) return "area.widgets must be strings";
    if ("props" in area && !isJson(area.props)) return "area.props not JSON";
  }
  if (!Array.isArray(bp.widgets)) return "blueprint.widgets must be array";
  for (const w of bp.widgets) {
    const err = checkWidgetInstance(w);
    if (err) return err;
  }
  return null;
}

function checkWidgetDef(m: unknown): string | null {
  if (!isObject(m)) return "manifest entry must be an object";
  if (typeof m.type !== "string" || m.type.length === 0) return "manifest.type missing";
  if (typeof m.version !== "string" || m.version.length === 0) return "manifest.version missing";
  if (typeof m.title !== "string") return "manifest.title missing";
  if ("capabilities" in m) {
    if (!Array.isArray(m.capabilities)) return "manifest.capabilities must be array";
    for (const c of m.capabilities) {
      const err = checkCapability(c);
      if (err) return err;
    }
  }
  if ("entry" in m && m.entry != null) {
    if (!isObject(m.entry)) return "manifest.entry must be object";
    if (typeof m.entry.kind !== "string" || !KNOWN_ENTRY_KINDS.has(m.entry.kind))
      return `unknown entry kind "${String(m.entry.kind)}"`;
    if (m.entry.kind === "esm") {
      if (typeof m.entry.source !== "string" || m.entry.source.length === 0)
        return "esm entry needs source";
    }
    if ("sandbox" in m.entry && typeof m.entry.sandbox !== "boolean")
      return "entry.sandbox must be boolean";
  }
  return null;
}

function checkBody(body: unknown): string | null {
  if (!isObject(body)) return "body must be an object";
  if (typeof body.kind !== "string" || !KNOWN_KINDS.has(body.kind as Body["kind"]))
    return `unknown body kind "${String(body.kind)}"`;

  switch (body.kind as Body["kind"]) {
    case "blueprint": {
      if (typeof body.op !== "string" || !KNOWN_OPS.has(body.op)) return "blueprint.op invalid";
      if (body.op === "set") {
        if (body.blueprint == null) return "blueprint op:set needs blueprint";
        return checkBlueprint(body.blueprint);
      }
      if (body.op === "patch") {
        if (body.patch == null) return "blueprint op:patch needs patch";
        return checkPatch(body.patch as JsonPatch, "blueprint");
      }
      return null;
    }
    case "state": {
      if (typeof body.scope !== "string" || body.scope.length === 0) return "state.scope missing";
      if (typeof body.op !== "string" || !KNOWN_OPS.has(body.op)) return "state.op invalid";
      if (body.op === "set" && body.state === undefined) return "state op:set needs state";
      if (body.op === "set" && !isJson(body.state)) return "state.state not JSON";
      if (body.op === "patch") {
        if (body.patch == null) return "state op:patch needs patch";
        return checkPatch(body.patch as JsonPatch, "state");
      }
      return null;
    }
    case "manifest": {
      if (typeof body.op !== "string" || !KNOWN_OPS.has(body.op)) return "manifest.op invalid";
      if (!Array.isArray(body.manifests)) return "manifest.manifests must be array";
      for (const m of body.manifests) {
        const err = checkWidgetDef(m);
        if (err) return err;
      }
      return null;
    }
    case "event": {
      if (typeof body.topic !== "string" || body.topic.length === 0) return "event.topic missing";
      if ("data" in body && !isJson(body.data)) return "event.data not JSON";
      return null;
    }
    case "error": {
      if (typeof body.code !== "string" || body.code.length === 0) return "error.code missing";
      if (typeof body.message !== "string") return "error.message missing";
      if ("detail" in body && !isJson(body.detail)) return "error.detail not JSON";
      return null;
    }
    case "intent": {
      if (typeof body.name !== "string" || body.name.length === 0) return "intent.name missing";
      if ("source" in body && typeof body.source !== "string") return "intent.source must be string";
      if ("params" in body && !isJson(body.params)) return "intent.params not JSON";
      return null;
    }
    case "subscribe":
    case "unsubscribe": {
      if (!Array.isArray(body.scopes)) return `${body.kind}.scopes must be array`;
      if (!body.scopes.every((s) => typeof s === "string")) return "scopes must be strings";
      return null;
    }
    case "hello": {
      if (typeof body.client !== "string" || body.client.length === 0) return "hello.client missing";
      if (typeof body.version !== "string") return "hello.version missing";
      if ("accept" in body && !Array.isArray(body.accept)) return "hello.accept must be array";
      return null;
    }
    case "ack": {
      if (typeof body.ref !== "string" || body.ref.length === 0) return "ack.ref missing";
      return null;
    }
  }
  return null;
}

/**
 * Validate an inbound Envelope's wire shape. Returns `{ok:true}` for a
 * well-formed envelope, otherwise `{ok:false, error}` with a short reason.
 * Pure, never throws — safe to call on any `unknown`.
 */
export function validateEnvelope(e: unknown): GuardResult {
  if (!isObject(e)) return fail("envelope must be an object");
  if (e.v !== 1) return fail(`envelope.v must be 1 (got ${String(e.v)})`);
  if (typeof e.id !== "string" || e.id.length === 0) return fail("envelope.id missing");
  if (typeof e.ts !== "number" || !Number.isFinite(e.ts)) return fail("envelope.ts invalid");
  if (typeof e.src !== "string" || e.src.length === 0) return fail("envelope.src missing");
  const bodyErr = checkBody(e.body);
  if (bodyErr !== null) return fail(bodyErr);
  return { ok: true };
}

// ---------------------------------------------------------------------------
// Surface Protocol v2 guard
// ---------------------------------------------------------------------------

export const SURFACE_LIMITS = {
  maxDocumentBytes: 1_048_576,
  maxPatchBytes: 65_536,
  maxPatchOperations: 256,
  maxBlueprintDepth: 16,
  maxBlueprintNodes: 512,
  maxWidgetInstances: 128,
  maxChildren: 32,
  maxIdentifierLength: 128,
} as const;

export const SURFACE_PROTOCOL_COMPONENT_MAX = 65_535;

export const SURFACE_FORBIDDEN_FIELDS = [
  "html",
  "css",
  "styleSheet",
  "javascript",
  "js",
  "script",
  "vue",
  "template",
  "eval",
  "expression",
  "function",
  "renderFunction",
  "render_function",
  "componentSource",
  "component_source",
  "sourceCode",
  "source_code",
  "innerHTML",
  "outerHTML",
  "dangerouslySetInnerHTML",
] as const;

export const SURFACE_ERROR_CODES = [
  "unsupported_major",
  "invalid_version",
  "invalid_revision",
  "revision_mismatch",
  "revision_gap",
  "invalid_blueprint",
  "duplicate_instance_id",
  "invalid_reference",
  "invalid_patch",
  "resource_limit",
  "document_too_large",
  "forbidden_executable_field",
  "resync_required",
] as const satisfies readonly SurfaceErrorCode[];

export type SurfaceGuardResult =
  | { ok: true }
  | { ok: false; code: SurfaceErrorCode; error: string; path?: string };

function surfaceFail(
  code: SurfaceErrorCode,
  error: string,
  path?: string,
): SurfaceGuardResult {
  return path === undefined ? { ok: false, code, error } : { ok: false, code, error, path };
}

function isFiniteJson(value: unknown, seen = new WeakSet<object>()): value is Json {
  if (value === null || typeof value === "string" || typeof value === "boolean") return true;
  if (typeof value === "number") return Number.isFinite(value);
  if (typeof value !== "object") return false;
  if (seen.has(value)) return false;
  seen.add(value);
  if (Array.isArray(value)) return value.every((item) => isFiniteJson(item, seen));
  return Object.values(value as Record<string, unknown>).every((item) => isFiniteJson(item, seen));
}

function forbiddenSurfaceField(value: unknown, seen = new WeakSet<object>()): string | null {
  if (value === null || typeof value !== "object") return null;
  if (seen.has(value)) return "circular_data";
  seen.add(value);
  if (Array.isArray(value)) {
    for (const child of value) {
      const found = forbiddenSurfaceField(child, seen);
      if (found !== null) return found;
    }
    return null;
  }
  for (const [key, child] of Object.entries(value as Record<string, unknown>)) {
    if ((SURFACE_FORBIDDEN_FIELDS as readonly string[]).includes(key)) return key;
    const found = forbiddenSurfaceField(child, seen);
    if (found !== null) return found;
  }
  return null;
}

function surfaceIdentifier(value: unknown): value is string {
  return (
    typeof value === "string" &&
    /^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(value) &&
    value.length <= SURFACE_LIMITS.maxIdentifierLength
  );
}

const MAX_SURFACE_REVISION = 18_446_744_073_709_551_615n;

function surfaceRevision(value: unknown): value is SurfaceRevision {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/.test(value)) return false;
  try {
    return BigInt(value) <= MAX_SURFACE_REVISION;
  } catch {
    return false;
  }
}

function surfaceVersion(value: unknown): SurfaceGuardResult {
  if (!isObject(value)) return surfaceFail("invalid_version", "protocol must be an object", "protocol");
  if (value.major !== 2) {
    return surfaceFail(
      "unsupported_major",
      `unsupported Surface protocol major ${String(value.major)}`,
      "protocol.major",
    );
  }
  if (typeof value.minor !== "number" || !Number.isInteger(value.minor) || value.minor < 0 || value.minor > SURFACE_PROTOCOL_COMPONENT_MAX) {
    return surfaceFail("invalid_version", "protocol.minor must be an unsigned 16-bit integer", "protocol.minor");
  }
  return { ok: true };
}

function surfaceDocumentBytes(value: unknown): number | null {
  try {
    const encoded = new TextEncoder().encode(JSON.stringify(value));
    return encoded.byteLength;
  } catch {
    return null;
  }
}

function surfacePointerSegments(pointer: string): string[] | null {
  if (pointer === "") return [];
  if (!pointer.startsWith("/") || pointer.includes("\0") || /~(?![01])/.test(pointer)) return null;
  const segments = pointer
    .slice(1)
    .split("/")
    .map((segment) => segment.replace(/~1/g, "/").replace(/~0/g, "~"));
  if (segments.some((segment) => ["__proto__", "prototype", "constructor"].includes(segment))) return null;
  return segments;
}

function immutableSurfacePointer(pointer: string): boolean {
  const segments = surfacePointerSegments(pointer);
  if (segments === null || segments.length === 0) return false;
  return ["kind", "protocol", "surfaceId", "revision"].includes(segments[0]);
}

function checkSurfaceWidget(
  widget: unknown,
  ids: Set<string>,
): SurfaceGuardResult {
  if (!isObject(widget)) return surfaceFail("invalid_blueprint", "widget instance must be an object", "widgets");
  if (!surfaceIdentifier(widget.id)) return surfaceFail("invalid_blueprint", "widget.id is invalid", "widgets.id");
  if (typeof widget.type !== "string" || widget.type.length === 0 || widget.type.length > SURFACE_LIMITS.maxIdentifierLength) {
    return surfaceFail("invalid_blueprint", "widget.type is invalid", "widgets.type");
  }
  if ("props" in widget && !isFiniteJson(widget.props)) {
    return surfaceFail("invalid_blueprint", "widget.props must be finite JSON", "widgets.props");
  }
  if (ids.has(widget.id)) {
    return surfaceFail("duplicate_instance_id", `duplicate widget instance id ${widget.id}`, "widgets.id");
  }
  ids.add(widget.id);
  return { ok: true };
}

interface SurfaceNodeState {
  nodeIds: Set<string>;
  widgetRefs: Set<string>;
  widgetIds: Set<string>;
  nodes: number;
}

function checkSurfaceNode(
  node: unknown,
  depth: number,
  state: SurfaceNodeState,
): SurfaceGuardResult {
  if (!isObject(node)) return surfaceFail("invalid_blueprint", "layout node must be an object", "root");
  if (depth > SURFACE_LIMITS.maxBlueprintDepth) {
    return surfaceFail("resource_limit", "blueprint depth exceeds the authority limit", "root");
  }
  state.nodes += 1;
  if (state.nodes > SURFACE_LIMITS.maxBlueprintNodes) {
    return surfaceFail("resource_limit", "blueprint node count exceeds the authority limit", "root");
  }
  if (!surfaceIdentifier(node.id)) return surfaceFail("invalid_blueprint", "layout node id is invalid", "root.id");
  if (state.nodeIds.has(node.id)) {
    return surfaceFail("duplicate_instance_id", `duplicate layout node id ${node.id}`, "root.id");
  }
  state.nodeIds.add(node.id);

  switch (node.type) {
    case "widget": {
      if (!surfaceIdentifier(node.instanceId)) {
        return surfaceFail("invalid_reference", "widget node instanceId is invalid", "root.instanceId");
      }
      if (!state.widgetIds.has(node.instanceId)) {
        return surfaceFail("invalid_reference", `missing widget instance ${node.instanceId}`, "root.instanceId");
      }
      if (state.widgetRefs.has(node.instanceId)) {
        return surfaceFail("duplicate_instance_id", `widget instance ${node.instanceId} is placed more than once`, "root.instanceId");
      }
      state.widgetRefs.add(node.instanceId);
      return { ok: true };
    }
    case "split": {
      if (node.orientation !== "horizontal" && node.orientation !== "vertical") {
        return surfaceFail("invalid_blueprint", "split.orientation is invalid", "root.orientation");
      }
      if (!Array.isArray(node.children) || node.children.length !== 2) {
        return surfaceFail("invalid_blueprint", "split must have exactly two children", "root.children");
      }
      for (const child of node.children) {
        const result = checkSurfaceNode(child, depth + 1, state);
        if (!result.ok) return result;
      }
      return { ok: true };
    }
    case "tabs": {
      if (!surfaceIdentifier(node.active)) return surfaceFail("invalid_reference", "tabs.active is invalid", "root.active");
      if (!Array.isArray(node.children) || node.children.length === 0 || node.children.length > SURFACE_LIMITS.maxChildren) {
        return surfaceFail("resource_limit", "tabs child count is outside the authority limit", "root.children");
      }
      if (!node.children.some((child) => isObject(child) && child.id === node.active)) {
        return surfaceFail("invalid_reference", `tabs.active ${node.active} does not name a child`, "root.active");
      }
      for (const child of node.children) {
        const result = checkSurfaceNode(child, depth + 1, state);
        if (!result.ok) return result;
      }
      return { ok: true };
    }
    case "stack": {
      if (!Array.isArray(node.children) || node.children.length === 0 || node.children.length > SURFACE_LIMITS.maxChildren) {
        return surfaceFail("resource_limit", "stack child count is outside the authority limit", "root.children");
      }
      for (const child of node.children) {
        const result = checkSurfaceNode(child, depth + 1, state);
        if (!result.ok) return result;
      }
      return { ok: true };
    }
    default:
      return surfaceFail("invalid_blueprint", `unknown layout node type ${String(node.type)}`, "root.type");
  }
}

function checkSurfaceBlueprint(blueprint: unknown): SurfaceGuardResult {
  if (!isObject(blueprint)) return surfaceFail("invalid_blueprint", "blueprint must be an object", "blueprint");
  if (blueprint.version !== 2) return surfaceFail("invalid_version", "blueprint.version must be 2", "blueprint.version");
  if (!Array.isArray(blueprint.widgets)) return surfaceFail("invalid_blueprint", "blueprint.widgets must be an array", "blueprint.widgets");
  if (blueprint.widgets.length > SURFACE_LIMITS.maxWidgetInstances) {
    return surfaceFail("resource_limit", "widget instance count exceeds the authority limit", "blueprint.widgets");
  }
  const widgetIds = new Set<string>();
  for (const widget of blueprint.widgets) {
    const result = checkSurfaceWidget(widget, widgetIds);
    if (!result.ok) return result;
  }
  const state: SurfaceNodeState = { nodeIds: new Set(), widgetRefs: new Set(), widgetIds, nodes: 0 };
  const root = checkSurfaceNode(blueprint.root, 1, state);
  if (!root.ok) return root;
  for (const id of widgetIds) {
    if (!state.widgetRefs.has(id)) {
      return surfaceFail("invalid_reference", `widget instance ${id} is not placed in the layout`, "blueprint.widgets");
    }
  }
  return { ok: true };
}

/** Validate a v2 Blueprint or a complete Surface snapshot. */
export function validateBlueprintV2(value: unknown): SurfaceGuardResult {
  try {
    const forbidden = forbiddenSurfaceField(value);
    if (forbidden !== null) return surfaceFail("forbidden_executable_field", `forbidden executable field ${forbidden}`);
    return checkSurfaceBlueprint(value);
  } catch (error) {
    return surfaceFail("invalid_blueprint", `blueprint guard failed: ${String(error)}`);
  }
}

export function validateSurfaceSnapshot(value: unknown): value is SurfaceSnapshot {
  return validateSurfaceSnapshotResult(value).ok;
}

export function validateSurfaceSnapshotResult(value: unknown): SurfaceGuardResult {
  try {
    if (!isObject(value)) return surfaceFail("invalid_version", "snapshot must be an object");
    if (value.kind !== "snapshot") return surfaceFail("invalid_version", "snapshot.kind must be snapshot", "kind");
    const version = surfaceVersion(value.protocol);
    if (!version.ok) return version;
    if (!surfaceIdentifier(value.surfaceId)) return surfaceFail("invalid_blueprint", "surfaceId is invalid", "surfaceId");
    if (!surfaceRevision(value.revision)) return surfaceFail("invalid_revision", "revision must be a decimal u64 string", "revision");
    const bytes = surfaceDocumentBytes(value);
    if (bytes === null || bytes > SURFACE_LIMITS.maxDocumentBytes) {
      return surfaceFail("document_too_large", "snapshot exceeds the document byte limit");
    }
    const blueprint = validateBlueprintV2(value.blueprint);
    if (!blueprint.ok) return blueprint;
    return { ok: true };
  } catch (error) {
    return surfaceFail("invalid_blueprint", `snapshot guard failed: ${String(error)}`);
  }
}

function checkSurfacePatchOp(op: unknown): SurfaceGuardResult {
  if (!isObject(op)) return surfaceFail("invalid_patch", "patch op must be an object", "patch");
  if (!["add", "remove", "replace", "move", "copy", "test"].includes(String(op.op))) {
    return surfaceFail("invalid_patch", `unknown patch operation ${String(op.op)}`, "patch.op");
  }
  if (typeof op.path !== "string" || surfacePointerSegments(op.path) === null) {
    return surfaceFail("invalid_patch", "patch.path must be an RFC 6901 pointer", "patch.path");
  }
  if (op.path === "" && op.op !== "test") {
    return surfaceFail("invalid_patch", "patch cannot replace or remove the snapshot root", "patch.path");
  }
  if (immutableSurfacePointer(op.path)) {
    return surfaceFail("invalid_patch", "patch cannot mutate immutable snapshot metadata", "patch.path");
  }
  if ((op.op === "move" || op.op === "copy") && (typeof op.from !== "string" || surfacePointerSegments(op.from) === null)) {
    return surfaceFail("invalid_patch", `${op.op} requires a safe from pointer`, "patch.from");
  }
  if ((op.op === "move" || op.op === "copy") && immutableSurfacePointer(op.from as string)) {
    return surfaceFail("invalid_patch", "patch cannot read immutable snapshot metadata", "patch.from");
  }
  if ((op.op === "move" || op.op === "copy") && op.from === "") {
    return surfaceFail("invalid_patch", "patch cannot read the snapshot root", "patch.from");
  }
  if ((op.op === "add" || op.op === "replace" || op.op === "test") && !("value" in op)) {
    return surfaceFail("invalid_patch", `${op.op} requires value`, "patch.value");
  }
  if ("value" in op && !isFiniteJson(op.value)) return surfaceFail("invalid_patch", "patch.value must be finite JSON", "patch.value");
  const forbidden = forbiddenSurfaceField(op.value);
  if (forbidden !== null) return surfaceFail("forbidden_executable_field", `forbidden executable field ${forbidden}`, "patch.value");
  return { ok: true };
}

export function validateSurfacePatchEvent(value: unknown): value is SurfacePatchEvent {
  return validateSurfacePatchEventResult(value).ok;
}

export function validateSurfacePatchEventResult(value: unknown): SurfaceGuardResult {
  try {
    if (!isObject(value)) return surfaceFail("invalid_patch", "patch event must be an object");
    if (value.kind !== "patch") return surfaceFail("invalid_version", "patch.kind must be patch", "kind");
    const version = surfaceVersion(value.protocol);
    if (!version.ok) return version;
    if (!surfaceIdentifier(value.surfaceId)) return surfaceFail("invalid_blueprint", "surfaceId is invalid", "surfaceId");
    if (!surfaceRevision(value.baseRevision) || !surfaceRevision(value.revision)) {
      return surfaceFail("invalid_revision", "patch revisions must be decimal u64 strings");
    }
    if (BigInt(value.revision as string) !== BigInt(value.baseRevision as string) + 1n) {
      return surfaceFail("revision_gap", "patch revision must be exactly baseRevision plus one");
    }
    if (!Array.isArray(value.patch)) return surfaceFail("invalid_patch", "patch must be an array", "patch");
    if (value.patch.length > SURFACE_LIMITS.maxPatchOperations) {
      return surfaceFail("resource_limit", "patch operation count exceeds the authority limit", "patch");
    }
    for (const op of value.patch) {
      const result = checkSurfacePatchOp(op);
      if (!result.ok) return result;
    }
    const forbidden = forbiddenSurfaceField(value);
    if (forbidden !== null) return surfaceFail("forbidden_executable_field", `forbidden executable field ${forbidden}`);
    const bytes = surfaceDocumentBytes(value);
    if (bytes === null || bytes > SURFACE_LIMITS.maxPatchBytes) {
      return surfaceFail("document_too_large", "patch exceeds the patch byte limit");
    }
    return { ok: true };
  } catch (error) {
    return surfaceFail("invalid_patch", `patch guard failed: ${String(error)}`);
  }
}

export function validateSurfaceMessage(value: unknown): value is SurfaceMessage {
  if (!isObject(value)) return false;
  return value.kind === "snapshot" ? validateSurfaceSnapshot(value) : validateSurfacePatchEvent(value);
}
