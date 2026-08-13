# PR #566 rollback no-op independent audit

- Date: 2026-08-13
- Audited branch: `codex/issue-140-rollback-semantics` after merging current `origin/main`
- Scope: `engine/src/chat_store.rs`, `engine/README.md`
- Verdict: **PASS**

## Findings

No blocking or non-blocking findings.

## Independent assessment

- Range validation happens before the no-op path, so invalid indices remain rejected without mutation.
- An explicit no-op is accepted only when the requested durable ID matches the persisted `active_leaf` with the repository's case-insensitive ULID matcher.
- Legacy no-op detection is deliberately narrower: `active_leaf` must be absent, all parent links must be absent, vector lengths must match, and the target must be the physical tail.
- A dangling persisted `active_leaf` cannot enter the no-op path. It reaches the normal save path, repairs the leaf to the requested message, increments revision, and updates persistence.
- Returning before `save()` preserves `updated_at`, revision, JSONL bytes, and metadata bytes for true no-op retries.
- Sibling branches are unaffected because rollback still removes only active-path entries after the selected target.
- The change is Engine-only; issue #319 visual review is not applicable.

## Verification

- `cargo test -p airp-core --lib chat_store::tests::rollback_to --locked -- --nocapture`: 5 passed.
- `cargo test -p airp-core --lib domain::tests::rollback --locked -- --nocapture`: 4 passed.
- `cargo test -p airp-core --lib chat_store::tests::rollback_repairs_dangling_persisted_active_leaf --locked -- --nocapture`: 1 passed.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed.

## Non-blocking workflow note

The Agent Browser Exploration failure is in generated test-script syntax before execution. Its workflow and report explicitly mark it non-blocking; it produced no crash, data-loss, or security evidence about this rollback change. The recurrence belongs to the already tracked agent-exploration process family and is not a product finding for #566.
