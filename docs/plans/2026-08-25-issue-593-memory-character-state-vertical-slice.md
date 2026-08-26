# Issue #593 — Memory + Character State real vertical slice

Date: 2026-08-25  
Roadmap: #564 / #577 PR 9

## Authority boundaries

- Resident Memory is the current session's `resident.md`. It is not the user
  model, volume memory, archive, or a structured collection with stable entry
  IDs. The Widget labels it as unclassified resident memory and does not expose
  the historical mock `pin/delete` semantics.
- Character State is the user-effective character's `state/live.json`, shared
  by that character's sessions. It is not conversation state or character-card
  metadata. Its schema, history, revision snapshots, and locking remain owned by
  `StateService`.
- Surface props are rebuildable observations. The Engine derives user,
  character, session, and Widget type from the accepted Surface registry;
  clients cannot submit those authority fields.
- A daemon bearer protects the local API but is not a user identity. User scope
  provides data-root isolation; this slice does not claim multi-tenant access
  control.

## First bounded write contract

Memory exposes one manual whole-document operation:

```text
memory.replace { content, expected_content_hash }
```

The hash is the SHA-256 of the exact projected content. The Engine compares it
inside the existing per-session memory mutation lock, enforces the resident
capacity, and atomically replaces the file only on a match. A conflict never
auto-replays because the user's draft must be reconciled against new authority.

Character State exposes a bounded top-level patch:

```text
characterState.patch { expected_revision, patch }
```

Only top-level `add`, `replace`, and `remove` operations are accepted. The
Engine compares the domain revision and applies the patch while holding the
existing character/state locks, validates the resulting schema, and commits
through `StateService`. Generation and state-replacing Agent writes share a
daemon-local character gate across every session for the same effective root,
so a model-produced `<state>` or Agent whole-state write cannot race the editor.
Session-only chat mutations remain independent. Whole-document replace, deep
arbitrary patching, and automatic conflict replay are intentionally out of scope.

## UI and recovery contract

- Both Widgets are read-only while a save is in flight, retain dirty drafts
  across 409 or uncertain results, and show the current source/scope plus hash
  or domain revision.
- Success triggers a fresh Surface read; the accepted snapshot/patch remains
  authoritative and unrelated Widgets keep stable identity.
- A failed or interrupted non-idempotent request is never automatically
  replayed. The UI refreshes authority and requires an explicit user decision.
- Resident Memory does not label free-form Markdown as verified fact, derived
  summary, or model advice. Character State displays stored state without
  inventing field-level provenance that the domain does not record.

## Remaining #593 boundaries

This slice does not close #593 by itself. Before closing the issue, independently
add real configured-provider acceptance where model state writes are involved.
Packaged restart credential recovery is now closed: the desktop shell performs
a bounded owned-sidecar restart, exchanges a fresh token without reloading the
document, and tells Vue to rebuild its Bus from authority while retaining dirty
Memory/State drafts and never replaying them automatically. Windows package
smoke kills the exact owned Engine PID and verifies lock ownership, old/new
token rejection/acceptance, authoritative state, draft retention, and explicit
post-recovery CAS writes without any provider configuration. Blind non-UI whole-state
writers are now closed: the same locked read both renders the model prompt state
and supplies the revision used by `<state>` finalization, and Agent
`get_character_state` returns revision metadata
that `update_character_state` must submit as `expected_revision`; stale writes
return conflict; Agent finalization reports `finalization_error` rather than a
false converged result. The finalizer extracts model state only from complete
raw provider output because the streaming FSM deliberately hides `<state>` from
visible cleaned output; an Agent end-to-end conflict test binds the message,
marker, state, activity receipt, and wire result to one generation. Field-level
relationship/plot writers remain lock-local
`mutate` operations over the latest state and therefore do not replay a stale
whole-state snapshot. Character State multi-file process-interruption recovery
is now closed by treating the immutable revision plus atomic `current_revision`
as the commit point and repairing `live.json` / `history.jsonl` projections on
every locked public read or write. This does not claim Windows power-loss
durability for directory metadata because the current std-only `sync_dir`
implementation is a no-op on Windows. Any work not completed in the delivering
PR remains explicit in #593 or a linked issue.
