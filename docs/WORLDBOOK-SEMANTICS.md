# AIRP Worldbook Semantic Contract

Status: version 4. Promotes `selective` from ST-only extension to canonical runtime field. `selective=true` entries require a `secondary_keys` match (in addition to primary key match) to activate. Version 3 advisory metadata (`secondary_keys`, `case_sensitive`, `extensions`) and import diagnostics are preserved. Version 2 runtime semantics (`constant`) are preserved unchanged. Version 1 behavior is preserved for entries without the `constant` field.

Last implementation check: 2026-07-18 at `main@63f1c5b`.

## Version 4 schema

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
      "selective": true,
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

### Runtime fields (v1/v2/v4, consumed by `trigger()`)

- `keys` — primary trigger keywords (OR semantics)
- `content` — injected text
- `enabled` — enable flag (defaults `true`)
- `priority` — sort weight (defaults `10`, descending)
- `constant` — always-inject flag (defaults `false`)
- `secondary_keys` — v3 advisory, **v4 runtime when `selective=true`**: secondary trigger keywords (OR semantics). See [v4 trigger rule](#version-4-runtime-contract-selective).
- `selective` — **v4 runtime** (defaults `false`): when `true`, primary key match alone is insufficient; at least one `secondary_keys` entry must also match the scan text.

### Preserved annotation

- `comment` — free-form annotation; preserved and used for diagnostics, but not consumed by `trigger()`.

### Advisory metadata fields (v3, NOT consumed by `trigger()`)

These fields are populated by the shared normalizer from SillyTavern aliases and preserved in the canonical document. They do **not** affect runtime trigger or injection behavior.

- `case_sensitive` — from ST `caseSensitive`. Current trigger uses case-sensitive `AhoCorasick::LeftmostLongest` by default; this field is advisory only.
- `extensions` — `BTreeMap<String, Value>` collecting all ST-only fields not in the canonical schema (`position`, `depth`, `probability`, `sticky`, `cooldown`, `delay`, `group`, `use_regex`, `match_whole_words`, `recursion`, and any unknown fields). Serialized as `null` when empty to keep canonical output clean. **v4: `selective` is no longer in `extensions`** — it is a canonical field.

All v3/v4 fields use `#[serde(default)]` for backward compatibility. `LorebookService::read` inspects raw JSON before deserialization so extension-only v3 `selective=true` is promoted without being confused with an absent top-level field; an explicit top-level `false` retains precedence.

## Version 4 runtime contract (selective)

### Trigger rule (v4)

The v4 activation contract extends v2 with a secondary-match gate:

```text
enabled && (constant || (primary_keyword_match && (!selective || no_valid_secondary_keys || secondary_keyword_match)))
```

- `selective` defaults to `false`. When `false`, behavior is identical to v3 (primary match alone activates).
- When `selective=true` and the entry is not `constant`:
  - Primary key match is still required (primary gate unchanged).
  - If `secondary_keys` is empty or contains only empty strings, the primary match alone activates the entry (matches ST behavior: selective with no secondary keys = primary-only).
  - If `secondary_keys` has at least one non-empty key, **any one** of them must also match the scan text (OR semantics, same as primary keys).
  - Secondary matching uses case-sensitive `text.contains(key)`, consistent with the primary Aho-Corasick DFA's runtime behavior. Empty secondary keys are filtered before matching.
- `constant=true` entries ignore `selective` entirely — they always inject when enabled, regardless of primary or secondary matches.

### Version 2 runtime contract (unchanged in v4, except selective gate)

The minimum activation contract for non-selective entries is:

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

Scene merges also preserve entries whose runtime fields match but advisory metadata differs. This keeps `case_sensitive`, `extensions`, and annotations available to future retrieval tools while the v4 injection path still emits identical content only once.

Scene prompt traces retain per-entry provenance through merge, activation, priority ordering, and content deduplication. Character-owned books use the path-independent logical `source_id` `character:<character_id>`; the scene-owned book uses `scene:<scene_id>`. `item_id` is the zero-based index of the entry in that source document before merge. Exact semantic duplicates retain the first source in scene merge order. These identifiers explain the current source document and entry; they are not immutable content revisions, and no filesystem path is exposed.

## Shared WorldbookNormalizer (v4)

[`normalize_worldbook`](../engine/src/orchestrator/worldbook_normalizer.rs) is the single normalization entry point shared by three import paths:

1. **PNG character_book import** — `handlers::persist_character_assets` calls `normalize_worldbook` on `card.data.character_book`.
2. **PUT `/v1/characters/:id/lorebook`** — accepts raw JSON (AIRP canonical, ST form, or bare array), normalizes before persisting.
3. **Agent `update_lorebook` tool** — normalizes the `lorebook` parameter before writing via `LorebookService`.

### SillyTavern alias normalization

The normalizer maps ST field aliases to AIRP canonical fields:

| SillyTavern alias | AIRP canonical | Notes |
|---|---|---|
| `keys` (array) | `keys` | Direct; empty strings filtered |
| `key` (string or array) | `keys` | Strings are comma-separated and trimmed; arrays are preserved |
| `content` (string) | `content` | Required; missing → invalid |
| `enabled` (bool) | `enabled` | Takes precedence over `disable` |
| `disable` (bool) | `enabled` | Inverted: `disable=true` → `enabled=false` |
| `priority` (int) | `priority` | Takes precedence over `order`/`insertion_order` |
| `order` (int) | `priority` | ST alias |
| `insertion_order` (int) | `priority` | ST alias, fallback after `order` |
| `constant` (bool) | `constant` | Direct |
| `comment` (string) | `comment` | Direct |
| `keysecondary` (array) | `secondary_keys` | v3 advisory; v4 runtime when `selective=true` |
| `caseSensitive` (bool) | `case_sensitive` | v3 advisory |
| `selective` (bool) | `selective` | **v4 canonical** (was ST-only extension in v3) |

### v4 selective migration

The normalizer promotes `selective` to canonical with the following precedence:

1. ST top-level `selective` (bool) — ST native field, authoritative source.
2. v3 `extensions.selective` (bool) — old AIRP canonical data where v3 normalizer placed `selective` into `extensions`. v4 migrates it back to canonical.
3. Absent → defaults to `false`.

When both top-level and `extensions.selective` are present, top-level wins (ST native takes precedence). `selective` is added to `CONSUMED_FIELDS` and will not appear in `extensions` after normalization.

### ST-only field preservation

Fields not in the canonical schema and not consumed as aliases are preserved in `extensions`:

`position`, `depth`, `probability`, `sticky`, `cooldown`, `delay`, `group`, `use_regex`, `match_whole_words`, `recursion`, and any unknown fields.

These are stored as-is in the `BTreeMap` for future use or manual review. They do **not** affect runtime behavior. Adding any of them to the runtime contract requires a new contract version with deterministic trigger and prompt-placement tests.

### Idempotency

Passing a canonical AIRP Lorebook JSON through `normalize_worldbook` produces an equivalent Lorebook with no redundant `extensions`. Alias source fields (`key`, `disable`, `order`, etc.) are consumed and not re-added to `extensions`. v4: `selective` is consumed to canonical and not re-added to `extensions`.

## Import diagnostics (v3, unchanged in v4)

`normalize_worldbook` returns `(Lorebook, WorldbookImportReport)`. The report tracks:

- `total_input` — entry count in source JSON
- `converted` — entries successfully converted to canonical form
- `aliases_normalized` — entries that used ST alias fields
- `advisory_preserved` — entries with advisory metadata (non-empty `secondary_keys` / `case_sensitive` / `extensions`)
- `source_error` — unsupported top-level shape, when present
- `invalid` — entries that couldn't be parsed (skipped, with reason)
- `needs_review` — entries that parsed but may not behave as expected (e.g., empty keys + non-constant → never triggers)

Invalid entries are skipped while valid entries continue. Replace operations fail closed when the source shape is unsupported or every input entry is invalid; explicit empty `entries` arrays/objects remain the supported way to clear a lorebook. Recognized fields with the wrong JSON type, including priorities outside the signed 32-bit range, are invalid instead of being silently defaulted or truncated. `needs_review` does not block writes. The report is returned in PUT API responses and Agent tool results.

## Fixtures

- Version 1 baseline: [`engine/tests/fixtures/worldbook/airp-v1-basic.json`](../engine/tests/fixtures/worldbook/airp-v1-basic.json). Exact rendered output is asserted in Rust tests. No `constant` field; backward compatible with v2/v3/v4.
- Version 2 constant semantics: [`engine/tests/fixtures/worldbook/airp-v2-constant.json`](../engine/tests/fixtures/worldbook/airp-v2-constant.json). Covers constant-without-keys, disabled-constant, constant+keyword dedup, and priority ordering. Exact rendered output is asserted in Rust tests.
- Version 4 selective semantics: [`engine/tests/fixtures/worldbook/airp-v4-selective.json`](../engine/tests/fixtures/worldbook/airp-v4-selective.json). Covers selective+secondary-match, selective+empty-secondary, non-selective, selective+secondary-mismatch-suppressed, and constant+selective-ignored. Exact rendered output is asserted in Rust tests.
- SillyTavern source: [`engine/tests/fixtures/worldbook/sillytavern-character-book-source.json`](../engine/tests/fixtures/worldbook/sillytavern-character-book-source.json). ST character_book with object-map entries covering all alias and ST-only field patterns. Used by normalizer unit tests.

## Change gate

Any semantic change must update, in the same PR:

1. this contract;
2. at least one source and normalized fixture;
3. deterministic trigger and final prompt-placement tests;
4. the compatibility and priority statements in `docs/CURRENT-BASELINE.md` and issue #126.

**v4 note:** The v4 `selective` runtime field is consumed by `Lorebook::trigger`. Deterministic trigger tests (`airp_v4_selective_fixture_has_deterministic_output` and the selective semantic test suite) prove the v4 activation contract. The normalizer unit tests (`test_st_top_level_selective_promoted_to_canonical`, `test_v3_extensions_selective_migrated_to_canonical`, `test_selective_round_trip_stable`) prove canonical normalization stability and v3→v4 migration compatibility.

## Version history

- **v4** (this version): promotes `selective` from ST-only extension to canonical runtime field. `selective=true` entries require a `secondary_keys` match (in addition to primary key match) to activate. `secondary_keys` is now runtime (when `selective=true`) rather than purely advisory. v3 `extensions.selective` is migrated to canonical. Normalizer `CONSUMED_FIELDS` updated.
- **v3**: adds shared `WorldbookNormalizer`, v3 advisory metadata (`secondary_keys`, `case_sensitive`, `extensions`), import diagnostics (`WorldbookImportReport`). Three import paths (PNG, PUT API, Agent tool) now share one normalizer. Runtime trigger semantics unchanged from v2.
- **v2**: adds `constant` field and runtime semantics; unifies `priority` default to `10` across convert/trigger/merge.
- **v1**: initial baseline. `keys`/`content`/`enabled`/`priority` with Aho-Corasick keyword trigger.
