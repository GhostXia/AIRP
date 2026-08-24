import { afterEach, describe, expect, it } from "vitest";
import { clearEnginePlugins, initEnginePlugins, missingDependencies, versionAtLeast } from "./plugin-deps";

afterEach(clearEnginePlugins);

describe("trusted plugin hints", () => {
  it("distinguishes missing, stopped, old, and available dependencies", () => {
    initEnginePlugins({ plugins: [
      { id: "com.example.stopped", host_api: "1", status: "stopped" },
      { id: "com.example.old", host_api: "1.2", status: "running" },
      { id: "com.example.ready", host_api: "2", status: "running" },
    ] });
    expect(missingDependencies({ trusted_plugins: [
      { id: "com.example.missing" },
      { id: "com.example.stopped" },
      { id: "com.example.old", min_host_api: "1.3" },
      { id: "com.example.ready", min_host_api: "2" },
    ] })).toEqual([
      { id: "com.example.missing", reason: "not-installed" },
      { id: "com.example.stopped", reason: "stopped" },
      { id: "com.example.old", min_host_api: "1.3", reason: "version-too-low" },
    ]);
  });

  it.each([
    ["1", "1", true], ["1.2", "1", true], ["2", "1.9", true],
    ["1", "2", false], ["1.2", "1.3", false], ["x", "1", false],
    ["1e2", "1", false], [" 1", "1", false], ["1", "0x10", false],
    ["999999999999999999999999999999", "1000000000000000000000000000000", false],
  ])("compares %s against %s", (actual, minimum, expected) => {
    expect(versionAtLeast(actual, minimum)).toBe(expected);
  });

  it("fails closed to not-installed when the plugin response is invalid", () => {
    expect(initEnginePlugins({ nope: [] })).toBe(false);
    expect(missingDependencies({ trusted_plugins: [{ id: "com.example.tts" }] }))
      .toEqual([{ id: "com.example.tts", reason: "not-installed" }]);
  });
});
