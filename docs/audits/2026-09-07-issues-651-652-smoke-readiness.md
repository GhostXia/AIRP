# Issues #651 / #652 — independent audit PASS

Base: `bed754331d8b9075dde7fe04965226b83d69364f`.
Implementation: `d8624ff`. Date: 2026-09-07.
Auditors read AGENTS.md and independently reviewed their disjoint scopes.

## #652: gateway certificate readiness

Auditor Plato (`01a07c1c-8af4-7e00-bfb1-9f7f86858574`) found no blockers.
The probe validates the actual served leaf against the copied CA and localhost
identity, bounds connection attempts/retries with a monotonic deadline, destroys
sockets and clears timers. CLI failure remains nonzero; the shell stops before
SPKI extraction. Later curl CA verification and browser TLS verification remain.

Initial nonblocking N1 (real certificate chain) and N2 (pre-cleanup connection
closure assertions) were implemented before merge. Independent re-audit passed
all eight tests, including root-only trust through an intermediate, exact leaf
fingerprint/SPKI, and closed connections before server cleanup. Remaining items: 0.
The auditor did not run the full Docker topology; exact-head CI must do that.

## #651: desktop lifecycle regression

Auditor Carson (`01a07c1e-ca43-71a0-8581-f172e059feae`) found no blockers.
Actual production shutdown/cleanup/reporting statements are extracted by AST;
UI and HTTP are simulated, but child process handles and exit waits are real.
This does not claim real airp-core, UI, HTTP or ownership-verification coverage;
the existing package smoke remains the separate real integration gate.

Independent runs on PowerShell 7.6.5 and Windows PowerShell 5.1.22621.5624 passed
success, cleanup-only failure and live engine on either closure, plus rejection
of three mutations: removal of either engine wait or premature success output.
Original startup/copy/mixed-failure cases also passed with no leftover fixtures.

- N1: exact AST statement selectors intentionally fail closed on structural or
  formatting changes. Keep this explicit coupling rather than a permissive
  extractor that might test the wrong block; tests may need updates when the
  production script is refactored. Record this no-change decision on #651 after merge.
- N2: fixture teardown now collects per-process failures, still attempts later
  cleanup, and preserves the body failure using the existing aggregate helper.
  Main-agent reruns passed on both PowerShell versions after this small follow-up.

## Main-agent validation

Production Node tests passed (30 after the added chain case); WebUI 210 tests
passed. Workspace tests passed (engine 1584 passed, five ignored), including the
sacred prompt boundary; fmt and clippy with warnings denied passed. Shell syntax
and diff checks passed. No toolchain/dependency versions were changed by this PR.
