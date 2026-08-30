# #577 PR 11a — Read-only Emotion and Inventory Surface slice

Date: 2026-08-30

Roadmap: #577 PR 11a

Status: implementation candidate

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

## Evidence target

- same-character State updates reach both Widget projections without changing
  unrelated Widget identity;
- characters and effective user roots do not share projection data;
- malformed, duplicate, oversized, or missing projection fields fail closed;
- a valid empty inventory is distinct from an unavailable inventory;
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
