// Audit routing engine for AIRP dependency governance (#192).
//
// Pure functions only. No I/O, no network, no filesystem. This module is the
// single source of truth for two routing decisions:
//
//   1. classifyInventory(record, config) — classifies a CURRENT dependency
//      record (from the SBOM inventory) as auto-pass / audit-required / block.
//      Used by discover-deps.mjs and generate-sbom.mjs (#190).
//
//   2. classifyUpgrade(current, target, depMeta, config) — classifies a
//      PROPOSED version upgrade into one of the five routing classes
//      {patch, minor, major, 0x-minor, prerelease} plus the patch-sensitive
//      override, per docs/DEV-GUIDE.md §7.1. Used by the upgrade detector
//      (dry-run fixtures in tests/routing-samples.json; the live detector is
//      a documented future step — see README.md "Deferred").
//
// Policy anchors:
//   - docs/DEV-GUIDE.md §7.1 (dependency version discovery & upgrade audit)
//   - docs/AGENTS.md "第三方经验吸收与独立实现" (independent implementation,
//     no third-party code reuse)
//   - tools/dep-governance/audit-routing.config.json (license tiers, sensitive
//     areas, dedup key format)
//
// This module deliberately uses only Node built-ins. The semver parser below
// is a minimal independent implementation covering the forms AIRP actually
// sees in Cargo.lock and package-lock.json; it is NOT a full SPDX/semver
// validator and does not pretend to be.

// ---------------------------------------------------------------------------
// Minimal semver parser.
//
// Accepts: MAJOR.MINOR.PATCH[-PRERELEASE][+BUILD]
// Examples handled:
//   "1.2.3", "0.1.0", "1.0.0-alpha", "2.0.0-beta.1+build.42",
//   "0.3.49", "1.7", "1.0.0-alpha.1"
// Returns null for unparseable input (caller decides how to treat null).
// ---------------------------------------------------------------------------

const SEMVER_RE =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z.-]+))?(?:\+([0-9A-Za-z.-]+))?$/;

// Loose form: MAJOR.MINOR (no patch). Cargo sometimes pins only major.minor
// in manifests; the lockfile always has full MAJOR.MINOR.PATCH. We accept
// the loose form by treating missing patch as 0, but only when the input
// clearly has no prerelease/build suffix (which requires a patch slot).
const LOOSE_SEMVER_RE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

/**
 * @typedef {Object} Semver
 * @property {number} major
 * @property {number} minor
 * @property {number} patch
 * @property {string|null} prerelease  // e.g. "alpha.1" (without leading "-")
 * @property {string|null} build       // e.g. "build.42" (without leading "+")
 * @property {string} raw              // original input string
 */

/**
 * Parse a version string into a Semver object, or null if unparseable.
 * Does NOT validate against the full semver 2.0 spec; it accepts the forms
 * AIRP actually encounters in lockfiles.
 *
 * @param {string} v
 * @returns {Semver|null}
 */
export function parseSemver(v) {
  if (typeof v !== "string") return null;
  const trimmed = v.trim();
  if (trimmed === "") return null;

  let m = SEMVER_RE.exec(trimmed);
  if (m) {
    return {
      major: Number(m[1]),
      minor: Number(m[2]),
      patch: Number(m[3]),
      prerelease: m[4] ?? null,
      build: m[5] ?? null,
      raw: trimmed,
    };
  }

  m = LOOSE_SEMVER_RE.exec(trimmed);
  if (m) {
    return {
      major: Number(m[1]),
      minor: Number(m[2]),
      patch: 0,
      prerelease: null,
      build: null,
      raw: trimmed,
    };
  }
  return null;
}

/**
 * Is the parsed version a prerelease? (Has a non-empty prerelease tag.)
 * @param {Semver} s
 * @returns {boolean}
 */
export function isPrerelease(s) {
  return s != null && s.prerelease != null && s.prerelease !== "";
}

/**
 * Compare two parsed semvers per semver 2.0 precedence rules (simplified):
 *   - majors, then minors, then patches numerically
 *   - a version with prerelease has LOWER precedence than the same without
 *   - prerelease identifiers compared dot-split, numeric < alphanumeric,
 *     numeric fields compared numerically, alphanumeric lexically
 * Build metadata is ignored for precedence (per semver 2.0 §10).
 *
 * Returns negative if a < b, 0 if equal, positive if a > b.
 *
 * @param {Semver} a
 * @param {Semver} b
 * @returns {number}
 */
export function compareSemver(a, b) {
  // Defensive: exported function should not crash on null. `classifyUpgrade`
  // filters null before calling this, but the export contract is public.
  if (a == null || b == null) {
    if (a == null && b == null) return 0;
    return a == null ? -1 : 1;
  }
  if (a.major !== b.major) return a.major - b.major;
  if (a.minor !== b.minor) return a.minor - b.minor;
  if (a.patch !== b.patch) return a.patch - b.patch;

  // No prerelease > has prerelease (for same major.minor.patch).
  const aPre = a.prerelease;
  const bPre = b.prerelease;
  if (aPre == null && bPre == null) return 0;
  if (aPre == null) return 1; // a has no pre, b has pre -> a > b
  if (bPre == null) return -1;

  const aIds = aPre.split(".");
  const bIds = bPre.split(".");
  const len = Math.min(aIds.length, bIds.length);
  for (let i = 0; i < len; i++) {
    const ai = aIds[i];
    const bi = bIds[i];
    const aNum = /^\d+$/.test(ai);
    const bNum = /^\d+$/.test(bi);
    if (aNum && bNum) {
      const an = Number(ai);
      const bn = Number(bi);
      if (an !== bn) return an - bn;
    } else if (aNum && !bNum) {
      return -1; // numeric < alphanumeric
    } else if (!aNum && bNum) {
      return 1;
    } else {
      if (ai < bi) return -1;
      if (ai > bi) return 1;
    }
  }
  return aIds.length - bIds.length;
}

// ---------------------------------------------------------------------------
// SPDX expression parser (independent implementation, per AGENTS.md).
//
// Grammar (subset of SPDX 2.1):
//   expression := or_expr
//   or_expr    := and_expr ("OR" and_expr)*
//   and_expr   := with_expr ("AND" with_expr)*
//   with_expr  := atom ("WITH" atom)?
//   atom       := LICENSE_ID | "(" expression ")"
//
// AST nodes:
//   {type: "license", id: "MIT"}
//   {type: "with", license: <AST>, exception: "LLVM-exception"}
//   {type: "or", left: <AST>, right: <AST>}
//   {type: "and", left: <AST>, right: <AST>}
//
// The parser is permissive: it only validates structure (grammar), not
// semantics (whether a license id is a real SPDX id). Semantic validation
// is done by the classifier (which checks against config tier lists) and
// by toSpdxExpression in generate-sbom.mjs (which checks against
// KNOWN_SPDX_IDS / KNOWN_SPDX_EXCEPTION_IDS).
//
// Returns null for unparseable input.
// ---------------------------------------------------------------------------

/**
 * Tokenize an SPDX expression string into a stream of tokens.
 * Tokens: {type: "id", value}, {type: "op", value: "OR"|"AND"|"WITH"},
 *         {type: "lparen"}, {type: "rparen"}.
 *
 * @param {string} expr
 * @returns {Array<object>}
 */
function tokenizeSpdx(expr) {
  const tokens = [];
  let i = 0;
  while (i < expr.length) {
    const c = expr[i];
    if (/\s/.test(c)) { i++; continue; }
    if (c === "(") { tokens.push({ type: "lparen" }); i++; continue; }
    if (c === ")") { tokens.push({ type: "rparen" }); i++; continue; }
    // Identifier or operator. SPDX identifiers are [A-Za-z0-9.-]+.
    let j = i;
    while (j < expr.length && /[A-Za-z0-9.-]/.test(expr[j])) j++;
    if (j === i) {
      // Unknown character; skip it (lenient parsing).
      i++;
      continue;
    }
    const word = expr.slice(i, j);
    if (word === "OR" || word === "AND" || word === "WITH") {
      tokens.push({ type: "op", value: word });
    } else {
      tokens.push({ type: "id", value: word });
    }
    i = j;
  }
  return tokens;
}

/**
 * Parse an SPDX expression string into an AST. Returns null for empty or
 * unparseable input. Throws internally are caught and converted to null.
 *
 * @param {string|null|undefined} expr
 * @returns {object|null}
 *   AST node, or null if unparseable.
 */
export function parseSpdxExpression(expr) {
  if (!expr || typeof expr !== "string") return null;
  const tokens = tokenizeSpdx(expr.trim());
  if (tokens.length === 0) return null;

  let pos = 0;
  const peek = () => tokens[pos];
  const consume = () => tokens[pos++];

  function parseAtom() {
    const t = peek();
    if (!t) throw new Error("unexpected end of expression");
    if (t.type === "lparen") {
      consume();
      const inner = parseOr();
      const next = consume();
      if (!next || next.type !== "rparen") throw new Error("expected )");
      return inner;
    }
    if (t.type === "id") {
      consume();
      return { type: "license", id: t.value };
    }
    throw new Error(`unexpected token ${JSON.stringify(t)}`);
  }

  function parseWith() {
    let left = parseAtom();
    const t = peek();
    if (t && t.type === "op" && t.value === "WITH") {
      consume();
      const exc = parseAtom();
      if (!exc || exc.type !== "license") {
        throw new Error("WITH requires a license-exception identifier");
      }
      left = { type: "with", license: left, exception: exc.id };
    }
    return left;
  }

  function parseAnd() {
    let left = parseWith();
    let t = peek();
    while (t && t.type === "op" && t.value === "AND") {
      consume();
      const right = parseWith();
      left = { type: "and", left, right };
      t = peek();
    }
    return left;
  }

  function parseOr() {
    let left = parseAnd();
    let t = peek();
    while (t && t.type === "op" && t.value === "OR") {
      consume();
      const right = parseAnd();
      left = { type: "or", left, right };
      t = peek();
    }
    return left;
  }

  try {
    const ast = parseOr();
    if (pos < tokens.length) {
      // Trailing tokens — expression didn't fully parse.
      return null;
    }
    return ast;
  } catch {
    return null;
  }
}

/**
 * Render an AST back to a canonical SPDX expression string. Used for SBOM
 * output so the rendered expression is deterministic regardless of input
 * whitespace/paren style.
 *
 * @param {object} ast
 * @returns {string}
 */
export function renderSpdxAst(ast) {
  if (!ast) return "";
  switch (ast.type) {
    case "license":
      return ast.id;
    case "with":
      return `${renderSpdxAst(ast.license)} WITH ${ast.exception}`;
    case "or":
      return `${renderSpdxAst(ast.left)} OR ${renderSpdxAst(ast.right)}`;
    case "and":
      return `${renderSpdxAst(ast.left)} AND ${renderSpdxAst(ast.right)}`;
    default:
      return "";
  }
}

/**
 * Is the AST a composite expression (OR / AND / WITH)? Used by the SBOM
 * generator to decide between `license.id` (single) and `license.expression`
 * (composite) per CycloneDX 1.5 schema.
 *
 * @param {object} ast
 * @returns {boolean}
 */
export function isCompositeSpdxAst(ast) {
  return ast != null && ast.type !== "license";
}

/**
 * Extract the flat set of license IDs from an AST, ignoring operators and
 * exceptions. Used for backward-compatible component listing and for
 * "does any component match a blocked license?" semantics where the caller
 * explicitly wants the flat set (e.g. diagnostic output).
 *
 * @param {object|null} ast
 * @returns {string[]}
 */
export function extractLicenseIds(ast) {
  if (!ast) return [];
  switch (ast.type) {
    case "license":
      return [ast.id];
    case "with":
      return extractLicenseIds(ast.license);
    case "or":
    case "and":
      return [...extractLicenseIds(ast.left), ...extractLicenseIds(ast.right)];
    default:
      return [];
  }
}

// ---------------------------------------------------------------------------
// Inventory routing (current deps → audit class).
// ---------------------------------------------------------------------------

/**
 * Normalize a license string for matching. SPDX expressions use OR/AND/WITH
 * and are case-sensitive in spec, but crates and npm often vary casing and
 * spacing. We normalize to a trimmed, single-spaced form for exact-match
 * against the configured allow/audit/block lists.
 *
 * SPDX 1.x used "/" as the OR separator (e.g. "BSD-3-Clause/MIT"); SPDX 2.1+
 * deprecated "/" in favor of the "OR" keyword. Cargo's `license` field
 * accepts both forms and many crates still ship the legacy "/" form. We
 * normalize "/" to " OR " so that both forms are treated identically by the
 * tier-matchers below. No SPDX license identifier contains "/", so this
 * substitution is safe.
 *
 * @param {string|null|undefined} license
 * @returns {string}  // "" if null/unknown
 */
export function normalizeLicense(license) {
  if (license == null) return "";
  const t = String(license).trim();
  if (t === "") return "";
  // Replace legacy "/" OR-separator with the SPDX 2.1+ " OR " keyword.
  // Surround with spaces so it splits cleanly downstream.
  const withOr = t.replace(/\s*\/\s*/g, " OR ");
  // Collapse internal whitespace runs to a single space.
  return withOr.replace(/\s+/g, " ").trim();
}

/**
 * Split an SPDX license expression into its component license identifiers.
 *
 * DEPRECATED: prefer `parseSpdxExpression` + `extractLicenseIds` for proper
 * paren/WITH handling. This function is retained for backward compatibility
 * with external callers; it now delegates to the AST-based implementation.
 *
 * Note: WITH clauses are preserved attached to their license (e.g.
 * `"Apache-2.0 WITH LLVM-exception"` → `["Apache-2.0 WITH LLVM-exception"]`),
 * and parentheses are honored so only top-level OR/AND operators split.
 *
 * @param {string} expr
 * @returns {string[]}
 */
export function splitLicenseExpression(expr) {
  if (!expr) return [];
  const ast = parseSpdxExpression(expr);
  if (!ast) {
    // Fall back to whitespace splitting for unparseable input.
    return String(expr).trim().split(/\s+/).filter((s) => s.length > 0);
  }
  // For a single license (no operators), return [rendered] to preserve WITH.
  if (ast.type === "license" || ast.type === "with") {
    return [renderSpdxAst(ast)];
  }
  // For composite expressions, extract individual license IDs (flattened).
  // This loses structure info but is what the legacy contract promised.
  return extractLicenseIds(ast);
}

/**
 * Tier ranking for combining classifications across SPDX expression operators.
 * Higher number = more permissive.
 *   auto-pass = 3 (best for OR, best for AND requires all auto-pass)
 *   audit-required = 2
 *   block = 1 (worst)
 */
const TIER_RANK = { "auto-pass": 3, "audit-required": 2, "block": 1 };

/**
 * Classify a single license identifier (no operators) against the config
 * tier lists. Honors the strong-copyleft-in-dev-scope downgrade: a strong
 * copyleft license in non-runtime scope is downgraded from `block` to
 * `audit-required` when `strong_copyleft_in_runtime_scope` and
 * `copyleft_in_dev_scope_only` are both set.
 *
 * @param {string} id              SPDX license id (e.g. "MIT", "GPL-3.0")
 * @param {Object} config          parsed audit-routing.config.json
 * @param {string} scope           "runtime" | "build" | "dev"
 * @returns {{class: string, reason: string}}
 */
function classifyLicenseId(id, config, scope) {
  const block = config.inventory_routing?.block ?? { licenses: [] };
  const audit = config.inventory_routing?.audit_required ?? { licenses: [] };
  const pass = config.inventory_routing?.auto_pass ?? { licenses: [] };

  if (block.licenses?.includes(id)) {
    const isStrongCopyleft = /^(GPL|AGPL|SSPL|BUSL)/.test(id);
    if (
      isStrongCopyleft &&
      scope !== "runtime" &&
      block.strong_copyleft_in_runtime_scope &&
      audit.copyleft_in_dev_scope_only
    ) {
      return {
        class: "audit-required",
        reason: `strong copyleft ${id} in ${scope} scope; file-level isolation audit required`,
      };
    }
    return {
      class: "block",
      reason: `license ${id} incompatible with AIRP MIT OR Apache-2.0 distribution in ${scope} scope`,
    };
  }
  if (audit.licenses?.includes(id)) {
    return {
      class: "audit-required",
      reason: `license ${id} requires dedicated audit (weak copyleft / non-commercial / attribution)`,
    };
  }
  if (pass.licenses?.includes(id)) {
    return { class: "auto-pass", reason: `permissive license (${id})` };
  }
  return {
    class: "audit-required",
    reason: `license ${id} not in any configured tier; add to audit-routing.config.json or verify permissive`,
  };
}

/**
 * Classify an SPDX expression AST node against the config tier lists.
 *
 * Operator semantics (SPDX 2.1):
 *   - `A OR B`: recipient may choose either. Classification is the BEST of
 *     A and B (recipient can choose the more permissive option).
 *   - `A AND B`: recipient must comply with both. Classification is the
 *     WORST of A and B (recipient is bound by the stricter).
 *   - `A WITH exception`: classification follows A (the exception may add
 *     or remove obligations but does not change the license tier for our
 *     routing purposes).
 *
 * Scope downgrade for strong copyleft in non-runtime scope is honored at
 * the leaf level (per-license), so OR/AND combinations propagate the
 * downgraded tier correctly.
 *
 * @param {Object} ast          AST node from parseSpdxExpression
 * @param {Object} config       parsed audit-routing.config.json
 * @param {string} scope        "runtime" | "build" | "dev"
 * @returns {{class: string, reason: string}}
 */
export function classifySpdxAst(ast, config, scope) {
  if (!ast) {
    return { class: "audit-required", reason: "empty SPDX expression" };
  }
  switch (ast.type) {
    case "license":
      return classifyLicenseId(ast.id, config, scope);
    case "with":
      // License WITH exception: classify the license; exception does not
      // change the tier. (We could refine this by recognizing specific
      // exception IDs in the future, e.g. GCC-exception-3.1 transforms
      // GPL-3.0 into a more permissive form, but that's out of scope.)
      return classifyLicenseId(ast.license.id, config, scope);
    case "or": {
      const l = classifySpdxAst(ast.left, config, scope);
      const r = classifySpdxAst(ast.right, config, scope);
      const best = TIER_RANK[l.class] >= TIER_RANK[r.class] ? l : r;
      return {
        class: best.class,
        reason: `OR (${l.class} | ${r.class}); recipient may choose ${best.class} branch`,
      };
    }
    case "and": {
      const l = classifySpdxAst(ast.left, config, scope);
      const r = classifySpdxAst(ast.right, config, scope);
      const worst = TIER_RANK[l.class] <= TIER_RANK[r.class] ? l : r;
      return {
        class: worst.class,
        reason: `AND (${l.class} & ${r.class}); recipient bound by stricter`,
      };
    }
    default:
      return {
        class: "audit-required",
        reason: `unknown AST node type ${ast?.type}`,
      };
  }
}

/**
 * Classify a dependency inventory record.
 *
 * @param {Object} record
 * @param {string} record.name
 * @param {string} record.version
 * @param {string} record.ecosystem     // "cargo" | "npm"
 * @param {string|null|undefined} record.license
 * @param {string} record.scope         // "runtime" | "dev" | "build"
 * @param {string} record.tier          // "first-party" | "third-party"
 * @param {Object} config              // parsed audit-routing.config.json
 * @returns {{class: "auto-pass"|"audit-required"|"block", reason: string}}
 */
export function classifyInventory(record, config) {
  if (!record) throw new TypeError("record required");
  if (!config) throw new TypeError("config required");

  // First-party deps (workspace members, npm root) auto-pass: they carry
  // AIRP's own license, not a third-party license.
  if (record.tier === "first-party") {
    return { class: "auto-pass", reason: "first-party (AIRP workspace member)" };
  }

  const norm = normalizeLicense(record.license);
  if (norm === "") {
    // Unknown license. Fail-visible: requires audit, NOT silent auto-pass.
    if (config.inventory_routing?.audit_required?.unknown_license) {
      return {
        class: "audit-required",
        reason: "license field empty or null; upstream license must be verified before release",
      };
    }
    return { class: "audit-required", reason: "license field empty or null" };
  }

  // Parse the normalized expression to an AST. If parsing fails, fall back
  // to audit-required (conservative: never silent auto-pass).
  const ast = parseSpdxExpression(norm);
  if (!ast) {
    return {
      class: "audit-required",
      reason: `license "${norm}" could not be parsed as an SPDX expression; manual classification required`,
    };
  }

  // Classify the AST. This handles OR/AND/WITH semantics correctly:
  //   - "MIT OR GPL-3.0" → auto-pass (recipient chooses MIT)
  //   - "MIT AND GPL-3.0" → block (recipient must comply with both)
  //   - "(GPL-3.0)" → block (parens stripped by parser)
  //   - "Apache-2.0 WITH LLVM-exception" → auto-pass (Apache-2.0 is permissive)
  return classifySpdxAst(ast, config, record.scope);
}

// ---------------------------------------------------------------------------
// Upgrade routing (proposed version bumps → routing class).
// ---------------------------------------------------------------------------

/**
 * Is this dependency name (or its area tags) in a sensitive area that
 * escalates patch bumps to patch-sensitive (issue required)?
 *
 * @param {string} name
 * @param {string[]} areaTags   // area tags the caller associates with this dep
 * @param {Object} config
 * @returns {boolean}
 */
export function isSensitive(name, areaTags, config) {
  const patterns = config.upgrade_routing?.sensitive_areas?.name_patterns ?? [];
  const tags = config.upgrade_routing?.sensitive_areas?.area_tags ?? [];
  const lcName = String(name ?? "").toLowerCase();
  for (const p of patterns) {
    if (p && lcName.includes(p.toLowerCase())) return true;
  }
  for (const t of areaTags ?? []) {
    if (tags.includes(t)) return true;
  }
  return false;
}

/**
 * Classify a proposed version upgrade.
 *
 * @param {string} currentVersion  // locked version today
 * @param {string} targetVersion   // proposed upstream version
 * @param {Object} depMeta
 * @param {string} depMeta.name
 * @param {string} depMeta.ecosystem  // "cargo" | "npm"
 * @param {string[]} [depMeta.areaTags]  // optional area tags for sensitivity
 * @param {Object} config
 * @returns {{
 *   class: "patch"|"minor"|"major"|"0x-minor"|"prerelease"|"patch-sensitive",
 *   routing: "auto-pr"|"issue"|"skip",
 *   reason: string,
 *   dedupKey: string,
 *   current: Semver|null,
 *   target: Semver|null,
 * }}
 */
export function classifyUpgrade(currentVersion, targetVersion, depMeta, config) {
  if (!depMeta) throw new TypeError("depMeta required");
  if (!config) throw new TypeError("config required");

  const current = parseSemver(currentVersion);
  const target = parseSemver(targetVersion);
  const dedupKey = makeDedupKey(depMeta.ecosystem, depMeta.name, targetVersion);

  // Unparseable versions cannot be auto-classified. Route to issue so a
  // human inspects; never silently auto-PR.
  if (current == null || target == null) {
    return {
      class: "patch-sensitive", // closest "issue required" bucket
      routing: "issue",
      reason: `unparseable version (current=${currentVersion}, target=${targetVersion}); manual classification required`,
      dedupKey,
      current,
      target,
    };
  }

  // Prerelease target: never auto-adopted, regardless of bump size.
  if (isPrerelease(target)) {
    return {
      class: "prerelease",
      routing: config.upgrade_routing.classes.prerelease.routing,
      reason: `target ${targetVersion} is a prerelease (${target.prerelease}); not auto-adopted per DEV-GUIDE §7.1`,
      dedupKey,
      current,
      target,
    };
  }

  const cmp = compareSemver(target, current);
  if (cmp === 0) {
    // Same version: no-op. We still return a class so the caller can record
    // the decision; routing is "skip" (nothing to do).
    return {
      class: "patch",
      routing: "skip",
      reason: `target ${targetVersion} equals current; no upgrade`,
      dedupKey,
      current,
      target,
    };
  }
  if (cmp < 0) {
    // Downgrade. Treat as issue — downgrades are rarely safe and need audit.
    return {
      class: "patch-sensitive",
      routing: "issue",
      reason: `target ${targetVersion} is older than current ${currentVersion}; downgrade requires audit`,
      dedupKey,
      current,
      target,
    };
  }

  // Determine bump class.
  let bumpClass;
  if (target.major !== current.major) {
    bumpClass = "major";
  } else if (current.major === 0) {
    // 0.x: any minor or patch bump on the 0.x line is treated as major risk.
    // Per DEV-GUIDE §7.1: "0.x 依赖的次版本按主版本风险处理". We classify
    // a same-major-0 bump as 0x-minor (covers both 0.1.2 -> 0.1.3 and
    // 0.1.2 -> 0.2.0, since both can change API stability on the 0.x line).
    bumpClass = "0x-minor";
  } else if (target.minor !== current.minor) {
    bumpClass = "minor";
  } else {
    bumpClass = "patch";
  }

  // Patch-sensitive override: a patch bump touching crypto/network/auth/
  // serialization-of-trusted-data/release-chain is escalated to issue.
  if (bumpClass === "patch") {
    if (isSensitive(depMeta.name, depMeta.areaTags, config)) {
      return {
        class: "patch-sensitive",
        routing: config.upgrade_routing.classes["patch-sensitive"].routing,
        reason: `patch bump on sensitive dep (${depMeta.name}); escalated to issue per DEV-GUIDE §7.1`,
        dedupKey,
        current,
        target,
      };
    }
  }

  const classCfg = config.upgrade_routing.classes[bumpClass];
  return {
    class: bumpClass,
    routing: classCfg.routing,
    reason: `${bumpClass} bump ${currentVersion} -> ${targetVersion} on ${depMeta.name}`,
    dedupKey,
    current,
    target,
  };
}

/**
 * Build the dedup key for an upgrade proposal. Two proposals with the same
 * key MUST update the same GitHub issue, never create a second one. The
 * class is intentionally NOT part of the key: a re-classification (e.g.
 * patch -> patch-sensitive after a sensitive-area config change) updates
 * the existing issue rather than spawning a new one.
 *
 * @param {string} ecosystem  // "cargo" | "npm"
 * @param {string} name
 * @param {string} targetVersion
 * @returns {string}
 */
export function makeDedupKey(ecosystem, name, targetVersion) {
  return `${ecosystem}:${name}@${targetVersion}`;
}

// ---------------------------------------------------------------------------
// Config loader helper. Reads + validates the JSON config. Used by CLI
// scripts (discover-deps.mjs, generate-sbom.mjs) and by tests.
// ---------------------------------------------------------------------------

/**
 * Validate a parsed config object has the required shape. Throws on missing
 * or malformed sections. Returns the same object on success.
 *
 * @param {Object} config
 * @returns {Object}
 */
export function validateConfig(config) {
  if (!config || typeof config !== "object") {
    throw new Error("config must be an object");
  }
  const ir = config.inventory_routing;
  if (!ir || !ir.auto_pass || !ir.audit_required || !ir.block) {
    throw new Error("config.inventory_routing.{auto_pass,audit_required,block} required");
  }
  const ur = config.upgrade_routing;
  if (!ur || !ur.classes || !ur.sensitive_areas || !ur.dedup) {
    throw new Error(
      "config.upgrade_routing.{classes,sensitive_areas,dedup} required",
    );
  }
  for (const c of ["patch", "minor", "major", "0x-minor", "prerelease", "patch-sensitive"]) {
    if (!ur.classes[c] || !ur.classes[c].routing) {
      throw new Error(`config.upgrade_routing.classes.${c}.routing required`);
    }
  }
  if (typeof ur.dedup.key_format !== "string" || !ur.dedup.key_format.includes("{target_version}")) {
    throw new Error("config.upgrade_routing.dedup.key_format must contain {target_version}");
  }
  if (!config.first_party || !Array.isArray(config.first_party.cargo_workspace_members)) {
    throw new Error("config.first_party.cargo_workspace_members required");
  }
  return config;
}
