# AIRP Worldbook Semantic Contract

Status: version 2. Adds `constant` runtime semantics and SillyTavern `constant` field normalization. Version 1 behavior is preserved for entries without the `constant` field.

## Version 2 runtime contract

The canonical persisted document is `characters/{character_id}/world/lorebook.json`:

```json
{
  "entries": [
    {"keys": ["moon gate"], "content": "...", "enabled": true, "priority": 10, "constant": false, "comment": "optional"},
    {"keys": [], "content": "...", "enabled": true, "priority": 30, "constant": true, "comment": "always injected"}
  ]
}
```

### Trigger rule

The minimum activation contract is:

```text
enabled && (constant || primary_keyword_match)
```

- `enabled` defaults to `true`. An entry with `enabled=false` is never injected, even when `constant=true`.
- `constant` defaults to `false` (or `None`). When `constant=true` and `enabled!=false`, the entry is injected on every generation regardless of keyword matches.
- `keys` uses OR semantics. Empty keys are ignored for keyword matching. Empty keys do not prevent a `constant=true` entry from injecting.
- Each entry activates at most once per generation. A `constant=true` entry whose keys also match the scan text is still injected exactly once.
- Constant entries and keyword-triggered entries share the same priority ordering and the single `[World Info/Lorebook Information]` output block.

### Ordering

Activated entries sort by descending `priority`; missing priority defaults to `10`. Equal priorities retain source-document order. This default is shared by `Lorebook::trigger`, `merge_lorebooks`, and `convert_character_book_to_lorebook` so that stored order does not drift from runtime output.

### Injection

Output is injected once under `[World Info/Lorebook Information]` in the RP system-prompt assembly. Control-plane agent observations are never scanned or injected. Scene merges deduplicate identical `content`, then apply the same ordering and trigger rules.

### Fixtures

- Version 1 baseline: [`engine/tests/fixtures/worldbook/airp-v1-basic.json`](../engine/tests/fixtures/worldbook/airp-v1-basic.json). Exact rendered output is asserted in Rust tests. No `constant` field; backward compatible with version 2.
- Version 2 constant semantics: [`engine/tests/fixtures/worldbook/airp-v2-constant.json`](../engine/tests/fixtures/worldbook/airp-v2-constant.json). Covers constant-without-keys, disabled-constant, constant+keyword dedup, and priority ordering across constant and keyword entries. Exact rendered output is asserted in Rust tests.

## SillyTavern compatibility boundary

The source fixture [`engine/tests/fixtures/worldbook/sillytavern-character-book-source.json`](../engine/tests/fixtures/worldbook/sillytavern-character-book-source.json) records fields commonly encountered during import. In version 2 the following fields are normalized into the runtime model:

- `keys` / `key` — primary trigger keywords (OR semantics)
- `content` — injected text
- `enabled` / `disable` (inverted) — enable flag
- `order` / `insertion_order` — mapped to `priority`
- `constant` — mapped to `constant`

The following are explicitly **not implemented yet** and must not be advertised as compatible: `secondary_keys` / `keysecondary`, `selective` + logic, probability, sticky/cooldown/delay, groups, recursion, `position` / `depth`-specific insertion, `caseSensitive`, `use_regex`, `match_whole_words`. Importers preserve these in the raw sidecar but they do not affect runtime behavior. Adding any of them requires a new fixture with exact trigger and prompt-placement assertions plus a contract version note.

## Change gate

Any semantic change must update, in the same PR:

1. this contract;
2. at least one source and normalized fixture;
3. deterministic trigger and final prompt-placement tests;
4. the compatibility and priority statements in `docs/CURRENT-BASELINE.md` and issue #126.

## Version history

- **v2** (this version): adds `constant` field and runtime semantics; unifies `priority` default to `10` across convert/trigger/merge.
- **v1**: initial baseline. `keys`/`content`/`enabled`/`priority` with Aho-Corasick keyword trigger.
