import type { TrustedPluginDependency, WidgetDef } from "../protocol/types";

export interface EnginePluginView {
  id: string;
  version: string;
  host_api: string;
  status: string;
}

export interface MissingPluginDependency extends TrustedPluginDependency {
  reason: "not-installed" | "stopped" | "version-too-low";
}

const installed = new Map<string, EnginePluginView>();

export function clearEnginePlugins(): void {
  installed.clear();
}

export function initEnginePlugins(payload: unknown): boolean {
  clearEnginePlugins();
  const list = payload && typeof payload === "object"
    ? (payload as { plugins?: unknown }).plugins
    : null;
  if (!Array.isArray(list)) return false;
  for (const value of list) {
    if (!value || typeof value !== "object") continue;
    const item = value as Record<string, unknown>;
    if (typeof item.id !== "string" || item.id.length === 0) continue;
    installed.set(item.id, {
      id: item.id,
      version: typeof item.version === "string" ? item.version : "",
      host_api: typeof item.host_api === "string" ? item.host_api : "",
      status: typeof item.status === "string" ? item.status : "stopped",
    });
  }
  return true;
}

export function versionAtLeast(actual: string, minimum?: string): boolean {
  if (!minimum) return true;
  const left = actual.split(".");
  const right = minimum.split(".");
  const length = Math.max(left.length, right.length);
  for (let index = 0; index < length; index += 1) {
    const a = left[index] ?? "0";
    const b = right[index] ?? "0";
    if (!/^\d+$/.test(a) || !/^\d+$/.test(b)) return false;
    // BigInt keeps arbitrarily long dirty segments exact; Number would round
    // or become Infinity and could incorrectly satisfy a minimum version.
    const x = BigInt(a);
    const y = BigInt(b);
    if (x !== y) return x > y;
  }
  return true;
}

export function missingDependencies(manifest: Pick<WidgetDef, "trusted_plugins">): MissingPluginDependency[] {
  const missing: MissingPluginDependency[] = [];
  for (const dependency of manifest.trusted_plugins ?? []) {
    const plugin = installed.get(dependency.id);
    if (!plugin) missing.push({ ...dependency, reason: "not-installed" });
    else if (plugin.status !== "running") missing.push({ ...dependency, reason: "stopped" });
    else if (!versionAtLeast(plugin.host_api, dependency.min_host_api)) {
      missing.push({ ...dependency, reason: "version-too-low" });
    }
  }
  return missing;
}
