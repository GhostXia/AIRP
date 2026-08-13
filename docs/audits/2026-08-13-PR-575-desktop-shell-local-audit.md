# PR #575 / #564 PR 3 desktop shell local causal audit

- Date: 2026-08-13
- Branch: `codex/564-desktop-shell`
- Base: `main@b2091b2`
- Scope: Vue shell, canonical token import, responsive/accessibility behavior, browser smoke and CI evidence
- Verdict: **PASS locally and in independent multimodal visual review; remote gate completion still required**

## Boundary assessment

- The browser path no longer constructs MockBus or implies an Engine connection. It renders a labelled fixed fixture.
- The existing Tauri transport remains temporary compatibility code; this PR does not add a business relay.
- The shell imports the canonical WebUI token file rather than cloning its values. Desktop-only tokens describe shell geometry and type roles.
- Four workspace presets are bounded UI navigation, not persisted layouts or Engine truth.
- Context Inspector and Activity content are labelled fixture data; neither reaches prompts, grants authority or writes domain state.
- Existing Blueprint v1 renderer remains inside the fixed preview only. No claim is made that the v2 recursive runtime is implemented.

## Findings resolved during implementation

### L1 — Browser preview silently used MockBus success state

Resolution: `App.vue` constructs a transport only in the explicit Tauri environment. Browser preview uses a fixed labelled Surface and persistent status banner.

### L2 — Old shell and core preview widgets retained cyberpunk hard-coded colors

Resolution: the shell, renderer boundary, WidgetHost errors, Chat and Characters preview consume canonical semantic tokens. This PR does not mechanically restyle every archived/scaffold Widget; later real Widget migrations own those components.

### L3 — Screenshot navigation changed the captured default workspace

Resolution: the smoke returns from World to Story before capture, so evidence records the intended default while still proving keyboard navigation.

### R1 — Connection status could report success after initialization failure

Resolution: the shell now tracks `preview` / `connecting` / `connected` / `failed` independently from environment detection, and retry reconstructs the bus when initialization failed.

### R2 — Short viewports hid the actionable error state

Resolution: the status banner compacts below 640px instead of disappearing. The smoke captures a 1024x600 error state with its recovery action visible.

### V1 — Main story text was truncated in four visual profiles

Resolution: preview chat rows now allow wrapped text within a larger virtual row. The independent visual re-review confirmed the complete sentence is readable in all five profiles.

### R3 — Compact Focus Mode retained an empty Inspector column

Resolution: the `max-width: 1180px` rule now reasserts the two-column Focus Mode layout. The shell smoke checks the computed grid has exactly two columns at 1024px.

### R4 — Tauri listener readiness and dispatch failure were not connection state

Resolution: `AgentBus.subscribe` may resolve asynchronously, `TauriBus` propagates listener attachment failures, and the shell marks connected only after subscription succeeds. Dispatch failures invalidate the active bus so retry reconstructs the transport. Attempt identities prevent overlapping retries from leaking a stale listener, and teardown is idempotent against late rejections.

### R5 — Compact Inspector overlay did not own the viewport height

Resolution: the overlay is bounded by `top: 0` and `bottom: 0`, contains its close control, and keeps the body scrollable. The 1024 smoke asserts its bounding box spans the viewport.

### R6 — Wrapped chat text could exceed the fixed virtualization row

Resolution: fixed-height virtualization remains the explicit scaffold performance contract, while each row's text owns a bounded scroll area. Arbitrarily long text remains reachable without changing spacer math or overlapping adjacent rows.

## Verification

- `npm --prefix ui run typecheck`: passed.
- `npm --prefix ui run test -- --run`: 15 files / 122 tests passed.
- `npm --prefix ui run build`: passed.
- `npm --prefix ui run smoke:shell`: 5 profiles passed with non-empty screenshots and no overflow/page errors.
- In-app browser interaction: workspace selection, Inspector collapse, Focus Mode, `aria-current`, no horizontal overflow, no console warnings/errors passed.
- Independent multimodal visual review: initial `BLOCKED`, then `PASS` after fixes. The reviewer independently inspected all five default profiles plus `1024x768-context-open.png`, `1024x600-error.png`, and `1440x900-keyboard-focus.png`; it confirmed complete story text, a readable overlay, visible error/retry state, visible keyboard focus, and no blocking overlap, truncation, or overflow.
- Remote audit bot and complete repository CI remain required before merge.
- `CURRENT-BASELINE.md` calibration is intentionally deferred until a post-merge recalibration; this unmerged PR is not recorded as completed history.

## Non-blocking observations

- The fixed preview still uses the v1 `BlueprintRenderer`; PR 4 owns replacement with the audited v2 runtime.
- Full screen-reader/manual Windows WebView2 evidence remains a PR 13 release gate, not proof supplied by this browser shell slice.
- Small low-contrast utility text and one-character compact navigation labels can be improved.
- English protocol/status labels mixed into the Chinese shell remain consistent but can be localized later.
- The visible focus ring can use stronger contrast.
- When the Engine is unavailable, the preview composer still looks actionable; a real connected slice should disable, queue, or explain unavailable send behavior.
- The smoke currently serves the Vite application rather than the just-built `dist`; production bundle consumption remains a later packaging/release gate.
- Smoke teardown requests Vite termination but does not yet wait for bounded process exit and explicit port release.
- The optional agent-test module is fail-closed in production but still emitted as a separate production chunk; a later hardening pass can exclude it entirely and assert its absence.
