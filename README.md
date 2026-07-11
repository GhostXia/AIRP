# AIRP

AIRP 是一个专精 Role Play 的 AI Agent 客户端，当前仓库已经收敛为“两盒”结构：

- **engine**：无头 AIRP 引擎，crate `airp-core`。负责 RP 数据层、上下文装配、LLM 调用、流式输出、agent loop 骨架和 HTTP/SSE API。
- **ui**：Tauri + Vue 桌面客户端，crate `airp-ui`。负责 Blueprint/widget 渲染、状态 patch、角色列表、角色卡导入入口，以及通过 Tauri bridge 调用 engine。
- **protocol**：crate `airp-state-protocol`。提供 UI 与引擎之间共用的 Envelope/Blueprint/Widget/capability 等线协议类型。

旧的 `gateway` 和 `mcp-server` 已不再是本 workspace 成员。它们保留为独立仓库/零件来源：Gateway 的传输、安全、MCP client 能力按需吸收；MCP-Server 的数据管理面按 `M_AGENT-2` 路线融入 engine。

AIRP-State-Protocol 同样按零件来源处理：必须吸收 Blueprint、Widget、state patch、guard、虚拟滚动、consent/sandbox 等成熟资产，但不继承其"通用 Agent UI 标准优先"的产品定位。见 [docs/UI-PROTOCOL-DECISION.md](docs/UI-PROTOCOL-DECISION.md)。

AIRP-Core/AIRPCLI、AIRP-MCP-Server、AIRP-Gateway 与 AIRP-State-Protocol 均为本项目作者自己的前序项目。当前仓库汇聚并重构其中经过复核的资产，而不是把它们作为第三方依赖；四个源项目统一按"吸收资产，不继承产品北极星"处理，详见 [docs/SOURCE-PROJECT-DECISIONS.md](docs/SOURCE-PROJECT-DECISIONS.md)。

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

- engine 已具备单回合 SSE 对话、OpenAI/Anthropic adapter、角色/会话/状态/场景/基础世界书、卷系统、拆解/analysis 和 settings/models 等 API。PR #100 留有一次 WebUI → engine → 真实 DeepSeek 的成功流式证据。
- 默认 Agent 工具注册表当前为 19 个工具；`GET /v1/agent/tools` 提供排序后的运行时目录，`/v1/agent/run` 已用 OpenAI/Anthropic 原生 structured tool call 做动态决策，经 engine capability/allowlist/confirm 门执行 typed observation，并只由 finalizer 做最终纯净生成。
- UI `BusRelay` 已直连 engine，角色导入与 id-keyed chat 已实现；desktop 现在持有并在退出时终止 sidecar，Windows workflow 已加入安装→启动→ready→退出 smoke，等待 CI artifact 实跑证据。
- 世界书已有 CRUD、确定性关键词触发与 v1 语义合同；StateService 在写入时强制 schema、revision 与串行边界。SillyTavern 高级世界书语义和稳定跨设备身份仍未完成。
- WebUI 仅作为后端可靠性和开发诊断面，不替代 Tauri/Vue 长期产品 UI。PR #106 已把 V2 运行态落到 `webui/`；provider/endpoint/model/runtime key 可编辑，应用后通过真实 `/v1/models` 请求验证，带真实凭据的远端运行证据仍需手动执行。
- 2026-07-11 本地 workspace tests、UI tests/typecheck、Rust fmt 与 `-D warnings` Clippy 均通过；仓库已有自动 PR gate。

当前权威状态、独立发现和路线排序见 [docs/PROJECT-AUDIT-2026-07-10.md](docs/PROJECT-AUDIT-2026-07-10.md)。实施入口见 [docs/DEV-GUIDE.md](docs/DEV-GUIDE.md)，长期原则见 [docs/PLAN.md](docs/PLAN.md)。

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

本仓有手动 GitHub Actions 打包 workflow，但它不是 PR gate。本地测试和人工 review 仍是主要门禁。`subagent_context_has_no_orchestrator_noise` 是干净提示词不变式，不得删除或削弱。

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

## 手动 CI 构建

Fork 后可在 GitHub Actions 里运行 **Manual build** workflow。它会在 Windows runner 上执行 Rust/UI 测试、打包 Tauri 桌面端，并上传 artifacts：

- `airp-ui.exe`
- `AIRP UI_0.1.0_x64-setup.exe`

当前 workflow 先覆盖 Windows，因为现有 sidecar 和 bundle 目标是 Windows 桌面包；macOS/Linux 需要补对应 sidecar 命名和 Tauri bundle 目标后再纳入 matrix。

## 关键文档

- [docs/DEV-GUIDE.md](docs/DEV-GUIDE.md)：当前开发交接与工程纪律
- [docs/PLAN.md](docs/PLAN.md)：长期设计计划
- [docs/WEBUI-BACKEND-VALIDATION.md](docs/WEBUI-BACKEND-VALIDATION.md)：临时 WebUI 后端可靠性验证路线
- [docs/SOURCE-PROJECT-DECISIONS.md](docs/SOURCE-PROJECT-DECISIONS.md)：四个源项目的资产吸收/北极星降级决策
- [docs/UI-PROTOCOL-DECISION.md](docs/UI-PROTOCOL-DECISION.md)：UI 协议与 Widget 的采纳/降级决策
- [docs/PARTS.md](docs/PARTS.md)：旧仓能力拆件清单
- [docs/MCP-SERVER-ABSORPTION.md](docs/MCP-SERVER-ABSORPTION.md)：MCP-Server 能力融入 engine 路线
- [docs/RISK-REGISTER.md](docs/RISK-REGISTER.md)：已知风险登记
- [docs/PROJECT-AUDIT-2026-07-10.md](docs/PROJECT-AUDIT-2026-07-10.md)：当前独立审计、风险和近期优先级
- [docs/DOC-AUDIT.md](docs/DOC-AUDIT.md)：文档权威层级与维护规则
- [docs/ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)：项目沿革、第三方设计参考与许可证边界（持续更新）

## Agent UI 测试面与用户控制

`ui/src/agent-test.ts` 是临时开发/测试面，用于自动化 UI 验证。它只在 dev/test 且显式开启时暴露 `window.__AIRP_AGENT_TEST__`，让 Codex 浏览器控制或 Playwright 这类测试 runner 能驱动 UI 并读取状态快照。

它不是普通用户功能，也不是 RP 使用所必需。默认关闭，且应只用于测试。想完全移除 agent 控制面的用户，只需要在 fork 后、手动构建前删除这个运行时模块：

```powershell
Remove-Item ui\src\agent-test.ts
```

删除后再运行 **Manual build**。`ui/src/App.vue` 只在文件存在时加载该模块，相关单测在模块不存在时不会阻断构建，因此不需要再改别的源码；构建出的 artifact 不会包含该测试面。

## 设计参考与致谢

AIRP 感谢角色扮演与 Agent 开源生态中公开分享设计和实现经验的项目。当前明确研究过的第三方项目包括 [SillyTavern](https://github.com/SillyTavern/SillyTavern)、[Hermes Agent](https://github.com/NousResearch/hermes-agent)、[NeuroBook](https://github.com/notnotype/neuro-book)、[pi-forge](https://github.com/MacroSony/pi-forge) 和 [llmlint](https://github.com/notnotype/llmlint)。

列入此处表示设计、产品或互操作性研究，不表示双方存在从属、合作或背书关系，也不自动表示复用了对方的代码、规则、数据或视觉资产。任何实际第三方资产复用都必须另行记录来源、版本、修改和许可证义务。详细分类及核验状态见 [docs/ACKNOWLEDGEMENTS.md](docs/ACKNOWLEDGEMENTS.md)。该清单是**待持续更新的活文档**，后续研究或采用新的项目时应同步补充。

## License

MIT OR Apache-2.0.
