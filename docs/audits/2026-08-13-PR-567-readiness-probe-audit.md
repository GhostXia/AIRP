# PR #567 disposable readiness probe independent audit

- Date: 2026-08-13
- Audited branch: `codex/issue-247-health-ready` after merging current `origin/main`
- Scope: Engine `/health`, production readiness probe, documentation, regression coverage
- Verdict: **PASS**

## Findings

No blocking or non-blocking product findings.

## Independent assessment

- `/health` remains an unauthenticated Engine liveness/local-state endpoint and does not publish a misleading aggregate `ready` field.
- Production readiness remains externally owned: authenticated gateway health, a provider-backed models call, and an optional typed SSE `done` exchange.
- Every successful probe-session creation reaches the force-delete loop regardless of SSE transport, HTTP, or frame-validation success.
- A 200 confirms deletion; 404 is accepted as idempotent confirmation that an earlier delete completed. Other statuses retry within a bounded budget.
- Cleanup failure returns 1 before the readiness-success branch, so a valid SSE exchange cannot mask leaked probe state.
- `force=true` avoids pre-delete backup pollution for synthetic probe sessions.
- Failed SSE attempts delete their own session before retrying; successful attempts also delete before the existing grace period and return.
- The change does not alter WebUI rendering; issue #319 visual review is not applicable.

## Audit-driven regression improvement

The existing static test asserted disposable session creation and SSE validation but did not lock the cleanup contract. This audit adds assertions for the force-delete URL, 200/404 success set, fail-closed return, and cleanup-before-success ordering.

## Verification

- `cargo test -p airp-core daemon::tests::health_settings --lib --locked`: 15 passed.
- `node --test deploy/production/*.test.mjs`: 27 passed.
- `node --test webui/tests/runtime-pages.test.mjs`: 27 passed, including the added cleanup-contract assertions.
- `bash -n deploy/production/smoke-ci.sh`: passed.
- `node --check deploy/production/verify-readiness-sse.mjs`: passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Workflow note

This PR did not trigger Agent Browser Exploration. All prior remote blocking checks passed on the pre-audit head; the audit commit must complete the current remote gate before merge.
