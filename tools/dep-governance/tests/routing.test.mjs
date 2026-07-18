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

test("compareSemver: null inputs do not crash (defensive export contract)", () => {
  // L7 fix: exported function must not throw on null. classifyUpgrade
  // filters null before calling, but the export is public API.
  assert.equal(compareSemver(null, null), 0);
  assert.ok(compareSemver(null, parseSemver("1.0.0")) < 0);
  assert.ok(compareSemver(parseSemver("1.0.0"), null) > 0);
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

test("splitLicenseExpression: splits on OR/AND, preserves WITH", () => {
  // DEPRECATED wrapper now delegates to AST parser. OR/AND composite
  // expressions are flattened to their component license ids; WITH clauses
  // are preserved attached to their license (the AST renderer re-emits
  // "Apache-2.0 WITH LLVM-exception" as a single token).
  assert.deepEqual(splitLicenseExpression("MIT OR Apache-2.0"), ["MIT", "Apache-2.0"]);
  assert.deepEqual(splitLicenseExpression("MIT AND Apache-2.0"), ["MIT", "Apache-2.0"]);
  assert.deepEqual(splitLicenseExpression("Apache-2.0 WITH LLVM-exception"), ["Apache-2.0 WITH LLVM-exception"]);
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
  // The AST classifier produces an "OR (...) reason" for composite expressions.
  assert.match(r.reason, /OR \(auto-pass \| auto-pass\)/);
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
// SPDX expression parser: parenthesized, WITH, OR/AND precedence.
// These exercise the A1 fix (independent recursive-descent parser).
// ---------------------------------------------------------------------------

test("classifyInventory: parenthesized GPL-3.0 blocks (parens stripped by parser)", () => {
  // Previously `(GPL-3.0)` bypassed the block list because the splitter
  // didn't strip parens. The AST parser strips parens and classifies the
  // inner license id.
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "(GPL-3.0)", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "block");
  assert.match(r.reason, /GPL-3\.0/);
});

test("classifyInventory: MIT OR GPL-3.0 auto-passes (recipient chooses MIT)", () => {
  // Previously this was over-blocked because the splitter naively checked
  // each component and saw GPL-3.0. The AST classifier takes the BEST of
  // the OR branches, so the recipient can choose MIT and avoid GPL-3.0.
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "MIT OR GPL-3.0", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "auto-pass");
  assert.match(r.reason, /OR \(auto-pass \| block\)/);
});

test("classifyInventory: MIT AND GPL-3.0 blocks (recipient bound by stricter)", () => {
  // AND takes the WORST of the branches. Recipient must comply with both,
  // so GPL-3.0 dominates and the record blocks.
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "MIT AND GPL-3.0", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "block");
  assert.match(r.reason, /AND \(auto-pass & block\)/);
});

test("classifyInventory: Apache-2.0 WITH LLVM-exception auto-passes", () => {
  // The WITH clause attaches an exception but does not change the license
  // tier. Apache-2.0 is permissive, so the record auto-passes.
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "Apache-2.0 WITH LLVM-exception", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "auto-pass");
  assert.match(r.reason, /Apache-2\.0/);
});

test("classifyInventory: parenthesized OR expression with mixed tiers", () => {
  // (MIT OR GPL-3.0) AND BSD-3-Clause
  // Outer AND: worst of (MIT OR GPL-3.0 = auto-pass) and BSD-3-Clause (auto-pass) = auto-pass.
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "(MIT OR GPL-3.0) AND BSD-3-Clause", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "auto-pass");
});

test("classifyInventory: GPL-3.0 AND (MIT OR Apache-2.0) blocks", () => {
  // Outer AND: worst of GPL-3.0 (block) and (MIT OR Apache-2.0 = auto-pass) = block.
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "GPL-3.0 AND (MIT OR Apache-2.0)", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "block");
});

test("classifyInventory: unparseable expression falls back to audit-required", () => {
  // Mismatched parens or trailing tokens → null AST → audit-required
  // (conservative: never silent auto-pass).
  const r = classifyInventory(
    { name: "x", version: "1", ecosystem: "cargo", license: "(MIT OR", scope: "runtime", tier: "third-party" },
    config,
  );
  assert.equal(r.class, "audit-required");
  assert.match(r.reason, /could not be parsed/);
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
