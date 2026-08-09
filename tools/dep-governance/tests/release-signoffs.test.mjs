import assert from "node:assert/strict";
import test from "node:test";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { validateReleaseSignoffs } from "../validate-release-signoffs.mjs";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const signoffs = JSON.parse(fs.readFileSync(path.join(HERE, "..", "..", "..", "docs", "sbom", "audit-signoffs.json"), "utf8"));

function inventoryFromSignoffs() {
  return {
    records: signoffs.records.map((record) => ({
      ...record,
      license_normalized: record.license,
      audit_class: "audit-required",
      audit_reason: "fixture audit reason",
    })),
  };
}

const metadata = {
  releaseTag: "v0.0.5",
  releaseCommit: "0123456789abcdef0123456789abcdef01234567",
  runUrl: "https://github.com/GhostXia/AIRP/actions/runs/12345",
  asOf: "2026-08-09",
};

test("committed signoffs cover exact 17-record audit-required set", () => {
  assert.equal(signoffs.expected_count, 17);
  assert.equal(signoffs.records.length, 17);
  assert.equal(new Set(signoffs.records.map((record) => `${record.ecosystem}:${record.name}:${record.version}`)).size, 17);
});

test("validator emits complete release metadata and obligations", () => {
  const report = validateReleaseSignoffs({ inventory: inventoryFromSignoffs(), signoffs, ...metadata });
  assert.equal(report.schema, "airp-release-audit-v1");
  assert.equal(report.audit_required_count, 17);
  assert.equal(report.release_tag, metadata.releaseTag);
  assert.equal(report.release_commit, metadata.releaseCommit);
  assert.equal(report.run_url, metadata.runUrl);
  assert.equal(report.signoff_status, "accepted-with-obligations");
  assert.ok(report.audit_required.every((record) => record.obligations.length > 0));
});

test("validator rejects missing, extra or drifted signoffs", () => {
  const missing = { ...signoffs, records: signoffs.records.slice(1) };
  assert.throws(() => validateReleaseSignoffs({ inventory: inventoryFromSignoffs(), signoffs: missing, ...metadata }), /signoff count/);

  const driftedInventory = inventoryFromSignoffs();
  driftedInventory.records[0] = { ...driftedInventory.records[0], version: "9.9.9" };
  assert.throws(() => validateReleaseSignoffs({ inventory: driftedInventory, signoffs, ...metadata }), /signoff identity mismatch/);

  const extra = { ...signoffs, records: [...signoffs.records, { ...signoffs.records[0], name: "unexpected" }] };
  assert.throws(() => validateReleaseSignoffs({ inventory: inventoryFromSignoffs(), signoffs: extra, ...metadata }), /signoff count/);
});

test("validator rejects invalid conclusions, obligations, evidence and expiry", () => {
  const badConclusion = { ...signoffs, records: signoffs.records.map((record, index) => index === 0 ? { ...record, conclusion: "rejected" } : record) };
  assert.throws(() => validateReleaseSignoffs({ inventory: inventoryFromSignoffs(), signoffs: badConclusion, ...metadata }), /invalid conclusion/);

  const badEvidence = { ...signoffs, records: signoffs.records.map((record, index) => index === 0 ? { ...record, evidence: [] } : record) };
  assert.throws(() => validateReleaseSignoffs({ inventory: inventoryFromSignoffs(), signoffs: badEvidence, ...metadata }), /requires non-empty evidence/);

  const expired = { ...signoffs, records: signoffs.records.map((record, index) => index === 0 ? { ...record, valid_until: "2026-08-08" } : record) };
  assert.throws(() => validateReleaseSignoffs({ inventory: inventoryFromSignoffs(), signoffs: expired, ...metadata }), /expired/);
});
