# AIRP Dependency Governance Tooling

> Implements #192 (dependency discovery + audit routing) and #190 (third-party
> notices + SBOM generation).
>
> Status: dev/CI artifact. The output in `docs/sbom/` is regenerated on demand
> and committed alongside dependency changes; it is NOT a formal release SBOM
> until `--fail-on-unknown` passes clean and a human has signed off on every
> `audit-required` record.

## What this directory contains

| File | Role |
|---|---|
| `audit-routing.config.json` | Policy config: license tiers (auto_pass / audit_required / block), upgrade routing classes, sensitive area patterns, dedup key format. **Edit this when adding a new license id or changing routing policy.** |
| `audit-routing.mjs` | Pure-function routing engine. No I/O, no network, no fs. Imports: none (Node built-ins only). |
| `discover-deps.mjs` | CLI: scans Cargo workspace + npm lockfile, classifies each record, writes `docs/sbom/inventory.{json,txt}`. |
| `generate-sbom.mjs` | CLI: consumes inventory, writes `docs/sbom/airp.spdx.json` (SPDX-2.3), `docs/sbom/airp.cdx.json` (CycloneDX 1.5), `docs/sbom/THIRD-PARTY-NOTICES.txt`. |
| `routing-dry-run.mjs` | CLI: runs `classifyUpgrade` against `fixtures/routing-samples.json` to prove the 5 routing classes + patch-sensitive override are exercised. Use this to validate config changes. |
| `fixtures/` | Hermetic test fixtures (sample Cargo.lock, package-lock.json, inventory, routing samples). |
| `tests/` | `node --test` suites: `routing.test.mjs`, `discover.test.mjs`, `sbom.test.mjs`. |

## Quick start

```bash
# 1. Discover dependencies (writes docs/sbom/inventory.{json,txt})
node tools/dep-governance/discover-deps.mjs \
  --repo-root . \
  --out-dir docs/sbom \
  --config tools/dep-governance/audit-routing.config.json

# 2. Generate SBOM + notices (writes docs/sbom/airp.{spdx,cdx}.json + THIRD-PARTY-NOTICES.txt)
node tools/dep-governance/generate-sbom.mjs \
  --inventory docs/sbom/inventory.json \
  --out-dir docs/sbom \
  --config tools/dep-governance/audit-routing.config.json

# 3. Validate routing config + dry-run the 5 upgrade classes
node tools/dep-governance/routing-dry-run.mjs

# 4. Run tests
node --test tools/dep-governance/tests/*.test.mjs
```

## When to regenerate

Regenerate `docs/sbom/` whenever:

1. **A dependency is added, removed, or its resolved version changes** — i.e.
   any change to `Cargo.lock`, `ui/package.json`, or `ui/package-lock.json`.
   The PR that changes a lockfile MUST also regenerate the SBOM in the same
   commit, so reviewers can see the license/provenance delta.
2. **`audit-routing.config.json` changes** — re-classification may move records
   between auto-pass / audit-required / block.
3. **A new formal release tag is cut** — the SBOM attached to a release must
   reflect the exact lockfile at that tag.
4. **Periodically (e.g. monthly)** even without lockfile changes, to catch
   upstream license/provenance metadata updates in `cargo metadata` output.

Do NOT regenerate on every CI run — the output is committed, not ephemeral.

## Permissions and token boundaries

**No network access required.** All scripts read only local files:

- `discover-deps.mjs` runs `cargo metadata --format-version 1` (offline; reads
  the local registry cache and `Cargo.lock`) and parses `ui/package-lock.json`.
  It does NOT call crates.io, npmjs.org, or any HTTP endpoint.
- `generate-sbom.mjs` reads `docs/sbom/inventory.json` and writes SBOM files. No
  network, no tokens, no GitHub API calls.
- `routing-dry-run.mjs` reads `fixtures/routing-samples.json`. No network.

**No GitHub tokens, no npm tokens, no cargo tokens.** These scripts are safe to
run in any environment, including air-gapped CI and reviewer machines.

The `resolved` URLs in the inventory (e.g.
`https://crates.io/api/v1/crates/<name>/<version>/download`) are sourced from
the local lockfile's checksum record — they are recorded for provenance, not
fetched.

## Output format

### `inventory.json`

Array of records, each with:

- `ecosystem`: `"cargo"` | `"npm"`
- `name`, `version`: as resolved in the lockfile
- `license`, `license_normalized`: raw and normalized SPDX string (or `null` /
  `""` if unknown)
- `repository`, `source`, `resolved`: provenance fields from cargo metadata /
  npm lockfile
- `integrity`: `sha256:<hex>` (cargo) or `sha512-<base64>` (npm)
- `scope`: `"runtime"` | `"build"` | `"dev"` (cargo: BFS over `dep_kinds`;
  npm: `dependencies` vs `devDependencies`)
- `tier`: `"first-party"` (AIRP workspace members + npm root) | `"third-party"`
- `audit_class`: `"auto-pass"` | `"audit-required"` | `"block"`
- `audit_reason`: human-readable explanation

### `airp.spdx.json`

SPDX-2.3 JSON document. AIRP itself is the root package
(`SPDXRef-AIRP`, license `MIT OR Apache-2.0`); every third-party component is a
`DEPENDS_ON` relationship from it. First-party records are NOT emitted as
separate packages (they would duplicate AIRP's own license).

Unknown licenses are recorded as `NOASSERTION` (SPDX's "we make no claim")
rather than guessed. `--fail-on-unknown` exits non-zero if any third-party
record has an unresolvable license, AFTER writing the SBOM.

### `airp.cdx.json`

CycloneDX 1.5 JSON BOM. Same component set as SPDX, with `bom-ref`, `purl`, and
`hashes` (SHA-256 / SHA-512). License representation follows the CycloneDX 1.5
schema: a single SPDX id uses `license.id`; a composite expression (OR/AND/WITH)
uses `license.expression`; an unknown license uses `license.name`.

### `THIRD-PARTY-NOTICES.txt`

Human-readable bundle. Runtime deps listed first (the ones that ship in release
artifacts); build/dev deps in a separate "NOT shipped" section. Records flagged
`block` or `audit-required` appear in a "Records requiring attention" section
at the end.

## Routing policy

See `audit-routing.config.json` for the authoritative config. Summary:

### Inventory classification (`classifyInventory`)

| Tier | Licenses | Behavior |
|---|---|---|
| `auto_pass` | MIT, Apache-2.0, BSD-2-Clause, BSD-3-Clause, ISC, Zlib, 0BSD, Unicode-DFS-2016, Unicode-3.0, Unlicense, CC0-1.0, OpenSSL, BSL-1.0 | Auto-pass. PR-ready. |
| `audit_required` | MPL-2.0, EPL-1.0/2.0, LGPL-2.0/2.1/3.0 (+`-or-later`), PolyForm-*, CC-BY-4.0, CC-BY-SA-4.0 | Dedicated audit issue required before release. |
| `block` | GPL-1.0/2.0/3.0, AGPL-3.0, SSPL-1.0, BUSL-1.1, Proprietary, UNLICENSED | Hard block in runtime scope. Dev-scope strong copyleft downgraded to `audit-required` (config: `strong_copyleft_in_runtime_scope: true`). |

- First-party records (AIRP workspace members + npm root `airp-ui`) always
  auto-pass; they carry AIRP's own license.
- Empty / null license → `audit-required` (fail-visible; never silent auto-pass).
- License present but not in any tier → `audit-required` (conservative).

### Upgrade routing (`classifyUpgrade`)

Per `docs/DEV-GUIDE.md` §7.1:

| Class | Trigger | Routing |
|---|---|---|
| `patch` | major>=1, patch bump (e.g. 1.2.3 → 1.2.4) | `auto-pr` (still subject to audit bot + human review) |
| `patch-sensitive` | patch bump on a sensitive area (crypto/network/auth/serialization/release-chain) | `issue` |
| `minor` | major>=1, minor bump (e.g. 1.2.3 → 1.3.0) | `issue` |
| `major` | major bump (e.g. 1.2.3 → 2.0.0) | `issue` |
| `0x-minor` | 0.x dependency, any minor or patch bump | `issue` (0.x treated as major risk per §7.1) |
| `prerelease` | target version has a prerelease tag | `skip` (never auto-adopted) |
| `downgrade` | target older than current | `issue` |
| `unparseable` | either version doesn't parse | `issue` |

Dedup key: `{ecosystem}:{name}@{target_version}` — intentionally excludes
`class`, so re-classification of the same upgrade updates the same issue rather
than creating a new one.

## Maintainer responsibilities

When adding or changing a dependency:

1. Update `Cargo.toml` / `ui/package.json` and run `cargo update` / `npm install`
   to refresh the lockfile.
2. **Regenerate the SBOM** in the same commit:
   ```bash
   node tools/dep-governance/discover-deps.mjs --repo-root . --out-dir docs/sbom
   node tools/dep-governance/generate-sbom.mjs --inventory docs/sbom/inventory.json --out-dir docs/sbom
   ```
3. Review the `audit-required` and `block` sections in
   `THIRD-PARTY-NOTICES.txt`. If a new license id appears that should be
   auto-pass, add it to `audit-routing.config.json` `inventory_routing.auto_pass.licenses`
   (and possibly `KNOWN_SPDX_IDS` in `generate-sbom.mjs`).
4. Update `docs/ACKNOWLEDGEMENTS.md` §3 table with the locked version,
   license, and provenance.

## Known limitations (follow-ups)

These are conservative behaviours that over-classify as `audit-required` or
`NOASSERTION`. They are safe (never under-classify), but produce noise. Fixes
are deferred to keep this slice reviewable:

1. **`splitLicenseExpression` is deprecated.** It now delegates to the AST-based
   SPDX parser (`parseSpdxExpression` + `extractLicenseIds`) for backward
   compatibility, but new callers should use the parser directly so they get
   the full AST (operators, exceptions, parentheses) rather than a flat list
   of license ids.

2. **`MIT-0` is not in `KNOWN_SPDX_IDS`** in `generate-sbom.mjs`. Add it when
   the parser is improved, or sooner if `dunce`'s license becomes a blocker.

3. **Live upgrade detector is not wired.** `routing-dry-run.mjs` proves the
   routing logic against fixtures; a live detector that queries upstream
   registries (crates.io, npmjs.org) for latest stable versions and feeds
   deltas into `classifyUpgrade` is a documented future step. Per
   `docs/DEV-GUIDE.md` §7.1, the live detector must not auto-merge and must
   dedup by `{ecosystem}:{name}@{target_version}`.

4. **No OS-level package SBOM.** The Debian base image packages
   (`deploy/production/Dockerfile.engine`) are not enumerated here. The final
   release SBOM must include OS packages (see `docs/ACKNOWLEDGEMENTS.md` §3
   "Debian Docker Official Image" row).

## Testing

```bash
node --test tools/dep-governance/tests/*.test.mjs
```

- `routing.test.mjs`: semver parse/compare, license normalization, inventory
  classification, upgrade routing, dedup keys, config validation.
- `discover.test.mjs`: Cargo.lock checksum parsing, npm lockfile v3 parsing,
  scope BFS, atomic write, inventory text rendering.
- `sbom.test.mjs`: SPDX/CycloneDX builders, purl, checksum conversion,
  notices text.

All tests are hermetic — they use `fixtures/` and do not touch the real repo's
inventory or run `cargo`/`npm`.

## Independent implementation

Per `AGENTS.md` "第三方经验吸收与独立实现": these scripts use only Node.js
built-ins (`fs`, `path`, `crypto`, `node:test`, `node:assert`). No SBOM library
(`cyclonedx-node-toolkit`, `spdx-tools`, etc.), no semver library, no
`@actions/*` runtime. The SPDX and CycloneDX document structures are built
from their public specs; the semver parser is a minimal independent
implementation covering the forms AIRP actually encounters in lockfiles.
