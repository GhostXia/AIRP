# AIRP 全项目独立审计（2026-07-10）

> **历史快照**：本文固定记录 2026-07-10 的审计判断，后续 PR #111、#118、#119、#121、#123、#124、#125 已改变多项状态。不要把本文的“待实现”直接当成当前任务；当前事实、开放问题与唯一下一步见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。

> 审计基线：`main` / `33ba5b5`（PR #100 已合并），并额外审查尚未合并的 PR #106。
>
> **后续状态（2026-07-11）**：本文是带基线的历史审计，不应把其中的 open issue 列表或“待实现”段落当作实时清单。PR #111 已完成双 provider planner、共享 domain services、安全/CORS/CI 与 WebUI V2 修复；当前开发分支进一步完成 #25、#101、#102、#103，使默认 registry 达到 19 个工具并增加运行时 catalog。实时文档路由见 [DOC-AUDIT.md](DOC-AUDIT.md)。
> 审计原则：源码与可重复验证高于既有文档；历史审计不是前提，只作为线索。
> 本文是当前项目状态与近期优先级的权威快照。长期产品原则仍以 [PLAN.md](PLAN.md) 为准，实际实施入口以 [DEV-GUIDE.md](DEV-GUIDE.md) 为准。
> 合并后更新（2026-07-10）：PR #106 已于 `64003c1` 合并；下文保留原始审计基线，并在 A-08 与路线排序中标注合并后的真实状态。

## 1. 审计范围与方法

本轮覆盖：

- `engine/`、`protocol/`、`ui/`、`webui/` 的 manifests、主要运行路径、测试与构建配置；
- 2026-07-05 至 2026-07-10 的 PR #77–#100（含 UI settings/history、HTTP CRUD、WebUI workbench、decompose、issue sweep 与 smoke evidence）及未合并 PR #106；
- 当前全部 open issues：#25、#28、#29、#31、#32、#33、#35、#36、#37、#69、#70、#87、#97、#98、#99、#101、#102、#103、#104、#105，并回看与近期 PR 对应的 closed issues；
- 仓库内所有已跟踪 Markdown 文档。`docs/audits/` 和 `docs/issues/` 按历史证据审查，不把旧结论当当前事实；`.atomcode/` 与运行时 `data/characters/` 中的忽略文件不属于仓库文档基线；
- 本地全 workspace 测试、UI 测试、TypeScript 类型检查、Rust 格式检查与 Clippy。

## 2. 执行摘要

AIRP 已经不是早期“只有 UI 框架和 mock”的项目。当前仓库具备可工作的无头 RP 引擎、OpenAI/Anthropic 流式适配、角色卡导入、基础世界书、会话与状态持久化、场景、卷系统、拆解工作流、Tauri bridge 和临时 WebUI 验证面。PR #100 还留下了一次真实 DeepSeek SSE 闭环证据。

但项目成熟度仍应定义为 **功能快速增长中的开发者预览**，而不是可安全交付的完整 Agent 客户端。主要原因不是代码量不足，而是以下边界尚未闭合：

1. `agent/run` 仍是固定 `echo → generate → finish` 骨架，工具结果没有回灌决策，也不持久化最终 assistant 消息；
2. HTTP 与 Agent 工具绕过同一数据服务层，聊天、状态和 destructive 操作的锁、校验和事务语义不一致；
3. 桌面 sidecar 退出管理、Windows 安装 smoke workflow、runtime-only secrets 与精确 CORS 白名单已在后续开发分支实现；真实 artifact smoke 和远端 provider 证据仍待 CI/人工运行；
4. Rust 全量测试通过，但 PR 自动门禁不存在，格式与 `-D warnings` Clippy 当前失败；
5. 多份承重文档把“源项目候选能力”“已有数据层”“已注册 Agent 工具”“已暴露 HTTP API”混写，明显高估当前能力。

因此，近期路线不应继续按工具数量横向扩张。应先建立可信基线，再完成持久化/安全边界，之后才实现真正的结构化 Agent loop。

## 3. 当前事实基线

### 3.1 已交付且有源码/测试支撑

- workspace 为 `engine + protocol + ui/src-tauri`；`webui/` 是独立零构建验证面，不是 workspace crate。
- engine 提供 `/v1/chat/completions`、`/v1/agent/run`、角色/会话/状态/场景/预设/拆解/分析/settings/models/version/health 等 HTTP 路由。
- OpenAI 与 Anthropic 流式适配均有响应头阶段超时；旧审计中的“请求无超时”已经失效。
- 默认 Agent 工具注册表后续达到 **19 个**工具：在原 15 个上增加 lorebook apply/merge、seal volume 与 context bundle。执行需 engine-side `call:tool`、可选 allowlist，破坏性工具还需按名确认；实际目录由 `GET /v1/agent/tools` 提供。
- 世界书已有角色卡内嵌解析、HTTP GET/PUT、Aho-Corasick 关键词匹配、`enabled` 与 `priority`；它不是 0%，但也不是 SillyTavern 语义完整实现。
- 角色卡/预设 deterministic decompose 与 analysis preview/apply 已实现。2026-07-07 的 3130 行 implementation plan 已成为历史实施记录，不再是待执行清单。
- PR #100 的 smoke evidence 证明：WebUI → engine → 真实 DeepSeek → SSE 文本回复路径在一次明确配置下成功。该证据不覆盖 Tauri 桌面包、Agent loop 或长期稳定性。
- `data/` 跟踪污染已经修正；当前只跟踪 README、默认 settings 和默认 style profile。

### 3.2 部分实现，禁止写成“完成”

| 能力 | 当前真实状态 | 完成仍需 |
|---|---|---|
| Agent loop | provider 原生 structured tool call → engine gate → typed observation → 动态再规划 → 纯净生成 → 共用 finalizer；有 step/token/wall-clock/cancel 闸 | Anthropic tool-call codec、更强失败恢复、MCP 工具源 |
| 世界书 | 基础 CRUD + OR 关键词触发 + priority | selective/secondary keys、constant、概率、sticky/cooldown/delay、group、位置/depth/order 的明确 AIRP 语义 |
| 状态 | `<state>` 提取、live/history 写盘、schema 读取与提示展示 | 写入前 schema 类型/range/字段策略校验，原子写与并发协调 |
| 用户隔离 | `UserId`、effective root 与若干路径 helper | 完整 persona API/tool、认证主体绑定、会话/配额/数据生命周期 |
| 扩展系统 | UI widget registry、consent、sandbox 已有 | engine capability 强制、事件/工具/存储扩展合同、版本与迁移 |
| MCP | 只有路线与源项目资产；依赖中有 `rmcp`，源码没有 MCP client/server 实现 | 先定义真实第三方 MCP client 用例，再实现适配层 |
| 桌面交付 | sidecar 打包、进程持有/退出清理、Windows 安装→ready→退出 smoke workflow 已实现 | CI artifact 实跑、异常重启、导入与真实对话 GUI smoke |
| WebUI V2 | PR #106 已合并；两个 hash-routed 视图 + 工作台 overlay、provider/endpoint/model/runtime key 编辑及 `/v1/models` 验证已实现 | 带真实凭据的 provider 证据；#105 中设计源遗留项仍需逐项处置 |

### 3.3 尚未实现

- 可用的 ReAct/plan-act-observe Agent runtime；
- engine 侧 authoritative capability enforcement；
- secrets store 或 OS keychain；
- 完整 user persona、plugin zero-schema、通用 artifact、技能/记忆自进化 runtime；
- Rust/TypeScript protocol 单源生成或漂移门禁；
- 自动 PR CI gate；
- 已验收的 Windows 桌面产品闭环。

## 4. 独立审计发现

严重度定义：P0 = 已确认的数据破坏/远程利用/无法使用；P1 = 合并或产品化前必须解决；P2 = 近期架构债；P3 = 文档/维护性改进。本轮没有证据足以判定 P0。

### A-01 · P1 · 数据写路径没有统一并发与事务边界

- Agent tools 在 `engine/src/agent/tools.rs` 使用 per-character 与 per-session 锁；HTTP chat pipeline、history rollback/regen 和 state 持久化未复用这些锁。
- `ChatLog::append` 是 append-only，但 rollback/regen/FIFO 会整体重写；并发的“读取旧快照 → append/rewrite”可能丢失顺序或覆盖另一请求结果。
- `state/live.json` 用直接 overwrite，history 用独立 append；崩溃或并发下两者可能不一致。

**方向**：抽出唯一 `ChatService` / `StateService`，让 HTTP、Tauri、Agent tools 共用锁、原子 replace、revision/idempotency 与错误语义。对应 issues #31、#35，并应补一个专门的并发一致性 issue。

### A-02 · P1 · 状态 schema 只展示，不约束写入

`engine/src/orchestrator/mod.rs` 会读取 schema 丰富提示，`persist_live_state` 却把模型输出直接写入 `live.json`。类型错误、未知字段或越界值都可落盘，且写失败只记录 warning，不反馈本次生成状态。

**方向**：在状态服务层定义显式策略（reject / clamp / preserve-unknown），验证后再原子提交 live + history；HTTP、Agent、模型提取必须走同一路径。对应 issue #36。

**后续状态**：已采用 reject 策略；`StateService` 串行化写入，校验 JSON Schema/现有 fields schema，原子替换 `live.json`，并写入递增 revision 的 history。模型提取与 Agent state tool 共用该服务。

### A-03 · P1 · 本地 HTTP 安全默认值不适合产品化

- CORS 已从 `Any` 收紧为 WebUI/Tauri 精确白名单，自定义来源显式走 `AIRP_CORS_ORIGINS`；
- `access_api_key` 是可选的，未配置时所有业务路由可直接访问；
- provider key 与 access key 已改为 runtime/env-only，普通配置序列化省略，旧明文字段加载时忽略；
- `card_path` 仍是可信本地 sidecar 才可接受的任意文件读取面。

绑定 loopback 降低了暴露面，但不能替代 origin 防护、默认鉴权与 secret storage。浏览器恶意页面、误暴露端口和本机多用户环境都需要单独建模。

**方向**：桌面模式使用启动时随机 token + 精确 origin；远程模式显式 opt-in；provider key 转 OS secret store；`card_path` 只在受信桌面 transport 开启。对应 issues #33、#35 与 RR-001。

### A-04 · P1 · 桌面 sidecar 生命周期未闭合

`ui/src-tauri/src/main.rs` spawn 后丢弃 child handle，只保留 event receiver。UI 无法可靠执行关闭、重启、崩溃退避、端口冲突恢复，也没有产品级状态机。

**方向**：在 Tauri managed state 持有 child handle，提供 health/restart/shutdown；应用退出时显式收尾；补真实安装包 smoke。对应 issues #29、#98。

**后续状态**：Tauri managed state 已持有 child handle，应用退出时显式 kill；Windows workflow 已加入静默安装、启动、ready 探测、关闭 UI、确认 sidecar 退出的 smoke。异常自动重启/退避仍未实现，artifact smoke 等 CI 实跑。

### A-05 · P1 · Agent loop 的名称和文档高估了实现

当前 `agent/mod.rs` 的计划是硬编码数组。tool result 只作为 SSE 事件发给客户端，没有进入下一次模型决策；generate 后直接 finish，且不走 chat finalizer 持久化 assistant 输出。它验证了 loop 外壳和纯净上下文不变式，但尚不是用户可依赖的 Agent runtime。

**方向**：先定义 provider-neutral structured tool-call transcript，再实现 `plan/act/observe` 状态机；每一步都保持角色平面与控制平面物理隔离。不要先扩充大量工具。对应 issue #97。

**后续状态**：固定计划已删除。OpenAI 与 Anthropic tool-call codec 归一为同一 `PlanAction`，执行前过注册表/capability/allowlist，结果作为 typed control observation 进入下一次决策；收敛后才跑纯净 RP generation，并共用 chat finalizer。神圣不变式与集成事件序列均有测试。

### A-06 · P1 · 验证覆盖健康，但工程门禁不健康

原始审计结果如下；后续分支已修复门禁：

- `cargo test --workspace`：engine 386 passed / 1 ignored；engine integration 3 + 11 + 5 passed；protocol 5 passed；Tauri 9 passed；doc tests passed；
- `npm test`：97 passed；`npm run typecheck`：passed；
- `cargo fmt --all -- --check`：failed，多文件未格式化；
- `cargo clippy --workspace --all-targets -- -D warnings`：failed，包含 `too_many_arguments`、`await_holding_lock` 等；
- 手动 `manual-build.yml` 不运行 engine/protocol 全量测试、fmt 或 clippy，也不是 PR gate。

**方向**：新增 pull_request workflow，至少运行 fmt、workspace tests、UI typecheck/tests；Clippy 先修到基线绿再设 `-D warnings`。神圣不变式必须在 CI 中显式可见。对应 issues #70、#98。

**后续状态**：`.github/workflows/pr-gate.yml` 已覆盖 fmt、严格 Clippy、workspace tests、神圣不变式、UI typecheck/tests 与 WebUI 语法检查。本地严格 Clippy 全绿；engine lib 397 passed/1 ignored，integration 3+11+5，protocol 与 Tauri tests、UI 98 tests 均通过。

### A-07 · P1 · 协议与 capability 只在一侧可信

Rust 与 TypeScript wire types 手工维护，存在漂移风险；UI consent 只保护渲染侧，engine 不依据 capability 对数据/工具调用做 authoritative enforcement。

**方向**：选定单一 schema/codegen 来源，加入跨语言 golden fixtures；capability 由 engine 颁发和校验，UI consent 只负责用户交互。对应 issues #28、#32。

**后续状态**：Rust serde wire 值与 TypeScript guard 共同受 `protocol/wire-discriminants.json` 测试约束；Agent tool 执行已有 engine-side capability/allowlist/confirm 强制。widget intent/MCP/hook 尚未接入同一授权服务，后续不得绕过。

### A-08 · P1 · PR #106 已修复运行态落位，#105 尚有剩余范围

原始审计检查未合并 PR #106 时发现：

- README 宣称“三页 SPA”，实际只有 `characters` 与 `session` 两个 `.app-view`；工作台仍是 overlay；
- 角色卡片只展示 ID 与固定文案，没有加载名称、描述、头像，也没有角色删除动作；
- 没有 PR #88 设计中的可编辑 provider/endpoint/key/model 配置体验；
- `webui/style.css` 的 streaming cursor 变成 `content:'鈻?;`，字符串未闭合，属于实际 CSS 损坏。

**后续状态**：PR #106 已修复 CSS、hash 导航、移动布局、角色详情/删除、session-scoped history/regen/rollback 和无鉴权提示；后续分支又补了 provider 配置与真实 `/v1/models` 验证。工作台仍按产品选择保留 overlay，不应再称为第三个 hash route。#105 暂不关闭：仍需带真实凭据的运行证据和设计源遗留项逐项决定。

### A-09 · P2 · 数据能力、HTTP API 与 Agent 工具三层持续漂移

当前存在三种不同能力面：底层 Rust 模块、HTTP routes、19 个 Agent tools。文档不得把“底层已有”写成“Agent 已能调用”，也不得把 MCP-Server 的 38 个候选工具写成本仓已交付规格。后续扩展继续以共享 domain service 为前置。

**方向**：先建立共享 domain services，再用 HTTP/Agent/MCP adapter 暴露；维护自动生成的 capability inventory。issues #31、#103 应以服务复用为前置，而不是直接复制 handler。

**后续状态**：Chat/State/Lorebook services 已供 pipeline、HTTP 与 Agent tools 复用；registry 为 19 个工具，`GET /v1/agent/tools` 已提供运行时 inventory。MCP adapter 仍后置。

### A-10 · P2 · Worldbook 是可用 MVP，不是完整语义

已有 OR keys 与 priority，但缺少 selective/secondary、constant、概率、sticky/cooldown/delay、group、位置/depth/order 等。也未明确哪些 SillyTavern 机械语义应保留，哪些应转换为 Agent 建议元数据/检索工具。

**进展**：AIRP v1 worldbook semantic contract 与兼容 fixture 已写入 `docs/WORLDBOOK-SEMANTICS.md` 和 `engine/tests/fixtures/worldbook/`，并由确定性输出测试守护。selective/constant/概率/sticky/group/位置深度仍明确列为未实现，避免把可导入误称为语义兼容。

### A-11 · P2 · 生命周期与稳定身份仍是数据模型缺口

会话缺 delete/archive/branch，per-user 认证主体未闭合，角色/会话/消息缺持久稳定 ID 与迁移策略。这会阻碍分支、swipe、同步、引用和长期记忆。

**方向**：共享角色卡解析合同（#25）已完成；继续推进 versioned storage schema、stable IDs、迁移与回滚，再构建自进化记忆。剩余对应 issues #35、#37。

### A-12 · P2 · “export context bundle 不变式”不应替代真实不变式

issue #102 计划通过输出文本包含特定措辞来守两仓边界；这只能验证模板字符串，不能证明运行时 subagent 没有协调器污染。当前可验证的不变式是 `subagent_context_has_no_orchestrator_noise`，以及更贴近真实装配产物的 `subagent_prepared_pipeline_has_no_orchestrator_noise`；后者直接断言最终角色平面没有协调器噪声。

**方向**：若实现 export bundle，把它定义为可观察/调试 artifact；安全不变式继续绑定真实 context construction，不绑定文案。

### A-13 · P2 · 文档完整但权威层级混乱

- `engine/README.md` 仍沿用源 AIRP-Core 的 standalone/乐高叙事，并列出不存在的 persona/plugin/MCP 工具；
- `PLAN.md`、`DEV-GUIDE.md`、`PARTS.md` 多处停留在 PR #13 或“仅 echo / worldbook 0%”；
- `docs/audits/` 的历史快照没有统一“已被后续实现取代”标记；
- decompose implementation plan 的任务仍全未勾选，易被误当成当前计划。

**方向**：采用四层文档权威：`README` 当前入口 → 本审计当前快照 → `PLAN` 长期原则 → 历史 audits/plans 只作证据。路线候选必须明确标注“未实现”。

## 5. 对现有 issues 的重新排序

### 现在做（基线与产品安全）

1. #98 桌面安装包真实 smoke；#29 sidecar 生命周期；
2. #70 自动 PR gate + fmt/clippy 基线；
3. #33 secrets；#32 capability enforcement；#28 protocol drift；
4. #35 session/per-user lifecycle；#36 state schema enforcement；
5. 新增：统一 Chat/State service 与并发一致性；
6. #105：PR #106 的运行态落位已完成；继续处理 provider 配置、真实运行验收与设计源遗留项，不以静态文件存在代替端到端证据。

### 随后做（Agent 脊柱）

1. #97 真正的 structured tool-call loop；
2. #31 HTTP/Agent tool parity，但通过共享 service 实现；
3. #101 已由 #102/#103 的真实调用方共同复用，保持单一 UTF-8 安全上限；
4. #103 的状态/世界书/卷工具已按共享服务与 destructive confirm 边界落地；
5. #102 已作为诊断导出落地；其措辞测试只证明产物合同，安全性仍由原有 subagent 不变式独立证明。

### 后做（能力扩张）

- #37 长期记忆/稳定 ID、#87 Agent-first workbench；
- plugin data、user persona、MCP client、skills/hooks、完整 SillyTavern parity；
- #104 的 contributor history rewrite 属破坏性仓库运维，不是产品路线，必须单独授权，不能夹在功能 PR 中。

## 6. 未来发展方向

### 阶段 0：可信基线

交付定义：自动 PR gate 全绿；文档不再高估；WebUI 有真实 engine/provider 运行证据；Windows artifact 有可重复启动证据。

### 阶段 1：统一数据与安全边界

将 filesystem 操作收敛为 versioned domain services。HTTP、Tauri、Agent tools 只做 adapter。完成锁、原子写、revision/idempotency、schema validation、secret store、默认鉴权、sidecar lifecycle。

### 阶段 2：真正的纯净 Agent runtime

实现 provider-neutral 的 structured tool-call loop：模型决策 → allowlist/capability → tool → typed observation → 下一步决策 → finalizer。继续用真实装配产物守干净提示词，不把 ReAct 文本塞入角色平面。

### 阶段 3：RP 数据模型成熟化

稳定 ID、schema version、迁移、分支/swipe、完整 worldbook contract、persona、长期记忆与跨会话检索。先保证可修正和可迁移，再追求复杂自动化。

### 阶段 4：产品 UI

Tauri/Vue 仍是长期产品面。WebUI 保持开发/诊断 harness。Agent-first workbench 依赖稳定 diff、revision 与 capability，不应提前把不稳定内部模型固化成 UI。

### 阶段 5：开放扩展

在 domain services 与 capability 模型稳定后，再开放 MCP client、事件 hook、skills、plugin storage 和第三方 widget。开放应表现为版本化合同与最小授权，而不是任意脚本直接读写内部目录。

## 7. 成功判据

下一阶段不是以“新增多少工具/文档/端点”衡量，而以以下可重复结果衡量：

1. 新 PR 自动运行 workspace tests、UI tests/typecheck、fmt 与 Clippy；
2. Windows 安装包可启动、选择/导入角色、发送消息、收到流式回复、正常退出 sidecar；
3. 同一会话并发 append/rollback/regen 不丢消息，live/history 状态原子一致；
4. schema 违规状态不会静默落盘；
5. provider key 不以明文出现在普通 settings 文件；
6. Agent tool result 真正影响后续决策，最终回答被持久化；
7. Rust/TS 协议 fixture 自动防漂移；
8. README、当前审计、issue 与实现能力表无互相矛盾。

## 8. 本轮结论

AIRP 的正确方向仍是“自有纯净 Agent runtime + RP 数据层 + Tauri 产品 UI”，但近期最有价值的工作是**收敛和闭环**，不是继续横向吸收源仓库功能。项目已经积累了足够多的功能原型；下一次质量跃迁来自统一服务边界、自动门禁、真实桌面证据和结构化 Agent loop。
