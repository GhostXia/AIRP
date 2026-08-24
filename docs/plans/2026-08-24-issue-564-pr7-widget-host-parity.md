# Issue #564 PR 7 — Widget Registry/Host security parity

Date: 2026-08-24

## Scope

This slice connects the Vue Blueprint host to the Engine-authoritative extension
catalog, grants, and trusted-plugin inventory. It does not consume the WebUI
slot plan and does not add Chat, Memory, or Character State write executors;
those remain PR 8–9.

## Decisions

- Vue fetches `/v1/extensions/catalog`, `/v1/grants`, and `/v1/plugins` before
  mounting the app. Catalog or grant failure leaves third-party widgets closed.
- The Engine contract is checked for catalog version, host API major, the exact
  capability set, unique widget types, sandboxed same-origin ESM sources, and
  trusted-plugin declarations before any manifest becomes visible.
- Production ESM manifests are never registered into the Vue host process.
  They load only through the shared static frame with `sandbox="allow-scripts"`
  and no `allow-same-origin`.
- Every bridge message is bound to the exact iframe window, a random bridge
  session, and the stable Widget instance id. The frame also verifies its
  parent window and parent origin. Teardown removes the listener and frame.
- WidgetContext remains the common interface. A third-party frame receives only
  its instance, current state projection, and Engine-granted capabilities; it
  never receives the bearer or host objects.
- Trusted-plugin requirements are non-blocking visibility hints. Missing,
  stopped, and insufficient-host-API states are distinguished; they do not
  silently manufacture plugin availability.

## Verification

- Vue tests cover catalog negotiation and failure closure, no in-process ESM
  registration, Engine bearer use, plugin dependency states, iframe identity
  gates, lifecycle teardown, and same-origin enforcement.
- WebUI parity tests lock the same window/session/instance gates and exact
  opaque sandbox attribute.
- The real-Engine Chrome smoke installs and grants a synthetic extension,
  verifies its digest-pinned catalog source, mounts it in the production static
  frame, and proves the frame cannot read session storage or the host DOM.
- The normal Rust, UI, WebUI, browser, and Windows package gates remain required
  before merge.
