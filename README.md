# AIRP

AIRP 是一个专精 Role Play 的 AI Agent 客户端，当前仓库已经收敛为“两盒”结构：

- **engine**：无头 AIRP 引擎，crate `airp-core`。负责 RP 数据层、上下文装配、LLM 调用、流式输出、agent loop 骨架和 HTTP/SSE API。
- **ui**：Tauri + Vue 桌面客户端，crate `airp-ui`。负责 Blueprint/widget 渲染、状态 patch、角色列表、角色卡导入入口，以及通过 Tauri bridge 调用 engine。
- **protocol**：crate `airp-state-protocol`。提供 UI 与引擎之间共用的 Envelope/Blueprint/Widget/capability 等线协议类型。

旧的 `gateway` 和 `mcp-server` 已不再是本 workspace 成员。它们保留为独立仓库/零件来源：Gateway 的传输、安全、MCP client 能力按需吸收；MCP-Server 的数据管理面按 `M_AGENT-2` 路线融入 engine。

AIRP-State-Protocol 同样按零件来源处理：必须吸收 Blueprint、Widget、state patch、guard、虚拟滚动、consent/sandbox 等成熟资产，但不继承其"通用 Agent UI 标准优先"的产品定位。见 [docs/UI-PROTOCOL-DECISION.md](docs/UI-PROTOCOL-DECISION.md)。

四个源项目统一按"吸收资产，不继承产品北极星"处理，详见 [docs/SOURCE-PROJECT-DECISIONS.md](docs/SOURCE-PROJECT-DECISIONS.md)。

## 项目取向

代码应当更开放、更透明、在未来更易修正、且更易迭代更新。具体含义：接口和扩展点清晰开放；状态、决策和错误可观察；模块边界低耦合、可替换；协议和数据结构版本化、小步演进。

## 目录

```text
D:\AIRP-Dev/
├── Cargo.toml              # workspace: engine, protocol, ui/src-tauri
├── Cargo.lock
├── engine/                 # airp-core: RP engine / HTTP daemon / agent loop
├── protocol/               # airp-state-protocol: Rust wire types + validator CLI
├── ui/                     # Tauri + Vue desktop UI
├── data/                   # runtime/sample RP data; personal card/session dirs ignored
├── docs/                   # design, development guide, risk register
└── AGENTS.md               # local toolchain and audit instructions
```

## 当前状态

- Phase 0 已完成：UI `BusRelay` 直连 engine `/v1/chat/completions`，可流式回填聊天。
- Phase 1 Task 1.1 已实现：UI 通过 Tauri dialog path-first 导入角色卡，engine 读盘解析并落库。
- Phase 1 Task 1.2 已完成：chat 状态已改为 id-keyed `{messages, order}`，`BusRelay` 不再依赖 `chat_lock`，每次 `chat.send` 用单个 patch envelope 原子创建 user/assistant 两行。
- 2026-07-03 审计 follow-up 已完成：Tauri 构建脚本、默认 settings、sandbox `postMessage` 目标、RFC6902 `test` 预校验、仓库 metadata 均已同步修正。
- 当前未解决/待验收事项见 [docs/DOC-AUDIT.md](docs/DOC-AUDIT.md) 和 [docs/DEV-GUIDE.md](docs/DEV-GUIDE.md)。

## 本地环境

本 Windows 工作区要求 Rust、Cargo、Node、npm 全局与缓存都走 `D:`，不要写到 `C:`。

PowerShell 运行 Rust 命令前设置：

```powershell
$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
```

npm 命令额外设置缓存到 D 盘：

```powershell
$env:npm_config_prefix = "D:\npm-global"
$env:npm_config_cache = "D:\npm-global\npm-cache"
```

## 验证

```powershell
# engine
cargo test -p airp-core
cargo test -p airp-core subagent_context_has_no_orchestrator_noise

# Tauri Rust shell
cargo test -p airp-ui

# frontend
cd ui
npm run typecheck
npm run test
```

本仓当前没有项目级 `.github` CI；本地测试和人工 review 是主要门禁。`subagent_context_has_no_orchestrator_noise` 是干净提示词不变式，不得删除或削弱。

## 运行

启动 engine：

```powershell
cargo run -p airp-core -- daemon --port 8000
```

启动前端开发服务器：

```powershell
cd ui
npm run dev
```

完整 Tauri 桌面应用：

```powershell
cd ui
npm run tauri dev
```

UI 默认连接 `http://127.0.0.1:8000`，可用 `AIRP_ENGINE_URL` 覆盖。

## 关键文档

- [docs/DEV-GUIDE.md](docs/DEV-GUIDE.md)：当前开发交接与工程纪律
- [docs/PLAN.md](docs/PLAN.md)：长期设计计划
- [docs/SOURCE-PROJECT-DECISIONS.md](docs/SOURCE-PROJECT-DECISIONS.md)：四个源项目的资产吸收/北极星降级决策
- [docs/UI-PROTOCOL-DECISION.md](docs/UI-PROTOCOL-DECISION.md)：UI 协议与 Widget 的采纳/降级决策
- [docs/PARTS.md](docs/PARTS.md)：旧仓能力拆件清单
- [docs/MCP-SERVER-ABSORPTION.md](docs/MCP-SERVER-ABSORPTION.md)：MCP-Server 能力融入 engine 路线
- [docs/RISK-REGISTER.md](docs/RISK-REGISTER.md)：已知风险登记
- [docs/DOC-AUDIT.md](docs/DOC-AUDIT.md)：文档审计后的待确认项

## License

MIT OR Apache-2.0.
