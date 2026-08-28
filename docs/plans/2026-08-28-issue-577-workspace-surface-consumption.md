# #577 PR 10d — saved Workspace drives session Surface structure

Date: 2026-08-28

Roadmap: #577 PR 10

Status: implementation candidate

## Authority boundary

Each Surface refresh reads the validated `default` Workspace from the same
effective data root as the requested session. The Engine lowers that saved
layout into the executable-free Surface v2 Blueprint and attaches current
domain projections by first-party Widget type:

- `core.chat` receives Chat props;
- `core.memory` receives Memory props;
- `core.character-state` receives Character State props;
- `core.activity` receives Activity props;
- other Workspace v1 allowlisted types remain structurally present with no
  runtime props until their projection exists.

Workspace remains the durable structure authority; Chat, Memory, Character
State, and Activity remain their own data authorities. Workspace assets never
store a copy of those props. A saved structural change that Surface v2 can
express produces a new Surface snapshot through the existing polling/replay
path. A props-only change keeps the existing patch behavior.

The effective-root lookup preserves existing user-root isolation. Invalid or
future-major Workspace data fails before replacing the registry entry, so an
already accepted Surface remains the last-known-good client state. That
display fallback does not preserve write authority: every intent revalidates
the current Workspace and exact accepted revision first.

## Explicit limitation

Workspace v1 persists `ratioBasisPoints`, validates it, and accepts
`resize_split`. Surface v2 `Split` currently has no ratio field, so lowering
cannot expose that saved value to the renderer. This PR therefore does not
advance the Surface revision or emit an event for a ratio-only change, and it
does not claim visible split resizing or dynamic layout adaptation. A later additive
Surface contract or Vue Workspace composition slice must consume the ratio
without creating a client-side shadow authority.

## Explicitly deferred

- Vue Workspace loading, editing controls, and CAS reconciliation;
- visible split ratio and responsive/dynamic layout behavior;
- migration apply/import and pre-upgrade backup/rollback;
- multi-workspace registry, switching, sharing, and arbitrary JSON import;
- extension-backed Workspace Widget types.
