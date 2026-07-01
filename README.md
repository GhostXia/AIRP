# AIRP

> **A**I **R**oleplay **P**latform — 一个整合仓库，把 AIRP 生态的四个独立模块收拢进单一 Cargo workspace，统一版本、统一依赖、统一构建。

AIRP 是一套面向 AI 角色扮演（RP）的乐高式平台。每个模块只做一件事，通过明确定义的协议契约彼此解耦。本仓库把它们合并到一个 monorepo 里，方便整体构建、联调与发布，同时保留各模块原本可独立使用的特性。

## 架构

```text
┌────────────┐   State Protocol   ┌─────────────┐   MCP (stdio/HTTP)   ┌────────────────┐
│  airp-ui   │ ◄──────────────► │ airp-gateway │ ◄──────────────────► │ airp-mcp-server │
│ (Tauri+Vue)│   Envelope/SSE    │  (协议桥)    │                      │  (数据工具箱)   │
└────────────┘                   └──────┬──────┘                      └────────┬───────┘
                                        │ 嵌入                                │
                                        ▼                                     ▼
                                 ┌─────────────┐                     上游 LLM / Agent
                                 │  airp-core  │ ◄──── 自调 LLM 的流式 RP 后端
                                 │  (Agent 端) │
                                 └─────────────┘
```

| 模块 | 路径 | 角色 | 技术栈 |
|------|------|------|--------|
| **airp-state-protocol** | `state-protocol` | 协议契约：Envelope wire 类型 + AgentBus trait。UI 与 Gateway 之间的合同。 | Rust lib + 验证 CLI |
| **airp-gateway** | `gateway` | 通用协议桥：前端 HTTP/SSE ↔ MCP 服务器。可选 `agentbus` feature 暴露 State-Protocol 接口。 | Rust lib |
| **airp-mcp-server** | `mcp-server` | 纯咨询式 MCP 服务器：角色卡 / 世界书 / 预设 / 会话的数据工具，不调用 AI。 | Rust bin (`airp-mcp`) |
| **airp-core** | `core` | Agent 后端：自调 LLM 的流式 RP 守护进程，注入上下文 + FSM 过滤 + XML 解包。 | Rust bin (`airp-core`) |
| **airp-ui** | `ui/` | 桌面 shell：Tauri + Vue 渲染 State Protocol Blueprint，开放 Widget Registry。 | TS/Vue + Rust/Tauri |

## 目录结构

```text
AIRP/
├── Cargo.toml              # workspace 根，统一依赖版本
├── Cargo.lock              # workspace 级锁文件
├── 
│   ├── state-protocol/     # airp-state-protocol (lib + airp-protocol bin)
│   ├── gateway/            # airp-gateway (lib, 可选 agentbus feature)
│   ├── mcp-server/         # airp-mcp-server (lib + airp-mcp bin)
│   └── core/               # airp-core (lib + airp-core bin)
└── ui/
    ├── src/                # Vue 前端 (protocol/registry/state/widgets)
    ├── src-tauri/          # Tauri Rust shell (airp-ui bin)
    ├── widgets/            # 内置 widget 清单 (card/chat/emotion/...)
    ├── package.json
    └── vite.config.ts
```

## 依赖关系

- `airp-state-protocol`：最底层，无内部依赖。
- `airp-gateway`：不依赖其他内部 crate；其 `agentbus` adapter 自带 Envelope 类型（与 state-protocol 镜像）。
- `airp-mcp-server`：独立，无内部依赖。
- `airp-core`：独立，无内部依赖（各模块原设计即解耦）。
- `airp-ui`（Tauri shell）：`airp-state-protocol = { workspace = true }`，通过 path 依赖复用同一套 wire 类型。

所有内部 path 依赖都通过 workspace 的 `[workspace.dependencies]` 统一声明。

## 构建

### 前置要求

- Rust toolchain（`rustc 1.96+`，`mcp-server` 用 `edition 2024`）
- Node.js 18+ 与 npm（仅 UI 前端需要）
- [Tauri 2 系统依赖](https://tauri.app/start/prerequisites/)（仅构建桌面 app 时）

### Rust 全部 crate

```bash
# 在仓库根目录
cargo check --workspace          # 类型检查全部 5 个 crate
cargo build --workspace --release  # 发布构建
cargo test --workspace            # 跑全部测试

# 单个 crate
cargo run -p airp-core            # 启动 Agent 后端守护进程
cargo run -p airp-mcp             # 启动 MCP 服务器
cargo run -p airp-protocol        # 校验一个 Envelope JSON 文件
```

### UI 桌面应用

```bash
cd ui
npm install
npm run dev        # 仅前端 dev server
npm run tauri dev  # 完整 Tauri 桌面应用（需系统依赖）
npm run build      # 前端生产构建
```

## 整合说明

本仓库合并了以下四个原本独立的仓库（保留各自完整历史于各自原仓库）：

| 原仓库 | 整合位置 |
|--------|----------|
| [AIRP-State-Protocol](https://github.com/GhostXia/AIRP-State-Protocol) | `state-protocol/` + `ui/` |
| [AIRP-Gateway](https://github.com/GhostXia/AIRP-Gateway) | `gateway/` |
| [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server) | `mcp-server/` |
| [AIRP-Core](https://github.com/GhostXia/AIRP-Core) | `core/` |

整合时所做的改动：
- 建立顶层 `Cargo.toml` workspace，5 个 crate 作为成员。
- 各 crate 的 `Cargo.toml` 改用 `workspace = true` 继承统一版本/作者/仓库地址。
- 共享依赖版本上提到 `[workspace.dependencies]`，减少重复声明。
- `ui/src-tauri` 对 `airp-state-protocol` 的 path 依赖改为 workspace 内引用。
- 统一 `Cargo.lock`（workspace 级），删除各 crate 各自的 lock 文件。
- `target/` 目录统一到仓库根，避免重复编译。

各模块的详细设计文档见其原仓库 README 及 `docs/` 目录。

## License

MIT OR Apache-2.0（双授权，与各源仓库一致）。
