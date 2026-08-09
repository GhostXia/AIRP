/**
 * Capability consent gate (issue #474).
 *
 * The host secures itself: a third-party (esm) widget must be explicitly
 * approved by the user before it loads, and only the capabilities the engine
 * granted are exposed. First-party (builtin) widgets need no consent. We do
 * not audit widget code — installing/approving it is the user's choice.
 *
 * The engine is the only authority and persists grants in extensions.json.
 * This module keeps a reactive, disposable mirror. A successful engine grant
 * snapshot (including an empty array) enables authoritative mode; engine
 * failure, malformed data, or identity mismatch is fail-closed. localStorage
 * is retained only for old unit-test/migration construction and is never read
 * by the production startup path.
 */

import { reactive } from "vue";
import type { WidgetDef, Capability } from "./../protocol/types";

type ManifestId = Pick<WidgetDef, "type" | "version" | "entry">;
type ManifestCaps = ManifestId & Pick<WidgetDef, "capabilities">;

export type GrantAction = "grant" | "revoke";

/** Engine `/v1/grants` / `/v1/extensions/grants` response entry. */
export interface ConsentGrant {
  id: string;
  type: string;
  version: string;
  source: string | null;
  digest: string;
  enabled: boolean;
  granted_capabilities: string[];
  granted_at: number | null;
}

/** Injectable engine client for tests and alternate shells. */
export interface ConsentAuthority {
  listGrants(): Promise<unknown>;
  updateGrant(id: string, action: GrantAction, capabilities?: Capability[]): Promise<unknown>;
}

/** Legacy storage interface retained for migration/unit-test fixtures only. */
export interface ConsentStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const STORAGE_KEY = "airp:consent-grants";
const granted = reactive(new Set<string>());
/** type -> engine grant views; multiple installed versions may share a type. */
const engineGrants = reactive(new Map<string, ConsentGrant[]>());
type EngineState = "uninitialized" | "ready" | "unavailable";
let engineState: EngineState = "uninitialized";
let engineAuthority: ConsentAuthority | null = null;
let storage: ConsentStorage | null = null;

/** Engine identity: id + type + version + source + digest (id is selected separately). */
function grantKey(m: ManifestId): string {
  const source = m.entry?.kind === "esm" ? m.entry.source ?? "" : "";
  return `${m.type}@${m.version}#${source}`;
}

function sourceOf(m: ManifestId): string {
  return m.entry?.kind === "esm" ? m.entry.source ?? "" : "";
}

/** Installed engine extensions use digest-pinned module URLs. */
function digestOf(m: ManifestId): string | null {
  const match = /^\/extensions\/([0-9a-f]{64})\/index\.js$/.exec(sourceOf(m));
  return match?.[1] ?? null;
}

/** Legacy cache persistence; never called while engineState is authoritative. */
function saveLegacy(): void {
  if (!storage || engineState !== "uninitialized") return;
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify([...granted]));
  } catch {
    // Storage may be unavailable/full; old test/migration path remains non-throwing.
  }
}

/**
 * Legacy local mirror initializer. Production startup calls initEngineGrants()
 * instead, so a browser cache can never authorize a widget there.
 */
export function initGrants(s?: ConsentStorage): void {
  if (engineState !== "uninitialized") return;
  const backend = s ?? (typeof localStorage !== "undefined" ? localStorage : null);
  if (!backend) return;
  storage = backend;
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return;
    const keys: unknown = JSON.parse(raw);
    if (Array.isArray(keys)) {
      for (const key of keys) {
        if (typeof key === "string") granted.add(key);
      }
    }
  } catch {
    // Corrupted or unavailable; start fresh.
  }
}

function normaliseGrant(value: unknown): ConsentGrant | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  if (typeof record.id !== "string" || record.id.length === 0) return null;
  if (typeof record.type !== "string" || record.type.length === 0) return null;
  if (typeof record.version !== "string" || record.version.length === 0) return null;
  if (!(record.source === null || typeof record.source === "string")) return null;
  if (typeof record.digest !== "string" || !/^[0-9a-f]{64}$/.test(record.digest)) return null;
  if (typeof record.enabled !== "boolean") return null;
  if (!Array.isArray(record.granted_capabilities)
    || record.granted_capabilities.some((cap) => typeof cap !== "string")) return null;
  if (!(record.granted_at === null || typeof record.granted_at === "number")) return null;
  return {
    id: record.id,
    type: record.type,
    version: record.version,
    source: record.source,
    digest: record.digest,
    enabled: record.enabled,
    granted_capabilities: [...record.granted_capabilities] as string[],
    granted_at: record.granted_at,
  };
}

/**
 * Inject a complete engine snapshot. Empty `grants` is a valid authoritative
 * result and deliberately does not fall back to localStorage.
 */
export function initGrantsFromEngine(payload: unknown): boolean {
  const list = Array.isArray(payload)
    ? payload
    : (payload && typeof payload === "object" && Array.isArray((payload as { grants?: unknown }).grants)
      ? (payload as { grants: unknown[] }).grants
      : null);
  if (!list) {
    markEngineUnavailable();
    return false;
  }
  const parsed: ConsentGrant[] = [];
  for (const value of list) {
    const record = normaliseGrant(value);
    if (!record) {
      markEngineUnavailable();
      return false;
    }
    parsed.push(record);
  }
  const ids = new Set<string>();
  engineGrants.clear();
  for (const record of parsed) {
    if (ids.has(record.id)) {
      markEngineUnavailable();
      return false;
    }
    ids.add(record.id);
    const records = engineGrants.get(record.type) ?? [];
    records.push(record);
    engineGrants.set(record.type, records);
  }
  granted.clear();
  engineState = "ready";
  return true;
}

/** Engine unavailable/invalid response: clear mirrors and fail closed. */
export function markEngineUnavailable(): void {
  engineGrants.clear();
  granted.clear();
  engineState = "unavailable";
}

export function configureEngineAuthority(authority: ConsentAuthority | null): void {
  engineAuthority = authority;
}

export function engineAuthorityState(): EngineState {
  return engineState;
}

export function hasEngineGrants(): boolean {
  return engineState === "ready";
}

/** Build the default same-origin engine client used by the standalone UI. */
function createFetchAuthority(): ConsentAuthority {
  const base = (): string => {
    if (typeof sessionStorage !== "undefined") {
      const configured = sessionStorage.getItem("airp_engine_url");
      if (configured) return configured.replace(/\/+$/, "");
    }
    return typeof location !== "undefined" ? location.origin : "";
  };
  const url = (path: string): string => {
    const origin = base();
    if (!origin) return path;
    try {
      return new URL(path, `${origin}/`).toString();
    } catch {
      return path;
    }
  };
  const headers = (extra: Record<string, string>): Record<string, string> => {
    const result = { ...extra };
    if (typeof sessionStorage !== "undefined") {
      const bearer = sessionStorage.getItem("airp_bearer");
      if (bearer) result.Authorization = `Bearer ${bearer}`;
    }
    return result;
  };
  return {
    async listGrants(): Promise<unknown> {
      const response = await fetch(url("/v1/extensions/grants"), {
        headers: headers({ Accept: "application/json" }),
      });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      return response.json();
    },
    async updateGrant(id, action, capabilities): Promise<unknown> {
      const body: { action: GrantAction; capabilities?: Capability[] } = { action };
      if (capabilities) body.capabilities = capabilities;
      const response = await fetch(url(`/v1/extensions/${encodeURIComponent(id)}/grants`), {
        method: "POST",
        headers: headers({ "Content-Type": "application/json", Accept: "application/json" }),
        body: JSON.stringify(body),
      });
      const payload: unknown = await response.json().catch(() => null);
      if (!response.ok) {
        const code = payload && typeof payload === "object"
          ? (payload as { error?: { code?: unknown } }).error?.code
          : undefined;
        throw new Error(typeof code === "string" ? code : `HTTP ${response.status}`);
      }
      return payload;
    },
  };
}

/** Load the engine snapshot before mounting the UI; failure is fail-closed. */
export async function initEngineGrants(authority = createFetchAuthority()): Promise<boolean> {
  configureEngineAuthority(authority);
  try {
    const payload = await authority.listGrants();
    return initGrantsFromEngine(payload);
  } catch {
    markEngineUnavailable();
    return false;
  }
}

function engineGrantFor(manifest: ManifestId): ConsentGrant | null {
  if (engineState !== "ready") return null;
  const digest = digestOf(manifest);
  if (!digest) return null;
  const records = engineGrants.get(manifest.type) ?? [];
  return records.find((record) => record.enabled
    && record.version === manifest.version
    && (record.source ?? "") === sourceOf(manifest)
    && record.digest === digest) ?? null;
}

export function isGranted(manifest: ManifestId): boolean {
  if (!needsConsent(manifest)) return false;
  if (engineState === "ready") {
    const record = engineGrantFor(manifest);
    return Boolean(record && record.granted_capabilities.length > 0);
  }
  if (engineState === "unavailable") return false;
  return granted.has(grantKey(manifest));
}

/** Request an engine mutation; only a successful response updates the mirror. */
export function grant(manifest: ManifestCaps): Promise<ConsentGrant | null> {
  if (!needsConsent(manifest)) return Promise.resolve(null);
  if (engineState === "ready") {
    const current = engineGrantFor(manifest);
    if (!current || !engineAuthority) {
      return Promise.reject(new Error("engine grant authority unavailable for widget identity"));
    }
    return engineAuthority.updateGrant(current.id, "grant", manifest.capabilities ?? [])
      .then((payload) => {
        const updated = normaliseGrant(payload);
        if (!updated || updated.id !== current.id || updated.type !== manifest.type
          || updated.version !== manifest.version
          || (updated.source ?? "") !== sourceOf(manifest)
          || updated.digest !== digestOf(manifest)) {
          throw new Error("engine returned an invalid grant record");
        }
        const records = engineGrants.get(updated.type) ?? [];
        const index = records.findIndex((record) => record.id === updated.id);
        if (index >= 0) records[index] = updated;
        else records.push(updated);
        engineGrants.set(updated.type, records);
        return updated;
      });
  }
  if (engineState === "unavailable") {
    return Promise.reject(new Error("engine grant authority unavailable"));
  }
  granted.add(grantKey(manifest));
  saveLegacy();
  return Promise.resolve(null);
}

export function revoke(manifest: ManifestId): Promise<ConsentGrant | null> {
  if (engineState === "ready") {
    const current = engineGrantFor(manifest);
    if (!current || !engineAuthority) {
      return Promise.reject(new Error("engine grant authority unavailable for widget identity"));
    }
    return engineAuthority.updateGrant(current.id, "revoke")
      .then((payload) => {
        const updated = normaliseGrant(payload);
        if (!updated || updated.id !== current.id || updated.type !== manifest.type
          || updated.version !== manifest.version
          || (updated.source ?? "") !== sourceOf(manifest)
          || updated.digest !== digestOf(manifest)) {
          throw new Error("engine returned an invalid grant record");
        }
        const records = engineGrants.get(updated.type) ?? [];
        const index = records.findIndex((record) => record.id === updated.id);
        if (index >= 0) records[index] = updated;
        else records.push(updated);
        engineGrants.set(updated.type, records);
        return updated;
      });
  }
  if (engineState === "unavailable") {
    return Promise.reject(new Error("engine grant authority unavailable"));
  }
  granted.delete(grantKey(manifest));
  saveLegacy();
  return Promise.resolve(null);
}

export function clearGrants(): void {
  granted.clear();
  engineGrants.clear();
  if (engineState === "uninitialized") saveLegacy();
  engineState = "uninitialized";
  engineAuthority = null;
  storage = null;
}

/** Third-party (esm) widgets need explicit consent; builtin ones do not. */
export function needsConsent(manifest: Pick<WidgetDef, "entry">): boolean {
  return manifest.entry?.kind === "esm";
}

/** May this widget mount now? Builtin: always; esm: exact engine grant identity. */
export function canMount(manifest: ManifestId): boolean {
  if (!needsConsent(manifest)) return true;
  return isGranted(manifest);
}

/** Capabilities effectively available to the widget (none until it may mount). */
export function effectiveCapabilities(manifest: ManifestCaps): Capability[] {
  if (!canMount(manifest)) return [];
  if (engineState === "ready") {
    const record = engineGrantFor(manifest);
    const declared = manifest.capabilities ?? [];
    return record
      ? record.granted_capabilities.filter((cap): cap is Capability => declared.includes(cap as Capability))
      : [];
  }
  return manifest.capabilities ?? [];
}

export { STORAGE_KEY };
