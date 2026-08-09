// Validate exact audit-required dependency sign-offs before release.
//
// Inventory identity is package + resolved version + normalized license +
// scope. Every audit-required record must have one current sign-off entry;
// missing, duplicate, drifted, rejected or expired entries fail closed.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DEFAULT_SIGNOFFS = path.resolve(HERE, "..", "..", "docs", "sbom", "audit-signoffs.json");

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function identity(record) {
  return [
    record.ecosystem,
    record.name,
    record.version,
    record.license ?? record.license_normalized,
    record.scope,
  ].join("|");
}

function sortRecords(records) {
  return [...records].sort((left, right) => identity(left).localeCompare(identity(right)));
}

function validateDate(value, label) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value ?? "")) {
    throw new Error(`${label} must be YYYY-MM-DD`);
  }
  const parsed = new Date(`${value}T00:00:00Z`);
  if (Number.isNaN(parsed.valueOf()) || parsed.toISOString().slice(0, 10) !== value) {
    throw new Error(`${label} is not a calendar date`);
  }
  return parsed;
}

function validateEvidence(evidence, label) {
  if (!Array.isArray(evidence) || evidence.length === 0) {
    throw new Error(`${label} requires non-empty evidence`);
  }
  for (const [index, item] of evidence.entries()) {
    if (typeof item === "string" && item.trim()) continue;
    if (!item || typeof item !== "object" || typeof item.ref !== "string" || !item.ref.trim()) {
      throw new Error(`${label}[${index}] must include a non-empty ref`);
    }
  }
}

function normalizedInventoryRecord(record) {
  return {
    ecosystem: record.ecosystem,
    name: record.name,
    version: record.version,
    license: record.license_normalized ?? record.license ?? null,
    scope: record.scope,
  };
}

function normalizedSignoffRecord(record) {
  return {
    ecosystem: record.ecosystem,
    name: record.name,
    version: record.version,
    license: record.license ?? record.license_normalized ?? null,
    scope: record.scope,
    conclusion: record.conclusion,
    obligations: record.obligations,
    reviewer: record.reviewer,
    reviewed_at: record.reviewed_at,
    valid_until: record.valid_until,
    evidence: record.evidence,
  };
}

/**
 * @param {object} options
 * @param {object} options.inventory
 * @param {object} options.signoffs
 * @param {string} options.releaseTag
 * @param {string} options.releaseCommit
 * @param {string} options.runUrl
 * @param {string} [options.asOf]
 * @returns {object}
 */
export function validateReleaseSignoffs({
  inventory,
  signoffs,
  releaseTag,
  releaseCommit,
  runUrl,
  asOf = new Date().toISOString().slice(0, 10),
}) {
  if (!inventory || !Array.isArray(inventory.records)) throw new Error("inventory.records must be an array");
  if (!signoffs || !Array.isArray(signoffs.records)) throw new Error("signoffs.records must be an array");
  if (signoffs.schema !== "airp-dependency-audit-signoffs-v1") throw new Error("unsupported signoff schema");
  if (!Number.isInteger(signoffs.expected_count) || signoffs.expected_count !== 17) {
    throw new Error("signoffs.expected_count must be exactly 17");
  }
  if (signoffs.records.length !== signoffs.expected_count) {
    throw new Error(`signoff count must be ${signoffs.expected_count}`);
  }
  if (!/^\S+$/.test(releaseTag ?? "")) throw new Error("releaseTag must be non-empty and contain no whitespace");
  if (!/^[0-9a-f]{40}$/i.test(releaseCommit ?? "")) throw new Error("releaseCommit must be a 40-character commit SHA");
  let parsedRunUrl;
  try {
    parsedRunUrl = new URL(runUrl);
  } catch {
    throw new Error("runUrl must be an absolute URL");
  }
  if (parsedRunUrl.protocol !== "https:") throw new Error("runUrl must use HTTPS");

  const asOfDate = validateDate(asOf, "asOf");
  validateDate(signoffs.reviewed_at, "signoffs.reviewed_at");
  if (typeof signoffs.reviewer !== "string" || !signoffs.reviewer.trim()) throw new Error("signoffs.reviewer is required");
  validateEvidence(signoffs.evidence, "signoffs.evidence");

  const actual = sortRecords(
    inventory.records.filter((record) => record.audit_class === "audit-required").map(normalizedInventoryRecord),
  );
  if (actual.length !== signoffs.expected_count) {
    throw new Error(`audit-required count changed: expected ${signoffs.expected_count}, got ${actual.length}`);
  }

  const rawSignoffs = signoffs.records.map(normalizedSignoffRecord);
  const signoffKeys = rawSignoffs.map(identity);
  if (new Set(signoffKeys).size !== signoffKeys.length) throw new Error("duplicate signoff identity");
  const actualKeys = actual.map(identity);
  const missing = actualKeys.filter((key) => !signoffKeys.includes(key));
  const extra = signoffKeys.filter((key) => !actualKeys.includes(key));
  if (missing.length || extra.length) {
    throw new Error(`signoff identity mismatch${missing.length ? `; missing: ${missing.join(", ")}` : ""}${extra.length ? `; extra: ${extra.join(", ")}` : ""}`);
  }

  const records = sortRecords(rawSignoffs).map((record) => {
    if (!["accepted", "accepted-with-obligations"].includes(record.conclusion)) {
      throw new Error(`${identity(record)} has invalid conclusion`);
    }
    if (typeof record.reviewer !== "string" || !record.reviewer.trim()) throw new Error(`${identity(record)} reviewer is required`);
    const reviewedAt = validateDate(record.reviewed_at, `${identity(record)} reviewed_at`);
    if (reviewedAt > asOfDate) throw new Error(`${identity(record)} reviewed_at is in the future`);
    const validUntil = validateDate(record.valid_until, `${identity(record)} valid_until`);
    if (validUntil < asOfDate) throw new Error(`${identity(record)} signoff is expired`);
    if (record.conclusion === "accepted-with-obligations" && (!Array.isArray(record.obligations) || record.obligations.length === 0)) {
      throw new Error(`${identity(record)} accepted-with-obligations requires obligations`);
    }
    if (!Array.isArray(record.obligations) || record.obligations.some((item) => typeof item !== "string" || !item.trim())) {
      throw new Error(`${identity(record)} obligations must be non-empty strings`);
    }
    validateEvidence(record.evidence, `${identity(record)} evidence`);
    return record;
  });

  return {
    schema: "airp-release-audit-v1",
    signoff_schema: signoffs.schema,
    signoff_reviewed_at: signoffs.reviewed_at,
    signoff_reviewer: signoffs.reviewer,
    release_tag: releaseTag,
    release_commit: releaseCommit.toLowerCase(),
    run_url: runUrl,
    as_of: asOf,
    audit_required_count: records.length,
    audit_required: records,
    signoff_status: "accepted-with-obligations",
    signoff_environment: "release",
  };
}

function parseArgs(argv) {
  const args = {
    inventory: null,
    signoffs: DEFAULT_SIGNOFFS,
    out: null,
    tag: null,
    commit: null,
    runUrl: null,
    asOf: new Date().toISOString().slice(0, 10),
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    const next = argv[index + 1];
    if (arg === "--inventory") args.inventory = next;
    else if (arg === "--signoffs") args.signoffs = next;
    else if (arg === "--out") args.out = next;
    else if (arg === "--tag") args.tag = next;
    else if (arg === "--commit") args.commit = next;
    else if (arg === "--run-url") args.runUrl = next;
    else if (arg === "--as-of") args.asOf = next;
    else if (arg === "--help" || arg === "-h") return null;
    else throw new Error(`unknown argument: ${arg}`);
    index += 1;
  }
  if (!args.inventory || !args.out || !args.tag || !args.commit || !args.runUrl) {
    throw new Error("required arguments: --inventory FILE --out FILE --tag TAG --commit SHA --run-url URL");
  }
  return args;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = parseArgs(process.argv.slice(2));
    if (!args) {
      process.stdout.write("Usage: node validate-release-signoffs.mjs --inventory FILE --out FILE --tag TAG --commit SHA --run-url URL [--signoffs FILE --as-of YYYY-MM-DD]\n");
      process.exit(0);
    }
    const report = validateReleaseSignoffs({
      inventory: readJson(args.inventory),
      signoffs: readJson(args.signoffs),
      releaseTag: args.tag,
      releaseCommit: args.commit,
      runUrl: args.runUrl,
      asOf: args.asOf,
    });
    fs.mkdirSync(path.dirname(path.resolve(args.out)), { recursive: true });
    fs.writeFileSync(args.out, `${JSON.stringify(report, null, 2)}\n`, "utf8");
    process.stdout.write(`validated ${report.audit_required_count} audit-required sign-offs for ${report.release_tag}\n`);
  } catch (error) {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  }
}
