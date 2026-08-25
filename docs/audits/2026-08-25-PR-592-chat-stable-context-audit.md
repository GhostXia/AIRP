# PR #592 — Chat stable-context audit

Date: 2026-08-25  
Scope: issue #589 stable context projection and responsive Chat chips  
Auditor: independent Terra audit agent

## Verdict

Pass. All blocking and non-blocking findings were addressed and independently
rechecked. No deferred finding remains.

## Findings and resolution

### A1 — Short-height layout hid the only stream cancellation control (blocking)

The initial `max-height: 420px` rule hid all generation actions, including the
only Stop control while a stream was active. The final implementation hides
only secondary retry/regenerate/continue controls. During streaming or stopping,
the visible Stop action replaces the already-disabled composer at this extreme
height. The 360×320 browser smoke injects streaming state, verifies Stop is
visible and enabled, clicks it, and still reaches the complete logical history
tail.

Status: resolved.

### A2 — Context chips could imply snapshot isolation that the intent contract does not provide (blocking)

Surface projection and Chat execution are separate reads. Persona bindings and
the canonical character Worldbook may change between them; `/v1/ui/intents`
does not carry a versioned context token. The product contract now explicitly
defines chips as the latest accepted Surface's current observations. The Chat
pipeline intentionally resolves, reads, and validates then-current authority at
execution. No snapshot-isolation or transaction-lock claim remains; any future
workflow requiring it must introduce a versioned token and reject drift.

Status: resolved by explicit bounded contract; no protocol expansion in this PR.

### N1 — Manifest role enum exceeded Engine history authority (non-blocking)

The manifest allowed `narrator`, while Engine history projects only
`user`/`assistant`/`system`. The enum was narrowed. Persona fields were also
aligned with the real no-user projection by accepting `null`, and
`persona_source` was restricted to the Engine's serialized variants.

Status: resolved.

### N2 — Verification text did not prove the Engine binary came from current source (non-blocking)

The baseline previously called the browser smoke's Engine “debug” while the
script defaults to a release path. Verification now requires an explicit
`AIRP_ENGINE_BINARY` pointing to an Engine built from the current checkout and
states that the default path alone is not freshness evidence.

Status: resolved.

## Evidence

- `cargo fmt --all -- --check`: passed.
- `cargo test --workspace --locked`: passed (Engine library 1,481 passed / 5
  ignored, plus all binary, integration, protocol, and Tauri tests).
- `npm test -- --run`: 21 files / 177 tests passed.
- `npm run typecheck`: passed.
- `npm run smoke:runtime`: passed with 5,000 messages (8 virtual rows, 1.00 ms
  p95 on the final measured run before the layout-only follow-up).
- `node responsive-browser-smoke.mjs`: passed at 360×320, including context
  strip overflow, stream cancellation, and logical-tail reachability.
- `AIRP_ENGINE_BINARY=D:\AIRP-Dev\target\release\airp-core.exe npm run
  smoke:http-bus`: passed after an explicit current-checkout release build and
  current bundle build.

## Deferred findings

None.
