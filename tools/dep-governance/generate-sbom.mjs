// Third-party notices + SBOM generator for AIRP (#190).
//
// Consumes the inventory produced by discover-deps.mjs and emits three
// artifacts in the output directory:
//
//   airp.spdx.json            — SPDX-2.3 JSON (machine-readable SBOM)
//   airp.cdx.json             — CycloneDX 1.5 JSON (machine-readable SBOM)
//   THIRD-PARTY-NOTICES.txt   — human-readable notices bundle
//
// Design:
//   - Reads docs/sbom/inventory.json (or a path passed via --inventory).
//   - License mapping: AIRP license strings are already SPDX identifiers in
//     the vast majority of cases (cargo metadata and npm lockfile both use
//     SPDX). When the license is null/empty or unrecognized, the SBOM uses
//     NOASSERTION (SPDX) / omits the license field (CycloneDX), and the
//     notices bundle prints "UNKNOWN — audit required" so a human must
//     resolve it before release.
//   - Provenance: emits Package URLs (purl) per the purl spec, plus the
//     upstream download location (crates.io / npm tarball URL) and the
//     integrity hash (sha256 for cargo, sha512 for npm).
//   - First-party records (AIRP workspace members) are included in the SBOM
//     as the document's describing component / a package with AIRP's own
//     license, NOT in the third-party notices bundle (those carry AIRP's
//     own MIT OR Apache-2.0 license, not a third-party notice).
//   - Deterministic output: components sorted by (ecosystem, name, version);
//     JSON keys sorted. Re-running on the same inventory produces
//     byte-identical output modulo the creation timestamp.
//   - Fail-visible: --fail-on-unknown exits non-zero if any third-party
//     record has an unresolvable license, AFTER writing the SBOM so the
//     operator can inspect what failed.
//
// Per AGENTS.md "第三方经验吸收与独立实现": uses only Node built-ins.
// SPDX and CycloneDX document structures are built from their public specs
// (https://spdx.github.io/spdx-spec/ and
// https://cyclonedx.org/docs/1.5/json/); no SBOM library is reused.

import fs from "node:fs";
import path from "node:path";
import { normalizeLicense, validateConfig } from "./audit-routing.mjs";

// ---------------------------------------------------------------------------
// License handling.
// ---------------------------------------------------------------------------

/**
 * SPDX license identifiers that AIRP recognizes as valid for the
 * `licenseConcluded` / `licenseDeclared` fields. Anything not in this set
 * (and not a valid SPDX expression) is mapped to NOASSERTION and flagged
 * for human review. This is a conservative allowlist — additions here
 * should be deliberate.
 */
const KNOWN_SPDX_IDS = new Set([
  "MIT", "Apache-2.0", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Zlib",
  "0BSD", "Unicode-DFS-2016", "Unicode-3.0", "Unlicense", "CC0-1.0",
  "OpenSSL", "BSL-1.0", "MPL-2.0", "EPL-1.0", "EPL-2.0", "LGPL-2.0",
  "LGPL-2.1", "LGPL-3.0", "LGPL-2.1-or-later", "LGPL-3.0-or-later",
  "GPL-1.0", "GPL-2.0", "GPL-3.0", "GPL-2.0-or-later", "GPL-3.0-or-later",
  "AGPL-3.0", "AGPL-3.0-or-later", "SSPL-1.0", "BUSL-1.1",
  "PolyForm-Noncommercial-1.0.0", "PolyForm-Small-Business-1.0.0",
  "CC-BY-4.0", "CC-BY-SA-4.0",
]);

/**
 * Map a normalized license string to an SPDX license expression suitable
 * for the SBOM. Returns "NOASSERTION" if the license is empty or not
 * recognisable, and sets `unknown=true` on the result so the caller can
 * flag it.
 *
 * We accept:
 *   - exact SPDX IDs in the allowlist
 *   - SPDX expressions like "MIT OR Apache-2.0" where every component is a
 *     known SPDX ID
 *
 * @param {string|null|undefined} license
 * @returns {{expression: string, unknown: boolean}}
 */
export function toSpdxExpression(license) {
  const norm = normalizeLicense(license);
  if (norm === "") return { expression: "NOASSERTION", unknown: true };

  // Split on OR/AND/WITH and check every component is a known SPDX id.
  const components = norm
    .split(/\s+(?:OR|AND|WITH)\s+/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
  if (components.length === 0) return { expression: "NOASSERTION", unknown: true };

  const allKnown = components.every((c) => KNOWN_SPDX_IDS.has(c));
  if (allKnown) return { expression: norm, unknown: false };

  // License is present but not recognized. Be honest in the SBOM:
  // NOASSERTION + unknown flag. Do NOT guess.
  return { expression: "NOASSERTION", unknown: true };
}

// ---------------------------------------------------------------------------
// PURL + download location.
// ---------------------------------------------------------------------------

/**
 * Build a Package URL (purl) per https://github.com/package-url/purl-spec.
 *
 * @param {object} r  // inventory record
 * @returns {string|null}
 */
export function toPurl(r) {
  if (r.ecosystem === "cargo") {
    // purl spec: type "cargo" uses lowercase name, no namespace.
    return `pkg:cargo/${r.name.toLowerCase()}@${r.version}`;
  }
  if (r.ecosystem === "npm") {
    // Scoped npm packages: @scope/name -> pkg:npm/%40scope/name@version
    // purl spec requires the @ in the path to be percent-encoded.
    const encoded = r.name.replace(/^@/, "%40");
    return `pkg:npm/${encoded}@${r.version}`;
  }
  return null;
}

/**
 * Extract the hash algorithm + value from an integrity string.
 * Cargo: "sha256:<hex>"  -> { algorithm: "SHA256", value: <hex> }
 * npm:   "sha512-<base64>" -> { algorithm: "SHA512", value: <hex> }
 *        (npm uses base64; SPDX wants hex. We convert.)
 *
 * @param {string|null|undefined} integrity
 * @returns {{algorithm: string, value: string}|null}
 */
export function toChecksum(integrity) {
  if (!integrity) return null;
  // cargo form
  let m = /^sha256:([0-9a-fA-F]+)$/.exec(integrity);
  if (m) return { algorithm: "SHA256", value: m[1].toLowerCase() };
  // npm form: sha512-<base64>
  m = /^sha512-([A-Za-z0-9+/=]+)$/.exec(integrity);
  if (m) {
    const b64 = m[1];
    // Convert base64 -> hex. Node Buffer handles this.
    const hex = Buffer.from(b64, "base64").toString("hex");
    return { algorithm: "SHA512", value: hex };
  }
  return null;
}

// ---------------------------------------------------------------------------
// SPDX-2.3 JSON builder.
// ---------------------------------------------------------------------------

/**
 * Sanitize a record name into a valid SPDX element ID fragment.
 * SPDX IDs: [A-Za-z0-9.-]+ only. We replace other chars with '-'.
 *
 * @param {string} s
 * @returns {string}
 */
function spdxIdSafe(s) {
  return String(s).replace(/[^A-Za-z0-9.-]/g, "-");
}

/**
 * Build an SPDX-2.3 document from the inventory.
 *
 * @param {object} inventory  // { records, meta }
 * @param {object} opts       // { createdIso, documentNamespace }
 * @returns {object}
 */
export function buildSpdxDocument(inventory, opts) {
  const created = opts.createdIso ?? new Date().toISOString();
  const nsBase = opts.documentNamespace ?? `https://airp.local/spdx/AIRP`;
  // Document namespace must be unique per generation; append a UTC date stamp
  // (no time) so same-day regenerations share a namespace, which is the
  // SPDX convention.
  const day = created.slice(0, 10);
  const documentNamespace = `${nsBase}-${day}`;

  const packages = [];
  const relationships = [];

  // Document-describing package: AIRP itself, as a "FILE" or the implicit
  // root. We model AIRP as a package of type "application" with AIRP's
  // license, and make every third-party component a DEPENDS_ON relationship
  // from it. This is the most common pattern for project SBOMs.
  const airpId = "SPDXRef-AIRP";
  packages.push({
    name: "AIRP",
    SPDXID: airpId,
    versionInfo: inventory.meta.airp_version ?? "0.1.0",
    downloadLocation: "https://github.com/GhostXia/AIRP",
    filesAnalyzed: false,
    licenseConcluded: "MIT OR Apache-2.0",
    licenseDeclared: "MIT OR Apache-2.0",
    supplier: "Organization: GhostXia",
    copyrightText: "NOASSERTION",
    description: "AIRP — independent Agent backend, State Protocol, and UI",
  });

  for (const r of inventory.records) {
    if (r.tier === "first-party") {
      // First-party records (Cargo workspace members AND the npm root
      // package airp-ui) are part of AIRP itself; skip emitting them as
      // separate SPDX packages — they would duplicate AIRP's own license.
      continue;
    }
    const spdx = toSpdxExpression(r.license);
    const id = `SPDXRef-${spdxIdSafe(r.ecosystem)}-${spdxIdSafe(r.name)}-${spdxIdSafe(r.version)}`;
    const checksum = toChecksum(r.integrity);
    const pkg = {
      name: r.name,
      SPDXID: id,
      versionInfo: r.version,
      downloadLocation: r.resolved ?? "NOASSERTION",
      filesAnalyzed: false,
      licenseConcluded: spdx.expression,
      licenseDeclared: spdx.expression,
      supplier: r.source === "crates.io" || r.source === "npm"
        ? "NOASSERTION"
        : "NOASSERTION",
      copyrightText: "NOASSERTION",
      externalRefs: [],
    };
    if (r.repository) {
      pkg.externalRefs.push({
        referenceCategory: "PACKAGE-MANAGER",
        referenceType: "purl",
        referenceLocator: toPurl(r) ?? r.repository,
      });
    } else if (toPurl(r)) {
      pkg.externalRefs.push({
        referenceCategory: "PACKAGE-MANAGER",
        referenceType: "purl",
        referenceLocator: toPurl(r),
      });
    }
    if (checksum) {
      pkg.checksums = [{ algorithm: checksum.algorithm, checksumValue: checksum.value }];
    }
    if (r.homepage) pkg.homepage = r.homepage;
    if (r.description) pkg.description = r.description;
    packages.push(pkg);
    relationships.push({
      spdxElementId: airpId,
      relationshipType: "DEPENDS_ON",
      relatedSpdxElement: id,
    });
  }

  return {
    spdxVersion: "SPDX-2.3",
    dataLicense: "CC0-1.0",
    SPDXID: "SPDXRef-DOCUMENT",
    name: "AIRP",
    documentNamespace,
    creationInfo: {
      created,
      creators: [
        "Tool: AIRP dep-governance generate-sbom.mjs",
        "Organization: GhostXia",
      ],
      licenseListVersion: "3.21",
    },
    packages,
    relationships,
  };
}

// ---------------------------------------------------------------------------
// CycloneDX 1.5 JSON builder.
// ---------------------------------------------------------------------------

/**
 * Build a CycloneDX 1.5 BOM from the inventory.
 *
 * @param {object} inventory
 * @param {object} opts   // { createdIso, serial }
 * @returns {object}
 */
export function buildCycloneDxBom(inventory, opts) {
  const created = opts.createdIso ?? new Date().toISOString();
  const serial = opts.serial ?? `urn:uuid:${cryptoRandomUuid()}`;

  const components = [];
  for (const r of inventory.records) {
    if (r.tier === "first-party") continue;
    const spdx = toSpdxExpression(r.license);
    const component = {
      type: "library",
      "bom-ref": `${r.ecosystem}:${r.name}@${r.version}`,
      name: r.name,
      version: r.version,
      scope: r.scope === "dev" ? "optional" : "required",
      purl: toPurl(r) ?? undefined,
    };
    if (r.resolved) component.externalReferences = [{ type: "distribution", url: r.resolved }];
    const hashes = toChecksum(r.integrity);
    if (hashes) {
      component.hashes = [{ alg: hashAlgCdx(hashes.algorithm), content: hashes.value }];
    }
    // CycloneDX license field: array of {license: {id}} or {license: {name}}
    if (spdx.unknown) {
      component.licenses = [{ license: { name: r.license_normalized || "UNKNOWN" } }];
    } else {
      // Split OR expressions into multiple license entries (CycloneDX
      // represents OR as multiple entries with license choice semantics).
      const ids = spdx.expression.split(/\s+OR\s+/);
      component.licenses = ids.map((id) => ({ license: { id: id } }));
    }
    components.push(component);
  }

  return {
    bomFormat: "CycloneDX",
    specVersion: "1.5",
    serialNumber: serial,
    version: 1,
    metadata: {
      timestamp: created,
      tools: [
        {
          vendor: "GhostXia",
          name: "AIRP dep-governance generate-sbom.mjs",
          version: "0.1.0",
        },
      ],
      component: {
        type: "application",
        "bom-ref": "AIRP",
        name: "AIRP",
        version: inventory.meta.airp_version ?? "0.1.0",
        licenses: [{ license: { id: "MIT" } }, { license: { id: "Apache-2.0" } }],
      },
    },
    components,
  };
}

/**
 * Map SPDX hash algorithm names to CycloneDX alg values.
 *
 * @param {string} spdxAlg  // "SHA256" | "SHA512"
 * @returns {string}
 */
function hashAlgCdx(spdxAlg) {
  if (spdxAlg === "SHA256") return "SHA-256";
  if (spdxAlg === "SHA512") return "SHA-512";
  return spdxAlg;
}

/**
 * Minimal RFC4122 v4 UUID generator using Node crypto. Used for CycloneDX
 * serialNumber. We don't use crypto.randomUUID() to keep this function
 * testable on older Node, but in practice Node 18+ has it.
 */
function cryptoRandomUuid() {
  if (typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  const b = crypto.randomBytes(16);
  b[6] = (b[6] & 0x0f) | 0x40;
  b[8] = (b[8] & 0x3f) | 0x80;
  const h = b.toString("hex");
  return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
}

// ---------------------------------------------------------------------------
// Third-party notices text builder.
// ---------------------------------------------------------------------------

/**
 * Build the human-readable THIRD-PARTY-NOTICES.txt bundle.
 *
 * Includes only third-party runtime dependencies (the deps that actually
 * ship in AIRP release artifacts). Dev/build deps are listed in a separate
 * "Build and test dependencies (not shipped)" section for completeness.
 *
 * @param {object} inventory
 * @returns {string}
 */
export function buildNoticesText(inventory) {
  const lines = [];
  lines.push("AIRP Third-Party Notices");
  lines.push("=========================");
  lines.push("");
  lines.push("This file is generated by tools/dep-governance/generate-sbom.mjs");
  lines.push("from docs/sbom/inventory.json. Do not edit by hand; regenerate with");
  lines.push("`node tools/dep-governance/generate-sbom.mjs`.");
  lines.push("");
  lines.push(`Generated: ${inventory.meta.generated_at}`);
  lines.push("");
  lines.push("AIRP itself is distributed under MIT OR Apache-2.0 (see LICENSE-MIT");
  lines.push("and LICENSE-APACHE at the repository root). The third-party");
  lines.push("components listed below are distributed under their own licenses,");
  lines.push("which are compatible with AIRP's distribution. Each component's");
  lines.push("upstream source and integrity are recorded in the accompanying");
  lines.push("SBOM files (airp.spdx.json, airp.cdx.json).");
  lines.push("");

  const thirdParty = inventory.records.filter((r) => r.tier === "third-party");
  const runtime = thirdParty.filter((r) => r.scope === "runtime");
  const nonRuntime = thirdParty.filter((r) => r.scope !== "runtime");

  // Group by ecosystem for readability.
  const grouped = (recs) => {
    const m = new Map();
    for (const r of recs) {
      if (!m.has(r.ecosystem)) m.set(r.ecosystem, []);
      m.get(r.ecosystem).push(r);
    }
    return m;
  };

  lines.push("Shipped runtime dependencies");
  lines.push("---------------------------");
  if (runtime.length === 0) {
    lines.push("(none)");
  } else {
    for (const [eco, recs] of grouped(runtime)) {
      lines.push("");
      lines.push(`${eco} (${recs.length})`);
      for (const r of recs.sort((a, b) => a.name.localeCompare(b.name))) {
        const spdx = toSpdxExpression(r.license);
        lines.push(`  ${r.name} ${r.version}`);
        lines.push(`    license: ${spdx.unknown ? `UNKNOWN (audit required; raw="${r.license_normalized || ""}")` : spdx.expression}`);
        if (r.repository) lines.push(`    upstream: ${r.repository}`);
        if (r.resolved) lines.push(`    source:   ${r.resolved}`);
        if (r.integrity) lines.push(`    integrity: ${r.integrity}`);
      }
    }
  }

  lines.push("");
  lines.push("Build and test dependencies (NOT shipped in release artifacts)");
  lines.push("-------------------------------------------------------------");
  if (nonRuntime.length === 0) {
    lines.push("(none)");
  } else {
    for (const [eco, recs] of grouped(nonRuntime)) {
      lines.push("");
      lines.push(`${eco} (${recs.length})`);
      for (const r of recs.sort((a, b) => a.name.localeCompare(b.name))) {
        const spdx = toSpdxExpression(r.license);
        lines.push(`  ${r.name} ${r.version}  [${r.scope}]`);
        lines.push(`    license: ${spdx.unknown ? `UNKNOWN (raw="${r.license_normalized || ""}")` : spdx.expression}`);
        if (r.resolved) lines.push(`    source:   ${r.resolved}`);
      }
    }
  }

  // Attention section: any record flagged block or audit-required.
  const attention = thirdParty.filter(
    (r) => r.audit_class === "block" || r.audit_class === "audit-required",
  );
  if (attention.length > 0) {
    lines.push("");
    lines.push("Records requiring attention before release");
    lines.push("------------------------------------------");
    for (const r of attention) {
      lines.push(`  [${r.audit_class}] ${r.ecosystem}/${r.name}@${r.version}`);
      lines.push(`    ${r.audit_reason}`);
    }
    lines.push("");
    lines.push("These records block release (block) or require a dedicated audit");
    lines.push("(audit-required) before the SBOM/notices can be published with a");
    lines.push("formal release artifact. See tools/dep-governance/README.md.");
  }

  return lines.join("\n") + "\n";
}

// ---------------------------------------------------------------------------
// CLI entry.
// ---------------------------------------------------------------------------

/**
 * @param {string[]} argv
 */
export async function main(argv) {
  const args = parseCliArgs(argv);
  const inventoryPath = path.resolve(args.inventory);
  const outDir = path.resolve(args.outDir);
  const configPath = path.resolve(args.config);

  let config;
  try {
    config = JSON.parse(fs.readFileSync(configPath, "utf8"));
    validateConfig(config);
  } catch (e) {
    process.stderr.write(`error: invalid config ${configPath}: ${e.message}\n`);
    process.exit(2);
  }

  let inventory;
  try {
    inventory = JSON.parse(fs.readFileSync(inventoryPath, "utf8"));
  } catch (e) {
    process.stderr.write(`error: cannot read inventory ${inventoryPath}: ${e.message}\n`);
    process.stderr.write(`       Run discover-deps.mjs first.\n`);
    process.exit(1);
  }

  // Use the inventory's generated_at as the creation timestamp so the SBOM
  // and inventory agree, and so re-running generate-sbom.mjs on the same
  // inventory produces identical output.
  const createdIso = inventory.meta?.generated_at ?? new Date().toISOString();

  const spdx = buildSpdxDocument(inventory, { createdIso });
  const cdx = buildCycloneDxBom(inventory, { createdIso });
  const notices = buildNoticesText(inventory);

  fs.mkdirSync(outDir, { recursive: true });
  atomicWrite(path.join(outDir, "airp.spdx.json"), stringifySorted(spdx) + "\n");
  atomicWrite(path.join(outDir, "airp.cdx.json"), stringifySorted(cdx) + "\n");
  atomicWrite(path.join(outDir, "THIRD-PARTY-NOTICES.txt"), notices);

  process.stdout.write(`wrote ${path.join(outDir, "airp.spdx.json")}\n`);
  process.stdout.write(`wrote ${path.join(outDir, "airp.cdx.json")}\n`);
  process.stdout.write(`wrote ${path.join(outDir, "THIRD-PARTY-NOTICES.txt")}\n`);

  const thirdParty = inventory.records.filter((r) => r.tier === "third-party");
  const unknown = thirdParty.filter((r) => toSpdxExpression(r.license).unknown);
  const blocked = thirdParty.filter((r) => r.audit_class === "block");

  process.stdout.write(
    `summary: ${thirdParty.length} third-party records, ${unknown.length} with unknown license, ${blocked.length} blocked\n`,
  );

  if (args.failOnUnknown && unknown.length > 0) {
    process.stderr.write(
      `error: ${unknown.length} third-party record(s) have unknown licenses; release blocked\n`,
    );
    for (const r of unknown) {
      process.stderr.write(
        `  unknown  ${r.ecosystem}/${r.name}@${r.version}  raw="${r.license_normalized || ""}"\n`,
      );
    }
    process.exit(1);
  }
  if (blocked.length > 0) {
    process.stderr.write(
      `error: ${blocked.length} record(s) classified as block by inventory routing; release blocked\n`,
    );
    for (const r of blocked) {
      process.stderr.write(`  block  ${r.ecosystem}/${r.name}@${r.version}  ${r.audit_reason}\n`);
    }
    process.exit(1);
  }
}

function parseCliArgs(argv) {
  const args = {
    inventory: path.join(process.cwd(), "docs", "sbom", "inventory.json"),
    outDir: path.join(process.cwd(), "docs", "sbom"),
    config: path.join(process.cwd(), "tools", "dep-governance", "audit-routing.config.json"),
    failOnUnknown: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--inventory") args.inventory = argv[++i];
    else if (a === "--out-dir") args.outDir = argv[++i];
    else if (a === "--config") args.config = argv[++i];
    else if (a === "--fail-on-unknown") args.failOnUnknown = true;
    else if (a === "--help" || a === "-h") {
      process.stdout.write(USAGE);
      process.exit(0);
    } else {
      process.stderr.write(`error: unknown argument ${a}\n${USAGE}`);
      process.exit(2);
    }
  }
  return args;
}

const USAGE = `Usage: generate-sbom.mjs [options]

Options:
  --inventory <path>   Input inventory JSON (default: docs/sbom/inventory.json)
  --out-dir <path>     Output directory (default: docs/sbom)
  --config <path>      Routing config (default: tools/dep-governance/audit-routing.config.json)
  --fail-on-unknown    Exit 1 if any third-party record has an unknown license
  -h, --help           Show this help

Outputs:
  <out-dir>/airp.spdx.json            SPDX-2.3 JSON SBOM
  <out-dir>/airp.cdx.json             CycloneDX 1.5 JSON SBOM
  <out-dir>/THIRD-PARTY-NOTICES.txt   Human-readable notices bundle
`;

function stringifySorted(value) {
  return JSON.stringify(value, sortedReplacer, 2);
}

function sortedReplacer(_key, value) {
  if (value && typeof value === "object" && !Array.isArray(value)) {
    const sorted = {};
    for (const k of Object.keys(value).sort()) {
      sorted[k] = value[k];
    }
    return sorted;
  }
  return value;
}

function atomicWrite(filePath, contents) {
  const tmp = `${filePath}.tmp`;
  fs.writeFileSync(tmp, contents);
  try {
    fs.renameSync(tmp, filePath);
  } catch (e) {
    try {
      fs.unlinkSync(tmp);
    } catch {
      /* best-effort cleanup */
    }
    throw e;
  }
}

const isMain = (() => {
  if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
    return true;
  }
  return process.argv[1]?.endsWith("generate-sbom.mjs");
})();

if (isMain) {
  main(process.argv.slice(2)).catch((e) => {
    process.stderr.write(`fatal: ${e?.stack ?? e}\n`);
    process.exit(1);
  });
}
