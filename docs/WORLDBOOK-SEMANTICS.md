# AIRP Worldbook Semantic Contract

Status: version 3. Adds shared `WorldbookNormalizer`, v3 advisory metadata fields (`secondary_keys`, `case_sensitive`, `extensions`), and import diagnostics (`WorldbookImportReport`). Version 2 runtime semantics (`constant`) are preserved unchanged. Version 1 behavior is preserved for entries without the `constant` field.

## Version 3 schema

The canonical persisted document is `characters/{character_id}/world/lorebook.json`:

```json
{
  "entries": [
    {
      "keys": ["moon gate"],
      "content": "...",
      "enabled": true,
      "priority": 10,
      "constant": false,
      "comment": "optional",
      "secondary_keys": ["night"],
      "case_sensitive": null,
      "extensions": null
    },
    {
      "keys": [],
      "content": "...",
      "enabled": true,
      "priority": 30,
      "constant": true,
      "comment": "always injected"
    }
  ]
}
```

### Runtime fields (v1/v2, consumed by `trigger()`)

- `keys` — primary trigger keywords (OR semantics)
- `content` — injected text
- `enabled` — enable flag (defaults `true`)
- `priority` — sort weight (defaults `10`, descending)
- `constant` — always-inject flag (defaults `false`)
- `comment` — free-form annotation

### Advisory metadata fields (v3, NOT consumed by `trigger()`)

These fields are populated by the shared normalizer from SillyTavern aliases and preserved in the canonical document. They do **not** affect runtime trigger or injection behavior. Future retrieval tools or advanced trigger semantics may consume them; until then they are "advisory metadata + future input."

- `secondary_keys` — from ST `keysecondary`. Preserved for future selective logic or retrieval tools.
- `case_sensitive` — from ST `caseSensitive`. Current trigger uses case-sensitive `AhoCorasick::LeftmostLongest` by default; this field is advisory only.
- `extensions` — `BTreeMap<String, Value>` collecting all ST-only fields not in the canonical schema (`selective`, `position`, `depth`, `probability`, `sticky`, `cooldown`, `delay`, `group`, `use_regex`, `match_whole_words`, `recursion`, and any unknown fields). Serialized as `null` when empty to keep canonical output clean.

All v3 fields use `#[serde(default)]` for backward compatibility: v1/v2 data deserializes without breakage.

## Version 2 runtime contract (unchanged in v3)

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

Activated entries sort by descending `priority`; missing priority defaults to `10`. Equal priorities retain source-document order. This default is shared by `Lorebook::trigger`, `merge_lorebooks`, and `normalize_worldbook` so that stored order does not drift from runtime output.

### Injection

Output is injected once under `[World Info/Lorebook Information]` in the RP system-prompt assembly. Control-plane agent observations are never scanned or injected. Scene merges preserve entries with distinct activation semantics, evaluate activation and priority for each entry, then deduplicate activated entries by identical `content`; the highest-priority activated variant wins and each content value is emitted once.

## Shared WorldbookNormalizer (v3)

[`normalize_worldbook`](../engine/src/orchestrator/worldbook_normalizer.rs) is the single normalization entry point shared by three import paths:

1. **PNG character_book import** — `handlers::persist_character_assets` calls `normalize_worldbook` on `card.data.character_book`.
2. **PUT `/v1/characters/:id/lorebook`** — accepts raw JSON (AIRP canonical, ST form, or bare array), normalizes before persisting.
3. **Agent `update_lorebook` tool** — normalizes the `lorebook` parameter before writing via `LorebookService`.

### SillyTavern alias normalization

The normalizer maps ST field aliases to AIRP canonical fields:

| SillyTavern alias | AIRP canonical | Notes |
|---|---|---|
| `keys` (array) | `keys` | Direct; empty strings filtered |
| `key` (string) | `keys` | Comma-separated, trimmed |
| `content` (string) | `content` | Required; missing → invalid |
| `enabled` (bool) | `enabled` | Takes precedence over `disable` |
| `disable` (bool) | `enabled` | Inverted: `disable=true` → `enabled=false` |
| `priority` (int) | `priority` | Takes precedence over `order`/`insertion_order` |
| `order` (int) | `priority` | ST alias |
| `insertion_order` (int) | `priority` | ST alias, fallback after `order` |
| `constant` (bool) | `constant` | Direct |
| `comment` (string) | `comment` | Direct |
| `keysecondary` (array) | `secondary_keys` | v3 advisory |
| `caseSensitive` (bool) | `case_sensitive` | v3 advisory |

### ST-only field preservation

Fields not in the canonical schema and not consumed as aliases are preserved in `extensions`:

`selective`, `position`, `depth`, `probability`, `sticky`, `cooldown`, `delay`, `group`, `use_regex`, `match_whole_words`, `recursion`, and any unknown fields.

These are stored as-is in the `BTreeMap` for future use or manual review. They do **not** affect runtime behavior. Adding any of them to the runtime contract requires a new contract version with deterministic trigger and prompt-placement tests.

### Idempotency

Passing a canonical AIRP Lorebook JSON through `normalize_worldbook` produces an equivalent Lorebook with no redundant `extensions`. Alias source fields (`key`, `disable`, `order`, etc.) are consumed and not re-added to `extensions`.

## Import diagnostics (v3)

`normalize_worldbook` returns `(Lorebook, WorldbookImportReport)`. The report tracks:

- `total_input` — entry count in source JSON
- `converted` — entries successfully converted to canonical form
- `aliases_normalized` — entries that used ST alias fields
- `advisory_preserved` — entries with advisory metadata (non-empty `secondary_keys` / `case_sensitive` / `extensions`)
- `invalid` — entries that couldn't be parsed (skipped, with reason)
- `needs_review` — entries that parsed but may not behave as expected (e.g., empty keys + non-constant → never triggers)

Invalid entries are skipped; the rest are processed. `needs_review` does not block writes. The report is returned in PUT API responses and Agent tool results.

## Fixtures

- Version 1 baseline: [`engine/tests/fixtures/worldbook/airp-v1-basic.json`](../engine/tests/fixtures/worldbook/airp-v1-basic.json). Exact rendered output is asserted in Rust tests. No `constant` field; backward compatible with v2/v3.
- Version 2 constant semantics: [`engine/tests/fixtures/worldbook/airp-v2-constant.json`](../engine/tests/fixtures/worldbook/airp-v2-constant.json). Covers constant-without-keys, disabled-constant, constant+keyword dedup, and priority ordering. Exact rendered output is asserted in Rust tests.
- SillyTavern source: [`engine/tests/fixtures/worldbook/sillytavern-character-book-source.json`](../engine/tests/fixtures/worldbook/sillytavern-character-book-source.json). ST character_book with object-map entries covering all alias and ST-only field patterns. Used by normalizer unit tests.

## Change gate

Any semantic change must update, in the same PR:

1. this contract;
2. at least one source and normalized fixture;
3. deterministic trigger and final prompt-placement tests;
4. the compatibility and priority statements in `docs/CURRENT-BASELINE.md` and issue #126.

**v3 note:** The v3 advisory metadata fields do not change runtime trigger or injection behavior. The existing v2 trigger tests remain the deterministic trigger proof. The normalizer unit tests (`test_idempotent_on_canonical_lorebook`, `test_idempotent_round_trip`) prove that v3 fields do not alter the runtime path.

## Version history

- **v3** (this version): adds shared `WorldbookNormalizer`, v3 advisory metadata (`secondary_keys`, `case_sensitive`, `extensions`), import diagnostics (`WorldbookImportReport`). Three import paths (PNG, PUT API, Agent tool) now share one normalizer. Runtime trigger semantics unchanged from v2.
- **v2**: adds `constant` field and runtime semantics; unifies `priority` default to `10` across convert/trigger/merge.
- **v1**: initial baseline. `keys`/`content`/`enabled`/`priority` with Aho-Corasick keyword trigger.
