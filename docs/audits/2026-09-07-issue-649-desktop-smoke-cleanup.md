# Issue #649 independent audit — PASS

Date: 2026-09-07. Base: `d70e3fb3b55154998a9c2c71ae63327969e35e03`.
Implementation: `f040bb1` (`codex/649-desktop-smoke-cleanup`).
Independent auditor: Descartes, agent `01a07bf3-9fef-7d83-a1da-c357917757d5`.

The auditor read AGENTS.md and independently reviewed the production scripts,
tests and workflow against the base. No blocking safety or correctness findings.
This code-audit verdict does not replace the bot gate or exact-head package CI.

## Findings and boundaries

- Verified engine handles are bound before executable-path and CIM parent checks.
  Cleanup uses retained process objects, not port-based killing or unchecked PIDs.
- Both normal shell closures wait up to five seconds for the captured current
  engine before accepting the empty owner record or reopening. Timeout remains a
  primary failure even if final cleanup subsequently kills the process.
- Copying excludes source `data/` and is protected by the cleanup `try/finally`.
  Recursive removal validates the absolute immediate TEMP child, exact generated
  name, and non-reparse root.
- Selective file-release retries use a monotonic deadline. The deadline bounds
  retries, not the duration of an individual filesystem operation.
- Primary-only failure retains ErrorRecord diagnostics. Mixed failures retain
  original exceptions and explicit combined messages on Windows PowerShell.
- Success is printed only after process cleanup, scratch removal and result
  checking. Both regression scripts run before packaging in Windows CI.

## Independent validation

PowerShell 7.6.5 and Windows PowerShell 5.1.22621.5624 each passed:

- Eight cleanup tests: actual exclusive file handles, delayed release, permanent
  contention, idempotence, safe path/root-junction rejection, real child exit,
  and original/aggregate error preservation.
- Three real-script integration scenarios: startup failure, copy failure,
  and primary plus cleanup failure, preserving source package data.
- Six ownership probes using the production function: real child acceptance and
  handle binding; wrong shell, port, executable and parent rejection without
  killing the child; verified-process shutdown. PID reuse was not synthesized.
- `git diff --check`.

Reviewed production-script SHA-256:
`FFE9CC41D79B9C2DDED3EC7A11155A7FB43B241025912552FE7E942AE542C8CC`.

Main-agent validation, not independently repeated by the auditor: workspace tests
(engine 1584 passed, five ignored), sacred prompt-boundary invariant, WebUI 210
tests, fmt, clippy with warnings denied, and rustdoc with warnings denied all
passed. Actual packaged desktop restart/credentials/cleanup smoke passed against
the existing August 26 local binaries; this is compatibility evidence only, not
an exact-head release claim. CI rebuilds and tests the current PR package.

## Unresolved nonblocking item — N1 (P2)

Integration scenarios do not reach normal shutdown or a successful body followed
by cleanup failure. Add a lifecycle fixture covering these paths, including empty
lock/closed port with an engine still alive. This would detect removal of the new
engine-exit waits and additional premature-success regressions. Current behavior
is correct by inspection and real local package evidence; this is a regression
coverage gap, not a blocker. Record a deduplicated follow-up issue **after merge**.
