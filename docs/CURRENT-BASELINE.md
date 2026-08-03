# AIRP 当前开发基线

> 基线日期：2026-08-03
> 代码基线：`main@e931bf7`（合并 PR #445 `feat(engine): backup/restore closed loop (E-P2-1, closes #342)`）
> 用途：冷启动开发、审计和产品判断的第一事实入口。  
> 真理顺序：当前源码、manifest、测试与可重复运行证据 > 本文 > 专题合同 > 路线图/研究材料 > 历史归档。

本文只记录当前代码树能够支持的结论。GitHub issues 是未完成工作的实时追踪面；PR、审计报告和历史测试数字只证明对应代码树，不自动证明当前 `HEAD`。

本次校准（2026-08-02 v0.0.3 docs-pass）做了三件事：

1. 将代码锚点从旧 `main@4f3f792` 对齐到当前 `main@830426e`；
2. 吸收 #398–#413 的取消、TurnCommit/Recovering、终态 marker recovery、production smoke 与依赖治理事实，**不把后续 issue 写成已交付**；
3. 保持活文档面收敛：已完成计划与桌面画布接力草案仍在 `docs/archive/`，阅读路径见 [README.md](README.md)。

增量校准（2026-08-03，W-06 闭合）：代码锚点从 `main@830426e` 推进到 `main@e931bf7`，吸收 PR #436/#439/#441（R1 锁序收敛 + 运行时强制 + 回归测试，closes #437/#438/#440）与 PR #445（#342 backup/restore 最小闭环）的交付事实。#342 标记为已交付（v1 限制见 §2.2）；R1 锁序合同补全见 [LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md) §1.5/§2.9/R4。本次为 docs-only 增量校准，§6 验证快照的 full-workspace 数字未在本 docs-pass 重跑，仅追加 #342 与 R1 回归测试的 PR 级证据。

## 1. 产品与仓库边界

AIRP 是面向 Role Play 的 AI Agent 客户端，采用“无头 Engine + 可换 UI”结构。

| 路径 | 当前职责 | 产品状态 |
|---|---|---|
| `engine/` | `airp-core`：RP 数据、prompt 装配、LLM adapter、Agent loop、HTTP/SSE | 唯一业务内核 |
| `webui/` | 无构建、多页面、同源 WebUI（当前 44 屏） | **正式产品交付主面** |
| `airp-engine-console/` | WebUI 视觉与交互样板 | 设计基线，不是第二套运行时 |
| `protocol/` | `airp-state-protocol`：共享线协议类型 | Rust workspace 成员 |
| `ui/`、`ui/src-tauri/` | Vue + Tauri 桌面客户端 | 保留维护线，**近期发布暂停** |
| `deploy/windows-webui/` | Windows 便携 WebUI 包 | 当前优先 artifact |
| `deploy/linux-webui/` | Linux musl 便携包 | 手动构建 artifact |
| `deploy/production/` | 单实例自托管 HTTPS preview | P0 拓扑，不是正式发布 |
| `data/` | 运行时数据根规范与安全样例 | 不是共享素材库 |
| `tools/` | 依赖治理、SBOM、Agent 浏览器探索 | 工程工具，不进入 RP 角色平面 |

Rust workspace 只有 `engine`、`protocol`、`ui/src-tauri`。AIRP-Core/AIRPCLI、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 是作者的第一方前序项目，不是当前 runtime 依赖；吸收边界见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)。

## 2. 当前能力矩阵

“已实现”按层描述，不把底层模块、HTTP route、UI 页面或一次测试互相冒充。

| 能力域 | Engine / 数据 | HTTP / Agent | WebUI | 当前边界 |
|---|---|---|---|---|
| 角色、Persona、Preset、场景 | CRUD、导入、绑定、revision、装配 | 主要 CRUD/导入/预览 route；相关 Agent tools（Persona **无**对称 Agent tool） | 管理、选择、导入与诊断入口 | backup/restore 最小闭环已交付（#342，PR #445，v1 限制见 §2.2 切片表）；完整导出/migration 未闭合（#346） |
| 会话与聊天（**产品主路径**） | durable JSONL、稳定 message ID、cursor、rollback、branch/swipe、per-session Coordinator façade；TurnCommit 记录 message/state/volume 提交进度 | OpenAI-compatible `/v1/chat/*` SSE、continue/regen/search、命名 session；generation-scoped `session-state`/`cancel` | 命名会话、流式聊天、Engine 协作停止、编辑/删除/分支/Swipe、导出 | 取消只接受当前 `generation_id`；stale/committing 分别 fail-closed，断线不等于取消。终态 marker 可在已持久化最终阶段后恢复清理，Recovering/未知提交仍 fail-closed。**产品 UI 只绑定本路径**；agent/tool 单 owner、自动 replay/repair、跨资源事务仍未交付（#394/#286）；backup/restore 最小闭环已交付（#342，PR #445），但跨资源一致性 backup 与完整灾难恢复未交付 |
| Conversation runtime（**Engine 合同，未绑产品 UI**） | versioned manifest、append-only event journal、message/turn/observability projection、scene round-robin、受控 policy 注入、长历史 checkpoint/summary 预算、legacy copy-only migration | `/v1/conversations*`、capabilities、policies、migration plan/execute/export/rollback；旧 chat/session/scene API 形状不变 | **尚未绑定**；客户端若接入只能经 capability discovery，不能注入 history/代码/调度语义 | 与 legacy Chat **双轨并存**；切流或冻结须战略决策（#381 E-P0-2）。自动 summary policy、内容型停止条件、全仓 migration registry、跨进程策略沙箱仍开放 |
| Worldbook / state / memory | v4 runtime、state history/schema、resident memory、revision | CRUD、图谱、事件、状态与记忆相关接口/工具 | 编辑、图谱、状态 HUD、记忆面板 | 大量 ST 字段仅为 advisory；完整 session 物化与记忆闭环未完成（#274） |
| Agent 与剧情 | 有界 loop、Director、Council、NPC、剧情弧、世界时钟、定时事件、遗忘曲线 | 约 30 个内置工具 + 可动态加载插件工具 | Agent run、剧情弧、群聊、世界事件 | 并发/失败路径有开放审计项（#284/#344/#381）；不是通用多 Agent 平台 |
| 创作工具 | 图片生成、角色模板、风格学习、对话示例、时间线、卡片 diff | 对应 HTTP | 屏 36–42 等已接入 | 功能存在 ≠ 真实 provider/工作流已验收 |
| Provider / 扩展 | 多 Provider 路由、OpenAI-compatible/Anthropic/Ollama、本地脚本/HTTP webhook 插件 | providers/routing/plugin-tools API；Agent registry 动态合并 | 设置与插件管理入口 | 插件非沙箱；HTTPS webhook 注册+请求 fail-closed DNS 与域名 pin 已落地（RR-014 近端修复 / #381 E-P0-3 / #329 N3）；非通用代码沙箱 |
| 部署 | production fail-closed 校验、原子配置更新、secret 脱敏 | loopback 默认；首方 gateway 同源代理 | Windows/Linux 便携包与 production preview | 非多租户；P1/P2/P3 发布门未闭合 |

### 2.1 结构性事实（2026-08-02 审查确认）

这些不是新功能承诺，而是避免误读代码树的硬事实：

1. **双轨会话**：正式 WebUI 走 `/v1/chat/*` + `ChatLog`/`ChatService`；Conversation 是并行 Engine 合同与 HTTP 面，**不能**因 route/测试存在就宣称产品已切换。**v0.0.3 决策（E-P0-2/B）**：冻结 Conversation 功能对称扩张，产品验收不切流。  
2. **单资源原子写 ≠ 跨资源事务**：`finalize` 可对 message → state → volume 逐步 fail-closed，崩溃后跨资源一致性仍是 best-effort（RR-004 / #286）。  
3. **Domain 写路径未完全闭合**：shared service 是目标边界；Agent tools 等路径仍可能直接 `replace_file` / `fs` 写（#381 E-P1-3 / #160）。  
4. **锁模型分裂**：character/session/state/persona/conversation/decay/FTS/quota 等多套锁；async 路径上存在 std 锁 + 锁内磁盘 I/O；poison 策略不一致（#284/#220/#381）。  
5. **桌面线暂停**：`ui/` 保留；画布接力等草案在 archive，不进入当前执行队列。

### 2.2 v0.0.3 收敛切片的已交付边界

以下记录的是当前 `main@830426e` 能由源码、测试或生产 harness 支持的边界，不把 release 计划写成能力：

| 切片 | 已实现 / 已验证 | 明确未包含 |
|---|---|---|
| #398 | Coordinator 提供 generation-scoped `session-state` 与 cooperative `cancel`；仅当前 generation 可取消，stale/committing 请求返回冲突，WebUI 保留 Engine 权威 `commit_state`。 | 浏览器断线不等于 Engine 取消；不改变跨资源恢复能力。 |
| #399/#403 | durable `TurnCommit` marker 覆盖 message、live state 与 current volume 阶段；中断后公开 `Recovering` 并拒绝新 mutation，marker schema/阶段状态 fail-closed。 | 不包含自动 replay/repair、volume sealing recovery、backup/restore。 |
| #409 | 仅在所有 expected 阶段已 durable 时清理 terminal marker；恢复清理与 session owner/admission registry 锁序串行化；non-terminal、unreadable、unsupported 与 all-false marker 保留为 recovery-required。 | 不包含 payload-aware 自动 replay/journal 或完整灾难恢复。 |
| #410/#411 | production mock smoke 覆盖 generation poll、stale/current cancel、严格 typed SSE terminal、取消后 history、临时 session cleanup；harness 对 cleanup、SSE、cancel poll、response body 与 deadline 使用绝对预算并 fail-closed，合法空响应体可结束；备份入口保持明确不可用，renderer 不发起 backup/restore API 调用。 | 这是 mock/CI 证据，不替代真实 provider、真实 browser 和维护者 Compose 验收；不改变 Engine/API 语义。 |
| #413 | lock-only 更新 `brace-expansion` 2.1.1→2.1.4、`postcss` 8.5.16→8.5.25、`nanoid` 3.3.15→3.3.16；`npm audit --json` 与 `--omit=dev` 均为 0。当前 SBOM 生成报告 693 third-party、unknown license 0、blocked 0；inventory 总记录 697（first-party 4、audit-required 17、auto-pass 680）。 | SBOM/notice 生成仍未成为 release pipeline 强制门禁；`ui` 依赖用于构建/测试，production gateway 只发布静态 WebUI，不把 `ui/node_modules` 当 runtime。 |
| #436/#439/#441（R1 锁序收敛 + 运行时强制） | `advance_plot` / `trigger_world_event` / `advance_clock` / `npc_action` / `run_seal_flow` 五个 agent-tool 路径补齐外层 `character_lock.read()`（R1）；`StateService::mutate` 拆为 `mutate_locked` + `mutate` 消除 re-entrancy；`lock_order` 模块提供 R1+R2 运行时 `debug_assert!` 强制（thread-local 栈 + RAII Guard，release 零开销）；4 条并发回归测试（各路径与 `delete_character` 经 `Barrier` 并发，30s 超时检测死锁 + TOCTOU）；lock-map cleanup race 修复（`remove_deleted_*_lock` 移到 write guard drop 之后）。 | R1/R2 仅在 `debug_assertions` 下强制，release build 无运行时强制；`advance_plot` 仍持 std 锁做同步 I/O（A3 debt）；W-01~W-04 follow-up 见 #442/#443/#444。 |
| #342（PR #445，backup/restore 最小闭环） | 手动 backup（Full / Character / Session scope）+ manifest schema v1（per-file SHA-256 + tree SHA-256）+ `verify_against_disk` 完整性校验 + restore（Full + scoped `swap_scoped_subtree`）+ `PreRestoreRollback` 回滚备份 + post-restore 校验 + `PreDelete` 自动备份（`delete_character` / `delete_session`，`force=true` 可跳过）+ secret 排除（`secrets.json` / `settings.json`）+ `BACKUP_LOCK` 串行化 + WebUI backup 管理界面。82 条 backup 测试通过（PR #445）。 | v1 限制：无自动定时备份；restore swap 阶段不持 `character_lock`（W-02，#447，v1 缓解为维护窗口执行）；Windows `sync_dir` no-op（W-03，#448）；跨资源一致性 backup 未交付；完整 migration / 导出未交付（#346）。审计遗留 W-01~W-06 见 #446/#447/#448/#449/#450/#451。 |

## 3. 必须保持的不变式

1. **干净提示词**：RP 角色平面只含 RP 数据；工具/调度/审计留在结构化控制平面。`subagent_context_has_no_orchestrator_noise` 神圣不可弱化。  
2. **Engine 单一真相**：handler、UI、Agent tool 不复制持久化规则；写路径应收敛到 shared service。  
3. **有界 Agent**：step/token/墙钟/取消/可观察事件；UI consent 不替代 Engine 授权。  
4. **用户资产优先**：不兼容演进必须有 versioned migration、升级前备份、完整性验证、可读导出与回滚。  
5. **安全默认关闭**：production 监听前 fail-closed；密钥不进普通 settings/URL/前端存储/日志；Web/远端不得启用任意本地路径导入。  
6. **第三方独立实现**：只吸收公开理念/需求/行为/互操作经验；不复制第三方代码、prompt、测试、数据或视觉资产。  
7. **审计门禁**：本地全绿只允许开 PR；审计 bot 通过并修完阻塞意见后，仍由人工 review 决定合并。

## 4. 当前不能宣称

- 不能宣称已正式发布、适合公网多租户、通过完整 P1/P2/P3，或已获市场验证。  
- 不能用页面数、工具数或 Phase 合入数量替代黄金路径成功率、恢复能力与继续使用意愿。  
- 不能宣称完整 session 自包含、跨资源 Turn 事务、全仓统一 migration registry、自动定时备份/恢复、浏览器矩阵或长会话 soak 已交付。  
- 不能把 Conversation 的 copy migration / 可观测性能力外推为 legacy Chat 产品路径或其它资产已具备同等恢复力。  
- 不能宣称完整 MCP 生态、任意插件沙箱、跨设备同步、多语言 UI 或正式资产规格已交付。  
- 不能把桌面 Tauri、production preview、Windows/Linux 便携包的测试结果互相外推。  
- 不能把 Worldbook/Preset **advisory** 字段写成已执行语义；以 [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md) 为准。  
- 不能把「已开 GitHub issue / 已写审计 umbrella」写成「风险已关闭」。
- 不能把 #398–#411 的取消、marker、Recovering 或 harness 证据外推为自动 replay/repair、Agent/tool 单 owner、性能 SLO、backup/restore 或真实 provider/browser/Compose 验收。
- #346 完整导出/migration、跨资源一致性 backup、自动定时备份/恢复、#286/#394 O3 replay/repair、#394 O2 Agent/tool ownership、#394 O4/#400 性能与兼容性基准，以及 P2/P3 的 SBOM release gate、签名、browser matrix、soak 仍未交付；它们是后续正式 release gates，不是当前代码已具备的能力。#342 backup/restore **最小闭环已交付**（PR #445，v1 限制见 §2.2），但不得外推为完整灾难恢复或自动定时备份。

## 5. 当前优先级

当前主线不是扩大功能面，而是把已合入能力收敛成可验证、可恢复的 **P1 有限试用**（**v0.0.3 后端门禁窗口**）：

**v0.0.3 P1 门状态（2026-08-02）**：代码与 mock/CI 证据已覆盖 #398–#413 的上述收敛切片；对 P1 有限试用，唯一尚未完成的外部硬阻塞是 [#130](https://github.com/GhostXia/AIRP/issues/130) 的维护者验收：真实 provider + 真实 browser + production Compose。CI mock、system Chrome 或本地单元测试不能替代该验收。

### 5.0 v0.0.3 已拍板决策

**E-P0-2 · Chat vs Conversation = 选项 B 冻结扩面（2026-07-30，当前仍有效）**

- 产品主路径与 v0.0.3 验收 **只绑定** legacy `/v1/chat/*` + `ChatLog` / `ChatService`。
- Conversation runtime（`/v1/conversations*` 及并行合同）在本窗口内 **冻结功能对称扩张**：仅允许安全修复、既有合同 bugfix、文档/测试诚实性维护；**不得**为 WebUI 切流或与 Chat 对等堆新能力。
- 选项 A（产品切流到 Conversation）需要独立战略决策、迁移/恢复证据与用户明确批准，**不**作为 v0.0.3 默认路径。
- 关联：[#381](https://github.com/GhostXia/AIRP/issues/381) E-P0-2、[#371](https://github.com/GhostXia/AIRP/issues/371)、[#344](https://github.com/GhostXia/AIRP/issues/344)、[#242](https://github.com/GhostXia/AIRP/issues/242)。

1. **Engine 一致性收敛**（[#381](https://github.com/GhostXia/AIRP/issues/381)）：  
   - ~~拍板 Chat vs Conversation（切流或冻结，E-P0-2）~~ → **已决策：B 冻结扩面**（见 §5.0）；
   - ~~Plugin DNS fail-closed + 请求时校验（E-P0-3，升权自 #329 N3）~~ → **已落地**（PR #384 / 见 SECURITY.md）；
   - Turn 级跨资源 commit/recovery（E-P0-1 → 执行面 #286，灾难恢复 #342）；  
   - 锁/async I/O/poison 与同 session 互斥（E-P0-4/5 → #284/#220/#160）。  
2. 用真实 provider 与真实浏览器验证：onboarding → 首聊 → 继续对话 → 刷新恢复 → 服务重启恢复（#130 是当前 P1 外部硬门）。
3. 校准 WebUI 运行时契约、视觉一致性、空/错/慢状态与 browser smoke（#311/#345 等）。  
4. ~~备份、恢复、migration 与回滚的最小闭环（#342，正式 P2 release gate，当前未交付）~~ → **#342 最小闭环已交付（PR #445）**：手动 backup/restore（Full/Character/Session scope）+ pre-delete backup + scoped restore + 完整性校验 + WebUI。剩余：完整 migration/导出（#346）、自动定时备份、跨资源一致性 backup、restore swap 持 character_lock（W-02/#447）。
5. 上述证据稳定前，默认不从 #312 启动无用户证据的新子系统扩张；遵守 #242 范围收敛。

### 5.1 实时工作入口（动手前用 `gh issue view` 复核状态）

| 主题 | Issue |
|---|---|
| Engine 审计 umbrella / 排序 | [#381](https://github.com/GhostXia/AIRP/issues/381) |
| Turn 两阶段 / 跨资源提交 | [#286](https://github.com/GhostXia/AIRP/issues/286) |
| per-session 并发串行化 | [#284](https://github.com/GhostXia/AIRP/issues/284) |
| 持久化/lock 遗留 | [#220](https://github.com/GhostXia/AIRP/issues/220) |
| 备份恢复导出 | [#342](https://github.com/GhostXia/AIRP/issues/342) |
| Plugin/engine 非阻塞遗留（含 DNS N3） | [#329](https://github.com/GhostXia/AIRP/issues/329) |
| Conversation migration 解耦 | [#371](https://github.com/GhostXia/AIRP/issues/371) |
| WebUI 能力展现 / 契约门禁 | [#311](https://github.com/GhostXia/AIRP/issues/311)、[#345](https://github.com/GhostXia/AIRP/issues/345) |
| 范围收敛 / 路线图索引 | [#242](https://github.com/GhostXia/AIRP/issues/242)、[#312](https://github.com/GhostXia/AIRP/issues/312) |

## 6. 验证快照

| 范围 | 命令 / 说明 | 结果 |
|---|---|---|
| WebUI | `node --test webui/tests/*.test.mjs` | **75 passed, 0 failed**（`main@830426e`，2026-08-02；PR #445 WebUI backup 测试含其中） |
| production harness unit | `node --test deploy/production/*.test.mjs` | **22 passed, 0 failed**（`main@830426e`，2026-08-02） |
| UI | `npm run typecheck`；`npm run test -- --run`（`ui/`） | typecheck 通过；Vitest **13 files / 98 passed**（`main@830426e`，2026-08-02） |
| Rust engine + protocol | `cargo test --workspace --exclude airp-ui --locked` | **1,282 passed, 5 ignored, 0 failed**（`main@830426e`，2026-08-02）。增量：PR #436/#439/#441/#445 新增 R1 回归 + backup 测试，`main@e931bf7` 数字 ≥ 此处；本 docs-pass 未重跑 full-workspace，不把旧数字写成 e931bf7 数字。 |
| Rust full workspace | `cargo test --workspace --locked` | 本机验证边界：`airp-ui` build script 需要生成的 `ui/src-tauri/binaries/airp-core-x86_64-pc-windows-gnu.exe`；因此未完成 full-workspace 测试，不把 exclude 结果写成 full workspace。 |
| #342 backup/restore（PR #445） | `cargo test -p airp-core --lib backup::` 等 | **82 passed, 0 failed**（PR #445，`main@e931bf7`）：manifest schema、atomic snapshot、scoped restore、pre-delete backup、secret 排除、path sandbox、BACKUP_LOCK 语义。神圣不变式 `subagent_context_has_no_orchestrator_noise` 通过。 |
| R1 锁序运行时强制（PR #441） | `cargo test -p airp-core --lib` | 4 条并发回归测试（`advance_plot`/`trigger_world_event`/`advance_clock`/`npc_action` 各与 `delete_character` 经 `Barrier` 并发）+ 9 条 R1 单测通过（PR #441）。 |
| npm dependency audit | `npm audit --json`；`npm audit --omit=dev --json`（`ui/`） | 两个命令均 exit 0，vulnerabilities total **0**；#413 后的 lock-only 版本见 §2.2。 |
| dependency governance / SBOM | `discover-deps.mjs --fail-on-block`；`generate-sbom.mjs --fail-on-unknown` | 均 exit 0；**693 third-party / unknown 0 / blocked 0**。inventory 总记录 697（first-party 4、audit-required 17、auto-pass 680）。 |
| production topology / 真实 provider·browser | CI mock/system-Chrome 与本地检查 | 不能替代 #130 维护者真实 provider + 真实 browser + Compose 验收；当前不宣称通过。 |

未在本次校准中完成的 maintainer-run production Compose、真实 provider/browser、Windows/Linux artifact、网络故障、进程崩溃和真实 provider 长会话，不得由本表推断为通过。本 docs-pass（2026-08-03）未重跑 full-workspace 测试；#342 与 R1 行为 PR 级证据，其余行仍为 `main@830426e` 证据。

## 7. 文档职责（校准后）

| 层级 | 文档 | 职责 |
|---|---|---|
| 事实入口 | [CURRENT-BASELINE.md](CURRENT-BASELINE.md)（本文） | 唯一人工维护的全仓能力基线 |
| 开发治理 | [DEV-GUIDE.md](DEV-GUIDE.md) | 地图、命令、不变式、交付流程 |
| 产品方向 | [PLAN.md](PLAN.md) | 稳定目标与阶段门；不复制 issue 队列 |
| 数据/运行时合同 | [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)、[CONVERSATION-CONTRACT.md](CONVERSATION-CONTRACT.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)、[LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md) | 目标与已交付边界必须分开读 |
| 安全/发布 | [SECURITY.md](SECURITY.md)、[RISK-REGISTER.md](RISK-REGISTER.md)、[WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)、[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) | 威胁、风险、拓扑与发布门 |
| 接口/扩展草案 | [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)、[AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md)、[ASSET-SPEC.md](ASSET-SPEC.md) | 已决策边界；未实现部分不得当 runtime 事实 |
| 来源治理 | [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)、[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) | 第一方/第三方吸收与 provenance |
| 文档地图 | [README.md](README.md) | 分层与阅读路径 |
| 审计原始记录 | `docs/audits/` | 按 PR 归档；不压缩成当前能力清单 |
| 历史 | `docs/archive/` | 已完成设计、草案与月度摘要；**不能覆盖本文** |

参考材料（研究用，非当前能力）：[CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md)、[MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)、[TAVERN-PARITY.md](TAVERN-PARITY.md)、[HERMES-MEMORY.md](HERMES-MEMORY.md)、[LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)。

### 7.1 本 docs-pass 的文档整理动作

| 动作 | 路径 | 原因 |
|---|---|---|
| 归档 | `docs/archive/2026-07-persona-http-api-plan.md`（原 `docs/PERSONA-HTTP-API-PLAN.md`） | 实施计划已交付；接口事实以源码+基线为准 |
| 归档 | `docs/archive/2026-07-29-desktop-ui-canvas-relay-plan.md`（原未跟踪活路径草案） | 桌面发布暂停；草案不占活文档位 |
| 保留 | `docs/audits/2026-07-28-desktop-ui-relay-plan-audit.md` | 计划级审计原始记录 |
| 刷新 | 本文、DEV-GUIDE、WEBUI-PRODUCTION-PLAN 及直接相关事实入口 | 对齐 `830426e`、#398–#413 与 #130 未解除状态 |

完整阅读路径与维护规则见 [README.md](README.md)。
