# PR #591 independent audit — Chat recovery and long-history evidence

Date: 2026-08-25

## Scope

Independent review of the #589 follow-up that adds fail-closed Chat mutation
reconciliation, explicit safe retry, latest-message following, and real-Engine
5,000-history browser evidence. The audit followed the repository charter and
treated the implementation and existing architecture as challengeable.

## Initial blocking findings

1. A cancelled stream cleared client operation state for an absent or unknown
   `commit_state`. Fixed so only explicit `not_committed` clears; every other
   state reconciles against the authoritative Surface.
2. A projection that looked committed could clear UI state before the projected
   Coordinator `recovering` phase was checked. Fixed by making the recovery lock
   take precedence over commit inference.
3. Recovery probes were keyed only by Widget ID and could let an old connection
   suppress or overwrite a new connection's probe. Fixed with bus-owned probe
   entries and an apply-time active-bus guard.
4. The pagination smoke originally waited only for the request. Fixed to require
   its response before asserting the rendered older page, then rerun against an
   Engine binary built from the current checkout.

## Final evidence

- `npm test -- --run`: 21 files, 177 tests passed.
- `npm run typecheck`: passed as part of the production bundle.
- `npm run bundle:webui`: passed; final bundle includes the reviewed source.
- `npm run smoke:http-bus` with
  `AIRP_ENGINE_BINARY=D:\AIRP-Dev\target\debug\airp-core.exe`: passed twice in
  the implementation run and once independently in the final audit.
- `npm run smoke:shell`: five profiles passed.
- `npm run smoke:runtime`: passed with eight virtual rows and 0.90 ms p95.
- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --locked`: passed, including the sacred
  `subagent_context_has_no_orchestrator_noise` invariant.

## Verdict

PASS. All blocking findings were fixed and independently rechecked. No deferred
non-blocking audit findings remain from this audit. Real configured-provider
acceptance and complete desktop credential recovery across an Engine process
restart remain explicit #589 gates, not claims of this PR.

## First audit-bot follow-up

The first completed CodeRabbit review reported four additional issues. All were
accepted and fixed: Swipe now resets recovery observations; retry is disabled
on read-only Surfaces; the current verification snapshot is separated from the
historical `main@830426e` results; and Persona/Scene/Worldbook context chips are
listed explicitly as an open #589 boundary. No audit-bot item was deferred.
