import { afterEach, describe, expect, it, vi } from "vitest";
import { getManifest } from "./manifests";
import { resolveWidget } from "./registry";
import {
  WIDGET_CAPABILITIES,
  applyEngineCatalog,
  engineCatalogState,
  initEngineCatalog,
  resetEngineCatalog,
} from "./engine-catalog";

function catalog(manifests: unknown[] = [{
  type: "acme.weather",
  version: "1.0.0",
  capabilities: ["read:state"],
  host_api: "1.2",
  trusted_plugins: [{ id: "com.example.weather", min_host_api: "1" }],
  entry: { kind: "esm", source: "/extensions/aabb/index.js", sandbox: true },
}]): Record<string, unknown> {
  return {
    version: 1,
    host_api_major: 1,
    capabilities: [...WIDGET_CAPABILITIES],
    manifests,
  };
}

afterEach(() => {
  resetEngineCatalog();
  vi.unstubAllGlobals();
});

describe("Engine extension catalog", () => {
  it("records a valid catalog without importing third-party ESM into the host", () => {
    expect(applyEngineCatalog(catalog())).toBe(true);
    expect(engineCatalogState()).toBe("ready");
    expect(getManifest("acme.weather")?.title).toBe("acme.weather");
    expect(resolveWidget("acme.weather")).toBeUndefined();
  });

  it.each([
    ["future host contract", { ...catalog(), host_api_major: 2 }],
    ["unknown capability contract", { ...catalog(), capabilities: [...WIDGET_CAPABILITIES.slice(1), "admin:all"] }],
    ["future widget host contract", catalog([{ type: "acme.bad", version: "1", title: "bad", host_api: "2", entry: { kind: "builtin" } }])],
    ["unsandboxed ESM", catalog([{ type: "acme.bad", version: "1", title: "bad", entry: { kind: "esm", source: "/x.js" } }])],
    ["protocol-relative ESM", catalog([{ type: "acme.bad", version: "1", title: "bad", entry: { kind: "esm", source: "//evil.example/x.js", sandbox: true } }])],
    ["ESM URL with a query", catalog([{ type: "acme.bad", version: "1", title: "bad", entry: { kind: "esm", source: "/x.js?mutable=1", sandbox: true } }])],
    ["unknown widget capability", catalog([{ type: "acme.bad", version: "1", title: "bad", capabilities: ["admin:all"], entry: { kind: "builtin" } }])],
    ["duplicate widget type", catalog([
      { type: "acme.same", version: "1", title: "one", entry: { kind: "builtin" } },
      { type: "acme.same", version: "2", title: "two", entry: { kind: "builtin" } },
    ])],
    ["invalid trusted plugin", catalog([{ type: "acme.bad", version: "1", title: "bad", trusted_plugins: [{ id: "../evil" }], entry: { kind: "builtin" } }])],
  ])("fails closed for %s", (_name, payload) => {
    expect(applyEngineCatalog(catalog())).toBe(true);
    expect(applyEngineCatalog(payload)).toBe(false);
    expect(engineCatalogState()).toBe("unavailable");
    expect(getManifest("acme.weather")).toBeUndefined();
  });

  it("fetches catalog and plugins with the desktop bearer", async () => {
    vi.stubGlobal("sessionStorage", {
      getItem: (key: string) => key === "airp_bearer" ? "secret-token" : null,
    });
    const seen: Array<{ path: string; auth: string | null }> = [];
    const fetchImpl = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input));
      seen.push({ path: url.pathname, auth: new Headers(init?.headers).get("Authorization") });
      const body = url.pathname.endsWith("/catalog") ? catalog() : { plugins: [] };
      return new Response(JSON.stringify(body), { status: 200, headers: { "Content-Type": "application/json" } });
    }) as unknown as typeof fetch;

    expect(await initEngineCatalog(fetchImpl, "http://127.0.0.1:8765")).toBe(true);
    expect(seen).toEqual([
      { path: "/v1/extensions/catalog", auth: "Bearer secret-token" },
      { path: "/v1/plugins", auth: "Bearer secret-token" },
    ]);
  });

  it("keeps the catalog fail-closed when Engine is unreachable", async () => {
    expect(applyEngineCatalog(catalog())).toBe(true);
    const fetchImpl = vi.fn(async () => { throw new Error("offline"); }) as unknown as typeof fetch;
    expect(await initEngineCatalog(fetchImpl, "http://127.0.0.1:8765")).toBe(false);
    expect(engineCatalogState()).toBe("unavailable");
    expect(getManifest("acme.weather")).toBeUndefined();
  });
});
