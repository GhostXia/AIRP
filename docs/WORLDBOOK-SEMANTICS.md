# AIRP Worldbook Semantic Contract

Status: version 1, implemented baseline. This document distinguishes accepted runtime semantics from compatibility candidates; a source field is not supported merely because import code can see it.

## Version 1 runtime contract

The canonical persisted document is `characters/{character_id}/world/lorebook.json`:

```json
{"entries":[{"keys":["moon gate"],"content":"...","enabled":true,"priority":10,"comment":"optional"}]}
```

- `keys` uses OR semantics. Empty keys are ignored and matching is case-sensitive substring matching over the current user message plus the bounded history assembled by the chat pipeline.
- `enabled` defaults to `true`.
- Each entry activates at most once per generation, even if several keys match.
- Activated entries sort by descending `priority`; missing priority is `10`. Equal priorities retain source-document order.
- Output is injected once under `[World Info/Lorebook Information]` in the RP system-prompt assembly. Control-plane agent observations are never scanned or injected.
- Scene merges deduplicate identical `content`, then apply the same ordering and trigger rules.

The executable fixture is [`engine/tests/fixtures/worldbook/airp-v1-basic.json`](../engine/tests/fixtures/worldbook/airp-v1-basic.json). Its exact rendered output is asserted in Rust tests.

## SillyTavern compatibility boundary

The source fixture [`engine/tests/fixtures/worldbook/sillytavern-character-book-source.json`](../engine/tests/fixtures/worldbook/sillytavern-character-book-source.json) records fields commonly encountered during import. In version 1 only `keys`, `content`, `enabled`, and insertion order mapped to `priority` are normalized into the runtime model.

The following are explicitly **not implemented yet** and must not be advertised as compatible: secondary/selective-key logic, `constant`, probability, sticky/cooldown/delay, groups, recursion, and position/depth-specific insertion. Importers may discard these fields today. Adding any of them requires a new fixture with exact trigger and prompt-placement assertions plus a contract version note.

## Change gate

Any semantic change must update, in the same PR:

1. this contract;
2. at least one source and normalized fixture;
3. deterministic trigger and final prompt-placement tests;
4. the compatibility and priority statements in `docs/CURRENT-BASELINE.md` and issue #126.
