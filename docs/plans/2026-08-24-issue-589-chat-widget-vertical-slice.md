# Issue #589 — Chat Widget real vertical slice

Date: 2026-08-24

## Scope

Connect the Engine-authored `core.chat` Surface to the existing durable Chat
pipeline without creating a second chat truth store. The slice includes session
selection/creation, history paging, send streaming, cooperative stop,
regenerate, continue, and swipe. Memory, character state, workspace persistence,
and third-party widget mutation remain outside this PR.

## Authority boundary

`POST /v1/ui/intents` accepts only Surface identity, Widget instance identity,
intent name, and intent parameters. Engine resolves the exact accepted snapshot
from its bounded Surface registry and derives effective data root, character,
session, user scope, and Widget type. Missing, ambiguous, stale, mismatched, and
unsupported targets fail closed. The machine contract is
`protocol/ui-intents.json`.

The established Chat handlers and pipeline remain the sole mutation path. Chat
SSE follows `protocol/sse-events.json`; `{type:done}` is accepted only after the
existing durable finalizer succeeds. Vue keeps streaming text as ephemeral
operation state and discards it after a newer canonical Surface revision arrives.

## UI behavior

- The Engine projection remains authoritative; older history and in-flight text
  are view-only overlays.
- The Chat Widget exposes send, stop, regenerate, continue, swipe, and bounded
  cursor paging. A `ResizeObserver` recalculates the virtual window when the
  responsive shell changes available height.
- Character and session chips use stable Engine identifiers. The desktop topbar
  can select an existing session or create and connect a new one; the first
  session is bootstrapped when a selected character has none.
- Only `core.chat` is writable in the production Surface. Memory, state,
  activity, and third-party widgets stay read-only unless a later executor opens
  them explicitly.

## Verification boundary

Automated tests cover trusted target resolution, forged/unregistered target
rejection, JSON and split-frame Chat SSE handling, typed stream errors, and the
existing Surface suite. A real configured provider remains a manual acceptance
gate and must not be represented by a mock result.

## 2026-08-25 recovery and long-history follow-up

The client treats an intent stream that ends without a protocol terminal as an
unknown commit outcome. It never replays that mutation automatically. Recovery
compares the next authoritative Chat projection with a pre-dispatch baseline
using stable message IDs and candidate indexes, while the projected Coordinator
phase distinguishes generating, committing, recovering, and idle. A retry is
offered only for an explicit `not_committed` error or after two fresh idle
snapshots both leave the canonical projection unchanged. Partial, changed but
incomplete, and recovery-required outcomes remain blocked for inspection.

The real-Engine browser smoke creates a 5,000-line canonical chat log at runtime
and verifies the initial latest-50 window, one cursor-based older page, bounded
virtual DOM, latest-message following, and DOM identity preservation for the
unrelated Memory, Character State, and Activity Widget hosts after a Chat patch.
It remains mock-provider evidence. Real configured-provider acceptance and the
complete desktop credential path across an Engine process restart remain open
gates in #589.
