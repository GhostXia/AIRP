# AIRP

AIRP is an AI Agent client specialized for role play. It uses a headless Engine with replaceable UIs and keeps characters, personas, presets, worldbooks, sessions, memory, Agent execution, and security boundaries in one auditable local data plane.

> Current baseline: 2026-08-09, `main@affa315`; current candidate: `v0.0.5-rc.2` (prerelease).
> This page is an entry point. See the [current development baseline](docs/CURRENT-BASELINE.md) for exact capabilities, gaps, and evidence boundaries.

## Repository layout

- `engine/`: `airp-core`, the only RP/Agent business core and HTTP/SSE service;
- `webui/`: the current primary product surface;
- `airp-engine-console/`: the visual and interaction reference for the WebUI;
- `protocol/`: shared wire protocol;
- `ui/`: retained Tauri + Vue desktop client; the shell now hosts same-origin `webui/`, while GUI-real-device and real-provider acceptance remain open;
- `deploy/windows-webui/`, `deploy/linux-webui/`: portable WebUI artifacts;
- `deploy/production/`: single-instance self-hosted HTTPS preview;
- `data/`: runtime data-root contract and safe repository examples;
- `tools/`: dependency governance, SBOM, and Agent browser-exploration tooling.

AIRP-Core/AIRPCLI, AIRP-MCP-Server, AIRP-Gateway, and AIRP-State-Protocol are first-party predecessor projects, not current runtime dependencies. See [source-project decisions](docs/SOURCE-PROJECT-DECISIONS.md).

## Current status

The current tree includes:

- OpenAI-compatible, Anthropic, Ollama, and multi-provider routing;
- JSON/PNG character cards, personas, presets, scenes, worldbooks, state, memory, and revisions;
- named sessions, durable history, cursors, edit/delete, branches, Swipe, continue/regen, and full-text search;
- a bounded Agent loop, 30 built-in tools, Director, Council, NPC actions, plot arcs, a world clock, and timed events;
- image generation, character templates, style learning, dialogue examples, worldbook graphs, timeline export, and card diffs;
- HTTP webhook and controlled local-script plugin tools that can join the Agent registry dynamically;
- 44 build-free WebUI screens covering onboarding, chat, asset management, and creation workflows;
- Windows/Linux portable WebUI artifacts, a production preview, SBOM generation, and an Agent browser-exploration layer.

This is still a **P1 limited-trial code candidate**, not a production release. `v0.0.5-rc.2` is the 2026-08-09 prerelease candidate at `main@affa315`; the candidate workflow validates the exact tag, builds the package, runs browser/desktop smoke, and the current release path uploads only the Windows portable package. Dependency inventory, SBOM, third-party notices, and sign-off data remain in `docs/sbom/` in the tagged git tree for development users to inspect; they are not CI/release attachments or sign-off gates, and existing rc.2 assets are outside this change. Formal `v0.0.5` remains blocked by [#130](https://github.com/GhostXia/AIRP/issues/130) (real provider + real browser + production Compose acceptance) and the missing required-reviewer configuration on the `release` environment. Feature presence does not prove real-user workflows, crash recovery, long-session behavior, upgrades/rollback, or market validation. The current priority is to close concurrency, persistence, and failure boundaries and then validate onboarding → first chat → refresh → service-restart recovery with real providers.

## Core principles

- RP prompts contain RP data only; tools, scheduling, and audit metadata stay in the structured control plane.
- The Engine is the single source of truth for data and business rules; UIs, handlers, and Agent tools do not duplicate persistence logic.
- Internal architecture may evolve or be rebuilt, but user assets must remain migratable, verifiable, exportable, and recoverable.
- AIRP learns from public third-party ideas, needs, behavior, and interoperability, while independently implementing its own code, prompts, tests, data, and visuals.
- Green local tests only authorize opening a PR. The audit bot must pass, blocking findings must be fixed, and a human reviewer decides whether to merge.

See [AGENTS.md](AGENTS.md) and the [development handover guide](docs/DEV-GUIDE.md) for the full rules.

## Quick start

### Development

Rust, Node.js, and npm are required:

```powershell
cargo run -p airp-core -- daemon --open-browser --webui-dir webui
```

The default URL is `http://127.0.0.1:8765/` (the engine default `daemon_port=8765`); first use opens onboarding. Development mode is loopback-only. Do not expose the Engine port to an untrusted network or browser origin. Portable packages and deploy scripts pass their own port explicitly (for example 8765 under `deploy/`); follow the script argument there.

### Windows portable package

Maintainers build it with:

```powershell
deploy/windows-webui/build.ps1
```

Users extract the artifact and run `Start-AIRP.cmd`. The package-local `data/` directory contains user assets and must be backed up and migrated when upgrading or moving the package, but ordinary asset backups and migration bundles must exclude `data/secrets.json`; handle that plaintext key file separately through encrypted, permission-restricted storage and transfer. Users do not need Rust, Node, Docker, WSL, or Tauri.

### Local verification

The maintainer-specific D-drive toolchain overrides are documented in [AGENTS.md](AGENTS.md); they are not project-wide requirements.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
node --test webui/tests/*.test.mjs

Push-Location ui
npm ci
npm run typecheck
npm test -- --run
Pop-Location
```

Release, production-topology, browser-exploration, and artifact checks are separate gates and cannot be inferred from unit/integration tests.

## Documentation

- [Current development baseline](docs/CURRENT-BASELINE.md)
- [Development handover guide](docs/DEV-GUIDE.md)
- [Product and architecture plan](docs/PLAN.md)
- [Security boundary](docs/SECURITY.md) / [risk register](docs/RISK-REGISTER.md)
- [Session and revision contract](docs/SESSION-DATA-DESIGN.md)
- [Worldbook semantics contract](docs/WORLDBOOK-SEMANTICS.md)
- [Full document map](docs/README.md)

## License

MIT OR Apache-2.0.
