# #564 PR 1: Blueprint Desktop Architecture and Baseline

> Status: accepted implementation baseline for issue [#564](https://github.com/GhostXia/AIRP/issues/564).
> Decision date: 2026-08-12.
> Code baseline: `main@7a90d88`.
> Scope: architecture and repeatable evidence only. This PR does not change a wire contract, Engine route, desktop entry point, or user asset.

## 1. Owner decision

AIRP will restore the protocol-driven Vue desktop UI because the original
Blueprint/Widget direction has better compatibility and extension boundaries
than reusing one WebUI page tree as the desktop product surface.

The restoration is not a return to the old demonstration UI. It keeps the
current WebUI product behavior and visual language while rebuilding the
desktop information architecture around versioned Surfaces and Widgets.

This decision supersedes the 2026-08-05 conclusion that reduced Blueprint to
the current WebUI slot plan and treated the Vue main surface as permanently
archived. That conclusion remains useful history: it accurately described the
C-P0 through C-P4 implementation at that time. It no longer defines the target
architecture after the Owner reopened the desktop direction in #564.

## 2. Target boundary

### Desktop product surface

- `ui/src/` remains the Vue 3 desktop product surface.
- A versioned `Blueprint` describes a bounded layout tree and stable Widget
  instances. It never contains arbitrary HTML, CSS, JavaScript, Vue code, or
  executable Agent output.
- The first workspace presets are `Story`, `World`, `Director`, and `Debug`.
  `Story` is the simple default; advanced surfaces use progressive disclosure.
- The shell has stable domain navigation, a replaceable central Surface, and a
  collapsible Context Inspector bound to the current Surface, Widget, or
  selected domain object.
- Agent work is observable through a separate Activity Center. Activity and
  orchestration metadata must not enter roleplay prompt context.

### Engine and transport

- The Engine remains the authority for characters, sessions, messages,
  memory, world books, character state, extension identity, grants, and
  privileged effects.
- The Engine will serve the desktop bundle at `/desktop/` in addition to the
  existing WebUI at `/`. Both clients use the same domain services and user
  assets.
- The Vue surface will use REST plus SSE through an `HttpEngineBus`. Tauri does
  not regain a hand-written per-intent business relay.
- Surface projections are reconstructable UI state, not a second domain truth.
  Broken or stale streams recover from an authoritative snapshot.

### Tauri shell

`ui/src-tauri/` keeps only native responsibilities already justified by the
current product:

- Engine sidecar spawn, readiness, shutdown, and duplicate-start protection;
- local data-root and packaged/portable semantics;
- access-key ownership and desktop-session token exchange;
- native error presentation and packaging.

The archived `docs/archive/2026-08-04-c-p0-desktop-shell/bus.rs` business relay
is explicitly not restored.

### Visual system

- `webui/assets/tokens.css` is the initial canonical token source.
- Vue may add desktop-specific semantic tokens, but it must not copy WebUI DOM
  trees or grow an unrelated theme.
- Existing WebUI screenshots remain the visual behavior baseline during the
  migration. A restored desktop screen is not required to have identical
  geometry; it must read as the same AIRP product and preserve proven states.

## 3. Data authority

| Data | Authority | Persistence rule |
|---|---|---|
| Characters, sessions, messages, memory, world books, character state | Existing Engine domain services | Existing versioned/atomic asset stores |
| Default Blueprint | Deterministic Engine Surface builder | Code or versioned first-party template |
| User workspace | Engine UI workspace service | New versioned asset with atomic write, backup, export, and rollback |
| Widget projection | Engine projection from domain state | Reconstructable; never a second domain store |
| SSE cursor and pending patch | Transport | Disposable; resync from snapshot |
| Focus, scroll, temporary expansion | Vue | Local and non-authoritative by default |
| Agent UI proposal | Bounded proposal | Applied only after UI policy and, by default, user confirmation |

No step in #564 may rewrite character cards, world books, sessions, or memory
merely to support the new UI. Workspace persistence is a new asset class and
must not be hidden inside an existing domain JSON blob.

## 4. Current asset disposition

| Current asset | Decision | Reason / next owner |
|---|---|---|
| `ui/src/App.vue` demo chrome and self-built minimal Blueprint | **Replace** | PR 3 builds the real shell and explicit loading/error/disconnect states. |
| `ui/src/components/BlueprintRenderer.vue` fixed area renderer | **Replace** | PR 4 implements bounded recursive Blueprint v2 rendering. |
| `ui/src/protocol/types.ts` manual v1 mirror | **Replace** | PR 2 selects a machine-readable authority and locks Rust/TS parity. |
| `ui/src/protocol/guard.ts` and atomic patch discipline | **Retain, rewrite for v2** | Structural validation and last-known-good behavior remain required. |
| `ui/src/protocol/bus.ts` MockBus | **Retain for tests only** | Production/browser preview must not silently fall back to mock success. |
| `ui/src/protocol/tauri-bus.ts` per-intent IPC path | **Delete after replacement** | PR 6 introduces one HTTP/SSE client used in browser and Tauri. |
| `ui/src/registry/` and `WidgetHost.vue` | **Retain and harden** | PR 7 restores parity with current Engine grants and opaque iframe hosting. |
| `ui/src/widgets/` | **Reassess individually** | Existing mock behavior is not delivery evidence; each Widget needs a real Engine vertical slice. |
| `ui/src-tauri/src/main.rs` lifecycle/token/package path | **Retain** | Native shell responsibility remains valid; navigation becomes dual-entry. |
| archived Tauri `bus.rs` | **Keep archived; never restore** | It duplicates business mapping and risks a second state authority. |
| `webui/` pages and REST/SSE behavior | **Retain during measured migration** | Current supported product and fallback; retirement requires RC evidence and Owner approval. |
| `webui/assets/tokens.css` | **Retain as canonical visual input** | Shared design language without sharing the full page shell. |
| `webui/assets/widgets/` runtime | **Retain for WebUI; establish parity** | It remains supported until each desktop replacement passes behavior and security gates. |
| `protocol/sse-events.json`, `widget-grants.json`, `widget-intents.json` | **Retain for their existing scopes** | UI Surface events get a separate contract; no unrelated chat-stream expansion. |
| `protocol/src/lib.rs` Blueprint v1 | **Migration input, then replace** | PR 2 defines v2 compatibility and negative fixtures. |

## 5. Baseline capture

The baseline intentionally separates reproducible static evidence from runtime
evidence that does not exist at this commit. A missing measurement is recorded
as missing; it is not inferred from an old CI artifact.

### Reproducible static evidence at `7a90d88`

Run from the repository root in PowerShell:

```powershell
git rev-parse HEAD
git status --short

npm --prefix ui run build
Get-ChildItem ui/dist -Recurse -File |
  Measure-Object -Property Length -Sum |
  Select-Object Count, Sum

Get-ChildItem webui/baseline-screenshots -File -Filter '*.jpg' |
  Measure-Object -Property Length -Sum |
  Select-Object Count, Sum

node --test webui/tests/*.test.mjs
```

Observed before this PR changed documentation:

| Evidence | Result |
|---|---|
| Vue `ui/dist` after a clean `npm --prefix ui run build` | 6 files, 113,855 bytes; generated output, not release evidence |
| Engine-staged `ui/src-tauri/webui-bundle` | 147 files, 4,009,670 bytes; generated staging output |
| Current WebUI source excluding baseline screenshots | 133 files, 966,475 bytes |
| WebUI visual baseline | 44 JPEG screens in `webui/baseline-screenshots/`; covers primary, advanced, empty, reconnect, error, modal, and extension states |

Generated `ui/dist` and `ui/src-tauri/webui-bundle` sizes can change with local
build state. The authoritative PR evidence is an `npm --prefix ui run build`
result plus the sizes captured immediately after that build.

### Runtime evidence still missing

At `7a90d88`, this checkout has no current packaged Tauri executable tied to
this exact commit. Therefore PR 1 does not claim numbers for:

- process start to first usable frame;
- Engine ready to first usable Story workspace;
- idle and post-history-load working set;
- WebView2 screenshots at 1024x768, 1440x900, 1920x1080, 125%, or 150%;
- real-provider desktop task success.

PR 3 must add a repeatable visual harness. PR 6 must capture browser and Tauri
startup/reconnect timings. PR 13 must run those measurements on a packaged
Windows artifact and compare them with the current WebUI entry on the same
machine, data fixture, provider configuration, and warm/cold-cache condition.

## 6. Migration and rollback

The existing WebUI stays available at `/` throughout M1. The new desktop
bundle is introduced at `/desktop/` and selected in Tauri only through an
explicit development/preview switch until RC gates pass.

Rollback before the default switch is selecting `/` again. Rollback after the
default switch remains a visible supported setting/startup escape hatch for at
least one stable release. Because both UIs call the same Engine domain services
and the desktop projection is reconstructable, UI rollback must not require a
domain-data downgrade.

Workspace schema upgrades require dry-run validation, pre-upgrade backup,
integrity verification, and rollback. Unknown major versions fail closed and
show a repairable placeholder; they do not overwrite the workspace.

## 7. PR 1 completion gate

This PR is complete only when:

1. this superseding decision is linked from the current baseline and UI
   protocol decision;
2. every relevant current UI asset has an explicit retain/replace/delete/add
   outcome;
3. static evidence can be regenerated with repository commands;
4. unmeasured runtime claims are explicitly marked missing and assigned to a
   later PR gate;
5. migration preserves the current WebUI and all user assets;
6. documentation checks and affected UI/WebUI tests pass;
7. the independent audit bot passes before merge.

Passing PR 1 does not mean Blueprint runtime, Surface API, or the restored
desktop UI has shipped. The first real closed loop is M1 after PR 1 through PR
9 in #564.
