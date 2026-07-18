// Tests for tools/dep-governance/generate-sbom.mjs (#190).
//
// Run with: node --test tools/dep-governance/tests/sbom.test.mjs
//
// Hermetic: uses fixtures/inventory-sample.json. Does NOT touch the real
// repo's inventory or run cargo/npm.

import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import {
  toSpdxExpression,
  toPurl,
  toChecksum,
  buildSpdxDocument,
  buildCycloneDxBom,
  buildNoticesText,
} from "../generate-sbom.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const INVENTORY_PATH = path.resolve(__dirname, "..", "fixtures", "inventory-sample.json");
const inventory = JSON.parse(fs.readFileSync(INVENTORY_PATH, "utf8"));

// ---------------------------------------------------------------------------
// toSpdxExpression
// ---------------------------------------------------------------------------

test("toSpdxExpression: known SPDX id", () => {
  const r = toSpdxExpression("MIT");
  assert.equal(r.expression, "MIT");
  assert.equal(r.unknown, false);
});

test("toSpdxExpression: known expression", () => {
  const r = toSpdxExpression("MIT OR Apache-2.0");
  assert.equal(r.expression, "MIT OR Apache-2.0");
  assert.equal(r.unknown, false);
});

test("toSpdxExpression: empty -> NOASSERTION + unknown", () => {
  const r = toSpdxExpression(null);
  assert.equal(r.expression, "NOASSERTION");
  assert.equal(r.unknown, true);
});

test("toSpdxExpression: unrecognized -> NOASSERTION + unknown (no guessing)", () => {
  const r = toSpdxExpression("WTFPL");
  assert.equal(r.expression, "NOASSERTION");
  assert.equal(r.unknown, true);
});

test("toSpdxExpression: expression with one unknown component -> NOASSERTION", () => {
  // MIT OR Foo-Bar -> not all components known, so NOASSERTION.
  const r = toSpdxExpression("MIT OR Foo-Bar");
  assert.equal(r.expression, "NOASSERTION");
  assert.equal(r.unknown, true);
});

test("toSpdxExpression: WITH clause (known license + known exception)", () => {
  const r = toSpdxExpression("Apache-2.0 WITH LLVM-exception");
  assert.equal(r.expression, "Apache-2.0 WITH LLVM-exception");
  assert.equal(r.unknown, false);
  assert.equal(r.isComposite, true); // WITH counts as composite per CycloneDX
  assert.equal(r.ast.type, "with");
});

test("toSpdxExpression: WITH unknown exception -> NOASSERTION", () => {
  const r = toSpdxExpression("Apache-2.0 WITH Made-Up-Exception");
  assert.equal(r.expression, "NOASSERTION");
  assert.equal(r.unknown, true);
});

test("toSpdxExpression: single id isComposite=false", () => {
  const r = toSpdxExpression("MIT");
  assert.equal(r.isComposite, false);
  assert.equal(r.ast.type, "license");
});

test("toSpdxExpression: OR expression isComposite=true", () => {
  const r = toSpdxExpression("MIT OR Apache-2.0");
  assert.equal(r.isComposite, true);
  assert.equal(r.ast.type, "or");
});

test("toSpdxExpression: MIT-0 recognized as valid SPDX id", () => {
  // L1 fix: MIT-0 (no-attribution MIT) was missing from KNOWN_SPDX_IDS.
  const r = toSpdxExpression("MIT-0");
  assert.equal(r.expression, "MIT-0");
  assert.equal(r.unknown, false);
  assert.equal(r.isComposite, false);
});

// ---------------------------------------------------------------------------
// toPurl
// ---------------------------------------------------------------------------

test("toPurl: cargo lowercase", () => {
  assert.equal(toPurl({ ecosystem: "cargo", name: "Tokio", version: "1.52.3" }), "pkg:cargo/tokio@1.52.3");
});

test("toPurl: npm scoped percent-encodes leading @", () => {
  assert.equal(toPurl({ ecosystem: "npm", name: "@scope/pkg", version: "2.0.0" }), "pkg:npm/%40scope/pkg@2.0.0");
});

test("toPurl: npm unscoped", () => {
  assert.equal(toPurl({ ecosystem: "npm", name: "vue", version: "3.5.39" }), "pkg:npm/vue@3.5.39");
});

// ---------------------------------------------------------------------------
// toChecksum
// ---------------------------------------------------------------------------

test("toChecksum: cargo sha256 hex", () => {
  const r = toChecksum("sha256:abc123def4567890abcdef0123456789abcdef0123456789abcdef0123456789");
  assert.equal(r.algorithm, "SHA256");
  assert.equal(r.value, "abc123def4567890abcdef0123456789abcdef0123456789abcdef0123456789");
});

test("toChecksum: npm sha512 base64 -> hex", () => {
  // "aaaa" in base64 -> 0x69aaa6e0 (4 bytes). Use a known value.
  const b64 = Buffer.from("0123456789abcdef", "hex").toString("base64");
  const r = toChecksum(`sha512-${b64}`);
  assert.equal(r.algorithm, "SHA512");
  assert.equal(r.value, "0123456789abcdef");
});

test("toChecksum: null/empty/garbage -> null", () => {
  assert.equal(toChecksum(null), null);
  assert.equal(toChecksum(""), null);
  assert.equal(toChecksum("garbage"), null);
});

// ---------------------------------------------------------------------------
// buildSpdxDocument
// ---------------------------------------------------------------------------

test("buildSpdxDocument: structure and AIRP root package", () => {
  const doc = buildSpdxDocument(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  assert.equal(doc.spdxVersion, "SPDX-2.3");
  assert.equal(doc.dataLicense, "CC0-1.0");
  assert.equal(doc.SPDXID, "SPDXRef-DOCUMENT");
  assert.equal(doc.name, "AIRP");
  assert.match(doc.documentNamespace, /https:\/\/airp\.local\/spdx\/AIRP-2026-07-18/);
  assert.equal(doc.creationInfo.created, "2026-07-18T00:00:00.000Z");
  assert.ok(doc.creationInfo.creators.some((c) => c.startsWith("Tool:")));

  // AIRP root package present with AIRP's own license.
  const airp = doc.packages.find((p) => p.SPDXID === "SPDXRef-AIRP");
  assert.equal(airp.licenseConcluded, "MIT OR Apache-2.0");
  assert.equal(airp.licenseDeclared, "MIT OR Apache-2.0");
});

test("buildSpdxDocument: skips first-party cargo members, keeps npm root + third-party", () => {
  const doc = buildSpdxDocument(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  // airp-core (cargo first-party) is NOT a separate package (it's part of AIRP root).
  assert.equal(doc.packages.find((p) => p.name === "airp-core" && p.SPDXID !== "SPDXRef-AIRP"), undefined);
  // airp-ui (npm first-party root) — also skipped from third-party packages.
  assert.equal(doc.packages.find((p) => p.name === "airp-ui" && p.SPDXID !== "SPDXRef-AIRP"), undefined);
  // tokio (third-party cargo) is present.
  const tokio = doc.packages.find((p) => p.name === "tokio");
  assert.ok(tokio);
  assert.equal(tokio.versionInfo, "1.52.3");
  assert.equal(tokio.licenseDeclared, "MIT");
  assert.match(tokio.downloadLocation, /^https:\/\/crates\.io\//);
  // unknown-license-crate has NOASSERTION.
  const unknown = doc.packages.find((p) => p.name === "unknown-license-crate");
  assert.equal(unknown.licenseDeclared, "NOASSERTION");
  // gpl-runtime-trap has GPL-3.0 in license (we record it honestly, not NOASSERTION).
  const gpl = doc.packages.find((p) => p.name === "gpl-runtime-trap");
  assert.equal(gpl.licenseDeclared, "GPL-3.0");
});

test("buildSpdxDocument: DEPENDS_ON relationships from AIRP root to each package", () => {
  const doc = buildSpdxDocument(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  // Every non-root package has a DEPENDS_ON relationship from SPDXRef-AIRP.
  const thirdPartyPackages = doc.packages.filter((p) => p.SPDXID !== "SPDXRef-AIRP");
  for (const pkg of thirdPartyPackages) {
    const rel = doc.relationships.find(
      (r) => r.spdxElementId === "SPDXRef-AIRP" && r.relationshipType === "DEPENDS_ON" && r.relatedSpdxElement === pkg.SPDXID,
    );
    assert.ok(rel, `expected DEPENDS_ON relationship to ${pkg.SPDXID}`);
  }
});

test("buildSpdxDocument: checksums attached (cargo sha256, npm sha512)", () => {
  const doc = buildSpdxDocument(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  const tokio = doc.packages.find((p) => p.name === "tokio");
  assert.equal(tokio.checksums[0].algorithm, "SHA256");
  assert.match(tokio.checksums[0].checksumValue, /^[0-9a-f]{64}$/);

  const vue = doc.packages.find((p) => p.name === "vue");
  assert.equal(vue.checksums[0].algorithm, "SHA512");
  assert.match(vue.checksums[0].checksumValue, /^[0-9a-f]{128}$/);
});

test("buildSpdxDocument: purl external refs", () => {
  const doc = buildSpdxDocument(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  const tokio = doc.packages.find((p) => p.name === "tokio");
  const purlRef = tokio.externalRefs.find((r) => r.referenceType === "purl");
  assert.equal(purlRef.referenceLocator, "pkg:cargo/tokio@1.52.3");
});

// ---------------------------------------------------------------------------
// buildCycloneDxBom
// ---------------------------------------------------------------------------

test("buildCycloneDxBom: structure and metadata component", () => {
  const bom = buildCycloneDxBom(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  assert.equal(bom.bomFormat, "CycloneDX");
  assert.equal(bom.specVersion, "1.5");
  assert.equal(bom.version, 1);
  assert.match(bom.serialNumber, /^urn:uuid:/);
  assert.equal(bom.metadata.timestamp, "2026-07-18T00:00:00.000Z");
  assert.equal(bom.metadata.component.name, "AIRP");
  assert.ok(bom.metadata.component.licenses.some((l) => l.license.id === "MIT"));
  assert.ok(bom.metadata.component.licenses.some((l) => l.license.id === "Apache-2.0"));
});

test("buildCycloneDxBom: components have bom-ref, purl, hashes", () => {
  const bom = buildCycloneDxBom(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  const tokio = bom.components.find((c) => c.name === "tokio");
  assert.equal(tokio.type, "library");
  assert.equal(tokio["bom-ref"], "cargo:tokio@1.52.3");
  assert.equal(tokio.purl, "pkg:cargo/tokio@1.52.3");
  assert.equal(tokio.hashes[0].alg, "SHA-256");
  assert.equal(tokio.scope, "required");
});

test("buildCycloneDxBom: dev-scoped components marked optional", () => {
  const bom = buildCycloneDxBom(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  // The fixture's runtime records are tokio, vue, gpl-runtime-trap, unknown-license-crate.
  // There are no dev-scoped third-party records in the fixture, so this test
  // just confirms the scope mapping logic for runtime -> required.
  for (const c of bom.components) {
    assert.equal(c.scope, "required", `${c.name} expected scope required`);
  }
});

test("buildCycloneDxBom: unknown license -> name field, not id", () => {
  const bom = buildCycloneDxBom(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  const unknown = bom.components.find((c) => c.name === "unknown-license-crate");
  assert.equal(unknown.licenses[0].license.name, "UNKNOWN");
  assert.equal(unknown.licenses[0].license.id, undefined);
});

test("buildCycloneDxBom: single SPDX id uses license.id (not expression)", () => {
  // tokio's license is "MIT" (single SPDX id, no operators). Per CycloneDX
  // 1.5 schema, this must be {license: {id: "MIT"}}, NOT {expression: "MIT"}.
  const bom = buildCycloneDxBom(inventory, { createdIso: "2026-07-18T00:00:00.000Z" });
  const tokio = bom.components.find((c) => c.name === "tokio");
  assert.equal(tokio.licenses.length, 1);
  assert.equal(tokio.licenses[0].license.id, "MIT");
  assert.equal(tokio.licenses[0].expression, undefined);
});

test("buildCycloneDxBom: OR composite expression uses license.expression field", () => {
  // Per CycloneDX 1.5 schema, composite SPDX expressions (OR/AND/WITH) must
  // use the `expression` field, NOT be split into multiple `license.id`
  // entries. Putting a composite expression in `license.id` is invalid.
  const synth = {
    meta: { generated_at: "2026-07-18T00:00:00.000Z", airp_version: "0.1.0" },
    records: [
      {
        ecosystem: "cargo", name: "dual", version: "1.0.0",
        license: "MIT OR Apache-2.0", license_normalized: "MIT OR Apache-2.0",
        source: "crates.io", resolved: "https://crates.io/api/v1/crates/dual/1.0.0/download",
        integrity: "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        scope: "runtime", tier: "third-party",
      },
    ],
  };
  const bom2 = buildCycloneDxBom(synth, { createdIso: "2026-07-18T00:00:00.000Z" });
  const dual = bom2.components.find((c) => c.name === "dual");
  assert.equal(dual.licenses.length, 1);
  assert.equal(dual.licenses[0].expression, "MIT OR Apache-2.0");
  assert.equal(dual.licenses[0].license, undefined);
});

test("buildCycloneDxBom: WITH expression uses license.expression field", () => {
  // WITH clauses are composite per CycloneDX 1.5 (they modify the license
  // with an exception, so they can't be a bare SPDX id in `license.id`).
  const synth = {
    meta: { generated_at: "2026-07-18T00:00:00.000Z", airp_version: "0.1.0" },
    records: [
      {
        ecosystem: "cargo", name: "llvm-licensed", version: "1.0.0",
        license: "Apache-2.0 WITH LLVM-exception",
        license_normalized: "Apache-2.0 WITH LLVM-exception",
        source: "crates.io", resolved: "https://crates.io/api/v1/crates/llvm-licensed/1.0.0/download",
        integrity: "sha256:0000000000000000000000000000000000000000000000000000000000000000",
        scope: "runtime", tier: "third-party",
      },
    ],
  };
  const bom2 = buildCycloneDxBom(synth, { createdIso: "2026-07-18T00:00:00.000Z" });
  const ll = bom2.components.find((c) => c.name === "llvm-licensed");
  assert.equal(ll.licenses.length, 1);
  assert.equal(ll.licenses[0].expression, "Apache-2.0 WITH LLVM-exception");
  assert.equal(ll.licenses[0].license, undefined);
});

// ---------------------------------------------------------------------------
// buildNoticesText
// ---------------------------------------------------------------------------

test("buildNoticesText: header, runtime section, attention section", () => {
  const text = buildNoticesText(inventory);
  assert.match(text, /AIRP Third-Party Notices/);
  assert.match(text, /Shipped runtime dependencies/);
  // tokio should be in the runtime section.
  assert.match(text, /tokio 1\.52\.3/);
  assert.match(text, /license: MIT/);
  // GPL record triggers the attention section.
  assert.match(text, /Records requiring attention before release/);
  assert.match(text, /\[block\] npm\/gpl-runtime-trap@1\.0\.0/);
  // Unknown-license record also in attention section.
  assert.match(text, /\[audit-required\] cargo\/unknown-license-crate@0\.2\.1/);
});

test("buildNoticesText: marks unknown licenses explicitly", () => {
  const text = buildNoticesText(inventory);
  // unknown-license-crate has null license -> "UNKNOWN (audit required; raw=\"\")"
  assert.match(text, /UNKNOWN \(audit required; raw=""\)/);
});

test("buildNoticesText: first-party records excluded from third-party listing", () => {
  const text = buildNoticesText(inventory);
  // airp-core and airp-ui are first-party; they should NOT appear as
  // third-party entries in the notices bundle (they carry AIRP's own license).
  assert.doesNotMatch(text, /  airp-core 0\.1\.0\b/);
  assert.doesNotMatch(text, /  airp-ui 0\.1\.0\b/);
});
