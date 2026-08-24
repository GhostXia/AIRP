# PR #588 independent audit — Widget host security parity

Date: 2026-08-24  
Auditor: CodeRabbit (independent PR review, run `8de5312a-31c7-46e4-b457-01fb03b96880`)  
Decision: pass after the fixes and dispositions below

## Scope reviewed

The audit reviewed all 21 changed files at `fa53ef3`, including the Vue Engine
catalog/grant/plugin bootstrap, both iframe bridges, WidgetHost lifecycle,
browser smoke, tests, and current-fact/security documents.

## Findings and disposition

| ID | Severity | Finding | Disposition |
|---|---|---|---|
| A1 | Minor | The stale-session browser probe posted stale and valid mount messages back-to-back without proving that only one mount ran. | Fixed. The sandbox fixture now records mount invocations and the real-browser assertion requires exactly one. |
| A2 | Major | Recreate the bridge if Vue reuses a WidgetHost for a different instance identity. | Not applicable after source verification. `BlueprintRenderer.vue` keys the enclosing `Teleport` by `instance.id` and keys `WidgetHost` by `instance.type`; a change to either identity destroys and recreates the host. State-only changes intentionally preserve the bridge. |
| A3 | Minor | Reject explicit empty `host_api` instead of defaulting it to major 1. | Rejected because it conflicts with the Engine authority. `engine::extensions::parse_host_api_major(Some("")) == 1` is test-locked, and `docs/WIDGET-DEVELOPMENT.md` explicitly defines missing or empty as the compatibility default. Vue must not invent a stricter divergent contract. |
| A4 | Major | Reject an empty Vue sandbox instance id before creating frame resources. | Fixed with an early guard and regression test proving no iframe is created. |

## Verification

- Vue typecheck and 167 tests pass.
- WebUI Widget runtime contract tests pass.
- Real Engine + Chrome smoke proves digest-pinned load, exactly one valid mount,
  granted capability projection, and opaque-frame storage/host-DOM isolation.
- The full pre-audit gate had already passed: Rust fmt/clippy/workspace tests,
  all WebUI tests, UI build, and the real Engine browser smoke.

There are no deferred audit findings from this review. A2 and A3 are resolved
as verified non-issues rather than future work.
