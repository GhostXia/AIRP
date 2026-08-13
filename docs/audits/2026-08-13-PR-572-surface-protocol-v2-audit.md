# PR #572 / issue #564 PR 2 Surface Protocol v2 causal audit

- Date: 2026-08-13
- Branch: `codex/564-surface-protocol-v2`
- Base: current `origin/main` after PR #567
- Scope: machine authority, Rust/TypeScript bindings and guards, shared
  fixtures, atomic client transition, explicit v1 migration
- Verdict: **PASS after fixes; remote independent audit still required**

## Findings resolved

### A1 — Root patch bypassed immutable metadata

The validators protected `/kind`, `/protocol`, `/surfaceId`, and `/revision`
but accepted a mutating operation whose path was the empty JSON Pointer. A
root replacement could therefore replace all protected fields at once.

Resolution: root mutation and root `from` reads are rejected; a root `test`
remains valid and read-only. The shared authority and Rust/TypeScript negative
tests lock this boundary. The client store also proves a rejected root
replacement preserves its last-known-good snapshot.

### A2 — Orphan Widget definitions were accepted

Both validators checked missing and duplicate Widget references but did not
require every declared Widget instance to appear exactly once in the layout.
This contradicted the PR 2 fixture plan and left lifecycle ownership ambiguous.

Resolution: Rust and TypeScript now compare declared IDs with placed IDs and
return `invalid_reference` for an orphan. The shared negative fixture is
consumed by both implementations.

### A3 — Revision overflow differed across languages

Rust used `saturating_add(1)`, so a patch from `u64::MAX` to `u64::MAX` was
accepted. TypeScript correctly rejected it because no adjacent u64 revision
exists.

Resolution: Rust uses checked addition. Shared fixtures cover both adjacency
overflow and an out-of-range wire revision.

### A4 — Raw wire errors and minor bounds were not parity-safe

Rust let serde classify an out-of-range revision as a generic shape error,
while TypeScript returned `invalid_revision`. TypeScript also accepted minor
components larger than Rust's `u16` representation.

Resolution: Rust preflights raw protocol/revision fields before typed
deserialization; the authority fixes protocol components to unsigned 16-bit,
and TypeScript enforces the same `0..=65535` minor range. Shared fixtures assert
stable `invalid_version` and `invalid_revision` codes in both languages.

## Independent boundary assessment

- Surface v2 remains separate from legacy Envelope v1 and chat SSE contracts.
- The contract contains declarative layout and JSON data only; forbidden
  executable-field names are rejected recursively, including additive data.
- Unknown same-major additive fields are accepted as opaque data; unknown
  majors fail closed.
- Snapshot and patch sizes, operation counts, depth, node count, child count,
  Widget count, identifier length, and decimal-u64 revisions are bounded.
- The TypeScript store applies operations to a clone, validates the complete
  candidate, and commits only after success. Failed operations, invalid
  post-patch structure, and revision mismatch preserve current and
  last-known-good state and request resynchronization.
- V1 migration is explicit and deterministic; validation does not silently
  reinterpret legacy documents as v2.
- This PR does not claim an Engine Surface endpoint, renderer, HttpEngineBus,
  Tauri entry, real Widget workflow, or user workspace delivery.

## Verification

- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --locked`: passed; core library 1,442 tests plus
  workspace integration and documentation tests.
- `cargo test -p airp-state-protocol --locked`: 12 passed.
- `cargo test -p airp-core --lib subagent_ --locked -- --nocapture`: 2 passed.
- `npm --prefix ui run typecheck`: passed.
- `npm --prefix ui run test -- --run`: 14 files / 119 tests passed.
- `npm --prefix ui run build`: passed.
- `node --test webui/tests/*.test.mjs`: 207 passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Remaining gate

No known blocking or non-blocking product findings remain in this audit. The
repository audit bot must independently review the pushed PR; this report does
not replace that gate or human merge approval.
