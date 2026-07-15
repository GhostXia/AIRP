# AIRP

AIRP 是一个专精 Role Play 的 AI Agent 客户端。产品采用“无头 engine + 可换 UI”结构：

- **engine**（`airp-core`）：RP 数据、prompt 装配、LLM adapter、Agent loop 与 HTTP/SSE；
- **webui**：当前正式产品交付主面；
- **ui**（`airp-ui`）：保留的 Tauri + Vue 桌面客户端，近期开发与打包验收暂停；
- **protocol**（`airp-state-protocol`）：UI/engine 共用的线协议类型。

当前权威实现基线是 `main@c54428e` / PR #177，详见 [当前开发基线](docs/CURRENT-BASELINE.md)。文档角色与最短阅读路径见 [文档地图](docs/README.md)。

## 项目原则

- RP 角色平面保持干净：工具、结果和编排脚手架走模型原生控制平面，不污染角色 prompt；
- Agent 执行有 step/token/墙钟/取消边界，并在 engine 强制 capability、allowlist 和破坏性确认；
- RP 数据由 engine shared service 统一管理，HTTP、Agent tool 和 UI 不各写一套；
- 大文件不驻留模型上下文、reactive store 或日志；服务端路径读取只允许可信本地调用；
- 扩展受控开放：结构化接口、capability、沙箱、用户同意与可审计变更；
- 代码和协议应开放、透明、易修正、易迭代。

AIRPCLI、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 原仓库是作者自己的第一方前序项目，仅作为资产来源。统一按“吸收资产，不继承产品北极星”处理，见 [源项目资产吸收决策](docs/SOURCE-PROJECT-DECISIONS.md)。第三方研究与独立实现边界见 [致谢与 provenance](docs/ACKNOWLEDGEMENTS.md)。

## 目录

```text
<repo>/
├── engine/                 airp-core
├── protocol/               airp-state-protocol
├── ui/                     Vue + Tauri desktop assets
│   └── src-tauri/          airp-ui
├── webui/                  current product WebUI
├── deploy/production/      self-hosted P0 preview bundle
├── data/                   runtime data-root contract and safe samples
├── docs/                   live docs, contracts, research and archive
└── .github/workflows/      PR gate and manual Windows build
```

Rust workspace 成员只有 `engine`、`protocol`、`ui/src-tauri`。旧 `gateway` / `mcp-server` 不在本 workspace，也不是 runtime 依赖。

## 当前状态

已交付的主要地基包括：

- OpenAI-compatible / Anthropic 流式对话和有界 structured tool-call Agent loop；
- 默认 21-tool registry、运行时 catalog 和 engine capability 门；
- 角色卡、命名 session、durable history、state、基础 worldbook、preset、scene、volume 与 analysis/decompose；
- Preset 规范化导入报告、原始输入 sidecar、版本目录与原子 current 指针，以及确认门控的 `get_preset` / `update_preset` Agent tools；
- 显式 `PromptAssemblyTrace` 数据模型骨架；调用方必须提供 provenance，不再从渲染文本反向猜测来源；
- 多 Persona 存储/HTTP/pipeline 以及 WebUI CRUD；
- worldbook v2 `constant` 语义与 v3 shared normalizer/导入诊断；
- WebUI 基础 RP 闭环、history window 与 rollback-by-ID；
- 单实例自托管 WebUI 的 production P0：同源 HTTPS、私有 engine、secret mounts、fail-closed 配置和真实 topology CI；
- 规范 session UUID、legacy metadata best-effort 修复，以及自包含 session/revision 的后续合同。

尚不能称正式发布。Persona/Preset/Worldbook 产品闭环、完整 session revision、migration、备份/恢复、可恢复删除、升级回滚、浏览器矩阵和长会话 soak 仍未完成。不要从本页推断细节；以 [CURRENT-BASELINE.md](docs/CURRENT-BASELINE.md) 为准。

## 开发环境

项目不限制 Rust、Node、npm、MSYS2、缓存或 target 的安装盘符。只需确保 `cargo`、`node` 和 `npm` 在当前 shell 的 `PATH` 中。

维护者本机因 `C:` 盘空间不足使用 `D:` 盘覆盖；该机器的完整环境变量记录在 [AGENTS.md](AGENTS.md)，不是项目级要求，也不应复制到其他贡献者环境。

## 本地运行

启动 engine：

```powershell
cargo run -p airp-core -- daemon --port 8000
```

启动 WebUI 开发环境：

```powershell
cd webui
node serve.js
```

Windows 可使用 `webui/start.bat` 启动本地开发依赖。上述路径均不是生产部署；不要把 8000 端口或静态开发服务器直接暴露到公网。

生产 P0 preview 的 prerequisites、bootstrap 和 TLS 模式见 [deploy/production/README.md](deploy/production/README.md)。

Tauri 桌面开发：

```powershell
cd ui
npm run tauri dev
```

桌面路线当前暂停；该命令只用于维护既有资产，不代表 packaged artifact 已通过发布验收。

## 验证

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise --locked -- --nocapture

cd ui
npm ci
npm run typecheck
npm run test -- --run
```

`.github/workflows/pr-gate.yml` 自动执行 Rust workspace、UI/WebUI 和 production topology 门禁。`.github/workflows/manual-build.yml` 负责手动 Windows desktop package。审计 bot 是合并前阻塞门禁：本地全绿只允许开 PR，必须等待审计通过并由人工 review 决定是否合并。

`main@c54428e` 的 [PR gate run 29408478974](https://github.com/GhostXia/AIRP/actions/runs/29408478974) 中 Rust workspace、UI and WebUI、Production topology、CodeRabbit 均通过。该结果只证明这个 commit，不自动证明后续改动。

## 关键文档

- [当前开发基线](docs/CURRENT-BASELINE.md)
- [开发交接指南](docs/DEV-GUIDE.md)
- [WebUI 正式上线计划](docs/WEBUI-PRODUCTION-PLAN.md)
- [产品与架构计划](docs/PLAN.md)
- [Session 存档与 revision 合同](docs/SESSION-DATA-DESIGN.md)
- [Worldbook 语义合同](docs/WORLDBOOK-SEMANTICS.md)
- [安全边界](docs/SECURITY.md) / [风险登记](docs/RISK-REGISTER.md)
- [完整文档地图](docs/README.md)

## License

MIT OR Apache-2.0.
