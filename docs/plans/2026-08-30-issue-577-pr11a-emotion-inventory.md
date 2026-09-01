# #577 PR 11a — Read-only Emotion and Inventory Surface slice

Date: 2026-08-30

Roadmap: #577 PR 11a

Status: implementation merged by PR #627; product slice remains candidate;
repeated-instance contract calibrated by #629

## Scope

This slice turns the existing first-party `core.emotion` and `core.inventory`
Widgets into truthful, read-only views of the accepted character-scoped State
asset. It does not create separate Emotion or Inventory persistence and does
not add an intent executor.

- `core.emotion` projects a valid integer `state.emotion` in `0..=100` and an
  optional bounded `state.emotion_label` or `state.mood` label.
- `core.inventory` projects a bounded `state.inventory` array. Each accepted
  item has a unique bounded `id`, bounded `name`, optional bounded icon, and an
  optional non-negative bounded quantity.
- Both projections carry Character State revision, timestamp, and source
  metadata. Missing and invalid values are explicit and never become a fake
  zero or an apparently empty inventory.
- The Vue Widgets are display-only. The stale `inventory.use` and
  `inventory.drop` manifest declarations are removed, and direct forged
  intents remain rejected by the Engine.

The saved Workspace remains the only layout authority. Users can place these
Widget types through the existing `open_widget` command, after which the
existing Surface polling/SSE path publishes their deterministic props. Saved
split ratios and the 760 CSS-pixel responsive boundary are unchanged.

## Repeated Inventory instance budget contract

A Surface may contain more than one `core.inventory` Widget, but the complete
Inventory projection can be close to the per-document Surface limit. The
Engine therefore applies this deterministic publication rule:

- Widget order is the accepted Surface Blueprint `widgets` array order. DOM
  order, active tab, visibility, and request timing do not affect selection.
- The first `core.inventory` instance receives the complete bounded projection.
  Every later instance receives `available:false` with
  `reason:"unavailable"`, while preserving the same Character State
  `revision`, `timestamp`, and `source` metadata when present.
- Degradation is presentation-only. It neither changes Character State nor
  implies that Inventory is absent, invalid, or empty. The current
  `move_widget` command changes only the layout tree, not the `widgets` array,
  so moving a Widget does not change which instance is first. Only an operation
  that changes `widgets` array order can change selection; for example, closing
  the first instance and opening it again appends it after surviving instances.
  Adding, removing, or reordering instances requires no Character State data
  migration.

The current v0.2.0 Inventory state schema also uses `reason:"unavailable"` for
source-side unavailability. Vue therefore renders the truthful generic copy
`不可用`; it does not claim a machine-level cause that the wire contract cannot
prove. A user can diagnose repeated-instance degradation only when the first
Inventory instance on the same accepted Surface is available and a later one,
with matching provenance, is unavailable. If the first instance is also
unavailable, the current UI cannot distinguish the cause.

A future UI that labels this state as `重复实例已降级` needs a distinct reason.
That change is not automatically additive because the v0.2.0 manifest uses a
closed reason enum: the current TypeScript parser fails unknown reasons closed
to generic unavailable, while strict old-schema consumers reject them. Rollout
must therefore be consumer-first: update and version the manifest, parser, and
fixtures before the Engine emits the reason, then gate producer emission on an
accepted compatible contract version. Older consumers must continue receiving
the existing generic reason during the compatibility window. This requirement
is tracked in issue #628; until it lands, diagnostics must retain generic copy.

## Evidence target

- same-character State updates reach both Widget projections without changing
  unrelated Widget identity;
- characters and effective user roots do not share projection data;
- malformed, duplicate, oversized, or missing projection fields fail closed;
- a valid empty inventory is distinct from an unavailable inventory;
- repeated Inventory instances follow accepted Blueprint order and preserve
  provenance when degraded;
- Vue presents source/revision and no write affordance;
- forged Emotion/Inventory write intents remain unsupported;
- Surface, Workspace, Vue, responsive-layout, and sacred prompt-boundary tests
  remain passing.

## Deferred

- Emotion or Inventory domain services and write executors;
- item use/drop semantics;
- structured Character State schema migration for existing free-form assets;
- PR 11b Quest + Map and PR 11c Characters/Card + diagnostics;
- Vue Workspace migration/recovery controls and arbitrary Workspace import.
