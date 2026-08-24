/**
 * Manifest registry — maps a widget `type` to its published manifest (WidgetDef).
 *
 * Lets the UI know a widget's `entry` (how to load it), props/state schema, and
 * requested capabilities. Production ESM entries remain opaque-frame-only;
 * an explicit importer can register them in-process for unit-test fixtures.
 *
 * Fed over the wire by a downstream `manifest` body (see ManifestMsg): `op:"set"`
 * replaces the whole set (call {@link clearManifests} first); `op:"patch"`
 * upserts the given subset by `type` (the incremental form for manifests — it is
 * an upsert of the `manifests` array, not an RFC 6902 JSON Patch).
 */

import type { WidgetDef, SetOrPatch } from "../protocol/types";
import { registerEsmWidget, unregisterWidget } from "./registry";
const manifests = new Map<string, WidgetDef>();
const fixtureEsmRegistered = new Set<string>();

export function registerManifest(manifest: WidgetDef): void {
  manifests.set(manifest.type, manifest);
}

export function getManifest(type: string): WidgetDef | undefined {
  return manifests.get(type);
}

export function allManifests(): WidgetDef[] {
  return [...manifests.values()];
}

/**
 * Drop every recorded manifest AND unregister the esm widgets they brought in
 * (builtins, registered elsewhere, are untouched). Used by `manifest op:"set"`
 * for a full reset.
 */
export function clearManifests(): void {
  for (const type of fixtureEsmRegistered) unregisterWidget(type);
  fixtureEsmRegistered.clear();
  manifests.clear();
}

/**
 * Record manifests. An explicit `importer` registers ESM widgets only as a
 * test-fixture seam; production catalog startup never supplies one.
 */
export function registerEsmWidgetsFromManifests(
  list: WidgetDef[],
  importer?: (s: string) => Promise<unknown>,
): void {
  for (const manifest of list) {
    registerManifest(manifest);
    // Explicit importers are a test/fixture seam only. Engine catalog startup
    // never supplies one, so production third-party ESM cannot enter the host
    // process and is loaded solely by the opaque sandbox frame.
    if (importer && manifest.entry?.kind === "esm" && manifest.entry.source) {
      registerEsmWidget(manifest.type, manifest.entry.source, importer);
      fixtureEsmRegistered.add(manifest.type);
    }
  }
}

/**
 * Apply a downstream `manifest` body: `op:"set"` clears then registers (full
 * replacement); `op:"patch"` upserts the subset by `type` (incremental). For
 * manifests, "patch" means an upsert of the `manifests` array — not an RFC 6902
 * JSON Patch. `importer` is injectable for testing.
 */
export function applyManifestMessage(
  op: SetOrPatch,
  list: WidgetDef[],
  importer?: (s: string) => Promise<unknown>,
): void {
  if (op === "set") clearManifests();
  registerEsmWidgetsFromManifests(list, importer);
}
