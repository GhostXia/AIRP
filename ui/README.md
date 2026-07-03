# AIRP UI

`ui/` 是 AIRP 的 Tauri + Vue 桌面客户端。它是 engine 的客户端，不再假设独立 Gateway 或 MockBus 作为默认后端。

UI 继承 AIRP-State-Protocol 的 Blueprint、Widget、patch、guard、虚拟滚动和沙箱资产，但不继承其通用协议优先定位。当前目标是 AIRP 专用桌面客户端；Widget 扩展必须先服务 RP 工作流。详见 [`../docs/UI-PROTOCOL-DECISION.md`](../docs/UI-PROTOCOL-DECISION.md)。

## 当前职责

- 渲染 Blueprint/widget UI。
- 通过 `TauriBus` 调用 Tauri command `airp_dispatch`。
- Tauri Rust 侧 `BusRelay` 直连 AIRP engine HTTP/SSE API。
- 将 engine SSE 输出转换为 State Protocol `state` patch，流式更新 `w-chat`。
- 通过 Tauri dialog 选择本地 PNG/JSON 角色卡路径，并发送 `characters.import` intent。

## 结构

```text
ui/
├── package.json
├── src/
│   ├── App.vue
│   ├── protocol/          # TS-side protocol mirror and TauriBus
│   ├── registry/          # widget registry, consent, sandbox bridge
│   ├── state/             # RFC6902 state store
│   └── widgets/           # first-party widgets
├── widgets/core/          # widget manifests
└── src-tauri/
    ├── Cargo.toml
    ├── capabilities/default.json
    └── src/
        ├── main.rs        # Tauri shell setup
        └── bus.rs         # live bridge to engine
```

The canonical Rust protocol crate lives in `../protocol`. The TypeScript types in `src/protocol/types.ts` intentionally mirror the subset used by the UI.

## Local Commands

Use D-drive npm cache/prefix in this workspace:

```powershell
$env:PATH = "D:\nodejs;" + $env:PATH
$env:npm_config_prefix = "D:\npm-global"
$env:npm_config_cache = "D:\npm-global\npm-cache"
```

```powershell
npm run dev
npm run typecheck
npm run test
npm run build
npm run tauri dev
```

Tauri Rust tests run from the repository root:

```powershell
cargo test -p airp-ui
```

## Runtime Notes

- Engine URL defaults to `http://127.0.0.1:8000`.
- Override with `AIRP_ENGINE_URL`.
- Historical baseline: the original AIRP-State-Protocol packaged `.exe` was verified to launch and support simple interaction, but it was not deeply tested.
- Character import is path-first: the UI sends only `card_path`; it must not put base64 card blobs into Vue state or widget props.
- Current chat streaming still uses `chat_lock` and `/messages/-/text`; Task 1.2 should move chat state to id-keyed messages and remove the lock.

## Open Items

- AIRP-Dev GUI end-to-end verification after packaging and engine integration.
- Perf spike with 100k messages.
- Reasoning/action rendering.
- Task 1.2 id-keyed chat message model.
