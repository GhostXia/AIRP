# AIRP UI

`ui/` 是 AIRP 的 Tauri + Vue 桌面客户端目录。当前运行事实仍是 C-P0 的 Tauri 壳同源承载 `webui/`；Owner 已在 [#564](https://github.com/GhostXia/AIRP/issues/564) 决定恢复协议驱动的 Vue Blueprint/Widget 桌面主面，因为该设计具有更好的兼容性与扩展性。目标架构、资产处置和双入口回退见 [`../docs/plans/2026-08-12-issue-564-desktop-architecture-baseline.md`](../docs/plans/2026-08-12-issue-564-desktop-architecture-baseline.md)。本页最后在 2026-08-21 的 PR #584 实现提交上复核。

历史候选发布事实：Actions run `31309894372` 在 commit `affa315a5917109e2ae337382cfcdcb36021073a` 对 artifact `airp-webui-windows-x64` 的包内 desktop smoke 成功；该证据不覆盖当前 `main@7a90d88`，也不替代 GUI 真机与真实 provider 验收。正式 `v0.0.5` 仍受 [#130](https://github.com/GhostXia/AIRP/issues/130) 和 `release` environment required reviewer 配置阻塞。

当前全仓状态与发布门槛见 [`../docs/CURRENT-BASELINE.md`](../docs/CURRENT-BASELINE.md)。

UI 继承 AIRP-State-Protocol 的 Blueprint、Widget、patch、guard、虚拟滚动和沙箱经验，但不继承其通用协议优先定位。当前目标是 AIRP 专用桌面客户端；Widget 扩展必须先服务 RP 工作流。详见 [`../docs/UI-PROTOCOL-DECISION.md`](../docs/UI-PROTOCOL-DECISION.md)。

## 当前运行事实

- `ui/src-tauri/` 负责 Engine sidecar 生命周期、data root、desktop-session token、原生错误与打包。
- Tauri 启动后导航到 Engine 同源承载的 WebUI；正式产品主面仍是 `webui/`。
- `ui/src/` 中的 Vue Blueprint/Widget 代码是可运行的迁移期桌面 preview，但尚不是正式产品主面。
- Vue preview 使用 canonical WebUI tokens，提供四个工作区、Focus Mode、Context Inspector，以及通过 Surface v2 guard 的固定 fixture。受限 runtime 支持 `split/tabs/stack/widget`、原子 accepted/pending/ephemeral store、稳定 Widget relocation 和局部错误隔离；它不使用 MockBus 冒充 Engine 成功。
- `createBus()` 的非 Tauri fallback 仍是测试/显式 demo 用 MockBus，但当前 `App.vue` 的浏览器 preview 不调用它；在 #564 PR 6 前不得把任一路径写成真实浏览器产品链路。
- 归档的 Tauri `BusRelay` 不在当前源码中，#564 也不会恢复逐 intent 的 Rust 业务 relay。

## #564 目标职责

- Vue 渲染 Engine 权威下发、通过 guard 的 Blueprint v2 和 Widget projection。
- 浏览器 `/desktop/` 与 Tauri 共用 REST + SSE `HttpEngineBus`。
- Tauri 只保留 native shell 职责，不解释聊天、记忆、状态或扩展 intent。
- 当前 WebUI 在迁移与市场观察期继续可用，且与桌面 UI 读写同一组 Engine domain service。
- MockBus 只用于显式测试和演示 fixture。

## 结构

```text
ui/
├── package.json
├── src/
│   ├── App.vue
│   ├── protocol/          # TS-side protocol mirror and TauriBus
│   ├── registry/          # widget registry, consent, sandbox bridge
│   ├── state/             # atomic Surface store + local ephemeral UI state
│   └── widgets/           # first-party widgets
├── widgets/core/          # widget manifests
└── src-tauri/
    ├── Cargo.toml
    ├── capabilities/default.json
    └── src/
        ├── main.rs        # Tauri shell lifecycle, token, navigation
        └── lifecycle.rs   # startup ownership and shutdown rules
```

The Rust protocol crate lives in `../protocol`. Surface v2 uses [`../protocol/surface-protocol-v2.json`](../protocol/surface-protocol-v2.json) as its machine-readable authority; Rust and TypeScript bindings remain manual mirrors locked by shared positive/negative/migration fixtures and parity tests. The older Envelope/Blueprint v1 types remain for demo compatibility and explicit migration only. The Vue preview now includes the bounded v2 renderer and client store, but still does not provide an Engine Surface endpoint, `HttpEngineBus`, or `/desktop/` product entry.

## Local Commands

AIRP does not require a particular drive for Node.js, npm, Rust, or their caches. Ensure `npm` and `cargo` are available on `PATH`; if you customize their homes or caches, use paths appropriate for your machine. The maintainer-only D-drive override is documented in [`AGENTS.md`](../AGENTS.md).

```powershell
npm run dev
npm run typecheck
npm run test
npm run build
npm run smoke:shell
npm run smoke:runtime
npm run build:engine-sidecar
npm run tauri dev
```

Tauri Rust tests run from the repository root:

```powershell
cargo test -p airp-ui
```

## Runtime Notes

- Engine URL defaults to `http://127.0.0.1:8765`.
- Override with `AIRP_ENGINE_URL`.
- Historical baseline: the original AIRP-State-Protocol packaged `.exe` was verified to launch and support simple interaction, but it was not deeply tested and is not current release evidence.
- The current trusted-local WebUI import path is path-first. Future desktop Widget import must use an Engine-authorized/native file-selection boundary and must not put base64 card blobs into Blueprint props or long-lived Vue state.
- The current WebUI is the supported product surface. The restored Tauri/Vue desktop is a target under #564; it becomes the default only after real Engine vertical slices, package smoke, GUI evidence, the observation window, and Owner approval.
- Agent UI Test Harness is dev/test-only. Enable with `?airp_agent_test=1`, `localStorage.AIRP_AGENT_TEST=1`, or `VITE_AIRP_AGENT_TEST=1`; then use `window.__AIRP_AGENT_TEST__` from Codex browser control or Playwright.
- Users who do not want any agent-control surface can delete `src/agent-test.ts` before building. `App.vue` loads the harness only when the module exists, and the related test does not block the build when the module is absent.

## CI Artifacts

The root `.github/workflows/manual-build.yml` workflow can be run manually on a fork. It builds the Windows Tauri package and uploads `airp-ui-windows` with the desktop exe and NSIS setup.

## Open Items

- Desktop screenshot acceptance: `ui/smoke-desktop-screenshots.ps1` is a stub that only checks Tauri artifact presence; wiring the WebView2 harness screenshots against `webui/baseline-screenshots/` is a follow-up task (Task #12 stub).
- AIRP-Dev packaged GUI end-to-end verification; source-level engine integration is already present.
- Package/runtime smoke: build the desktop artifact, launch it, select/import a character, send one message, and receive a streamed reply with real settings.
- Agent UI Test Harness 已接入 Playwright runtime smoke，并由 PR gate 保存截图与 `runtime-evidence.json`；剩余范围是连接 Codex 浏览器控制和真实 Engine Surface 链路。
- Perf spike with 100k messages.
- Reasoning/action rendering.
