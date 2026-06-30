import { describe, it, expect } from "vitest";
import type { Component } from "vue";
import {
  registerVueWidget,
  registerModuleWidget,
  registerEsmWidget,
  resolveWidget,
  registeredTypes,
} from "./registry";
import type { WidgetModule } from "./widget-module";

const stub = {} as Component;

describe("widget registry", () => {
  it("registers and resolves a vue widget", () => {
    registerVueWidget("test.vue", () => stub);
    const reg = resolveWidget("test.vue");
    expect(reg?.kind).toBe("vue");
    if (reg?.kind === "vue") expect(reg.load()).toBe(stub);
  });

  it("registers and resolves a module widget", () => {
    const mod: WidgetModule = { mount() {} };
    registerModuleWidget("test.module", () => mod);
    const reg = resolveWidget("test.module");
    expect(reg?.kind).toBe("module");
    if (reg?.kind === "module") expect(reg.load()).toBe(mod);
  });

  it("returns undefined for an unknown type", () => {
    expect(resolveWidget("nope.unknown")).toBeUndefined();
  });

  it("lists registered types", () => {
    registerVueWidget("test.listed", () => stub);
    expect(registeredTypes()).toContain("test.listed");
  });

  it("registerEsmWidget loads via the injected importer", async () => {
    const mod: WidgetModule = { mount() {} };
    const importer = async () => ({ default: () => mod });
    registerEsmWidget("test.esm", "https://example.com/w.mjs", importer);
    const reg = resolveWidget("test.esm");
    expect(reg?.kind).toBe("module");
    if (reg?.kind === "module") {
      expect(await reg.load()).toBe(mod);
    }
  });
});
