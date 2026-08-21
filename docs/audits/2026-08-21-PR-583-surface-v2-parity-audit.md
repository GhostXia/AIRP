# PR #583 — Surface v2 parity coverage audit

- Date: 2026-08-21
- Scope: `ui/src/protocol/surface-v2.test.ts`, `protocol/src/lib.rs`
- Issue: #574
- Head reviewed: `67163d52e7ab5c96930a61fe41fdeb5aa2c37a98`
- Independent local verdict: **PASS**

## Boundary

This PR changes tests only. It does not change the Surface v2 wire contract, JSON Pointer executor, guards, authority document, fixtures, or immutable-pointer allocation behavior.

## Findings

No blocking findings.

- The successful patch case directly executes `copy`, `move`, and array `-` append, then verifies revision advancement, resulting values, and `lastKnownGood`.
- The malicious-pointer table covers a `__proto__` segment and malformed `~2` escape. Both cases require `invalid_patch`/resync and compare the accepted snapshot plus `lastKnownGood` with deep pre-patch copies.
- Rust authority parity now binds all eight resource limits, including `maxWidgetInstances`, `maxChildren`, and `maxIdentifierLength`.
- The low-value immutable-pointer-list allocation optimization remains intentionally unimplemented, matching the explicit #574 decision that profiling evidence is required first.

## Verification

- `cargo fmt --all -- --check`: passed.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed.
- `cargo test --workspace --locked`: passed, including 12 protocol tests and the sacred `subagent_context_has_no_orchestrator_noise` invariant.
- `npm --prefix ui run typecheck`: passed.
- `npm --prefix ui run test -- --run`: 15 files / 125 tests passed.
- `npm --prefix ui run build`: passed.
- `npm --prefix ui run smoke:shell`: 5 profiles passed.
- `node --test webui/tests/*.test.mjs`: 207 tests passed.
- Independent incremental re-review after the type-safe structural assertion adjustment: **PASS**.

## Merge readiness

Local implementation and independent audit gates pass. Merge remains blocked until PR #583 CI and the repository audit bot complete successfully with no unresolved blocking comments.
