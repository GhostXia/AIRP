/**
 * Reactive state store. Holds one value per protocol "scope" (a widget instance
 * id, "session", or dotted path). Updated from downstream `state` messages:
 * `set` replaces a scope, `patch` applies an RFC 6902 JSON Patch in place.
 *
 * `applyJsonPatch` implements the full op set (add/remove/replace/move/copy/test)
 * and is reused for `blueprint op:patch`. Note: application is in-place and not
 * transactional — a failing `test` throws after earlier ops have applied.
 *
 * Performance contract: the full history lives in the Gateway. This store only
 * holds the live windowed slice the UI currently renders.
 */

import { reactive } from "vue";
import type { Json, JsonPatch, PatchOp } from "../protocol/types";

export const stateStore = reactive<Record<string, Json>>({});

export function setState(scope: string, value: Json): void {
  stateStore[scope] = value;
}

export function patchState(scope: string, patch: JsonPatch): void {
  if (stateStore[scope] == null || typeof stateStore[scope] !== "object") {
    stateStore[scope] = {};
  }
  applyJsonPatch(stateStore[scope], patch);
}

/** Apply an RFC 6902 JSON Patch document to `root` in place. */
export function applyJsonPatch(root: Json, patch: JsonPatch): void {
  for (const op of patch) applyOp(root, op);
}

function pointerTokens(path: string): string[] {
  return path
    .split("/")
    .slice(1)
    .map((t) => t.replace(/~1/g, "/").replace(/~0/g, "~"));
}

function getAtPointer(root: Json, toks: string[]): Json | undefined {
  let cur: unknown = root;
  for (const t of toks) {
    if (cur == null || typeof cur !== "object") return undefined;
    cur = (cur as Record<string, unknown>)[t];
  }
  return cur as Json | undefined;
}

function resolveParent(root: Json, toks: string[]): { parent: any; key: string } | null {
  let parent: any = root;
  for (let i = 0; i < toks.length - 1; i++) {
    parent = parent?.[toks[i]];
    if (parent == null) return null;
  }
  return { parent, key: toks[toks.length - 1] };
}

function addAt(parent: any, key: string, value: unknown): void {
  if (Array.isArray(parent)) {
    if (key === "-") parent.push(value);
    else parent.splice(Number(key), 0, value);
  } else {
    parent[key] = value;
  }
}

function removeAt(parent: any, key: string): void {
  if (Array.isArray(parent)) parent.splice(Number(key), 1);
  else delete parent[key];
}

function clone<T>(v: T): T {
  return v == null ? v : (structuredClone(v) as T);
}

// Full RFC 6902: add / remove / replace / move / copy / test.
function applyOp(root: Json, op: PatchOp): void {
  const toks = pointerTokens(op.path);
  if (toks.length === 0) return;
  const target = resolveParent(root, toks);
  if (!target) return;
  const { parent, key } = target;

  switch (op.op) {
    case "add":
      addAt(parent, key, op.value);
      break;
    case "replace":
      parent[key] = op.value;
      break;
    case "remove":
      removeAt(parent, key);
      break;
    case "copy": {
      const value = getAtPointer(root, pointerTokens(op.from ?? ""));
      addAt(parent, key, clone(value));
      break;
    }
    case "move": {
      const fromToks = pointerTokens(op.from ?? "");
      const value = getAtPointer(root, fromToks);
      const from = resolveParent(root, fromToks);
      if (from) removeAt(from.parent, from.key);
      addAt(parent, key, value);
      break;
    }
    case "test": {
      const actual = getAtPointer(root, toks);
      if (JSON.stringify(actual) !== JSON.stringify(op.value)) {
        throw new Error(`JSON Patch test failed at ${op.path}`);
      }
      break;
    }
  }
}
