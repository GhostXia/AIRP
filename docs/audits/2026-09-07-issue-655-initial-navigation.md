# Issue #655 — independent navigation audit PASS

Date: 2026-09-07. Base: `3561da5be2439c75854b4226710f035e015c46eb`.
Auditor: Ramanujan (`01a07c58-3362-7321-9d36-4adc217103ae`), independent and
read-only, following AGENTS.md and the complete issue acceptance requirements.
Scope: initial-navigation helper/test, production restart smoke, and CI test list.

## Findings

- Only the exact `ERR_CERT_VERIFIER_CHANGED` initial navigation error permits
  retry, with two attempts maximum under one monotonic deadline. Loading after
  document commit is outside the retry catch; history and chat actions cannot
  be replayed by this helper.
- Before each attempt, a Node TLS connection independently verifies the chain,
  hostname and original expected SPKI. The production launcher supplies
  `NODE_EXTRA_CA_CERTS` with the trust bundle and retains the original SPKI
  across restart. Chromium's exact SPKI argument and `ignoreHTTPSErrors: false`
  remain unchanged.
- Diagnostics expose only validated hashes, error codes, counters and browser
  version, not navigation queries or credentials. Existing HTTP status and
  post-restart continuity assertions remain intact.

## Independent validation and disposition

Initial targeted suite: 30/30; combined production Node suite: 60/60. Additional
adversarial checks passed for same-key leaf rotation, trust failure on retry,
initial pin mismatch and a nontransient second-attempt failure.

Two P3 observations were fixed before merge:

1. The retry delay survived outer deadline rejection until its own timer fired.
   It now receives an AbortSignal and is cancelled in `finally`. Re-audit passed
   31/31 targeted tests. The new cleanup assertion passed three in-memory runs
   against the fix and rejected the old uncancelled implementation in all three
   runs, identifying its one surviving timer. No second navigation occurred.
2. The comment claimed document commit preceded script execution. It now only
   describes the actual post-commit catch boundary, without that stronger claim.

Re-audit confirmed the smoke/workflow were unchanged and both observations were
resolved. Blocking findings: none. Additional nonblocking actions: none.

## Main-agent checks and evidence limits

Final combined production Node suite: 61/61; WebUI: 210/210; dependency governance:
101/101; full Rust workspace (engine 1584 passed, five ignored), sacred library
invariant, formatting and clippy with warnings denied passed. UI tests/build and
runtime/responsive browser checks are recorded in the companion #654 audit.

The first local workspace attempt lacked the isolated checkout's Tauri engine
sidecar; after building/staging it, the full workspace passed. A later broad
filtered sacred-test command hit Windows access-denied launching an unrelated
integration binary; the exact library-only invariant invocation passed.

The original Chrome verifier root cause remains unproven. Diagnostics sample
separate Node TLS connections, not Chrome's failed handshake or verifier state,
and cannot exclude rotation between samples. Original SPKI comparison covers
key identity, not a pre-restart leaf fingerprint. This is bounded mitigation
and diagnostic coverage, not a root-cause claim. Exact-head production topology,
mandatory robot audit and merge review remain required.
