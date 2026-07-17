# AIRP

AIRP 是一个专精 Role Play 的 AI Agent 客户端。产品采用“无头 engine + 可换 UI”结构：

- **engine**（`airp-core`）：RP 数据、prompt 装配、LLM adapter、Agent loop 与 HTTP/SSE；
- **webui**：当前正式产品交付主面；
- **ui**（`airp-ui`）：保留的 Tauri + Vue 桌面客户端，近期开发与打包验收暂停；
- **protocol**（`airp-state-protocol`）：UI/engine 共用的线协议类型。

当前权威实现基线是 `main@15cb6c0` / PR #215，详见 [当前开发基线](docs/CURRENT-BASELINE.md)。文档角色与最短阅读路径见 [文档地图](docs/README.md)。

## 项目原则

- RP 角色平面保持干净：工具、结果和编排脚手架走模型原生控制平面，不污染角色 prompt；
- Agent 执行有 step/token/墙钟/取消边界，并在 engine 强制 capability、allowlist 和破坏性确认；
- RP 数据由 engine shared service 统一管理，HTTP、Agent tool 和 UI 不各写一套；
- 大文件不驻留模型上下文、reactive store 或日志；服务端路径读取只允许可信本地调用；
- 扩展受控开放：结构化接口、capability、沙箱、用户同意与可审计变更；
- 代码和协议应开放、透明、易修正、易迭代。

## 周期性代际升级

为避免 AIRP 被旧架构、旧工具链或旧一代 agent 的能力上限长期锁死，用户可以每半年或每年显式启动一次代际升级：

- 启动时必须通过官方一手信息核验并使用当时最新、正式发布、适合复杂软件工程的旗舰级大模型，覆盖主导规划、关键实现和独立审计；
- 允许破坏式重构，也明确允许从空白架构**推倒重建**，不因旧实现已经可运行而禁止重新设计；
- 若改动比例大到无法通过有界 PR 维持 `main` 可发布，必须建立独立 `remake/<cycle>` 分支或等价隔离产品线，与原项目并行发展；原项目在此期间继续获得必要维护、安全修复和数据导出支持；
- remake 不得凭开发者或 agent 自评取代原项目。启动前须定义市场判据和观察窗口，并以自愿试用/迁移、留存、核心任务成功率、稳定性、用户反馈与继续使用意愿等可复核证据判断；
- 只有市场证据持续表明 remake 整体优于原项目，并经用户明确批准，才允许按功能、用户和数据批次逐步替代。迁移、资产验证与回滚窗口完成前，不得彻底下线原项目；
- 推倒旧结构不等于破坏用户资产：不兼容变化仍必须具备版本化 migration、升级前备份、完整性验证、可读导出和可演练回滚，并继续通过安全、测试、许可证、PR 审计及人工 review 门禁。

本规则的 agent 执行细则见 [AGENTS.md](AGENTS.md) 的“周期性代际重构特例”。

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
- 真实 chat 装配驱动的 `PromptAssemblyTrace`、无写副作用脱敏 preview，以及 WebUI 本轮有效配置与有序装配摘要；
- 多 Persona 存储/HTTP/pipeline、WebUI CRUD、自动/显式选择、effective source 与角色/session 绑定闭环；
- worldbook v4 `constant` + `selective`/`secondary_keys` 运行时语义、presence-aware v3 迁移、shared normalizer/导入诊断、普通用户主面板编辑与 PNG/JSON 到最终 prompt 的端到端回归；
- WebUI 基础 RP 闭环、history window 与 rollback-by-ID；
- 单实例自托管 WebUI 的 production P0：同源 HTTPS、私有 engine、secret mounts、fail-closed 配置和真实 topology CI；
- 规范 session UUID、legacy metadata best-effort 修复，以及自包含 session/revision 的后续合同。

尚不能称正式发布。Persona 高级生命周期、Preset 完整生命周期、Worldbook 完整资产生命周期、完整 session revision、migration、备份/恢复、可恢复删除、升级回滚、浏览器矩阵和长会话 soak 仍未完成。不要从本页推断细节；以 [CURRENT-BASELINE.md](docs/CURRENT-BASELINE.md) 为准。

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

`.github/workflows/pr-gate.yml` 自动执行 Rust workspace、UI/WebUI 和 production topology 门禁。`.github/workflows/manual-build.yml` 负责手动 Windows desktop package。审计 bot 是合并前阻塞门禁：本地全绿只允许开 PR，必须等待审计通过并由人工 review 决定是否合并。

`main@15cb6c0` 的 [push gate run 29590129817](https://github.com/GhostXia/AIRP/actions/runs/29590129817) 已通过 Rust workspace（含 warning-free rustdoc 与干净提示词不变式 `subagent_context_has_no_orchestrator_noise`）、UI and WebUI、Production topology；覆盖 Phase 2h 6 类 revision 字段填充、`*_revision_unavailable` 诊断、orphan revision_dir 恢复与 onboarding wizard 22 项 L1/L2 + 43 项 L4 回归。证据只证明该 commit/PR head，不自动证明后续改动。

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
