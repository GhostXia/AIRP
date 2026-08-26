import type { Json } from "../protocol/types";

export type JsonObject = Record<string, Json>;

export type ParsedObjectDraft =
  | { ok: true; value: JsonObject }
  | { ok: false; error: string };

function isJsonObject(value: Json): value is JsonObject {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function equalJson(left: Json, right: Json): boolean {
  if (Object.is(left, right)) return true;
  if (Array.isArray(left) || Array.isArray(right)) {
    return Array.isArray(left)
      && Array.isArray(right)
      && left.length === right.length
      && left.every((value, index) => equalJson(value, right[index]));
  }
  if (isJsonObject(left) || isJsonObject(right)) {
    if (!isJsonObject(left) || !isJsonObject(right)) return false;
    const leftKeys = Object.keys(left);
    const rightKeys = Object.keys(right);
    return leftKeys.length === rightKeys.length
      && leftKeys.every((key) => hasOwn(right, key) && equalJson(left[key], right[key]));
  }
  return false;
}

function hasOwn(value: object, key: string): boolean {
  return Object.prototype.hasOwnProperty.call(value, key);
}

function pointerSegment(key: string): string {
  return key.replace(/~/g, "~0").replace(/\//g, "~1");
}

export function parseObjectDraft(draft: string): ParsedObjectDraft {
  let value: Json;
  try {
    value = JSON.parse(draft) as Json;
  } catch (error) {
    return {
      ok: false,
      error: `JSON 解析错误：${error instanceof Error ? error.message : String(error)}`,
    };
  }
  if (!isJsonObject(value)) {
    return { ok: false, error: "角色状态必须是顶层 JSON object。" };
  }
  return { ok: true, value };
}

/** Build the Engine-supported top-level add/replace/remove patch only. */
export function topLevelObjectPatch(current: JsonObject, draft: JsonObject): Json[] {
  const patch: Json[] = [];
  for (const key of Object.keys(current)) {
    if (!hasOwn(draft, key)) patch.push({ op: "remove", path: `/${pointerSegment(key)}` });
  }
  for (const [key, value] of Object.entries(draft)) {
    if (!hasOwn(current, key)) {
      patch.push({ op: "add", path: `/${pointerSegment(key)}`, value });
    } else if (!equalJson(current[key], value)) {
      patch.push({ op: "replace", path: `/${pointerSegment(key)}`, value });
    }
  }
  return patch;
}

export function formatObjectDraft(value: JsonObject): string {
  return JSON.stringify(value, null, 2);
}

export function characterCount(value: string): number {
  return Array.from(value).length;
}

export function isAuthoritativeRevision(value: unknown): value is number {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0;
}

export function preservesDraftOnAuthorityRefresh(status: string): boolean {
  return status === "conflict" || status === "error";
}

export function editorIsReadOnly(readOnly: boolean | undefined, status: string): boolean {
  return Boolean(readOnly) || status === "saving";
}
