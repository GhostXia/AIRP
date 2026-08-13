# PR #575 / #564 PR 3 desktop shell local causal audit

- Date: 2026-08-13
- Branch: `codex/564-desktop-shell`
- Base: `main@b2091b2`
- Scope: Vue shell, canonical token import, responsive/accessibility behavior, browser smoke and CI evidence
- Verdict: **PASS locally; remote independent audit still required**

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

## Verification

- `npm --prefix ui run typecheck`: passed.
- `npm --prefix ui run test -- --run`: 15 files / 121 tests passed.
- `npm --prefix ui run build`: passed.
- `npm --prefix ui run smoke:shell`: 5 profiles passed with non-empty screenshots and no overflow/page errors.
- In-app browser interaction: workspace selection, Inspector collapse, Focus Mode, `aria-current`, no horizontal overflow, no console warnings/errors passed.
- Remote audit bot and complete repository CI remain required before merge.

## Non-blocking observations

- The fixed preview still uses the v1 `BlueprintRenderer`; PR 4 owns replacement with the audited v2 runtime.
- Full screen-reader/manual Windows WebView2 evidence remains a PR 13 release gate, not proof supplied by this browser shell slice.
