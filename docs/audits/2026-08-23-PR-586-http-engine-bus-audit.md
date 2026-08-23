# PR #586 HttpEngineBus Audit

- Date: 2026-08-23
- Initial audited head: `eefd6796b8e909c63b720bce884e7cfa99c4af80`
- Corrected implementation head: `54ac9f4f572a738f383c3b145c3e553e5dbd5d5d`
- Auditors: independent read-only audit agent and CodeRabbit repository audit bot
- Scope: authenticated snapshot/SSE transport, `/desktop/` hosting, read-only Engine projection,
  Tauri dual entry, token rotation, packaging, and real-Engine browser evidence
- Initial verdict: **BLOCKED** by two independent P1 findings, then seven CodeRabbit findings
- Final disposition: every finding was fixed in the PR branch; no deferred audit findings remain

## Findings and disposition

1. **Transient snapshot failure stopped resynchronization** — fixed by putting snapshot reloads in
   the same bounded retry loop as SSE reconnects; a 503-then-success test protects the path.
2. **Read-only authorization was inferred from projected chat shape** — fixed with an explicit
   read-only contract from renderer through WidgetHost and widgets, plus host-level intent
   suppression. The real-Engine smoke uses an oversized projection and asserts zero intent calls.
3. **Superseded bus callbacks could overwrite the active connection state** — fixed by binding
   connection/error callbacks to the current initialization attempt.
4. **Repeated resync could request-storm the Engine** — fixed with escalating backoff shared by
   malformed-stream resync and transient snapshot failures; accepted events reset the counter.
5. **Bundle build inherited the caller's working directory** — fixed by anchoring the npm build to
   `$PSScriptRoot` with guaranteed location restoration; direct root invocation is tested.
6. **Current baseline contradicted the PR 6 delivery facts** — fixed in the repository map and
   capability matrix while preserving dated PR 1–5 statements as history.
7. **Surface reads and the reserved intent transport seam were conflated** — fixed in security and
   plan documentation; the future write path remains capability- and executor-gated.
8. **Post-PR-6 documentation retained stale review anchors** — recalibrated Security, Risk Register,
   and UI protocol decision metadata to the August 23 candidate.
9. **The chat placeholder exposed an internal PR number** — replaced with durable user-facing text.

CodeRabbit also warned that the data favicon might be out of scope. It is retained as an in-scope
smoke control: it prevents an ambient `/favicon.ico` request from being confused with the deliberate
missing-asset 404 contract while adding no executable or external asset.

## Validation evidence

- `npm run typecheck`: passed
- `npm test -- --run`: 18 files / 144 tests passed
- `npm run build`: passed
- `npm run smoke:runtime`: passed
- root-invoked `ui/bundle-webui.ps1`: passed
- `npm run smoke:http-bus` against the release Engine: passed
- `cargo fmt --all -- --check`: passed
- `cargo clippy --workspace --all-targets --locked -- -D warnings`: passed
- `cargo test -p airp-core daemon::tests::local_webui --locked`: 4 passed
- Full workspace, release lock-order, sacred subagent invariants, Rustdoc, WebUI, and Tauri gates
  passed before the final audit fixes; all affected TypeScript, packaging, and browser gates were
  rerun afterward.
- `git diff --check`: passed

The initial repository audit is complete and all review threads are resolved by code or an explicit
scope disposition. Per the Owner's instruction not to wait for a second audit, merge remains gated
by GitHub CI and zero unresolved review threads rather than another included CodeRabbit pass.
