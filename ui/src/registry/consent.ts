/**
 * Capability consent gate (PLAN task E).
 *
 * The host secures itself: a third-party (esm) widget must be explicitly
 * approved by the user before it loads, and only the capabilities it declared
 * are granted. First-party (builtin) widgets need no consent. We do NOT audit
 * the widget's code — installing/approving it is the user's choice (SECURITY.md).
 *
 * Consent is bound to the widget's **identity** `{type, version, source}`, not
 * just its `type`: if a manifest later swaps its `source` or bumps `version`,
 * the old approval does NOT carry over and the user must re-consent.
 *
 * **Persistence**: grants are saved to `localStorage` (key `airp:consent-grants`)
 * so they survive page reloads. Call `initGrants()` once at app startup to
 * restore previously saved grants. The storage is injectable for testing.
 *
 * Reactive + unit-tested: grants live in a reactive Set so the host re-renders
 * when the user approves; the decision functions are covered in CI.
 */

import { reactive } from "vue";
import type { WidgetDef, Capability } from "./../protocol/types";

type ManifestId = Pick<WidgetDef, "type" | "version" | "entry">;
type ManifestCaps = ManifestId & Pick<WidgetDef, "capabilities">;

/** localStorage key for persisted grants. */
const STORAGE_KEY = "airp:consent-grants";

/** Injectable storage interface (for testing without real localStorage). */
export interface ConsentStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

const granted = reactive(new Set<string>());

/** The storage backend. Defaults to localStorage; override via `initGrants()`. */
let storage: ConsentStorage | null = null;

/** Identity a grant is bound to: type + version + (esm) source. */
function grantKey(m: ManifestId): string {
  const source = m.entry?.kind === "esm" ? m.entry.source ?? "" : "";
  return `${m.type}@${m.version}#${source}`;
}

/** Persist the current grant set to storage (if configured). */
function save(): void {
  if (!storage) return;
  try {
    storage.setItem(STORAGE_KEY, JSON.stringify([...granted]));
  } catch {
    // localStorage may be full or unavailable; degrade gracefully.
  }
}

/**
 * Initialize consent persistence. Loads previously saved grants from storage
 * and configures future grant/revoke/clear calls to auto-persist.
 *
 * Call once at app startup (e.g., in `main.ts` or `App.vue` onMounted).
 * If never called, consent is in-memory only (backward compatible).
 *
 * @param s - Storage backend. Defaults to `localStorage` if omitted.
 *            Pass a mock in tests to avoid touching the real localStorage.
 *            If omitted AND `localStorage` is unavailable (non-DOM / SSR),
 *            this is a no-op and consent stays in-memory — it never throws.
 */
export function initGrants(s?: ConsentStorage): void {
  const backend = s ?? (typeof localStorage !== "undefined" ? localStorage : null);
  if (!backend) return; // no storage available; stay in-memory (backward compatible)
  storage = backend;
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (raw) {
      const keys: unknown = JSON.parse(raw);
      if (Array.isArray(keys)) {
        for (const k of keys) {
          if (typeof k === "string") granted.add(k);
        }
      }
    }
  } catch {
    // Corrupted or unavailable; start fresh.
  }
}

export function isGranted(manifest: ManifestId): boolean {
  return granted.has(grantKey(manifest));
}
export function grant(manifest: ManifestId): void {
  granted.add(grantKey(manifest));
  save();
}
export function revoke(manifest: ManifestId): void {
  granted.delete(grantKey(manifest));
  save();
}
export function clearGrants(): void {
  granted.clear();
  save();
}

/** Third-party (esm) widgets need explicit consent; builtin ones do not. */
export function needsConsent(manifest: Pick<WidgetDef, "entry">): boolean {
  return manifest.entry?.kind === "esm";
}

/** May this widget mount now? Builtin: always. esm: only once this exact identity is granted. */
export function canMount(manifest: ManifestId): boolean {
  if (!needsConsent(manifest)) return true;
  return isGranted(manifest);
}

/** Capabilities effectively available to the widget (none until it may mount). */
export function effectiveCapabilities(manifest: ManifestCaps): Capability[] {
  if (!canMount(manifest)) return [];
  return manifest.capabilities ?? [];
}
