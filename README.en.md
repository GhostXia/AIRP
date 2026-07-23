# AIRP

English | [中文](README.md)

AIRP is an AI Agent client specialized for Role Play. The product follows a "headless engine + swappable UI" structure:

- **engine** (`airp-core`): RP data, prompt assembly, LLM adapter, Agent loop and HTTP/SSE;
- **webui**: the current primary product delivery surface;
- **ui** (`airp-ui`): a retained Tauri + Vue desktop client, with recent development and packaging verification paused;
- **protocol** (`airp-state-protocol`): the wire-protocol types shared between UI and engine.

The authoritative implementation is defined by `main` and the [current development baseline](docs/CURRENT-BASELINE.md); pinned commits only certify the corresponding historical code tree. For document roles and the shortest reading path, see the [document map](docs/README.md).

## Project Principles

- Keep the RP character plane clean: tools, results and orchestration scaffolding flow through the model's native control plane and do not pollute character prompts;
- Agent execution is bounded by step/token/wall-clock/cancellation limits, and the engine enforces capability, allowlist and destructive-action confirmation;
- RP data is managed centrally by the engine shared service; HTTP, Agent tools and UI do not each implement their own copy;
- Large files are not retained in the model context, reactive store or logs; server-side path reads are only allowed from trusted local calls;
- Extension is controlled but open: structured interfaces, capability, sandboxing, user consent and auditable changes;
- Code and protocols should be open, transparent, easy to correct and easy to iterate.

## Periodic Generational Upgrades

To prevent AIRP from being locked in by legacy architecture, old toolchains or the capability ceiling of prior agent generations, the user may explicitly launch a generational upgrade every half year or year:

- At launch, the latest, officially released flagship LLM suitable for complex software engineering must be verified through first-party official information and used for leading planning, key implementation and independent audit;
- Breaking refactors are allowed, including starting from a blank architecture and **rebuilding from scratch**; the fact that an old implementation already runs does not forbid redesign;
- If the change scope is too large to keep `main` releasable through bounded PRs, an isolated `remake/<cycle>` branch or equivalent isolated product line must be established to evolve in parallel with the original project; the original project continues to receive necessary maintenance, security fixes and data export support during this period;
- A remake must not replace the original project based on developer or agent self-evaluation. Before launch, market criteria and an observation window must be defined, and judgments must be based on reviewable evidence such as voluntary trial/migration, retention, core task success rate, stability, user feedback and willingness to continue use;
- Only when market evidence consistently shows that the remake is overall superior to the original project, and with explicit user approval, may it gradually replace the original by function, user and data batches. Before migration, asset verification and the rollback window are complete, the original project must not be fully taken offline;
- Breaking the old structure does not mean destroying user assets: incompatible changes must still provide versioned migration, pre-upgrade backup, integrity verification, readable export and rehearsed rollback, and must continue to pass security, testing, licensing, PR audit and human review gates.

The agent execution details for this rule are in [AGENTS.md](AGENTS.md) under "Periodic Generational Refactor Exception".

The original AIRPCLI, AIRP-MCP-Server, AIRP-Gateway and AIRP-State-Protocol repositories are the author's own first-party predecessor projects, used only as asset sources. They are uniformly handled as "absorb assets, do not inherit the predecessor's product North Star", see [Source Project Asset Absorption Decisions](docs/SOURCE-PROJECT-DECISIONS.md). For third-party research and independent-implementation boundaries, see [Acknowledgements and Provenance](docs/ACKNOWLEDGEMENTS.md).

## Directory

```text
<repo>/
├── engine/                 airp-core
├── protocol/               airp-state-protocol
├── ui/                     Vue + Tauri desktop assets
│   └── src-tauri/          airp-ui
├── webui/                  current product WebUI
├── deploy/windows-webui/   current portable Windows WebUI package
├── deploy/production/      retained self-hosted P0 preview assets
├── data/                   runtime data-root contract and safe samples
├── docs/                   live docs, contracts, research and archive
└── .github/workflows/      PR gate and manual Windows build
```

Only `engine`, `protocol` and `ui/src-tauri` are members of the Rust workspace. The legacy `gateway` / `mcp-server` are not in this workspace and are not runtime dependencies.

## Current Status

Major foundations already delivered include:

- OpenAI-compatible / Anthropic streaming chat and a bounded structured tool-call Agent loop;
- A default 21-tool registry, runtime catalog and engine capability gate;
- Character cards, named sessions, durable history, state, basic worldbook, preset, scene, volume and analysis/decompose;
- Preset normalization import reports, raw input sidecar, version directories and an atomic current pointer, plus confirmation-gated `get_preset` / `update_preset` Agent tools;
- A `PromptAssemblyTrace` driven by real chat assembly, side-effect-free desensitized preview, and the WebUI's effective-configuration and ordered assembly summary for the current turn;
- Multi-Persona storage/HTTP/pipeline, WebUI CRUD, auto/explicit selection, effective source and character/session binding closure;
- worldbook v4 `constant` + `selective`/`secondary_keys` runtime semantics, presence-aware v3 migration, shared normalizer/import diagnostics, normal-user main-panel editing and end-to-end regression from PNG/JSON to final prompt;
- WebUI basic RP loop, history window and rollback-by-ID;
- Production P0 for single-instance self-hosted WebUI: same-origin HTTPS, private engine, secret mounts, fail-closed configuration and real-topology CI;
- P1 first-chat failure closure and safety boundary: critical persistence failures do not return false success, SSE preserves commit state, uncertain commit state is not blindly retried, and client errors and diagnostics are desensitized;
- P1 manual cold-backup/rollback escape path: archive hash verification, independent rollback volume verification, and resuming public listening only after read-only verification passes;
- Canonical session UUID, legacy metadata best-effort repair, and follow-up contracts for self-contained session/revision.

P1 limited-trial code candidates exist but have not yet reached release condition. We are continuing to develop the first-chat golden path, recovery path and high-value product gaps, and producing repeatable evidence with real providers, real browsers, production topology and automated/human acceptance. Persona advanced lifecycle, full Preset lifecycle, full Worldbook asset lifecycle, complete session revision, versioned migration, automatic backup/restore, recoverable deletion, formal upgrade rollback, browser matrix and long-session soak are not yet complete. Do not infer details from this page; defer to [CURRENT-BASELINE.md](docs/CURRENT-BASELINE.md).

## Development Environment

The project does not restrict the install drive for Rust, Node, npm, MSYS2, cache or target. Just make sure `cargo`, `node` and `npm` are in the current shell's `PATH`.

The maintainer's local machine uses a `D:` drive override due to insufficient `C:` space; the full environment variable record for that machine is in [AGENTS.md](AGENTS.md). It is not a project-level requirement and should not be replicated to other contributor environments.

## Local Run

Start the engine:

```powershell
cargo run -p airp-core -- daemon --port 8000
```

Start the WebUI development environment:

```powershell
cd webui
node serve.js
```

On Windows, `webui/start.bat` can be used to start local development dependencies. None of these paths are user deliverables; do not expose port 8000 or the static dev server directly to the public internet.

The current priority delivery path is the portable Windows WebUI package: maintainers can run `deploy/windows-webui/build.ps1` to produce `dist/airp-webui-windows-x64.zip`; on GitHub Release, the same build is automatically attached to the Release after full smoke passes. Users download, extract and double-click the readable `Start-AIRP.cmd`; it directly starts the engine, does not invoke PowerShell, does not request administrator privileges, and does not perform installation. Users do not need Rust, Node, Docker, WSL or Tauri; `data/` and `config.json` stay inside the extracted directory, and provider keys are stored centrally in `data/secrets.json` inside the package and are not echoed back via API/UI by default. Back up and migrate `data/` before upgrading. The package listens only on `127.0.0.1` and must not be forwarded to a LAN or the public internet. The `deploy/production/` self-hosted topology is retained but is not a current landing prerequisite.

Tauri desktop development:

```powershell
cd ui
npm run tauri dev
```

The Tauri desktop route is a long-term development item; this command is only for maintaining existing assets and does not represent a near-term product delivery path.

## Verification

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:RUSTDOCFLAGS = "-D warnings"
cargo doc --workspace --no-deps --locked
Remove-Item Env:RUSTDOCFLAGS
cargo test --workspace --locked
cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise --locked -- --nocapture

cd ui
npm ci
npm run typecheck
npm run test -- --run
```

`.github/workflows/pr-gate.yml` automatically enforces the Rust workspace, UI/WebUI and existing production topology regression gates. `.github/workflows/webui-windows-build.yml` handles the current portable Windows WebUI artifact and real-Chrome acceptance; `.github/workflows/manual-build.yml` only retains the long-term Tauri desktop build. CodeRabbit (`.coderabbit.yaml` with `request_changes_workflow: true`) is the pre-merge blocking audit gate: a fully-green local run only permits opening a PR; you must wait for the audit to pass and a human review to decide whether to merge. (Gemini Code Assist has been sunset and uninstalled.)

PR #232 final head `29b52fa`'s [PR gate run 29645599733](https://github.com/GhostXia/AIRP/actions/runs/29645599733) passed the Rust workspace (including warning-free rustdoc and the clean prompt invariant `subagent_context_has_no_orchestrator_noise`), UI and WebUI, production topology and CodeRabbit, and was then merged as the code-tree-equivalent merge commit `main@2a14b7e`. Remote evidence: 756 lib (740 engine pass + 1 ignored + 6 protocol + 9 ui) + 25 integration tests, WebUI 97 tests, ui Vitest 98 tests, production topology 104 checks / 0 failures; this calibration locally re-computed dep-governance 90 tests. Evidence only certifies that PR head and the corresponding merged code tree; it does not automatically certify subsequent changes.

## Key Documents

- [Current Development Baseline](docs/CURRENT-BASELINE.md)
- [Development Handover Guide](docs/DEV-GUIDE.md)
- [WebUI Production Plan](docs/WEBUI-PRODUCTION-PLAN.md)
- [Product and Architecture Plan](docs/PLAN.md)
- [Session Archive and Revision Contract](docs/SESSION-DATA-DESIGN.md)
- [Worldbook Semantics Contract](docs/WORLDBOOK-SEMANTICS.md)
- [Security Boundary](docs/SECURITY.md) / [Risk Register](docs/RISK-REGISTER.md)
- [Full Document Map](docs/README.md)

## License

MIT OR Apache-2.0.
