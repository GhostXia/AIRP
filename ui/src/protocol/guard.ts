/**
 * Runtime structural guard for inbound Envelopes.
 *
 * TS types only protect code we write ourselves — a real Gateway / IPC feed is
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

import type { Body, Capability, Json, JsonPatch, PatchOpKind } from "./types";

export type GuardResult = { ok: true } | { ok: false; error: string };

const KNOWN_KINDS = new Set<Body["kind"]>([
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
]);

const KNOWN_OPS = new Set(["set", "patch"]);
const KNOWN_PATCH_OPS = new Set<PatchOpKind>([
  "add",
  "remove",
  "replace",
  "move",
  "copy",
  "test",
]);
const KNOWN_CAPS = new Set<Capability>([
  "read:memory",
  "write:memory",
  "read:worldbook",
  "read:state",
  "write:state",
  "call:tool",
]);
const KNOWN_ENTRY_KINDS = new Set(["builtin", "esm"]);
const KNOWN_LAYOUT_KINDS = new Set(["dock", "grid", "stack", "tabs"]);

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
