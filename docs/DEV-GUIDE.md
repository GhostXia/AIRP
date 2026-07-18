# AIRP 开发交接指南

> 读者：冷启动、没有聊天上下文的实现或审计 Agent
>
> 最后校准：2026-07-18，`main@2a14b7e`
>
> 真理顺序：源码/manifest/测试/可重复证据 > [CURRENT-BASELINE.md](CURRENT-BASELINE.md) > 专题合同 > 长期计划 > 历史归档/聊天。

## 1. 开始前

1. 读取根目录 `AGENTS.md`，遵守本机工具链、独立审计和 PR 门禁规则。
2. 运行 `git status --short --branch`，保留用户已有改动；不要把 ignored runtime data 当成仓库文档或清理目标。
3. 阅读 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 和与任务直接相关的合同；用 `gh issue view <id>` 复核实时范围。
4. 对任何“已实现”声明，先找到当前源码入口和测试；旧计划、issue 标题或 PR 描述不是运行时证据。
5. 代码任务使用 `codex/` 分支和 PR；本地全绿只允许开 PR，不允许绕过审计 bot 或人工 review 合并。

当前主线是 WebUI 单实例、自托管、单用户 P1 有限试用验证。每项产品能力优先纵向贯通：

```text
engine shared service → HTTP/SSE → WebUI → production/browser tests
```

桌面 Tauri/Vue 代码保留，但开发、打包和性能计划暂停。恢复前先重新校准基线。

## 2. 仓库地图

```text
<repo>/
├── engine/                 airp-core：domain、存储、prompt、LLM、Agent、daemon
├── protocol/               airp-state-protocol：共享线协议与 validator
├── ui/                     Vue + Tauri 桌面客户端资产
│   └── src-tauri/          airp-ui Rust shell / sidecar bridge
├── webui/                  当前产品 WebUI
├── deploy/production/      P0 OCI/Compose/Caddy preview 拓扑
├── data/                   运行时数据根规范与安全样例
├── docs/                   活文档、专题合同、研究资料和 archive
└── .github/workflows/      PR gate 与手动 Windows build
```

Rust workspace 成员只有 `engine`、`protocol`、`ui/src-tauri`。AIRP-MCP-Server、AIRP-Gateway 等前序仓库不在 workspace；吸收规则见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)。

### Engine 主要边界

- `domain.rs` 与各 shared service：数据和业务不变量；
- `data_dir/`：路径、原子替换与数据根；
- `chat_store.rs`：durable history 与 session metadata；
- `chat_pipeline.rs` / `orchestrator/`：RP prompt 装配；
- `agent/`：bounded loop、registry、工具和控制平面；
- `daemon/handlers/`：HTTP adapter，handler 不应重新实现 domain 规则；
- `daemon/tests/`：route 合同与安全测试。

新增能力先进入 shared service，再由 HTTP、Agent tool 和 UI 暴露。不要把“底层有函数”混写成“HTTP 可用”“Agent 可调用”或“UI 已交付”。

## 3. 强制不变式

### 3.1 干净提示词

RP 角色平面只包含角色卡、世界书、Preset、Persona、state、记忆和历史。工具定义/调用/结果、规划和编排元数据走 provider 原生结构化控制平面；禁止 in-prompt ReAct 或把 orchestrator 指令拼进角色自然语言。

`subagent_context_has_no_orchestrator_noise` 是神圣门禁，不得删除、改弱或用白名单掩盖污染。

### 3.2 有界与受控执行

- Agent loop 必须有 step、token、墙钟、取消和可观察事件；
- capability、allowlist、破坏性确认、幂等和并发写串行化由 engine 强制；
- UI consent 只是交互提示，不是安全授权真值；
- Agent/第三方生成的任意代码不得直接执行。

### 3.3 数据单一真相与版本化

- RP 数据只由 engine shared service 持久化；handler、UI 和 Agent tool 不各写一份；
- ID 在反序列化/路径边界立即校验；显示名不作目录身份；
- 写入使用原子替换、revision conflict 和明确迁移；失败不得留下半提交；
- session 当前/目标边界以 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md) 为准；未交付阶段不能写成 runtime 事实；
- 改 wire contract 时同步 schema、Rust/TS binding、fixture 和文档。

### 3.4 大对象与路径安全

- 大文件优先可信本地 path、multipart 或流式上传；base64 只作无二进制通道时的兜底；
- 大 blob 不进入模型上下文、reactive store、Blueprint、历史或日志；
- `card_path` 只允许可信本地桌面调用。Web/远端调用必须用受限 content upload，production mode 必须拒绝 local-path import；
- 所有输入有大小、类型、路径 containment 和资源上界。

### 3.5 性能

- 完整历史留在 engine，UI 使用 cursor/window；
- 稳定 durable ID 作 DOM/state key；
- patch 与流式增量更新优先，禁止每 token 重建整段历史或 markdown；
- 离屏组件、listener 和任务必须释放；
- WebUI window 不等于虚拟列表。10k/100k 与内存上界没有证据时不得宣称完成。

### 3.6 第三方研究与实现

只吸收理念、需求、公开行为、协议/格式和互操作性经验。AIRP 使用自己的 domain model、命名、控制流、安全边界和测试独立实现；不得复制、翻译或移植第三方代码、prompt、规则、测试、数据或视觉资产。研究和普通依赖 provenance 记录在 [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md)。

## 4. 当前数据合同

- `AIRP_DATA_DIR` > 开发模式 `cwd/data` > 打包程序 OS per-user `airp/`；
- 仓库只跟踪 `data/README.md`、`data/settings.json` 和 `data/styles/profiles/default.md`；
- 命名 session 目录 UUID 是 session/history/metadata 的唯一规范 ID；
- 当前已隔离命名 session 的 `history/` 与 `memory/`；完整 state、角色卡、worldbook 工作副本和 unified revision 尚未全部落地；
- 新数据根不得创建根级 `world.md`/`items.md`，新角色不得创建 legacy `worldbooks/`；已有用户数据只兼容读取，不自动删除；
- 运行时只应依赖明确 manifest/ID，不用递归扫描或文件名猜测替代合同。

数据布局细节见 [data/README.md](../data/README.md)，revision 与恢复合同见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)。

## 5. 安全与部署

- development 默认 loopback；production 必须使用 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md) 的 fail-closed 配置；
- Windows 便携 WebUI 只允许 `127.0.0.1` 同源运行；启动器必须把 `AIRP_DATA_DIR` 固定到包内 `data/`，把 `config.json` 固定到包根，并清除继承的 deployment/access/CORS 环境变量、显式禁用 `AIRP_ALLOW_LOCAL_PATH`；
- provider key 和 engine bearer 不持久化到普通 settings、前端、日志或诊断；
- 浏览器只访问同源 gateway，不接收 engine 私有 URL/bearer；
- `webui/start.bat`、`serve.js` 和 `cargo run` 都是开发路径，不是生产部署；
- 当前 artifact 验收使用 `.github/workflows/webui-windows-build.yml`；Tauri manual build 是长期维护线，Docker/WSL 不是当前落地依赖；
- P0 topology 全绿不等于正式发布，P1–P3 见 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)。

安全改动同步 [SECURITY.md](SECURITY.md) 与 [RISK-REGISTER.md](RISK-REGISTER.md)。

## 6. 本地环境与命令

项目不限制工具链盘符。维护者本机因 `C:` 空间不足使用以下覆盖，其他贡献者不应照搬：

```powershell
$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:npm_config_prefix = "D:\npm-global"
$env:npm_config_cache = "D:\npm-global\npm-cache"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
Set-Location D:\AIRP-Dev
```

维护者 checkout 使用仓库本地 `D:\AIRP-Dev\target`。命令若试图在该机重新填充 `C:\Users\<user>\.cargo`、`.rustup` 或 npm cache，应停止并纠正环境。

### 最小验证矩阵

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
$env:RUSTDOCFLAGS = "-D warnings"
cargo doc --workspace --no-deps --locked
Remove-Item Env:RUSTDOCFLAGS
cargo test --workspace --locked
cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise --locked -- --nocapture

Push-Location ui
npm ci
npm run typecheck
npm run test -- --run
Pop-Location
```

Rustdoc 采用“合同正确性优先”策略：CI 要求公共文档能够生成，且不存在坏链接、
无效 HTML 或其他 rustdoc 警告。只有能解释公共 API 合同、错误语义、副作用、
并发或安全边界的注释才应补充；不按第三方工具的私有百分比给显而易见的字段和
私有 helper 批量填充注释。

需要盘点公共项缺失文档时，使用可复现的稳定工具链命令：

```powershell
$env:RUSTDOCFLAGS = "-W missing-docs"
cargo doc --workspace --no-deps --locked
Remove-Item Env:RUSTDOCFLAGS
```

该结果是维护清单，不是 CI 门禁；若未来启用 `missing_docs` 门禁，必须先明确稳定的
公共扩展面并一次性记录基线，不能让既有缺口阻止无关修复。

按范围补充：

- WebUI engine-truth smoke：`node webui/smoke.mjs`；
- production：`deploy/production/` 配置、build 与 topology smoke；
- desktop：`cargo test -p airp-ui`，恢复桌面发布时再跑 sidecar/build/installer smoke；
- docs-only：相对链接检查、`git diff --check`、术语/日期/PR 基线扫描。

不要把旧提交的测试数字复制为当前结果。记录命令、commit、通过/失败数量和未覆盖边界。

## 7. 工作流与审计

- 代码改动：`codex/<scope>` 分支 → 相关本地测试 → PR → 审计 bot → 修复全部阻塞意见 → 人工 review 决定是否合并；
- 审计必须独立判断，可提出不同设计并质疑历史；详细 charter 见根 `AGENTS.md`；
- 审计 bot pending、失败或有阻塞意见时不得合并；
- PR 合并后，审计中未修的意见去重、分类并写入 GitHub issue；合并前不得提前创建最终遗留清单；
- 只暂存明确文件，禁止 `git add .` / `git add -A`；
- 不提交真实角色卡、聊天、Persona、世界书、API key、日志、缓存或本地路径；
- 不因“顺手”扩大 task 范围。发现跨边界问题先记录证据，再由用户或 issue 决定。

### 7.1 依赖版本发现与升级审计

- manifest 使用有上界的兼容范围，lockfile 记录实际解析版本，CI 使用锁定安装；不要用无上界的 `>=` 让普通安装静默跨入未来主版本。
- 自动检测器只比较当前锁定版本与上游最新稳定版本，不直接修改主分支，也不自动采用 prerelease。相同依赖和目标版本必须去重，避免重复 issue。
- 新增普通第三方依赖或改变任何已解析版本（包括补丁版本）都必须核验上游来源、许可证、用途和分发义务；manifest 记录有界兼容范围，lockfile 固定实际版本与完整性，当前 provenance 同步到 [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md)。
- 补丁版本可由自动化创建 PR；PR 必须附上版本差异、上游 release/security 链接、依赖审计结果和按影响范围选择的门禁。自动 PR 仍受审计 bot 与人工 review 约束。
- 次版本或主版本变化（例如 `7.0 → 7.1`、`8 → 9`）必须先创建或更新 GitHub issue，审计 changelog、迁移/弃用、Node/Rust/平台下限、插件与运行时兼容性、许可证/provenance、安全公告、数据或构建产物变化，以及回滚路径；审计结论明确后再用独立 PR 升级。
- `0.x` 依赖的次版本按主版本风险处理。即使只是补丁版本，只要涉及数据格式、网络/权限边界、密码学、构建/发布链或已知行为变化，也升级为 issue + 专项审计。
- “发现有新版本”不是升级理由；升级 PR 要说明用户价值、风险、验证证据和不升级的后果。安全修复可提高优先级，但不得跳过兼容性验证。

仓库已落地 `tools/dep-governance/`（PR #218）：`discover-deps.mjs` 扫描 Cargo workspace 与 npm package-lock.json v3 并按 BFS 划分 runtime/build/dev 作用域，`audit-routing.mjs` 提供纯函数审计路由（`classifyInventory` auto-pass/audit-required/block + `classifyUpgrade` 五类升级路由 + patch-sensitive 覆盖），`generate-sbom.mjs` 输出 SPDX-2.3 / CycloneDX 1.5 SBOM 与人类可读第三方声明；当前 SBOM 快照存于 `docs/sbom/`。该工具链是手动运行的离线工具（无上游版本比较、不自动开 PR），自动化版本检测与去重 issue 仍是 #192 后续切片；在自动化落地前，开发任务涉及依赖时仍需人工查询上游稳定版本和安全公告，不得把本节误述为 CI 强制门禁。

## 8. 文档维护

- 能力变化同步 `CURRENT-BASELINE.md`；执行顺序变化同步 `WEBUI-PRODUCTION-PLAN.md`；
- 稳定合同写进已有专题文档，不为每个 PR 新建永久 Markdown；
- “已实现”必须区分 domain/data、HTTP、Agent tool、WebUI、desktop 和 production evidence；
- 研究/candidate 文档必须有状态声明，不得混入当前 capability inventory；
- 已完成计划先把仍有效合同吸收进活文档，再压缩到 `docs/archive/`；
- 实时未完成工作只留 GitHub issues，文档只保留分组和稳定依赖关系。

## 9. 当前接手点

1. PR #232 已形成 P1 有限试用代码候选；当前第一优先级是继续开发首聊黄金路径，并用真实 provider、真实浏览器和生产拓扑建立可重复验收，分别覆盖页面刷新恢复与服务重启恢复；
2. 优先修复首聊阻断、永久 loading、不可行动错误、secret 泄露、虚假成功和关键资产静默损坏，同时继续补齐直接影响可用版本的产品缺口；自动化、agent 辅助检查和维护者人工验收都可以形成工程证据，但不得把局部测试冒充端到端通过；
3. #114 Persona/Preset 高级生命周期、[SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md) 完整 session/revision/恢复分期和 #220 deferred 性能/重构项原则上进入 P2；若其中某项直接决定 P1 可用性、数据安全或可重复验收，可以按独立证据提前；
4. P2 运维与恢复；
5. P3 release candidate；
6. 工程治理后续切片（#192 自动版本检测与去重 issue、release pipeline 强制 SBOM 度量、发布签名）按 #192 推进，不抢占产品主线。

动手前必须重新查询 issues 和 `main`，不要把本节当成永久队列。
