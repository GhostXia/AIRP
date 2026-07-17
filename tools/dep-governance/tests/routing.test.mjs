// Tests for tools/dep-governance/audit-routing.mjs (#192).
//
// Run with: node --test tools/dep-governance/tests/routing.test.mjs
//
// These tests are hermetic: they use the fixtures in
// tools/dep-governance/fixtures/ and do NOT touch the real repo's Cargo.lock
// or package-lock.json. This keeps them stable across dependency upgrades.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  parseSemver,
  compareSemver,
  isPrerelease,
  normalizeLicense,
  splitLicenseExpression,
  classifyInventory,
  classifyUpgrade,
  isSensitive,
  makeDedupKey,
  validateConfig,
} from "../audit-routing.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CONFIG_PATH = path.resolve(__dirname, "..", "audit-routing.config.json");
const FIXTURE_PATH = path.resolve(__dirname, "..", "fixtures", "routing-samples.json");

const config = validateConfig(JSON.parse(fs.readFileSync(CONFIG_PATH, "utf8")));

// ---------------------------------------------------------------------------
// parseSemver
// ---------------------------------------------------------------------------

test("parseSemver: standard forms", () => {
  assert.deepEqual(parseSemver("1.2.3"), {
    major: 1, minor: 2, patch: 3, prerelease: null, build: null, raw: "1.2.3",
  });
  assert.deepEqual(parseSemver("0.0.0"), {
    major: 0, minor: 0, patch: 0, prerelease: null, build: null, raw: "0.0.0",
  });
  assert.deepEqual(parseSemver("1.0.0-alpha"), {
    major: 1, minor: 0, patch: 0, prerelease: "alpha", build: null, raw: "1.0.0-alpha",
  });
  assert.deepEqual(parseSemver("2.0.0-beta.1+build.42"), {
    major: 2, minor: 0, patch: 0, prerelease: "beta.1", build: "build.42", raw: "2.0.0-beta.1+build.42",
  });
});

test("parseSemver: loose major.minor form", () => {
  // Some manifests pin only major.minor; the lockfile always has full semver.
  // We accept the loose form by treating missing patch as 0.
  const r = parseSemver("1.7");
  assert.equal(r?.major, 1);
  assert.equal(r?.minor, 7);
  assert.equal(r?.patch, 0);
});

test("parseSemver: rejects garbage", () => {
  assert.equal(parseSemver("latest"), null);
  assert.equal(parseSemver(""), null);
  assert.equal(parseSemver(null), null);
  assert.equal(parseSemver("1.2.3.4"), null); // 4-part is not semver
  assert.equal(parseSemver("v1.2.3"), null); // leading v not accepted
});

// ---------------------------------------------------------------------------
// compareSemver
// ---------------------------------------------------------------------------

test("compareSemver: basic precedence", () => {
  assert.ok(compareSemver(parseSemver("1.0.0"), parseSemver("2.0.0")) < 0);
  assert.ok(compareSemver(parseSemver("2.0.0"), parseSemver("2.0.0")) === 0);
  assert.ok(compareSemver(parseSemver("2.0.1"), parseSemver("2.0.0")) > 0);
  assert.ok(compareSemver(parseSemver("1.2.3"), parseSemver("1.2.4")) < 0);
});

test("compareSemver: prerelease has lower precedence", () => {
  assert.ok(compareSemver(parseSemver("1.0.0-alpha"), parseSemver("1.0.0")) < 0);
  assert.ok(compareSemver(parseSemver("1.0.0"), parseSemver("1.0.0-alpha")) > 0);
  // alpha < beta for same major.minor.patch
  assert.ok(compareSemver(parseSemver("1.0.0-alpha"), parseSemver("1.0.0-beta")) < 0);
  // numeric prerelease fields compared numerically
  assert.ok(compareSemver(parseSemver("1.0.0-alpha.1"), parseSemver("1.0.0-alpha.2")) < 0);
  assert.ok(compareSemver(parseSemver("1.0.0-alpha.10"), parseSemver("1.0.0-alpha.9")) > 0);
  // numeric < alphanumeric
  assert.ok(compareSemver(parseSemver("1.0.0-1"), parseSemver("1.0.0-alpha")) < 0);
});

test("compareSemver: build metadata ignored", () => {
  assert.equal(
    compareSemver(parseSemver("1.0.0+build.1"), parseSemver("1.0.0+build.2")),
    0,
  );
});

test("isPrerelease", () => {
  assert.equal(isPrerelease(parseSemver("1.0.0")), false);
  assert.equal(isPrerelease(parseSemver("1.0.0-alpha")), true);
  assert.equal(isPrerelease(parseSemver("1.0.0+build")), false); // build only, no pre
});

// ---------------------------------------------------------------------------
// License helpers
// ---------------------------------------------------------------------------

test("normalizeLicense: trims and collapses whitespace", () => {
  assert.equal(normalizeLicense("  MIT  "), "MIT");
  assert.equal(normalizeLicense("MIT  OR  Apache-2.0"), "MIT OR Apache-2.0");
  assert.equal(normalizeLicense(null), "");
  assert.equal(normalizeLicense(undefined), "");
  assert.equal(normalizeLicense(""), "");
});

test("normalizeLicense: rewrites legacy '/' OR-separator to ' OR '", () => {
  // SPDX 1.x used "/" as OR; SPDX 2.1+ uses "OR". Cargo accepts both.
  assert.equal(normalizeLicense("BSD-3-Clause/MIT"), "BSD-3-Clause OR MIT");
  assert.equal(normalizeLicense("Apache-2.0 / MIT"), "Apache-2.0 OR MIT");
  assert.equal(normalizeLicense("MIT/Apache-2.0"), "MIT OR Apache-2.0");
  // No SPDX license id contains "/", so this is unambiguous.
  assert.equal(normalizeLicense("MIT"), "MIT");
});

test("splitLicenseExpression: splits on OR/AND/WITH", () => {
  assert.deepEqual(splitLicenseExpression("MIT OR Apache-2.0"), ["MIT", "Apache-2.0"]);
  assert.deepEqual(splitLicenseExpression("MIT AND Apache-2.0"), ["MIT", "Apache-2.0"]);
  assert.deepEqual(splitLicenseExpression("Apache-2.0 WITH LLVM-exception"), ["Apache-2.0", "LLVM-exception"]);
  assert.deepEqual(splitLicenseExpression("MIT"), ["MIT"]);
  assert.deepEqual(splitLicenseExpression(""), []);
});

// ---------------------------------------------------------------------------
// classifyInventory
// ---------------------------------------------------------------------------

test("classifyInventory: first-party auto-passes", () => {
  const r = classifyInventory(
    { name: "airp-core", version: "0.1.0", ecosystem: "cargo", license: "MIT OR Apache-2.0", scope: "runtime", tier: "first-party" },
    config,
  );
  assert.equal(r.class, "auto-pass");
  assert.match(r.reason, /first-party/);
});

test("classifyInventory: permissive license auto-passes", () => {
  for (const lic of ["MIT", "Apache-2.0", "BSD-3-Clause", "ISC", "Zlib"]) {
    const r = classifyInventory(
      { name: "x", version: "1", ecosystem: "cargo", license: lic, scope: "runtime", tier: "third-party" },
      config,
    );
    assert.equal(r.class, "auto-pass", `expected auto-pass for ${lic}`);
  }
});

test("classifyInventory: MIT OR Apache-2.0 expression auto-passes", () => {
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "MIT OR Apache-2.0", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "auto-pass");
});

test("classifyInventory: legacy '/' OR-separator auto-passes when both permissive", () => {
  // Cargo accepts SPDX 1.x "/" form (e.g. "BSD-3-Clause/MIT"); this must be
  // treated identically to "BSD-3-Clause OR MIT".
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "BSD-3-Clause/MIT", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "auto-pass");
  assert.match(r.reason, /all components permissive/);
});

test("classifyInventory: unknown license requires audit", () => {
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: null, scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "audit-required");
  assert.match(r.reason, /empty or null/);
});

test("classifyInventory: unlisted license requires audit (no silent auto-pass)", () => {
  // A license string the config doesn't recognize should NOT silently auto-pass.
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "WTFPL", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "audit-required");
  assert.match(r.reason, /not in any configured tier/);
});

test("classifyInventory: GPL-3.0 in runtime scope blocks", () => {
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "GPL-3.0", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "block");
  assert.match(r.reason, /GPL-3.0/);
});

test("classifyInventory: GPL-3.0 in dev scope downgraded to audit-required", () => {
  // Strong copyleft in dev-only scope doesn't ship in the binary, so it
  // doesn't block release; but it still needs an audit to confirm file-level
  // isolation (dev-deps must not leak into runtime).
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "GPL-3.0", scope: "dev", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "audit-required");
  assert.match(r.reason, /dev scope/);
});

test("classifyInventory: AGPL-3.0 in runtime blocks", () => {
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "npm", license: "AGPL-3.0", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "block");
});

test("classifyInventory: MPL-2.0 requires audit (weak copyleft)", () => {
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "MPL-2.0", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "audit-required");
  assert.match(r.reason, /MPL-2.0/);
});

// ---------------------------------------------------------------------------
// isSensitive
// ---------------------------------------------------------------------------

test("isSensitive: name pattern match", () => {
  assert.equal(isSensitive("sha2", [], config), true);
  assert.equal(isSensitive("tokio", [], config), true);
  assert.equal(isSensitive("rustls", [], config), true);
  assert.equal(isSensitive("serde", [], config), false);
});

test("isSensitive: area tag match", () => {
  assert.equal(isSensitive("fast-hasher", ["cryptography"], config), true);
  assert.equal(isSensitive("fast-hasher", [], config), false);
  assert.equal(isSensitive("anything", ["network"], config), true);
});

test("isSensitive: case-insensitive name match", () => {
  assert.equal(isSensitive("SHA2", [], config), true);
  assert.equal(isSensitive("Tokio", [], config), true);
});

// ---------------------------------------------------------------------------
// classifyUpgrade: load all fixture samples and assert expected class/routing.
// This is the #192 acceptance #1 proof.
// ---------------------------------------------------------------------------

test("classifyUpgrade: all fixture samples match expected class and routing", () => {
  const fixture = JSON.parse(fs.readFileSync(FIXTURE_PATH, "utf8"));
  const samples = fixture.samples;
  assert.ok(samples.length >= 12, `expected >=12 samples, got ${samples.length}`);

  const seenClasses = new Set();
  for (const s of samples) {
    const decision = classifyUpgrade(
      s.current,
      s.target,
      { name: s.name, ecosystem: s.ecosystem, areaTags: s.areaTags ?? [] },
      config,
    );
    seenClasses.add(decision.class);
    assert.equal(
      decision.class,
      s.expected_class,
      `${s.id}: class ${decision.class} !== expected ${s.expected_class} (reason: ${decision.reason})`,
    );
    assert.equal(
      decision.routing,
      s.expected_routing,
      `${s.id}: routing ${decision.routing} !== expected ${s.expected_routing}`,
    );
  }

  // The five routing classes from issue #192 acceptance #1 must all appear.
  for (const required of ["patch", "minor", "major", "0x-minor", "prerelease"]) {
    assert.ok(seenClasses.has(required), `fixture must exercise class ${required}`);
  }
  // The patch-sensitive override must also appear.
  assert.ok(seenClasses.has("patch-sensitive"), "fixture must exercise patch-sensitive override");
});

test("classifyUpgrade: prerelease target is never auto-adopted", () => {
  // Even a "downgrade" to a prerelease is classified as prerelease (skip),
  // because the prerelease check fires before the bump-class logic.
  const d = classifyUpgrade(
    "2.0.0",
    "1.0.0-alpha",
    { name: "x", ecosystem: "cargo" },
    config,
  );
  assert.equal(d.class, "prerelease");
  assert.equal(d.routing, "skip");
});

test("classifyUpgrade: dedup key excludes class (re-classification updates existing issue)", () => {
  const d1 = classifyUpgrade("1.0.0", "1.0.1", { name: "serde", ecosystem: "cargo" }, config);
  const d2 = classifyUpgrade("1.0.0", "1.0.1", { name: "serde", ecosystem: "cargo", areaTags: ["cryptography"] }, config);
  // d1 is patch (auto-pr), d2 is patch-sensitive (issue) — but dedup keys
  // must be equal so a later re-classification updates the same issue
  // rather than spawning a new one.
  assert.equal(d1.dedupKey, d2.dedupKey);
  assert.equal(d1.dedupKey, "cargo:serde@1.0.1");
  assert.notEqual(d1.class, d2.class);
});

// ---------------------------------------------------------------------------
// makeDedupKey
// ---------------------------------------------------------------------------

test("makeDedupKey: format", () => {
  assert.equal(makeDedupKey("cargo", "serde", "1.0.1"), "cargo:serde@1.0.1");
  assert.equal(makeDedupKey("npm", "@scope/pkg", "2.0.0"), "npm:@scope/pkg@2.0.0");
});

// ---------------------------------------------------------------------------
// validateConfig
// ---------------------------------------------------------------------------

test("validateConfig: rejects missing sections", () => {
  assert.throws(() => validateConfig({}), /inventory_routing/);
  assert.throws(() => validateConfig({ inventory_routing: {} }), /inventory_routing/);
  assert.throws(
    () => validateConfig({
      inventory_routing: { auto_pass: {}, audit_required: {}, block: {} },
    }),
    /upgrade_routing/,
  );
});

test("validateConfig: rejects missing class routing", () => {
  const bad = JSON.parse(JSON.stringify(config));
  delete bad.upgrade_routing.classes.patch;
  assert.throws(() => validateConfig(bad), /classes\.patch/);
});

test("validateConfig: rejects bad dedup key_format", () => {
  const bad = JSON.parse(JSON.stringify(config));
  bad.upgrade_routing.dedup.key_format = "{ecosystem}:{name}";
  assert.throws(() => validateConfig(bad), /key_format/);
});

test("validateConfig: accepts the shipped config", () => {
  // The actual config file must validate.
  assert.equal(validateConfig(config), config);
});
