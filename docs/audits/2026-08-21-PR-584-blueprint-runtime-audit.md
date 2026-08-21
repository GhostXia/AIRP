# PR #584 Blueprint v2 Runtime Independent Audit

- Date: 2026-08-21
- Audited implementation: `61d41d94` (`feat(ui): add Blueprint v2 runtime`)
- Scope: `ui/` Blueprint v2 renderer, Surface state, Widget lifecycle, runtime smoke, and affected WebUI/CI checks
- Auditor posture: independent read-only review under the repository Audit Agent Charter
- Verdict: **PASS for audited implementation `61d41d94` only**; later PR-head changes require the final-head gate below.

## Review result

The final implementation has no blocking or deferred non-blocking findings. The audit independently inspected the renderer/store/lifecycle paths, reproduced edge cases, and reran focused browser and unit evidence. Earlier findings were fixed before this verdict:

1. Widget type replacement now remounts the implementation without sacrificing stable instance relocation. The Teleport is keyed by collision-free validated instance ID and the inner host by widget type.
2. Widget props and state are independent. Missing state and explicit `null` remain `null`; module widgets read live instance metadata and receive state notifications separately.
3. In-place nested state patches now advance a per-scope revision, notifying module and sandbox consumers without a deep watch or cloning a 5,000-message scope.
4. Failed initialization patches preserve missing and `null` scopes and do not advance their revision.
5. Tab activation clears focus when the focused widget becomes hidden.
6. The runtime smoke scrolls the 5,000-message fixture through middle and final windows while keeping the rendered row count bounded.

## Lifecycle and containment evidence

- Moving an unchanged widget between layout branches preserves the same mounted host.
- Replacing a widget type unmounts the old module exactly once and renders the new implementation/fallback.
- Props-only, state-object, nested state-patch, and explicit-null updates retain their distinct semantics.
- Unknown and throwing widgets degrade within their own host while an unaffected sibling remains mounted.
- A rejected Surface patch preserves revision and last-known-good rendering and requests resynchronization.
- Async acquisition and teardown behavior is covered by focused lifecycle tests.

## Validation evidence

- `npm run typecheck`: passed
- `npm test -- --run`: 17 files, 140 tests passed
- `npm run build`: passed
- `npm run smoke:shell`: 5 viewport/scale profiles passed
- `npm run smoke:runtime`: passed; observed warm-patch p95 0.7–1.1 ms and 16 virtual rows in the final local runs
- `node --test webui/tests/*.test.mjs`: 207 tests passed
- Runtime screenshot inspected at 1440×900: no shell overlap or horizontal overflow; fallback boundaries and actual Surface revision were visible
- `git diff --check`: passed

## Scope honesty

This PR delivers the bounded client-side PR4 runtime over a labelled local Surface v2 fixture. It does not claim an Engine Surface API, a real `HttpEngineBus`, `/desktop/` production serving, or a real RP domain loop; those remain roadmap PR5 and PR6 work.

## Merge gates

This independent audit is complete. GitHub CI, CodeRabbit review of the final PR head, unresolved-thread checks, and human merge review remain required before merge.
