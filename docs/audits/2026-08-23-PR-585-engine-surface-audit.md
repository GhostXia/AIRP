# PR #585 Engine Surface Audit

- Date: 2026-08-23
- Audited head: `1ecda503188e0cfb1363b54dc806de4f38832f84`
- Auditor: CodeRabbit repository audit bot
- Scope: authenticated Engine Surface snapshot/SSE, bounded replay, read-only projection, and durable redacted Activity receipts
- Initial verdict: **BLOCKED** with four inline findings and three review-summary integrity findings
- Final disposition: all seven findings fixed in the PR branch; no deferred audit findings remain

## Findings and disposition

1. **Agent prepare failure omitted a durable receipt** — fixed by preserving the admitted
   operation's generation ID and existing read-only session directory before pipeline preparation,
   then recording a redacted `upstream_error` if preparation fails.
2. **Unleased Agent finalization failure omitted a receipt** — fixed by retaining
   `FinalizerCtx.session_dir` independently of the optional coordinator lease while keeping the
   generation ID optional.
3. **Surface SSE ignored graceful shutdown** — fixed by subscribing to the daemon shutdown watch
   channel and ending the stream promptly; an endpoint test verifies EOF after broadcast.
4. **Surface entries were unbounded** — fixed with a 128-entry least-recently-published cap. Ring
   events for an evicted scope are removed, and a racing active request falls back to its freshly
   projected snapshot rather than failing.
5. **Activity read-modify-write could lose concurrent receipts** — fixed by reusing the bounded
   per-session memory mutation lock around the complete read/append/trim/atomic-replace sequence;
   a concurrent writer test verifies all receipts survive.
6. **Props patching assumed aligned Widget arrays** — fixed by requiring equal Blueprint version,
   layout root, Widget count, instance IDs, and Widget types. Structural changes now publish a full
   snapshot.
7. **Read-only session resolution duplicated character path construction** — fixed by using the
   existing non-creating `character_dir_path` helper.

## Validation evidence

- `cargo fmt --all -- --check`: passed
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed
- `cargo test --workspace --locked`: passed; Engine 1466 passed / 5 ignored, protocol 13 passed,
  Tauri 25 passed, all integration and doc tests passed
- `cargo test -p airp-core --release --features lock-order-runtime --lib lock_order:: --locked -- --nocapture`: 15 passed
- `cargo test -p airp-core --lib subagent_ --locked -- --nocapture`: 2 passed
- `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked`: passed
- `git diff --check`: passed

The repository bot's included review quota was exhausted by the first audit. Per the Owner's
instruction not to wait for a second audit, the corrected head is gated by the evidence above plus
GitHub CI and zero unresolved review threads before merge.
