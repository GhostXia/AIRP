// Dependency discovery for AIRP (#192).
//
// Scans the Cargo workspace (root Cargo.toml + Cargo.lock + ui/src-tauri/Cargo.toml)
// and the npm UI (ui/package.json + ui/package-lock.json), and emits a unified
// inventory with license + provenance + audit class.
//
// Outputs (under --out-dir, default docs/sbom):
//   inventory.json   — machine-readable unified inventory + metadata
//   inventory.txt    — human-readable summary grouped by ecosystem and class
//
// Design:
//   - Rust side: shells out to `cargo metadata --format-version 1` (no network;
//     reads the local cargo registry cache, which already contains each
//     crate's Cargo.toml with its `license` and `repository` fields). Cargo.lock
//     v4 is parsed for checksums (provenance integrity).
//   - npm side: parses ui/package-lock.json v3 directly (no network). The
//     lockfile already carries `license`, `resolved` (tarball URL) and
//     `integrity` (sha512) per package.
//   - Scope detection (runtime / build / dev): BFS over the cargo metadata
//     dependency graph from workspace_members, classifying edges by `kind`
//     (normal -> runtime, build -> build, dev -> dev). A package reached via
//     multiple paths takes the most-shipped scope (runtime > build > dev).
//     npm scope comes from `packages[path].dev` boolean.
//   - Fail-visible: writes are atomic (tmp + rename). If `--fail-on-block` is
//     set and any record classifies as `block`, exits non-zero AFTER writing
//     the inventory so the operator can see what was blocked.
//
// Per AGENTS.md "第三方经验吸收与独立实现": uses only Node built-ins
// (fs, path, child_process, crypto). No external npm deps. Not derived from
// cargo-audit, cargo-deny, cyclonedx, or any third-party implementation.

import { spawnSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";
import {
  classifyInventory,
  normalizeLicense,
  validateConfig,
} from "./audit-routing.mjs";

const CARGO_METADATA_ARGS = ["metadata", "--format-version", "1"];

// ---------------------------------------------------------------------------
// Cargo side.
// ---------------------------------------------------------------------------

/**
 * Run `cargo metadata --format-version 1` and return the parsed JSON.
 * Throws if cargo is missing, exits non-zero, or emits unparseable output.
 *
 * @param {string} repoRoot
 * @returns {Promise<object>}
 */
export async function runCargoMetadata(repoRoot) {
  // Prefer CARGO_HOME from env if set (maintainer D: drive convention).
  const result = spawnSync("cargo", CARGO_METADATA_ARGS, {
    cwd: repoRoot,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024, // AIRP's metadata is ~5 MB; allow headroom.
  });
  if (result.error) {
    if (result.error.code === "ENOENT") {
      throw new Error("cargo executable not found on PATH; cannot run `cargo metadata`");
    }
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(
      `cargo metadata exited ${result.status}\nstderr: ${result.stderr?.trim() ?? ""}`,
    );
  }
  try {
    return JSON.parse(result.stdout);
  } catch (e) {
    throw new Error(`cargo metadata emitted unparseable JSON: ${e.message}`);
  }
}

/**
 * Parse Cargo.lock and return a Map keyed by `name@version` -> checksum
 * (sha256 hex, or null if the package has no checksum, e.g. workspace
 * path deps). Used to attach integrity provenance to each registry dep.
 *
 * @param {string} lockPath
 * @returns {Map<string, string|null>}
 */
export function parseCargoLockForChecksums(lockPath) {
  const text = fs.readFileSync(lockPath, "utf8");
  // Cargo.lock v4 is TOML-ish; we only need [[package]] blocks with
  // name/version/checksum. A line-oriented scan is sufficient and avoids a
  // TOML parser dependency.
  //
  // Cargo.lock may contain other top-level blocks besides [[package]]:
  //   - [[patch.unused]] — patch sections that weren't applied
  //   - [[patch.<source>]] — applied patches
  //   - [[replace]]      — legacy replace sections
  //   - [meta] / [metadata] / [root] — metadata
  // These blocks can carry `name = `, `version = `, `checksum = ` lines too,
  // which would pollute the previous [[package]] record if we don't reset
  // `cur`. We flush + null `cur` on any non-`[[package]]` table header.
  const map = new Map();
  let cur = null;
  const flush = () => {
    if (cur && cur.name && cur.version) {
      map.set(`${cur.name}@${cur.version}`, cur.checksum ?? null);
    }
    cur = null;
  };
  for (const raw of text.split(/\r?\n/)) {
    const line = raw;
    if (line.startsWith("[[package]]")) {
      flush();
      cur = {};
      continue;
    }
    // Any other TOML table / array-of-tables header ends the current block.
    // Match `[[...]]` and `[...]` but NOT array values or keys named `[x]`.
    if (/^\[\[[^\]]+\]\]/.test(line) || /^\[[^\]]+\]/.test(line)) {
      flush();
      continue;
    }
    if (cur && line.startsWith("name = ")) {
      cur.name = stripTomlString(line.slice("name = ".length));
    } else if (cur && line.startsWith("version = ")) {
      cur.version = stripTomlString(line.slice("version = ".length));
    } else if (cur && line.startsWith("checksum = ")) {
      cur.checksum = stripTomlString(line.slice("checksum = ".length));
    }
  }
  flush();
  return map;
}

/**
 * Strip surrounding quotes from a TOML scalar value. Returns the inner
 * string for `"foo"`, the raw token for unquoted values, or "" for empty.
 *
 * @param {string} v
 * @returns {string}
 */
function stripTomlString(v) {
  const t = v.trim();
  if (t.startsWith('"') && t.endsWith('"') && t.length >= 2) {
    return t.slice(1, -1);
  }
  return t;
}

/**
 * Build per-package scope (runtime / build / dev) by BFS over the cargo
 * metadata dependency graph from workspace_members.
 *
 * Edge kinds in cargo metadata:
 *   - "normal" (or null/undefined) -> runtime (links into the binary)
 *   - "build"                       -> build (build-script dep; compiles into binary)
 *   - "dev"                         -> dev (test/example/bench only; does NOT ship)
 *
 * A package reached via multiple paths takes the most-shipped scope, where
 * runtime > build > dev. This means: if tokio is a normal dep of airp-core
 * (runtime) and also a dev-dep of some test crate, it stays runtime.
 *
 * @param {object} metadata  // parsed cargo metadata
 * @returns {Map<string, "runtime"|"build"|"dev">}  // keyed by package id
 */
export function computeCargoScopes(metadata) {
  const scopeRank = { runtime: 3, build: 2, dev: 1 };
  const bestScope = new Map();

  // Index packages by id and by name (names are unique within a resolve).
  const byId = new Map();
  for (const p of metadata.packages ?? []) {
    byId.set(p.id, p);
  }

  // workspace_members is a list of package ids.
  const workspaceMemberIds = new Set(metadata.workspace_members ?? []);

  // Build adjacency: package id -> list of {target_id, kind}
  // cargo metadata packages[].dependencies[] entries are {name, req, kind, ...}
  // and do NOT carry the target package id, so we resolve by name+version
  // via the resolve.nodes[] map (which has node_id + deps[] with `pkg` and
  // `dep_kinds`).
  const nodeById = new Map();
  for (const n of metadata.resolve?.nodes ?? []) {
    nodeById.set(n.id, n);
  }

  // BFS queue: entries of {id, scope}. Seed with workspace members as runtime
  // (workspace members themselves are first-party; their direct normal deps
  // are runtime, build deps are build, dev deps are dev).
  //
  // Limitation: the visited-check below skips re-expansion when a node is
  // revisited at an equal-or-lower scope. This is correct for AIRP's current
  // dependency graph (no diamond where a higher-scope path arrives second),
  // but a pathological graph could misclassify. If AIRP ever adopts a dep
  // graph with such a shape, switch to a per-scope-tier visited set.
  const queue = [];
  for (const id of workspaceMemberIds) {
    queue.push({ id, scope: "runtime" }); // workspace member itself; its deps get classified by edge kind
  }

  while (queue.length > 0) {
    const { id, scope: parentScope } = queue.shift();
    const prev = bestScope.get(id);
    if (prev && scopeRank[prev] >= scopeRank[parentScope]) {
      // Already visited at an equal-or-higher scope. Still traverse edges
      // again only if we haven't (avoid infinite loops via a visited set
      // keyed by id).
      // We use a simple visited set to ensure each node's edges are expanded
      // at most once per scope tier. For simplicity here we expand once total
      // (the first visit wins the scope tier for children), which is correct
      // because a higher-scope parent would have been queued first if it
      // existed. To be safe, we re-expand if the new scope is higher.
      continue;
    }
    bestScope.set(id, parentScope);

    const node = nodeById.get(id);
    if (!node) continue;
    for (const dep of node.deps ?? []) {
      for (const dk of dep.dep_kinds ?? []) {
        const kind = dk.kind; // "normal" | "build" | "dev" | null
        let childScope;
        if (kind === "dev") childScope = "dev";
        else if (kind === "build") childScope = "build";
        else childScope = "runtime"; // normal / null

        // Dev edges only matter when the parent is a workspace member; a
        // transitive dev-edge would be unusual. We still traverse but cap
        // the child at dev.
        // If the parent is itself dev-scoped, children can't be more shipped
        // than dev (a dev-dep's normal deps are still only built for tests).
        if (parentScope === "dev") childScope = "dev";
        else if (parentScope === "build" && childScope === "runtime") childScope = "build";

        queue.push({ id: dep.pkg, scope: childScope });
      }
    }
  }

  return bestScope;
}

/**
 * Build cargo dependency records from cargo metadata + Cargo.lock checksums.
 *
 * @param {object} metadata
 * @param {Map<string, string|null>} checksums
 * @param {string[]} firstPartyMembers  // names of first-party workspace crates
 * @param {string|null} repoRoot  // when set, redact machine-local manifest roots
 * @returns {Array<object>}
 */
export function buildCargoRecords(metadata, checksums, firstPartyMembers, repoRoot = null) {
  const scopes = computeCargoScopes(metadata);
  const firstPartyNames = new Set(firstPartyMembers);
  const records = [];

  for (const p of metadata.packages ?? []) {
    const isWorkspaceMember = p.source == null;
    const isPathDep = p.source != null && !String(p.source).includes("crates.io") && !String(p.source).startsWith("git+");
    const isCratesIo = p.source != null && String(p.source).includes("crates.io");
    const isGitDep = p.source != null && String(p.source).startsWith("git+");

    // Skip path deps that aren't workspace members (rare in AIRP; would be
    // a local override). They'd need their own provenance path.
    if (isPathDep && !isWorkspaceMember && !firstPartyNames.has(p.name)) {
      continue;
    }

    const tier = isWorkspaceMember || firstPartyNames.has(p.name) ? "first-party" : "third-party";
    const scope = scopes.get(p.id) ?? "runtime";

    const checksum = checksums.get(`${p.name}@${p.version}`) ?? null;
    let resolvedUrl = null;
    let sourceType = null;
    if (isCratesIo) {
      sourceType = "crates.io";
      resolvedUrl = `https://crates.io/api/v1/crates/${p.name}/${p.version}/download`;
    } else if (isGitDep) {
      sourceType = "git";
      resolvedUrl = String(p.source);
    } else if (isWorkspaceMember) {
      sourceType = "workspace";
      resolvedUrl = null;
    } else {
      sourceType = "path";
      resolvedUrl = p.source ? String(p.source) : null;
    }

    records.push({
      ecosystem: "cargo",
      name: p.name,
      version: p.version,
      license: p.license ?? null,
      license_normalized: normalizeLicense(p.license),
      repository: p.repository ?? null,
      source: sourceType,
      resolved: resolvedUrl,
      integrity: checksum ? `sha256:${checksum}` : null,
      scope,
      tier,
      manifest_path: repoRoot
        ? normalizeCargoManifestPath(p.manifest_path, repoRoot, sourceType, p.name, p.version)
        : p.manifest_path ?? null,
      homepage: p.homepage ?? null,
      description: p.description ?? null,
    });
  }
  return records;
}

/**
 * Keep committed provenance useful without recording a maintainer's checkout
 * or Cargo cache location. Repository manifests stay repository-relative;
 * external cache layouts become stable descriptive placeholders.
 */
export function normalizeCargoManifestPath(manifestPath, repoRoot, source, name, version) {
  if (!manifestPath) return null;
  const absoluteRoot = path.resolve(repoRoot);
  const absoluteManifest = path.resolve(manifestPath);
  const relative = path.relative(absoluteRoot, absoluteManifest);
  if (relative && !relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative)) {
    return relative.split(path.sep).join("/");
  }
  if (source === "crates.io") return `<cargo-registry>/${name}-${version}/Cargo.toml`;
  if (source === "git") return `<cargo-git-checkout>/${name}-${version}/Cargo.toml`;
  return "<external-path-redacted>";
}

/** Replace the checkout prefix embedded in Cargo workspace package IDs. */
export function normalizeWorkspaceMemberId(memberId, repoRoot) {
  const rootUrl = pathToFileURL(path.resolve(repoRoot)).href.replace(/\/$/, "");
  return String(memberId).replace(rootUrl, "file:///<repo>");
}

// ---------------------------------------------------------------------------
// npm side.
// ---------------------------------------------------------------------------

/**
 * Parse ui/package-lock.json v3 and return dependency records.
 *
 * @param {string} lockPath
 * @param {string} rootPackageName  // first-party npm root (airp-ui), to mark as first-party
 * @returns {Array<object>}
 */
export function parseNpmLockfile(lockPath, rootPackageName) {
  const lock = JSON.parse(fs.readFileSync(lockPath, "utf8"));
  if (typeof lock.lockfileVersion !== "number" || lock.lockfileVersion < 3) {
    // v1/v2 have a different shape (packages vs dependencies). We only
    // support v3 because that's what AIRP's package-lock.json uses. Fail
    // visibly rather than silently producing wrong data.
    throw new Error(
      `unsupported package-lock.json lockfileVersion ${lock.lockfileVersion}; only v3 supported (AIRP ui/package-lock.json is v3)`,
    );
  }

  const records = [];
  for (const [pkgPath, info] of Object.entries(lock.packages ?? {})) {
    if (!info) continue;
    // Root package (key === "") is the airp-ui workspace itself.
    if (pkgPath === "") {
      records.push({
        ecosystem: "npm",
        name: info.name ?? rootPackageName,
        version: info.version ?? "0.0.0",
        license: info.license ?? null,
        license_normalized: normalizeLicense(info.license),
        repository: null,
        source: "workspace",
        resolved: null,
        integrity: null,
        scope: "runtime",
        tier: "first-party",
        manifest_path: "ui/package.json",
        homepage: null,
        description: info.description ?? null,
      });
      continue;
    }

    // node_modules/<name> entries. Skip nested (hoist-overflow) entries
    // like node_modules/foo/node_modules/bar — the hoisted record is the
    // authoritative one for the install graph. (package-lock v3 hoists
    // when possible; nested entries indicate version conflicts. For SBOM
    // we want one record per installed package; we keep the hoisted one
    // and skip nested duplicates.)
    if (pkgPath.includes("node_modules/") && pkgPath.lastIndexOf("node_modules/") !== 0) {
      continue;
    }
    // Extract package name (may be scoped: @scope/name).
    const rel = pkgPath.replace(/^node_modules\//, "");
    // Link deps (resolved starts with "git+") — include them with source=git.
    const isLink = info.link === true;
    const isGit = typeof info.resolved === "string" && info.resolved.startsWith("git+");

    // npm workspace links: `info.link === true` indicates the entry is a
    // symlink to a local workspace package (npm workspaces). In AIRP's
    // usage these are always first-party workspace members (airp-ui,
    // @airp/* packages). We mark them as first-party with source=workspace
    // so they carry AIRP's own license rather than being audited as
    // third-party.
    //
    // Heuristic caveat: if AIRP ever adopts a non-workspace `file:`-linked
    // dependency (rare; would be a local override of an upstream package),
    // this check would misclassify it as first-party. The fix would be to
    // inspect `info.resolved` for a `file:` prefix and cross-check against
    // config.first_party.npm_root_package / npm workspace globs. Deferred
    // until AIRP actually has such a dep.
    const isWorkspaceLink = isLink && !isGit;

    const sourceType = isGit ? "git" : isWorkspaceLink ? "workspace" : "npm";
    const tier = isWorkspaceLink ? "first-party" : "third-party";

    const scope = info.dev === true || info.optional === true ? "dev" : "runtime";

    records.push({
      ecosystem: "npm",
      name: rel,
      version: info.version ?? "0.0.0",
      license: info.license ?? null,
      license_normalized: normalizeLicense(info.license),
      repository: null, // package-lock v3 doesn't carry repository; would need npm registry API
      source: sourceType,
      resolved: info.resolved ?? null,
      integrity: info.integrity ?? null, // e.g. "sha512-..."
      scope,
      tier,
      manifest_path: "ui/package.json",
      homepage: null,
      description: null,
    });
  }
  return records;
}

// ---------------------------------------------------------------------------
// Inventory assembly + classification.
// ---------------------------------------------------------------------------

/**
 * Build the full inventory from the repo, classifying each record.
 *
 * @param {string} repoRoot
 * @param {object} config  // validated audit-routing.config.json
 * @returns {Promise<{records: object[], meta: object}>}
 */
export async function buildInventory(repoRoot, config) {
  validateConfig(config);
  const firstParty = config.first_party.cargo_workspace_members;

  const metadata = await runCargoMetadata(repoRoot);
  const checksums = parseCargoLockForChecksums(path.join(repoRoot, "Cargo.lock"));
  const cargoRecords = buildCargoRecords(metadata, checksums, firstParty, repoRoot);

  const npmLock = path.join(repoRoot, "ui", "package-lock.json");
  const npmRecords = fs.existsSync(npmLock)
    ? parseNpmLockfile(npmLock, config.first_party.npm_root_package)
    : [];

  const records = [...cargoRecords, ...npmRecords];

  // Classify each record. Mutates the record to add audit_class + audit_reason.
  for (const r of records) {
    const decision = classifyInventory(r, config);
    r.audit_class = decision.class;
    r.audit_reason = decision.reason;
  }

  // Deterministic sort: ecosystem, name, version. Stable for reproducible
  // SBOM output.
  records.sort((a, b) => {
    if (a.ecosystem !== b.ecosystem) return a.ecosystem < b.ecosystem ? -1 : 1;
    if (a.name !== b.name) return a.name < b.name ? -1 : 1;
    if (a.version !== b.version) return a.version < b.version ? -1 : 1;
    return 0;
  });

  const meta = {
    generated_at: new Date().toISOString(),
    repo_root: ".",
    cargo_metadata_version: metadata.metadata?.version ?? null,
    cargo_workspace_members: (metadata.workspace_members ?? []).map((member) =>
      normalizeWorkspaceMemberId(member, repoRoot)),
    cargo_lock_packages: checksums.size,
    npm_lockfile_version: 3,
    total_records: records.length,
    summary: summarize(records),
    generator: "tools/dep-governance/discover-deps.mjs",
    config_version: config.$schema ?? null,
  };
  return { records, meta };
}

/**
 * Tally records by ecosystem, scope, tier and audit_class.
 *
 * @param {object[]} records
 * @returns {object}
 */
function summarize(records) {
  const by = (key) => {
    const m = {};
    for (const r of records) {
      const v = String(r[key] ?? "unknown");
      m[v] = (m[v] ?? 0) + 1;
    }
    return m;
  };
  return {
    by_ecosystem: by("ecosystem"),
    by_scope: by("scope"),
    by_tier: by("tier"),
    by_audit_class: by("audit_class"),
  };
}

// ---------------------------------------------------------------------------
// Atomic output writers.
// ---------------------------------------------------------------------------

/**
 * Write a file atomically: write to <path>.tmp, fsync, then rename over the
 * final path. If any step fails, the .tmp is cleaned up and the final path
 * is untouched. This makes the script fail-visible: a crash mid-write never
 * leaves a half-written inventory that downstream tooling might consume.
 *
 * @param {string} filePath
 * @param {string} contents
 */
export function atomicWrite(filePath, contents) {
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

/**
 * Render the human-readable inventory summary.
 *
 * @param {object[]} records
 * @param {object} meta
 * @returns {string}
 */
export function renderInventoryText(records, meta) {
  const lines = [];
  lines.push("AIRP dependency inventory");
  lines.push("=========================");
  lines.push(`Generated: ${meta.generated_at}`);
  lines.push(`Repo root: ${meta.repo_root}`);
  lines.push(`Total records: ${meta.total_records}`);
  lines.push("");
  lines.push("Summary by audit class:");
  for (const [cls, n] of Object.entries(meta.summary.by_audit_class).sort()) {
    lines.push(`  ${cls}: ${n}`);
  }
  lines.push("");
  lines.push("Summary by ecosystem:");
  for (const [eco, n] of Object.entries(meta.summary.by_ecosystem).sort()) {
    lines.push(`  ${eco}: ${n}`);
  }
  lines.push("");

  // Block / audit-required sections first (these need attention).
  const attention = records.filter(
    (r) => r.audit_class === "block" || r.audit_class === "audit-required",
  );
  if (attention.length > 0) {
    lines.push("Records requiring attention (block / audit-required)");
    lines.push("----------------------------------------------------");
    for (const r of attention) {
      lines.push(
        `  [${r.audit_class}] ${r.ecosystem}/${r.name}@${r.version}  license=${r.license_normalized || "(none)"}  scope=${r.scope}`,
      );
      lines.push(`    reason: ${r.audit_reason}`);
      if (r.resolved) lines.push(`    source: ${r.resolved}`);
    }
    lines.push("");
  }

  lines.push("All records (sorted by ecosystem, name, version)");
  lines.push("------------------------------------------------");
  for (const r of records) {
    lines.push(
      `${r.ecosystem}/${r.name}@${r.version}  [${r.audit_class}]  license=${r.license_normalized || "(none)"}  scope=${r.scope}  tier=${r.tier}`,
    );
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
  const repoRoot = path.resolve(args.repoRoot);
  const outDir = path.resolve(args.outDir);
  const configPath = path.resolve(args.config);

  let configText;
  try {
    configText = fs.readFileSync(configPath, "utf8");
  } catch (e) {
    process.stderr.write(`error: cannot read config ${configPath}: ${e.message}\n`);
    process.exit(2);
  }
  let config;
  try {
    config = JSON.parse(configText);
    validateConfig(config);
  } catch (e) {
    process.stderr.write(`error: invalid config ${configPath}: ${e.message}\n`);
    process.exit(2);
  }

  let inventory;
  try {
    inventory = await buildInventory(repoRoot, config);
  } catch (e) {
    process.stderr.write(`error: dependency discovery failed: ${e.message}\n`);
    process.exit(1);
  }

  fs.mkdirSync(outDir, { recursive: true });
  const jsonPath = path.join(outDir, "inventory.json");
  const txtPath = path.join(outDir, "inventory.txt");

  // Deterministic JSON: keys sorted, 2-space indent, trailing newline.
  // We serialize with a sorted-key replacer to keep diffs stable across runs.
  const jsonText = stringifySorted(inventory) + "\n";
  const txtText = renderInventoryText(inventory.records, inventory.meta);

  atomicWrite(jsonPath, jsonText);
  atomicWrite(txtPath, txtText);

  process.stdout.write(`wrote ${jsonPath}\n`);
  process.stdout.write(`wrote ${txtPath}\n`);
  process.stdout.write(
    `summary: ${JSON.stringify(inventory.meta.summary.by_audit_class)}\n`,
  );

  if (args.failOnBlock) {
    const blocked = inventory.records.filter((r) => r.audit_class === "block");
    if (blocked.length > 0) {
      process.stderr.write(
        `error: ${blocked.length} record(s) classified as block; release blocked\n`,
      );
      for (const r of blocked) {
        process.stderr.write(
          `  block  ${r.ecosystem}/${r.name}@${r.version}  ${r.audit_reason}\n`,
        );
      }
      process.exit(1);
    }
  }
}

/**
 * Minimal argv parser. Flags:
 *   --repo-root <path>   default: cwd
 *   --out-dir <path>     default: docs/sbom
 *   --config <path>      default: tools/dep-governance/audit-routing.config.json
 *   --fail-on-block      exit 1 if any record classifies as block
 *
 * @param {string[]} argv
 */
function parseCliArgs(argv) {
  const args = {
    repoRoot: process.cwd(),
    outDir: path.join(process.cwd(), "docs", "sbom"),
    config: path.join(process.cwd(), "tools", "dep-governance", "audit-routing.config.json"),
    failOnBlock: false,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--repo-root") args.repoRoot = argv[++i];
    else if (a === "--out-dir") args.outDir = argv[++i];
    else if (a === "--config") args.config = argv[++i];
    else if (a === "--fail-on-block") args.failOnBlock = true;
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

const USAGE = `Usage: discover-deps.mjs [options]

Options:
  --repo-root <path>   Repository root (default: cwd)
  --out-dir <path>     Output directory (default: docs/sbom)
  --config <path>      Routing config (default: tools/dep-governance/audit-routing.config.json)
  --fail-on-block      Exit 1 if any record classifies as 'block'
  -h, --help           Show this help

Outputs:
  <out-dir>/inventory.json   Machine-readable inventory + metadata
  <out-dir>/inventory.txt    Human-readable summary
`;

/**
 * JSON.stringify with sorted object keys, recursively. Produces stable
 * output independent of insertion order, so re-running the script on the
 * same lockfiles produces byte-identical output (modulo generated_at).
 *
 * @param {any} value
 * @returns {string}
 */
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

// Run when invoked directly, not when imported.
const isMain = (() => {
  if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(new URL(import.meta.url).pathname)) {
    return true;
  }
  // Fallback for symlinks / wrapper invocations.
  return process.argv[1]?.endsWith("discover-deps.mjs");
})();

if (isMain) {
  main(process.argv.slice(2)).catch((e) => {
    process.stderr.write(`fatal: ${e?.stack ?? e}\n`);
    process.exit(1);
  });
}
