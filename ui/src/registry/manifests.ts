/**
 * Manifest registry — maps a widget `type` to its published manifest (WidgetDef).
 *
 * Lets the UI know a widget's `entry` (how to load it), props/state schema, and
 * requested capabilities. `registerEsmWidgetsFromManifests` wires any
 * `entry.kind === "esm"` manifest into the component registry so third-party
 * widgets load dynamically from their `source`.
 *
 * Fed over the wire by a downstream `manifest` body (see ManifestMsg): `op:"set"`
 * replaces the whole set (call {@link clearManifests} first); `op:"patch"`
 * upserts the given subset by `type` (the incremental form for manifests — it is
 * an upsert of the `manifests` array, not an RFC 6902 JSON Patch).
 */

import type { WidgetDef, SetOrPatch } from "../protocol/types";
import { registerEsmWidget, unregisterWidget } from "./registry";

const manifests = new Map<string, WidgetDef>();
// Types this module registered into the component registry (esm widgets only),
// so a `set` (full replacement) can drop the stale ones without touching builtins.
const esmRegistered = new Set<string>();

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
  for (const type of esmRegistered) unregisterWidget(type);
  esmRegistered.clear();
  manifests.clear();
}

/**
 * Record manifests and auto-register their esm widgets into the component
 * registry. `importer` is injectable for testing.
 */
export function registerEsmWidgetsFromManifests(
  list: WidgetDef[],
  importer?: (s: string) => Promise<unknown>,
): void {
  for (const manifest of list) {
    registerManifest(manifest);
    if (manifest.entry?.kind === "esm" && manifest.entry.source) {
      registerEsmWidget(manifest.type, manifest.entry.source, importer);
      esmRegistered.add(manifest.type);
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
