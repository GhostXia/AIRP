# AIRP 当前开发基线

> 基线日期：2026-07-29
> 代码基线：Conversation Batch 6 工作树（基于 `main@f908b03`）
> 用途：冷启动开发、审计和产品判断的第一事实入口。
> 真理顺序：当前源码、manifest、测试与可重复运行证据 > 本文 > 专题合同 > 路线图/研究材料 > 历史归档。

本文只记录当前代码树能够支持的结论。GitHub issues 是未完成工作的实时追踪面；PR、审计报告和历史测试数字只证明对应代码树，不自动证明当前 `HEAD`。

## 1. 产品与仓库边界

AIRP 是面向 Role Play 的 AI Agent 客户端，采用“无头 Engine + 可换 UI”结构。

| 路径 | 当前职责 | 产品状态 |
|---|---|---|
| `engine/` | `airp-core`：RP 数据、prompt 装配、LLM adapter、Agent loop、HTTP/SSE | 唯一业务内核 |
| `webui/` | 无构建、多页面、同源 WebUI | 当前正式产品交付主面 |
| `airp-engine-console/` | WebUI 视觉与交互样板 | 设计基线，不是第二套运行时 |
| `protocol/` | `airp-state-protocol`：共享线协议类型 | Rust workspace 成员 |
| `ui/`、`ui/src-tauri/` | Vue + Tauri 桌面客户端 | 保留维护线，近期发布暂停 |
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
| 角色、Persona、Preset、场景 | CRUD、导入、绑定、revision、装配 | 主要 CRUD/导入/预览 route；相关 Agent tools | 已有管理、选择、导入与诊断入口 | 高级生命周期、完整导出/恢复仍未闭合 |
| 会话与聊天 | durable JSONL、稳定 message ID、cursor、rollback、branch/swipe | OpenAI-compatible chat SSE、continue/regen/search | 命名会话、流式聊天、编辑/删除/分支/Swipe、导出 | 长会话虚拟化、跨资源事务与完整恢复仍开放 |
| Conversation runtime | UI 无关的 versioned manifest、append-only event journal、可重建 message/turn lifecycle projection、幂等 scene round-robin 回合执行；受控外部 Rust policy 可运行时注入并受 provenance/capability/lifecycle/resource gate 约束；支持确定性提交的串行/并行 plan 与 speaker 数量停止条件；同步 I/O 隔离到 blocking worker；长历史 prompt 由 Engine 预算、可重建 checkpoint 与可验证 summary 前缀有界投影；legacy character chat 与 Council 可通过 versioned、copy-only、digest-verified adapter 显式迁移，scene/group 无 speaker 证据时停在 `needs_review` | `/v1/conversations*`、turn 状态/显式取消、descriptor v2 `/v1/conversation-policies`；migration plan/execute/export/rollback；旧 chat/session/scene API 形状不变；未知、停用、失败、panic 或超时策略执行 fail-closed | 尚未绑定具体 UI，客户端只消费 Engine 合同且不能注入 history、代码或调度语义 | 自动 summary 生成 policy、内容型停止条件、通用审计 projection、全仓统一 migration registry、跨进程 provider reconciliation 与沙箱化跨进程/动态策略仍开放 |
| Worldbook / state / memory | v4 runtime、state history/schema、resident memory、revision | CRUD、图谱、事件、状态与记忆相关接口/工具 | 编辑、图谱、状态 HUD、记忆面板 | advisory 语义、完整 session 物化与生命周期未完成 |
| Agent 与剧情 | 有界 loop、Director、Council、NPC、剧情弧、世界时钟、定时事件、遗忘曲线 | 30 个内置工具；运行时还可加载插件工具 | Agent run、剧情弧、群聊、世界事件 | 并发/失败路径仍有开放审计项；不是通用可配置多 Agent 平台 |
| 创作工具 | 图片生成、角色模板、风格学习、对话示例、时间线、卡片 diff | 对应 HTTP 接口 | 屏 36–42 已接入 | 功能存在不等于真实 provider、数据恢复与用户工作流已验收 |
| Provider / 扩展 | 多 Provider 路由、OpenAI-compatible/Anthropic、Ollama、本地脚本/HTTP webhook 插件工具 | provider/routing/plugin-tool 管理接口；插件工具动态进入 registry | 屏 43–44 已接入 | 外部 MCP client、插件签名/生态、跨设备同步未交付 |
| 部署 | production fail-closed 校验、原子配置更新、secret 脱敏 | loopback 默认；首方 gateway 同源代理 | Windows/Linux 便携包与 production preview | 不是多租户服务；P1/P2/P3 发布门禁未全部通过 |

### 2.1 Agent 工具边界

当前默认 registry 有 30 个内置工具：

- session：`list_sessions`、`start_session`、`append_message`、`get_recent_context`、`rollback_messages`；
- character：`list_characters`、`get_character`、`delete_character`；
- state/worldbook：`get_character_state`、`update_character_state`、`get_lorebook`、`update_lorebook`、`apply_lorebook`、`merge_lorebooks`；
- preset/volume/analysis：`get_preset`、`update_preset`、`seal_volume`、`export_context_bundle`、`enhance_analysis`、`apply_enhanced_analysis`；
- world/plot/NPC：`trigger_world_event`、`list_world_events`、`advance_clock`、`get_clock`、`npc_action`、`update_relationship`、`advance_plot`、`get_plot_status`；
- search/debug：`session_search`、`echo`。

插件工具可从配置动态追加，因此 `GET /v1/agent/tools` 才是某次运行的实际目录。所有执行仍受 capability、allowlist、破坏性确认和运行预算约束。插件工具不是外部 MCP 集成。

### 2.2 2026-07-25 至 2026-07-26 新增实现

以下代码已合入 `main`，此前基线尚未纳入：

- PR #314：对话导出、Drift 回滚、场景加角色、关系图、状态 HUD、Decompose/Analysis 入口；
- PR #316：Director、世界时钟/定时事件、剧情弧、NPC 行动轮、Council、记忆遗忘曲线；
- PR #317：群聊、TTS/BGM/动效/分享入口、场景插图、角色模板；
- PR #323：风格学习、对话示例、Worldbook 图谱、时间线导出、角色卡版本对比；
- PR #328：多 Provider 路由、Ollama、自定义插件工具；
- PR #333/#334/#335/#338：chat finalize 顺序、seal 后维护、world-event 锁序和 `advance_clock` session 锁修复。

这些合入证明代码与当时门禁通过，不等于真实用户、真实 provider、长会话、崩溃恢复或正式发布已经通过。

## 3. 必须保持的不变式

1. **角色平面纯净**：角色 prompt 不得混入工具定义、调度、审计或 orchestrator 噪声；`subagent_context_has_no_orchestrator_noise` 是阻塞门禁。
2. **Engine 单一真相**：业务规则和持久化由 shared service 承担；handler、Agent tool 与 UI 不得各自复制规则或直写数据。
3. **有界执行**：Agent run 保持 step、token、wall-clock、取消和可观察事件边界；UI consent 不替代 Engine 授权。
4. **用户资产优先**：不兼容演进必须有 versioned migration、升级前备份、完整性验证、可读导出与回滚；不得静默丢失角色卡、世界书、会话或记忆。
5. **安全默认关闭**：production 在监听前 fail-closed；密钥不进入普通 settings、URL、前端存储、日志或诊断；Web/远端不得启用任意本地路径导入。
6. **第三方独立实现**：只吸收公开理念、需求、行为和互操作经验；不复制第三方代码、prompt、测试、数据或视觉资产。
7. **审计门禁**：本地全绿只允许开 PR；审计 bot 通过并修复全部阻塞意见后，仍由人工 review 决定是否合并。

## 4. 当前不能宣称

- 不能宣称 AIRP 已正式发布、适合公网多租户、通过完整 P1/P2/P3，或已经获得市场验证。
- 不能用 44 个页面、30 个内置工具或 Phase 1–5.3 的合入数量替代黄金路径成功率、恢复能力、稳定性和继续使用意愿。
- 不能宣称完整 session 自包含、跨资源事务、全仓统一 migration registry、自动定时备份/恢复、浏览器矩阵或长会话 soak 已交付。Conversation 专用 copy migration 的可读导出与有条件回滚不能外推为其他资产也已具备同等恢复能力。
- 不能宣称完整 MCP client/服务器生态、任意插件沙箱、跨设备同步、多语言 UI 或正式资产规格已交付。
- 不能把桌面 Tauri 资产、production preview、Windows/Linux 便携包之间的测试结果互相外推。
- 不能把保留的 Worldbook advisory 字段写成已执行语义；当前 runtime 合同以 [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md) 为准。

## 5. 当前优先级

当前主线不是继续扩大功能面，而是把已合入能力收敛成可验证、可恢复的 P1 有限试用：

1. 修复开放的并发、持久化、失败注入和安全审计项，尤其是会改变用户资产、泄露 secret 或产生虚假成功的路径；
2. 用真实 provider 和真实浏览器验证 onboarding → 首聊 → 继续对话 → 页面刷新恢复 → 服务重启恢复；
3. 校准屏 34–44 的运行时契约、视觉一致性、空/错/慢状态和 browser smoke；
4. 补齐备份、恢复、migration 与回滚的最小 P1/P2 边界；
5. 只有上述证据稳定后，才从 issue #312 继续选择 5.4–5.7 或其他扩展项。

实时工作项以 GitHub issues 为准。当前需要特别复核的入口包括 #320–#339 的审计遗留、#311 的 WebUI 展现缺口、#242 的范围收敛和 #312 的历史路线图；issue 状态可能随时变化，动手前必须重新查询。

## 6. 验证快照

本次文档校准针对 Conversation Batch 6 工作树（基于 `main@f908b03`）于 2026-07-29 运行以下本地验证；该批在合并前不冒充 `main`：

| 范围 | 命令 | 结果 |
|---|---|---|
| Rust workspace | `cargo test --workspace --locked` | engine lib 1136 passed / 4 ignored；engine main 4 passed；6 个 integration binaries 合计 38 passed；protocol 6 passed；Tauri shell 9 passed；总计 1193 passed / 4 ignored |
| Rust 静态门禁 | `cargo fmt --all -- --check`；workspace clippy `-D warnings`；rustdoc `-D warnings` | 通过 |
| Conversation 长历史（Batch 4 历史基准，Batch 5 未重跑 release benchmark） | 50,000 events release benchmark + 50 次 append-aware projection soak；10,000 events 默认测试 | PR #367 / `main@66abbd6` 证据：cold 117 ms；每次先 append 再 projection 的均值 5.329 ms；输出最多 128 messages，checkpoint 普通 suffix 增量扩展，删除/失配/篡改可重建 |
| WebUI | `node --test webui/tests/*.test.mjs` | 67 passed |
| Vue/Tauri UI | `npm run typecheck`；`npm test -- --run`（`ui/`） | typecheck 通过；98 passed |
| 工程工具 | dep-governance；agent-exploration Node tests | 本批未复跑，不从历史结果外推 |
| 仓库/文档 | `git diff --check` | 通过 |

未在本次校准中运行的 production topology、Windows/Linux artifact、真实 provider/browser、网络故障、进程崩溃和真实 provider 长会话，不得由本表推断为通过。

## 7. 文档职责

- [DEV-GUIDE.md](DEV-GUIDE.md)：开发入口、命令、工作流和不变式；
- [PLAN.md](PLAN.md)：稳定产品方向与阶段门，不复制实时 issue 队列；
- [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)、[CONVERSATION-CONTRACT.md](CONVERSATION-CONTRACT.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)：数据和运行时合同；
- [SECURITY.md](SECURITY.md)、[RISK-REGISTER.md](RISK-REGISTER.md)、[WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)：安全、风险和部署边界；
- [README.md](README.md)：完整文档分层与阅读路径；
- `docs/audits/`：PR 级审计原始记录，保留追溯，不作为当前能力清单；
- `docs/archive/`：已完成设计和历史摘要，不能覆盖当前基线。
