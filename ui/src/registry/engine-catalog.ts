import type { Capability, TrustedPluginDependency, WidgetDef } from "../protocol/types";
import { currentBearer } from "../protocol/desktop-session";
import { applyManifestMessage, clearManifests } from "./manifests";
import { clearEnginePlugins, initEnginePlugins } from "./plugin-deps";

export const WIDGET_HOST_API_MAJOR = 1;
export const WIDGET_CAPABILITIES: Capability[] = [
  "read:memory", "write:memory", "read:worldbook", "read:state", "write:state", "call:tool",
];

type CatalogState = "uninitialized" | "ready" | "unavailable";
let state: CatalogState = "uninitialized";

function hostMajor(value: unknown): number | null {
  if (value === undefined || value === "") return 1;
  if (typeof value !== "string") return null;
  const segments = value.split(".");
  if (segments.some((segment) => !/^(0|[1-9]\d*)$/.test(segment))) return null;
  const major = Number(segments[0]);
  return Number.isSafeInteger(major) && major > 0 ? major : null;
}

function trustedPlugins(value: unknown): TrustedPluginDependency[] | null {
  if (value === undefined) return [];
  if (!Array.isArray(value)) return null;
  const result: TrustedPluginDependency[] = [];
  for (const item of value) {
    if (!item || typeof item !== "object") return null;
    const record = item as Record<string, unknown>;
    if (typeof record.id !== "string" || record.id.length === 0 || record.id.length > 128
      || record.id.startsWith(".") || record.id.endsWith(".")
      || record.id.includes("/") || record.id.includes("\\")) return null;
    if (!(record.min_host_api === undefined || typeof record.min_host_api === "string")) return null;
    if (record.min_host_api !== undefined
      && (record.min_host_api.length === 0 || hostMajor(record.min_host_api) === null)) return null;
    result.push({ id: record.id, ...(record.min_host_api === undefined ? {} : { min_host_api: record.min_host_api }) });
  }
  return result;
}

function manifest(value: unknown): WidgetDef | null {
  if (!value || typeof value !== "object") return null;
  const record = value as Record<string, unknown>;
  if (typeof record.type !== "string" || record.type.length === 0) return null;
  if (typeof record.version !== "string" || record.version.length === 0) return null;
  if (record.title !== undefined && (typeof record.title !== "string" || record.title.length === 0)) return null;
  if (hostMajor(record.host_api) !== WIDGET_HOST_API_MAJOR) return null;
  if (!record.entry || typeof record.entry !== "object") return null;
  const entry = record.entry as Record<string, unknown>;
  if (entry.kind !== "builtin" && entry.kind !== "esm") return null;
  if (entry.kind === "esm") {
    if (typeof entry.source !== "string" || entry.sandbox !== true) return null;
    try {
      const source = new URL(entry.source, "http://airp.invalid");
      if (source.origin !== "http://airp.invalid" || entry.source !== source.pathname
        || source.search !== "" || source.hash !== "") return null;
    } catch {
      return null;
    }
  }
  const capabilities = record.capabilities === undefined ? [] : record.capabilities;
  if (!Array.isArray(capabilities)
    || capabilities.some((capability) => !WIDGET_CAPABILITIES.includes(capability as Capability))
    || new Set(capabilities).size !== capabilities.length) return null;
  const dependencies = trustedPlugins(record.trusted_plugins);
  if (!dependencies) return null;
  return {
    ...(record as unknown as WidgetDef),
    title: typeof record.title === "string" ? record.title : record.type,
    capabilities: capabilities as Capability[],
    trusted_plugins: dependencies,
    entry: {
      kind: entry.kind,
      ...(entry.source === undefined ? {} : { source: entry.source as string }),
      ...(entry.sandbox === undefined ? {} : { sandbox: entry.sandbox as boolean }),
    },
  };
}

export function applyEngineCatalog(payload: unknown): boolean {
  const reject = (): false => {
    clearManifests();
    clearEnginePlugins();
    state = "unavailable";
    return false;
  };
  if (!payload || typeof payload !== "object") return reject();
  const record = payload as Record<string, unknown>;
  if (record.version !== 1 || record.host_api_major !== WIDGET_HOST_API_MAJOR) return reject();
  const catalogCapabilities = record.capabilities;
  if (!Array.isArray(catalogCapabilities)
    || catalogCapabilities.length !== WIDGET_CAPABILITIES.length
    || WIDGET_CAPABILITIES.some((capability) => !catalogCapabilities.includes(capability))) return reject();
  if (!Array.isArray(record.manifests)) return reject();
  const parsed: WidgetDef[] = [];
  const types = new Set<string>();
  for (const value of record.manifests) {
    const item = manifest(value);
    if (!item || types.has(item.type)) return reject();
    types.add(item.type);
    parsed.push(item);
  }
  applyManifestMessage("set", parsed);
  state = "ready";
  return true;
}

function engineUrl(path: string, base?: string): URL {
  const origin = base ?? sessionStorage.getItem("airp_engine_url") ?? location.origin;
  return new URL(path, `${origin.replace(/\/+$/, "")}/`);
}

async function engineJson(path: string, fetchImpl: typeof fetch, base?: string): Promise<unknown> {
  const headers = new Headers({ Accept: "application/json" });
  const bearer = currentBearer();
  if (bearer) headers.set("Authorization", `Bearer ${bearer}`);
  const response = await fetchImpl(engineUrl(path, base), {
    headers,
    signal: AbortSignal.timeout(5_000),
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response.json();
}

export async function initEngineCatalog(fetchImpl: typeof fetch = fetch, base?: string): Promise<boolean> {
  clearManifests();
  clearEnginePlugins();
  state = "unavailable";
  try {
    const [catalog, plugins] = await Promise.all([
      engineJson("/v1/extensions/catalog", fetchImpl, base),
      engineJson("/v1/plugins", fetchImpl, base).catch(() => ({ plugins: [] })),
    ]);
    if (!applyEngineCatalog(catalog)) return false;
    initEnginePlugins(plugins);
    return true;
  } catch {
    clearManifests();
    clearEnginePlugins();
    return false;
  }
}

export function engineCatalogState(): CatalogState {
  return state;
}

export function resetEngineCatalog(): void {
  clearManifests();
  clearEnginePlugins();
  state = "uninitialized";
}
