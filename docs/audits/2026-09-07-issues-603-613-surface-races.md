# Issues #603 / #613 — independent audit PASS

Date: 2026-09-07. Base: `bed754331d8b9075dde7fe04965226b83d69364f`.
Source implementation: `e92d72d`, cherry-picked into the combined issue batch.
Auditor: Carson (`01a07c1e-ca43-71a0-8581-f172e059feae`), independent and read-only,
following AGENTS.md. No blocking or nonblocking action items remain.

## Findings

- Initialization rechecks its attempt after asynchronous session listing before
  reconnecting. Surface apply checks disposal, attempt and exact Bus identity.
  Both the current candidate and its subsequently published Bus may apply; stale
  candidates cannot mutate accepted state.
- The orchestration test extracts and executes the real App function, rather
  than reproducing its guard. Delayed candidate and session-list boundaries are
  deterministic; the actual atomic Surface store is used.
- The Chat test compiles the actual SFC and executes reactive props, template
  binding and scroll handling. A failed history load remains latched at the top;
  leaving and re-entering the threshold triggers one same-cursor retry. Synthetic
  scroll after prepending does not chain requests.

## Validation and limits

Independent validation: 28 Vitest files, 210 tests; typecheck; production build.
Five in-memory mutations were rejected: remove attempt, identity, disposal or
post-session-list checks, and break the pagination latch. The four added tests
passed again with normal source. No production source was changed by the auditor.

FakeBus does not prove HTTP/SSE transport behavior. The custom Vue renderer does
not prove browser layout, scroll anchoring or real DOM dispatch. These fixtures
test the targeted orchestration/pagination rules; existing browser CI remains.

Main-agent validation also passed Blueprint runtime and responsive browser
smoke in the isolated worktree, alongside UI build/tests. No dependency update
is included. npm audit independently reported GHSA-2v37-7h3g-55p8 in the existing
Vite -> PostCSS -> nanoid@3.3.16 dependency chain; record this separate finding
after merge, then handle through dependency-governance review. It was not caused
or resolved by these source changes.
