# AIRP

AIRP 是专精 Role Play 的 AI Agent 客户端。项目采用“无头 Engine + 可换 UI”结构，把角色、Persona、Preset、Worldbook、会话、记忆、Agent 执行和安全边界集中在一个可审计的本地数据面。

> 当前基线：2026-07-30，`main@4f3f792`。
> 本页只提供产品入口；准确能力、缺口和验证边界见 [当前开发基线](docs/CURRENT-BASELINE.md)。

## 仓库结构

- `engine/`：`airp-core`，唯一 RP/Agent 业务内核与 HTTP/SSE 服务；
- `webui/`：当前正式产品交付主面；
- `airp-engine-console/`：WebUI 视觉与交互样板；
- `protocol/`：共享线协议；
- `ui/`：保留的 Tauri + Vue 桌面客户端，近期发布暂停；
- `deploy/windows-webui/`、`deploy/linux-webui/`：便携 WebUI artifacts；
- `deploy/production/`：单实例自托管 HTTPS preview；
- `data/`：运行时数据根规范与仓库安全样例；
- `tools/`：依赖治理、SBOM 和 Agent 浏览器探索工具。

AIRP-Core/AIRPCLI、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 是作者的第一方前序项目，不是当前 runtime 依赖。吸收决策见 [SOURCE-PROJECT-DECISIONS.md](docs/SOURCE-PROJECT-DECISIONS.md)。

## 当前状态

当前代码树已经提供：

- OpenAI-compatible、Anthropic、Ollama 与多 Provider 路由；
- 角色卡 JSON/PNG、Persona、Preset、场景、Worldbook、state、memory、revision；
- 命名 session、durable history、cursor、编辑/删除、branch、Swipe、continue/regen 与全文搜索；
- 有界 Agent loop、30 个内置工具、Director、Council、NPC、剧情弧、世界时钟和定时事件；
- 场景插图、角色模板、风格学习、对话示例、Worldbook 图谱、时间线导出、角色卡 diff；
- HTTP webhook/受控本地脚本插件工具，可动态加入 Agent registry；
- 44 个无构建 WebUI 屏、首次启动向导、聊天主面、资产管理与创作工具页面；
- Windows/Linux 便携 WebUI、production preview、SBOM 与 Agent 浏览器探索测试层。

这仍是 **P1 有限试用代码候选**，不是正式发布。功能存在不等于真实用户工作流、崩溃恢复、长会话、升级/回滚或市场验证已经通过。当前优先级是修复并发/持久化/失败边界，验证真实 provider 的 onboarding → 首聊 → 刷新 → 服务重启恢复，再扩大功能面。

## 核心原则

- RP 角色 prompt 只包含 RP 数据；工具、调度和审计留在结构化控制平面。
- Engine 是数据和业务规则的单一真相；UI、handler 与 Agent tool 不复制持久化规则。
- 内部架构可以迭代或重建，但用户资产必须可迁移、验证、导出和回滚。
- 只吸收第三方公开理念、需求、行为和互操作经验；AIRP 独立实现，不复制第三方代码、prompt、测试、数据或视觉资产。
- 本地测试全绿只允许开 PR；审计 bot 通过、阻塞意见修复并经人工 review 后才能合并。

完整规则见 [AGENTS.md](AGENTS.md) 和 [开发交接指南](docs/DEV-GUIDE.md)。

## 快速开始

### 开发模式

需要 Rust、Node.js 和 npm：

```powershell
cargo run -p airp-core -- daemon --open-browser --webui-dir webui
```

默认打开 `http://127.0.0.1:8765/`。首次运行进入 onboarding。开发模式只绑定 loopback；不要把 Engine 端口暴露给不可信网络或浏览器 origin。

### Windows 便携包

维护者运行：

```powershell
deploy/windows-webui/build.ps1
```

用户下载并解压 artifact 后双击 `Start-AIRP.cmd`。包内 `data/` 是用户资产根，升级或移动前必须备份并一并迁移，但普通资产备份/迁移包必须排除 `data/secrets.json`；该明文密钥文件只能通过加密且权限受限的渠道单独处理。便携包不需要用户安装 Rust、Node、Docker、WSL 或 Tauri。

### 本地验证

维护者本机的 D 盘工具链覆盖见 [AGENTS.md](AGENTS.md)；它不是项目通用要求。

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

发布、production topology、browser exploration 和 artifact 有各自独立门禁，不能由上述单元/集成测试外推。

## 文档

- [当前开发基线](docs/CURRENT-BASELINE.md)
- [开发交接指南](docs/DEV-GUIDE.md)
- [产品与架构计划](docs/PLAN.md)
- [安全边界](docs/SECURITY.md) / [风险登记](docs/RISK-REGISTER.md)
- [Session 与 revision 合同](docs/SESSION-DATA-DESIGN.md)
- [Worldbook 语义合同](docs/WORLDBOOK-SEMANTICS.md)
- [完整文档地图](docs/README.md)

## 许可证

MIT OR Apache-2.0。
