# AIRP-State-Protocol

> **通用优先，不绑定任何项目。** 本项目始终以「通用」为核心理念——不捆绑任何特定项目或厂商，任何第三方都能便捷接入、自由用于任何用途。

**AIRP 生态的 UI 项目**（Tauri + Vue 显示层），同时承载它所渲染的**状态/渲染协议**。UI 通过一个**开放的 Widget Registry** 渲染由 Agent 产出的声明式 **Blueprint**，让 Agent 能驱动一个**长期存在、可扩展、低 Token** 的界面。

> 一仓两面：① Tauri+Vue 应用（显示层）；② 它与 AgentBus（如 AIRP-Gateway）之间，与具体 Agent 无关的 State Protocol 契约（Schema + Rust/TS 绑定 + widget manifest）。

## 乐高，不是套件

每一块都能**单独使用**，不强制你引入其余部分：

- **只用协议契约**：拿 `schema/` + `bindings/`（Rust 或 TS 类型）在你自己的项目里描述/校验 UI 状态与 Blueprint。不需要本 UI，也不需要 Gateway / MCP。
- **只用 UI**：Tauri + Vue 应用配**任意** `AgentBus` 实现——内置 `MockBus` 无任何后端即可跑；接你自己的后端只需实现 `dispatch` + `subscribe`，不绑定 AIRP-Gateway。
- **只用 / 自写一个 widget**：按 `mount` 接口（或 manifest）做一个 widget，挂到任何使用本协议的 UI 上，不需要其余组件。

> 下面的「生态」只是把这些（再加 AIRP 的后端）**拼满时**的样子——是**可选组合，不是运行前置**。任何一层都可单独使用、可换任意实现。

## 生态定位（可选的完整组合）

```
AIRP-State-Protocol  (本仓库, Tauri + Vue UI + 协议契约)  ← Widget Registry 渲染 Blueprint
   ↕  State Protocol (Tauri IPC / HTTP / SSE / WS)
AIRP-Gateway         (AgentBus 实现)                      ← 热：会话缓存 + patch 差分
   ↕  MCP
AIRP-MCP             (工具与数据)                          ← 冷：落盘真相
   ↕
Agent Runtime        (推理)
```

| 仓库 | 角色 |
|------|------|
| [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server) | 角色扮演数据管理（纯 MCP，不推理） |
| [AIRP-Gateway](https://github.com/GhostXia/AIRP-Gateway) | 协议桥 / AgentBus 默认实现 |
| **AIRP-State-Protocol**（本仓库） | UI 应用（Tauri+Vue）+ State Protocol 契约（Schema + Rust/TS 绑定 + 规范） |

## 仓库结构

```
# —— UI 应用（Tauri + Vue）——
index.html  vite.config.ts  tsconfig.json  package.json
src/
  main.ts  App.vue                         # 挂载；订阅 AgentBus，分发 Blueprint/state；启动恢复同意
  protocol/types.ts                        # 复用 bindings/typescript 的协议类型
  protocol/bus.ts                          # AgentBus 接口 + MockBus（无后端可跑）
  protocol/tauri-bus.ts  bus-factory.ts    # TauriBus（Tauri IPC）+ 按环境选 bus 的工厂
  state/store.ts                           # 按 scope 的响应式状态 + RFC6902 patch
  registry/                                # Widget Registry：type → 组件加载器；consent 同意闸门 + sandbox 桥
  widgets/                                 # 第一方 widget 组件（ChatWidget/EmotionWidget）
  components/                              # BlueprintRenderer + WidgetHost
src-tauri/                                 # Tauri 桌面壳（Rust）：airp_dispatch/airp:envelope 桥；tauri-build 产 .exe

# —— State Protocol 契约 ——
schema/airp-state-protocol.schema.json     # 真相：JSON Schema (draft 2020-12)
schema/widget-manifest.schema.json         # widget manifest 校验入口（开放扩展契约）
bindings/rust/                             # Rust 类型 + AgentBus trait（给 Gateway）
bindings/typescript/                       # TS 类型 + 类型守卫（UI 复用）
widgets/core/                              # 第一方 widget manifest（chat/memory/emotion/...）
docs/spec/protocol.md                      # 规范 v1
docs/extension-points.md                   # 扩展点/自定义点面板（所有可替换接口集中列出）
docs/widget-authoring.md                   # widget 作者指南（第三方接入）
docs/AIRP-架构与状态协议-背景整理.md         # 决策与性能契约背景
examples/                                  # 可被 schema 校验的 Envelope 示例
CONTRIBUTING.md                            # 贡献指南（含「如何加 widget」）
.github/workflows/ci.yml                   # 云端验证（rust/ts/schema/ui 均在 CI 跑）
```

## 核心概念（速览）

- **Envelope**：线上每条消息的信封（`v/id/ts/src/body`）。
- **Body**：按 `kind` 判别的消息——下行 `blueprint`/`state`/`manifest`/`event`/`error`，上行 `intent`/`subscribe`/`unsubscribe`/`hello`/`ack`。
- **Blueprint**：由 RP 推导的声明式界面（layout + widgets + theme）。**UI 只渲染，不生成。**
- **State/Patch**：状态以 RFC 6902 JSON Patch 增量同步，降低 Token。
- **Capability**：widget/agent 声明、Gateway 强制的权限。
- **AgentBus**：进程内 trait 契约；任何实现者可替换默认 Gateway。
- **开放 Widget**：widget 系统对任何第三方开放——用自己的命名空间（`namespace.name`）发 manifest 即可接入，无需改协议核心。组件**不限框架**（`mount` 接口，任意技术实现）。见 [widget 作者指南](docs/widget-authoring.md)、[CONTRIBUTING](CONTRIBUTING.md)、责任边界 [SECURITY.md](docs/SECURITY.md)。
- **同意闸门 + 沙箱**（宿主自守）：第三方 esm widget 须用户**授权后才加载**，且仅下发其声明的 capability；授权绑定 `{type, version, source}` 身份（换源/升版需重授权），并持久化到 `localStorage` 跨 reload 免重授。manifest 可声明 `entry.sandbox: true` 走 **opaque-origin iframe** 隔离（`allow-scripts` 无 `allow-same-origin`，碰不到宿主 DOM/全局/同源资源；`WidgetContext` 经 `postMessage` 桥接）。我们不审核插件代码——接入与否是用户的选择（[SECURITY.md](docs/SECURITY.md)）。

详见 [规范](docs/spec/protocol.md)。**所有可自定义/可替换的接口面（写 widget、接自己的后端、覆盖 esm 导入、沙箱、同意后端、自定义界面、只用协议契约…）集中在 [扩展点面板](docs/extension-points.md)。**

## 关键约束

- **路线 2（Blueprint，非 Agent 写代码）**：Agent 产出声明式数据，UI 渲染。安全 + 稳定 + 低 Token。
- **性能契约**：性能问题是「有界 vs 无界」，不是「算力多寡」。强制虚拟滚动、历史窗口分页、patch 优先、稳定 ID、重活进 Rust。详见背景文档 §6。
- **传输无关**：协议不绑定任何传输；不绑定 Claude Artifacts。

## 验证

编译与验证全部在 CI（`.github/workflows/ci.yml`）执行：

- `rust`：`cargo build` + `cargo test`（`bindings/rust` 协议绑定，Linux runner）。
- `tauri`：`cargo build` + `cargo test`（`src-tauri` 桌面壳含 `airp_dispatch`/`airp:envelope` 桥，含 WebKit/GTK 系统依赖 + 前端 `npm run build` 产 `dist`）。
- `typescript`：`tsc --noEmit`。
- `schema`：`ajv` 用 schema 校验 `examples/*.json` 与 `widgets/**/*.json`。
- `ui`：`vue-tsc` + `vite build` + `vitest`（含 `TauriBus` / bus 工厂 / store patch / registry / standalone / **边界 runtime guard** / **协议闭环 e2e smoke** 用例）。

> 本地不需安装工具链。UI 桌面壳的 `airp_dispatch` command + `airp:envelope` 事件桥已落地（`src-tauri/src/bus.rs`），当前 relay 是 mock，真连 AIRP-Gateway 为运行时验证项。手动 `tauri-build.yml` 产 Windows `.exe`。下行 Envelope 在进 registry/store 前经 `src/protocol/guard.ts` 结构骨架校验（非法回 `error` + 记 `busError`，fail-closed，不引 ajv）。

## 状态

`v0.1` 开发中。已定决策见背景文档 §5（前端框架 = **Vue**）；仍开放问题见 §7。**推进进度、下一步任务、工作规则见 [docs/PLAN.md](docs/PLAN.md)。**

## License

双协议授权，与 [AIRP-Gateway](https://github.com/GhostXia/AIRP-Gateway) 看齐：**MIT OR Apache-2.0**，使用者任选其一。

- [LICENSE-MIT](LICENSE-MIT)
- [LICENSE-APACHE](LICENSE-APACHE)
