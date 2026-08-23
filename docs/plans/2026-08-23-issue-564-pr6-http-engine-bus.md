# Issue #564 PR 6 — HttpEngineBus and dual desktop entry

Date: 2026-08-23

## Scope

This slice connects the Vue Blueprint runtime to the authenticated Engine
Surface API delivered by PR 5. It does not add chat, memory, character-state,
or extension write executors; those remain PR 7–9.

## Decisions

- `/` remains the supported WebUI rollback entry; `/desktop/` serves the Vue
  bundle from the same Engine origin.
- Browser and Tauri use one `HttpEngineBus` over REST plus authenticated
  streaming `fetch` SSE. The archived Tauri business relay is not restored.
- The bearer arrives only through `#airp-token`, moves to session storage before
  the first Engine request, and is removed from the URL. Every request resolves
  the current token so proactive shell renewal and 401 rotation take effect.
- SSE cursors remain opaque. A cursor advances only after the corresponding
  Surface message passes the atomic store. Broken streams reconnect from the
  last accepted cursor; malformed events, revision gaps, or rejected patches
  force a fresh snapshot.
- Engine Widget props are rendered directly as the reconstructable projection;
  they are not copied into a second Vue domain store.
- `AIRP_DESKTOP_UI=blueprint` opts Tauri into `/desktop/`. The default remains
  `/`, and a missing Blueprint bundle visibly falls back to WebUI.

## Verification

- TypeScript guards cover token scrubbing/rotation, snapshot/SSE patching,
  cursor replay, malformed-event resync, 401 renewal, and cleanup.
- Engine router tests cover the legacy root, `/desktop/`, scoped SPA fallback,
  missing-asset 404 behavior, and shared security headers.
- A real-Engine Chrome smoke imports a synthetic character, creates a session,
  opens `/desktop/` with a short desktop-session token, consumes the token
  fragment, rotates the token, and verifies a live non-fixture Surface under
  the production CSP. An oversized chat projection also proves the PR 6
  transport remains explicitly read-only.
- Windows package smoke checks the bundled Vue assets and exercises both the
  default WebUI entry and the explicit Blueprint entry.
