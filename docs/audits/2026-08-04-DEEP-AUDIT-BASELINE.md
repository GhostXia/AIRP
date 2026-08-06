# 深度审计基线报告：文档宣称对账 / API 真值 / Bug 清单 / RR 复核

> 审计日期：2026-08-04
> 审计基线：`main@1b14a7c959ea71b8f5ad7841575b200af31894a9`（短 hash `1b14a7c`，PR #454 合并后），工作区审计开始时干净
> 审计依据：AGENTS.md「审计 Agent 守则」三原则——独立审计、可提己见、可质疑历史并查证
> 输入材料：三份前置只读研究报告（文档对账 Alex、实证验证 Tina、架构边界 Eric）+ 桌面 UI 背景研究（Sam/Jack/Ryan）
> 性质：docs 为主 + 一处 Rust 注释修正 + README 端口漂移修正；不修任何 bug；非阻塞项按 AGENTS.md 规则在 PR 合并后建 issue（本任务不建）

## 0. 审计纪律执行说明（必读）

1. 研究报告中的全部行号只作为线索，本报告每条结论均回 `HEAD` 源码亲自复核。**复核期间发现研究报告若干行号与 HEAD 有 ±3~35 行漂移**（研究基于工作区快照，且审计过程中有并行修复任务改动工作区），本报告一律以 `git show HEAD:<path>` 复核后的行号为准，并在 §10 逐条注明与研究结论不符之处。
2. **审计过程中工作区被并行修复任务污染**（约 2026-08-04 01:42 起出现未提交的 `session_recovery` / SSE 契约 / DX-1 effective-root 修复等改动，涉及 26 个文件）。本审计全部测试数字与 HEAD 事实取证均完成于污染之前或对 `HEAD` 对象直接取证；凡引用工作区文件处均已交叉确认该文件未被并行任务改动（`bus.rs`、`WidgetHost.vue`、`api-client.js`、`config.rs`、`tools.rs`、docs/ 全部未动）。
3. 本审计不修 bug。唯一代码改动是 `engine/src/agent/tools.rs` 的一行族计数注释（漂移修正），已通过 `cargo fmt --all -- --check`（exit 0）与 `cargo test -p airp-core --lib tools`（80 passed / 0 failed）确认无破坏。

## 1. 审计基线（本次实际重跑，非引用旧数字）

环境：维护者 D 盘工具链覆写（`RUSTUP_HOME=D:\.rustup`、`CARGO_HOME=D:\.cargo`、PATH 前置 `D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs`），Node 本机 v24.14.0（CI 为 20.19.0，见 §1.3 差异说明）。

| 验证项 | 命令 | 结果 |
|---|---|---|
| Rust 工作区测试 | `cargo test --workspace --no-fail-fast` | **全绿 0 失败**。airp-core lib **1324 passed / 5 ignored**；main 4；集成 agent_run 4、chat_message_branch_endpoint 7、openai_compat 11、production_startup 5、sse_wiremock 5、swipe_endpoint 6；airp-state-protocol 6（含 `rust_wire_discriminants_match_shared_contract`）；airp-ui 9。**合计 1381 passed / 0 failed** |
| Clippy | `cargo clippy --workspace --all-targets --no-deps --locked -- -D warnings` | 干净通过，无警告 |
| 格式 | `cargo fmt --all -- --check` | exit 0（含本审计注释修正后复跑） |
| WebUI 语法 | `node --check webui/assets/*.js` | 23 个文件全部通过（0 语法失败） |
| WebUI 测试（HEAD 跟踪面） | `node --test webui/tests/*.test.mjs` | **76 passed / 0 failed**（8 个被 git 跟踪的 `.test.mjs`，与 CI 命令同口径，与 GitHub `main@1b14a7c` 的 "UI and WebUI" job 日志 `# pass 76 # fail 0` 一致） |
| CI 交叉核验 | `gh run list --branch main` | `main@1b14a7c` 全部 5 个 job success（Rust lint / UI and WebUI / Rust doc / Rust test / Production topology） |

**基线外发现（工作区未跟踪文件，非 HEAD 事实）**：并行任务在工作区新增的 `webui/tests/sse-contract.test.mjs`（未提交）在本机运行报 **2 个确定性失败**（`webui consumer surfaces the contract error envelope`、`webui consumer treats error event and error payload type consistently`）。失败根因恰好证实本审计独立发现的 BUG-8（§5.8）：HEAD 的 `api-client.js` 不解析引擎错误帧的嵌套 `error` 对象。该失败不属于 HEAD 基线，记录为并行任务测试与 HEAD 代码的契约缺口，供其修复任务参照。

## 2. 宣称对账主表（14 篇 docs，逐条「宣称 → HEAD 代码锚点 → 裁决」）

裁决四级：**属实 / 夸大 / 过时 / 未交付但表述模糊**。合同类逐条核验，参考类验页首定位声明。

### 2.1 README.md / README.en.md

| # | 宣称 | HEAD 代码锚点（本审计复核） | 裁决 |
|---|---|---|---|
| C1 | "30 个内置工具"（README.md:29、README.en.md:29） | `agent/tools.rs:195-225` 注册序逐族清点：echo 1 + session 5 + character 3 + state_lorebook 6 + preset 2 + volume_context 2 + analysis 2 + world_event 4 + npc 2 + plot 2 + search 1 = **30** | **属实** |
| C2 | "44 个无构建 WebUI 屏"（README.md:32） | `webui/screens/` 实测 44 个 HTML（01–44） | **属实** |
| C3 | "`--webui-dir` 可换 UI"（README.md:54） | `main.rs:79-83`：loopback-only + 与 `access_api_key` 互斥；`daemon/mod.rs:695-712`（HEAD）注入 `runtime-config.js` 与同源 CSP（`mod.rs:719`）。换 UI 隐含"遵守同一 CSP/注入契约" | **属实（有条件）**：边界条件文档未写明，见 §8 非阻塞 N3 |
| C4 | 开发模式"默认打开 http://127.0.0.1:8765/"（README.md:57、README.en.md:57） | 引擎默认 `daemon_port=8000`（`config.rs:138`、仓库根 `config.json:6`）；README 命令不传 `--port`，实际监听 8000。8765 仅由 `deploy/windows-webui/Start-AIRP.cmd:32`、`deploy/linux-webui/start-airp.sh:39,63` 显式传入 | **过时**——已在本审计修正为 8000（§9） |

### 2.2 CURRENT-BASELINE.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C5 | "Conversation 与 legacy Chat 双轨、未绑产品 UI、冻结扩面"（§2.1 / §5.0，L55-57） | `/v1/conversations*` 13 条 + `/v1/scenes/:scene_id/conversations` 1 条存在（`daemon/mod.rs:481-539`）；grep `webui/assets/*.js` 对 `conversations` **0 命中**；CI 冻结守卫 `engine/tests/conversation_freeze_check.ps1` 存在 | **属实** |
| C6 | "#398 generation-scoped cancel，stale/committing fail-closed"（§2 表 ~L68） | `session_coordinator.rs:124`（`session_busy`）、`:133`（`session_recovery_required`）、`:135`（ULID generation_id）、`:175-181`（`stale_generation`/`generation_committing`） | **属实** |
| C7 | "#399/#403 durable TurnCommit marker；**不能宣称**自动 replay/repair"（~L69） | `turn_commit.rs:195-201`（HEAD）注释明言 "fail closed until a payload-aware replay path exists"；`recover_completed_turn`（:202）只清理终态 marker | **属实**（含自认缺口，诚实） |
| C8 | "#342 backup/restore 最小闭环已交付（82 条 backup 测试通过）"（~L74） | 5 端点齐备（`daemon/mod.rs:653-668`：create/list/get/delete/verify/restore）；backup 测试包含在 1324 lib 测试内本次全绿；PR #445 专项审计已入库 | **属实** |
| C9 | 自认缺口：W-02 restore swap 不持 `character_lock`（#447）、W-03 Windows `sync_dir` no-op（#448）、跨资源一致性 backup 未交付 | 与 LOCK-ORDER-CONTRACT.md:173 登记一致 | **属实**（债务已登记，非隐瞒） |

### 2.3 DEV-GUIDE.md

| # | 宣称 | 锚点 | 裁决 |
|---|---|---|---|
| C10 | "Tauri/Vue 桌面线保留维护、近期发布暂停；恢复前必须重校基线"（DEV-GUIDE.md:23 附近） | 与 CURRENT-BASELINE §1/§6 一致；`ui/src-tauri` 9 条测试本次通过、CI 持续构建 sidecar（暂停但未断链） | **属实** |

### 2.4 TAVERN-PARITY.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C11 | Swipe / Branch / 编辑 / 回滚 / 流式标"已有"（L27-32 表） | 路由 `daemon/mod.rs:343`（completions SSE）、`:358-360`（swipe、branch/switch）、`:347-356`（history/rollback/regen/continue/delete/message）；UI 侧 `chat-space.js` 全部调用命中（§4 diff 表） | **属实** |
| C12 | Impersonate、轮次控制/talkativeness、宏、Author's Note 等标缺口 | 路由表无对应端点，grep 无实现 | **属实**（缺口自认诚实） |

### 2.5 LONG-HISTORY-CONTRACT.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C13 | durable message-ID、legacy 确定性派生、`message_ids` 并行数组（§2） | `chat_store.rs:39-46`（`message_ids` + `derive_legacy_id` 注释契约） | **属实** |
| C14 | cursor `limit`/`before`、rollback-by-ID、窗口按激活路径返回 | `domain/chat.rs:154-162`（`history_window`、limit clamp 1-200、默认 50）；HEAD `handlers/chat.rs:31-35`（limit/before 分流）；rollback message_id/index 二选一（HEAD `types.rs` `validate_rollback_target`）；webui 实传 `limit=50`+`before`（`chat-space.js`） | **属实** |
| C15 | §2.5 自认"history_window 先全量加载再切窗，后续流式分页" | `domain/chat.rs:162` 先 `self.history(...)` 全量 → O(n)，与自认一致 | **属实**（性能债已自认，见 §7 D4） |

### 2.6 SESSION-DATA-DESIGN.md

| # | 宣称 | 锚点 | 裁决 |
|---|---|---|---|
| C16 | 命名 session UUID 身份已交付；"session 自包含/可复现"**分阶段未交付** | 文档头明列 Phase 进度与未交付项；与 RR-011（Open）一致；命名 session 测试通过（`named_session_isolated_from_default_and_delete_does_not_leak`） | **属实**（未交付部分表述明确，无模糊） |

### 2.7 HERMES-MEMORY.md（参考类）

| # | 宣称 | 锚点 | 裁决 |
|---|---|---|---|
| C17 | 页首定位"研究，未交付；AIRP 现有 resident memory/FTS5 为自己实现，不以本文档为准" | 头部声明存在；memory 模块为第一方实现（`engine/src/memory/` 测试全绿） | **属实**（定位诚实） |

### 2.8 LEARN-NEUROBOOK.md（参考类）

| # | 宣称 | 锚点 | 裁决 |
|---|---|---|---|
| C18 | 页首定位"研究参考，不代表 AIRP 实现；许可证 AGPL-3.0，仅理念参考不复制" | 头部声明存在，与 AGENTS.md 第三方吸收规则一致 | **属实** |

### 2.9 WORLDBOOK-SEMANTICS.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C19 | v4 `selective` 运行时语义：二次匹配门、空 secondary 回退 primary、constant 豁免、secondary OR 语义 | `orchestrator/lorebook.rs:129-149` 逐句对应；fixture 测试 `airp_v4_selective_fixture_has_deterministic_output` 通过 | **属实** |

### 2.10 CONVERSATION-CONTRACT.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C20 | Conversation 为 Engine 一级资源；**WebUI 未绑定**；v1 冻结扩面、策略门控 | 14 条相关路由（`daemon/mod.rs:481-539`）；webui 零消费（§4）；`conversation_policy.rs` fail-closed 测试 `unknown_config_and_planning_timeout_fail_closed` 通过 | **属实** |

### 2.11 BACKUP-RESTORE.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C21 | v1 闭环：scope 备份、manifest schema v1、SHA-256 校验、restore + PreRestoreRollback、secret 排除、PreDelete 自动备份 | `engine/src/backup/`（manifest/snapshot）+ `handlers/backups.rs`；`delete_character_creates_pre_delete_backup_by_default`、`restore_scoped_backup_preserves_unrelated_data` 等测试通过；PR #445 审计（含 scoped restore 文档修正 B-01）已入库 | **属实** |
| C22 | W-01~W-06 遗留与 v1 限制 | 与 CURRENT-BASELINE C9 一致，issue 号齐备 | **属实** |

### 2.12 MCP-SERVER-ABSORPTION.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C23 | "默认 Agent registry 30 个内建工具 + 插件动态追加；`GET /v1/agent/tools` 是事实目录"；四层纪律（data/agent/HTTP/UI，#23） | C1 同证（30 个）；`/v1/agent/tools` 路由 `daemon/mod.rs:346`；层级描述与实现一致 | **属实** |

### 2.13 CAPABILITY-ABSORPTION.md（参考类）

| # | 宣称 | 锚点 | 裁决 |
|---|---|---|---|
| C24 | 页首定位"战略筛选清单，不是全量吸收授权；能力现状以 CURRENT-BASELINE 为准" | 头部声明存在 | **属实** |

### 2.14 WEBUI-PRODUCTION-ARCHITECTURE.md

| # | 宣称 | HEAD 代码锚点 | 裁决 |
|---|---|---|---|
| C25 | P0 fail-closed 生产拓扑：生产校验、同源/CSP、CORS 单一 origin、引擎私网 | `config.rs:201-213`（启动期 fast-fail 校验入口）；`production_startup.rs` 5 测试全绿（缺 access key 拒启动、非 loopback 拒绝等）；CORS `daemon/mod.rs:746-765`；`deploy/production/` + CI Production topology job success | **属实** |

**对账统计（25 条）**：属实 23（其中 2 条为"属实（有条件/自认缺口）"）、夸大 0、过时 1（C4，已修正）、未交付但表述模糊 0。参考类文档页首免责定位全部诚实；合同类"未交付"边界声明与源码一致。

## 3. API 真值清单（HEAD `daemon/mod.rs:342-676`，全量导出）

真值源：`engine/src/daemon/mod.rs` HEAD，`v1_routes` 自 :342 起，:669-672 `auth_middleware` route_layer（全部 `/v1/*` 带 bearer 鉴权门），:674-676 Governor 限流（10 req/s burst 语义），:679-680 `/version`、`/health` 不经鉴权。默认 body limit（axum 2MB）除显式标注外。

| 方法 | 路由 | body-limit 特化 |
|---|---|---|
| POST | /v1/chat/completions（SSE） | 默认 |
| POST | /v1/chat/preview | 默认 |
| POST | /v1/agent/run；GET /v1/agent/tools | 默认 |
| POST | /v1/chat/history、/session-state、/cancel、/rollback、/regen、/continue、/delete、/swipe、/branch/switch、/search | 默认 |
| PUT | /v1/chat/message | **2MB 显式** |
| GET/POST | /v1/characters；POST /v1/characters/import | import **10MB** |
| GET/PUT/DELETE | /v1/characters/:character_id | PUT 2MB |
| POST | /v1/characters/:character_id/reextract | 默认 |
| GET | /v1/characters/:character_id/avatar、/lorebook（PUT 2MB）、/state、/state/history、/state/schema、/world-events、/plot-arc（PUT 2MB）、/images、/images/:filename、/sessions/:session_id/images/:filename、/revisions、/revisions/diff、/revisions/:revision_id、/lorebook/graph、/drift（PUT）、/drift/rollback（POST）、/analysis、/analysis/*filename（POST 1MB） | — |
| POST | /v1/image/generate（2MB）、/v1/characters/:character_id/dialogue-examples（**64KB**） | 显式 |
| GET | /v1/character-templates、/:template_id；POST /:template_id/instantiate（2MB） | — |
| GET/POST | /v1/scenes；GET /v1/scenes/:scene_id；POST /v1/scenes/:scene_id/characters | — |
| GET | /v1/models、/v1/presets、/v1/presets/:preset_id；POST /v1/presets/import（2MB） | — |
| GET/PUT | /v1/users/:user_id/persona；GET /persona/effective；GET/POST /personas；GET/PUT/DELETE /personas/:persona_id；POST/DELETE /personas/:persona_id/bindings | — |
| GET/POST | /v1/sessions/:character_id；DELETE /v1/sessions/:character_id/:session_id；GET /:session_id/timeline、/timeline/export | — |
| Conversation 家族（13 条） | /v1/conversations（GET/POST 2MB）、/v1/conversation-policies、/v1/conversation-capabilities、/v1/conversation-migrations/plan（POST 2MB）、/v1/conversation-migrations（POST 2MB）、/:migration_id/export、/:migration_id/rollback、/v1/conversations/:id、/:id/events（POST 2MB）、/:id/turns（POST 2MB）、/:id/turns/:turn_id、/:turn_id/observability、/:turn_id/cancel | `mod.rs:481-536` |
| POST | /v1/scenes/:scene_id/conversations（2MB） | `mod.rs:538` |
| GET/POST | /v1/settings | 默认 |
| GET/POST | /v1/providers（POST 2MB）；GET /v1/providers/resolve；GET/PUT /v1/provider-routing（PUT 2MB） | — |
| GET/POST/DELETE | /v1/plugin-tools（POST 2MB）、/:name、/:name/test（POST **1MB**） | — |
| POST | /v1/style/review、/v1/style/learn（2MB）；GET /v1/style/profiles、/:profile_id | — |
| GET/PUT | /v1/memory/resident（PUT 2MB）、/v1/memory/user-model（PUT 2MB） | — |
| POST | /v1/onboarding/complete | 默认 |
| POST | /v1/characters/:character_id/decompose（1MB）、/v1/presets/:preset_id/decompose（1MB） | — |
| GET/POST/DELETE | /v1/backups（POST **64KB**）、/:backup_id、/:backup_id/verify（POST）、/:backup_id/restore（POST） | `mod.rs:653-668` |

### 3.1 与 webui/assets/*.js 调用面双向 diff（全量提取 `client.request/stream` 与字符串拼接）

**A. 引擎独有、webui 零消费**：

| 端点 | 定性 |
|---|---|
| `/v1/conversations*`（13 条） | **有意双轨**（C5/C20），非缺陷 |
| `/v1/scenes/:scene_id/conversations` | **死路由候选**：group-chat.js 走 legacy `/v1/scenes` + `/v1/sessions` + `/v1/chat/completions`（`group-chat.js:70,78,127,159`），scene→conversation 绑定无人消费 |
| `/v1/characters/:character_id/avatar` | webui 头像用首字母占位（`chat-space.js:216-219`），端点未被消费 |
| `/v1/users/:user_id/persona/effective` | 0 消费（persona 页走 `/personas` 显式列表） |
| quota | **无任何 HTTP 面**（`daemon/mod.rs:185` 仅内部 `QuotaConfig`；屏 21 只编辑 settings JSON），与基线"配额策略非实时账单"自述一致 |

**B. webui 幻影调用（UI 有、引擎无）**：**未发现**。全部 webui 端点（含 backups verify/restore、drift、revisions/diff、timeline/export、lorebook/graph、reextract、dialogue-examples、decompose、analysis enhance/apply）均命中路由表。

**C. 双端一致**：chat 全家族 13 条、characters/lorebook/state/plot-arc、sessions、presets(+import)、providers(+resolve/routing)、plugin-tools、style 4 条、memory 2 条、models、settings、onboarding/complete、image/generate、agent/run+tools、character-templates(+instantiate)、scenes。

## 4. Bug 清单（按严重度；均为 HEAD 事实，只记录不修）

### BUG-1 · 多用户数据根不对称：history 与 rollback 不走 effective root（高）

- **现象**：带 `user_id` 的多用户隔离下，history 读取与 rollback 写入落在与 mutation 不同的数据根；Coordinator lease key 前缀不一致。
- **证据（HEAD）**：`handlers/chat.rs:27-44` `get_chat_history` 恒用 `state.data_root`（:32/:40），不 `resolve_effective_root`；`handlers/chat.rs:76-86` `rollback_chat` 注释 "RollbackRequest intentionally has no user_id"，:86 `effective_root = state.data_root.clone()`；HEAD `types.rs` 的 `RollbackRequest` 无 `user_id` 字段。同族 session-state/cancel/regen/continue/delete/swipe/edit/branch 全部解析 effective root。`data/users/default/` 目录已存在，路径现实可达。
- **修复归属**：**阻塞级候选**（数据一致性）。并行修复任务已在工作区实现 DX-1 对齐修复（未提交），本审计确认其方向正确；以其 PR 走正常门禁。

### BUG-2 · TurnCommit 非终态 marker → 会话永久 fail-closed，无产品级逃生（高）

- **现象**：commit 中途崩溃留下非终态/不可读 marker，`session_recovery_required`（`session_coordinator.rs:133`）永久拒绝 mutation；无自动 replay/repair、无 UI 恢复入口，唯一逃生是手工冷备份 runbook。
- **证据（HEAD）**：`turn_commit.rs:195-201` 注释 "until a payload-aware replay path exists"；`recover_completed_turn`（:202）仅清理终态 marker。CURRENT-BASELINE #409 已登记为已知残留（#286/#394 未交付）。
- **修复归属**：**阻塞级候选**（可用性锁死）。并行任务工作区已出现 `session_recovery` handler（未提交），方向一致。

### BUG-3 · Tauri SSE 丢失 done 终态：无法区分"完成"与"中断"（中高）

- **证据**：引擎终态帧 `{"type":"done"}`（HEAD `stream.rs:240`）；`bus.rs:559-573` `EngineChunk` 只有 body/think/action_options 三变体 → done 帧反序列化失败被静默 `continue`（`bus.rs:678-682`），`run_chat_stream` 靠连接关闭返回 `Ok(())`（`bus.rs:700-705`）。违背 UI-PROTOCOL-DECISION"错误落到明确边界"。
- **归属**：非阻塞（桌面线暂停中），记入 issue。

### BUG-4 · Tauri 错误帧保真度降级（中）

- **证据**：引擎错误帧含结构化 `{code, retryable, commit_state}`（HEAD `stream.rs:223-238`）；`bus.rs:671-673` 拍平成字符串 `"engine stream error: {data_line}"`。同一合同两个 UI 两种保真度。
- **归属**：非阻塞，issue。

### BUG-5 · Tauri legacy 单轨漂移：无 session_id / generation_id（中）

- **证据**：`bus.rs:611-615` `chat.send` 请求体不含 `session_id`；`bus.rs:384` 注释自认"只读 legacy 单 session"。停留在 SESSION-DATA-DESIGN 兼容豁免轨道，与 #398/#399 主路径脱节。
- **归属**：非阻塞，issue（桌面恢复前置项）。

### BUG-6 · widget 沙箱缺省关闭：第三方 esm 默认进程内执行（中）

- **证据**：`ui/src/components/WidgetHost.vue:34-36` `sandboxed = manifest.entry.sandbox === true`（opt-in），:32-33 注释 "in-process esm stays the default"。第三方 esm widget 未显式声明 sandbox 即与宿主同进程运行，consent 只控 capability 不控隔离。
- **归属**：非阻塞（扩展面未开放，RR-007 方向已登记），issue 建议"缺失即沙箱"反转默认。

### BUG-7 · 破坏性操作依赖 window.confirm，交互一致性缺失（低中）

- **证据**：15 处原生 confirm——`console-runtime.js:263,416,504,760,907,971,988`、`chat-space.js:458,466,508`、`provider-management.js:295,335`、`plugin-tools.js:431`、`role-list.js:59`。无统一确认组件、无撤销路径，与 STYLEGUIDE 的设计稿确认弹窗（15-confirm-modal）不一致。
- **归属**：非阻塞，issue（webui 体验债）。

### BUG-8 · webui 丢失引擎错误信封结构化字段（code/retryable/commit_state）（中，本审计新发现）

- **现象**：引擎错误帧把结构化字段嵌在 `payload.error` 下（HEAD `stream.rs:229-238`），但 `api-client.js:97` `throw new AirpStreamError(errorMessage(payload,…), payload)` 传入的是顶层 payload；`AirpStreamError` 构造器（`api-client.js:17-25`）读 `detail.code/retryable/commit_state`——顶层不存在这些键，恒为 `undefined/false/undefined`。
- **后果**：`chat-space.js:594` 依据 `error.commitState` 判定 "partially_committed/unknown" 的 fail-closed 提示对**引擎来源错误永远失效**（仅客户端自造的 stream_incomplete/stream_transport 携带 commitState）。研究报告 Eric 的"webui 完整保留 commit_state（api-client.js:17-25）"结论在引擎错误路径**不成立**。
- **旁证**：并行任务新增（未提交）的 `webui/tests/sse-contract.test.mjs` 两个用例对 HEAD 代码确定性失败，失败根因即本条。
- **归属**：非阻塞（UI 提示保真度，非数据损坏——引擎侧写入语义不受影响），issue。

### 次级项（已登记债务，复核确认仍存在）

| ID | 内容 | HEAD 证据 |
|---|---|---|
| W-02 | restore swap 阶段仅持 `BACKUP_LOCK` 不持 `character_lock` | LOCK-ORDER-CONTRACT.md:173；CURRENT-BASELINE #342 行（#447） |
| D2 | std 锁内同步 fs I/O 阻塞 tokio worker（agent tool 路径，A3 debt） | CURRENT-BASELINE #436 行自认 `advance_plot` 等路径；LOCK-ORDER-CONTRACT §6.2 |
| D3 | R1/R2 锁序仅 `debug_assertions` 强制，release 无运行时强制 | CURRENT-BASELINE #436/#441 行自认 |
| D4 | `history_window` 全量 load 后切窗，长历史 O(n) | `domain/chat.rs:162`；LONG-HISTORY-CONTRACT §2.5 自认 |
| D5 | 死路由 `/v1/scenes/:scene_id/conversations` 等（§3.1-A） | `daemon/mod.rs:538` + webui 0 消费 |
| D6 | quota 无实时用量 HTTP 面 | `daemon/mod.rs:185` 仅内部配置 |
| D7 | widget consent 授权持久化在 localStorage（`ui/src/registry/consent.ts:13`），跨浏览器/清缓存即失忆，非引擎权威 | consent.ts:13,27-39 |

## 5. RR-001~014 逐条复核（Current control → 对应测试/证据 → 裁决）

| RR | Current control（登记） | 复核证据（HEAD） | 裁决 |
|---|---|---|---|
| RR-001 card_path | Tauri 官方文件对话框取路径，引擎校验 PNG/JSON | `production_startup::production_daemon_rejects_local_path_import_before_serving` 通过；RR-013 浏览器包禁用 `AIRP_ALLOW_LOCAL_PATH` | **控制成立**（信任前提=本地可信 UI；多客户端暴露前须补 caller 能力检查，登记一致） |
| RR-002 明文密钥 | runtime-only、响应脱敏、序列化省略 | `error::tests::into_response_500_redacts_message`、`into_response_path_escape_redacts_server_path`、`upstream_error_envelope_is_versioned_recoverable_and_redacted`、`models.rs:311` `redact_endpoint_clears_userinfo_password_query_fragment` 全通过 | **缓解属实** |
| RR-003 本地 origin/鉴权 | loopback + 限流；桌面进程级 bearer | `main.rs` 测试 `local_webui_requires_loopback`、`local_webui_rejects_access_authentication` 通过；Governor 全 /v1 覆盖（HEAD `mod.rs:674-676`） | **缓解属实**（loopback≠身份，登记措辞诚实） |
| RR-004 写路径/原子性 | 共享服务 + 原子替换 + 锁；跨资源事务未交付 | `concurrent_appends_do_not_lose_messages`、`concurrent_append_and_rollback_no_half_state`、`delete_session_serializes_with_concurrent_appends` 通过；TurnCommit 是进度记录非事务（BUG-2） | **部分缓解属实**，Open 部分诚实 |
| RR-005 state schema | StateService 写前校验 | `state_service_validates_schema_and_assigns_revisions`、`state_schema_without_properties_rejects_all_additional_fields` 通过 | **缓解属实** |
| RR-006 sidecar 生命周期 | Tauri 持有/轮询/终止 | airp-ui 9 测试通过（含 `sidecar_settings_reads_port_but_not_plaintext_access_key`）；packaged smoke 仍为 release 门 | **缓解属实** |
| RR-007 协议/能力权威漂移 | 双侧单测 + 运行时 guard | Rust 侧 `rust_wire_discriminants_match_shared_contract` 通过；**webui 无协议守卫**——纯字符串端点契约（§3），端点漂移仅运行时暴露；BUG-8 即此类漂移的现症 | **部分缓解属实**，webui 面缺口为本审计重点确认项 |
| RR-008 PR 质量门禁 | pr-gate.yml 全套 + dep-governance 手工 | `main@1b14a7c` 5 job 全 success（gh 实证）；SBOM 未接 release 强制门（登记一致） | **缓解属实**，Required direction 未闭合 |
| RR-009 生产网关/引擎权威 | 网关替换 Authorization、引擎私网、fail-closed | Production topology CI job success；`deploy/production/*.test.mjs` 22 测试在 CI 通过；`production_startup` 5 测试 | **P0 控制属实**，P1-P3 Open 诚实 |
| RR-010 前端工具链漏洞 | lockfile 硬化、0 advisory | CI UI job 绿；`npm audit` 前后对比记录在 RR-010 文本（本审计未重跑 npm audit，注明"未重跑"） | **缓解属实（CI 证据）** |
| RR-011 session 快照完整性 | 身份/布局清理已交付，自包含未交付 | Open 与 SESSION-DATA-DESIGN C16 一致 | **Open 属实** |
| RR-012 preview 泄漏/副作用 | 真实装配路径但只回有界元数据、无写入 | `chat_preview_returns_redacted_trace_without_writes`（daemon/tests/chat.rs:511）、`preview_pipeline_is_write_free_and_traces_actual_payload`（chat_pipeline/tests.rs:165）通过 | **缓解属实** |
| RR-013 便携包数据边界 | 包内固定数据根、同源、清环境变量、smoke | `deploy/windows-webui/smoke-package.ps1` 存在；release 级证据门未闭（登记一致） | **初版控制属实** |
| RR-014 插件网络/本地代码 | DNS fail-closed + 请求时重解析 pin、loopback-only 明文、路径 canonicalize | `plugin_tool.rs:1235` `resolve_public_host_addrs_fail_closed_on_dns_error_empty_and_internal` 通过；Residual（TOCTOU/非沙箱）登记诚实 | **部分缓解属实** |

## 6. 耦合审计结论

1. **protocol crate 是桌面线专属**：`airp-state-protocol` 仅被 engine（Capability 类型，`agent/tools.rs:34`）与 Tauri shell（`ui/src-tauri/src/bus.rs`）消费；webui 完全绕过协议层直连 REST+SSE。"可换 UI"实际靠 `/v1/` REST 契约维系。此为被 SOURCE-PROJECT-DECISION §1 允许的取舍，非侵蚀，但代价是 RR-007 的 webui 缺口。
2. **三份手写 SSE 消费端**（webui `api-client.js:56-126`、Tauri `bus.rs:638-705`、`deploy/production/sse-consumer`）解析同一引擎合同（HEAD `stream.rs:206-244`），无共享机器可校验产物——BUG-3/4/8 全部源于此漂移面。协议层已有 `wire-discriminants.json` 先例，HTTP/SSE 面缺同等级机器产物（RR-007 Required direction）。
3. **端口口径分裂已收敛为两处**：引擎默认 8000（config.rs:138、Tauri `main.rs:24`、config.json），deploy/便携包显式 8765。README 漂移（8765）已在本审计修正（§9）。
4. **Conversation 双轨债**：conversation 家族大文件与 ChatService 长期并行，冻结扩面决策执行中（CI 守卫在位），属已登记债务。
5. **存量巨石**：`chat_store.rs`、conversation 家族、`plugin_tool.rs`（~66KB）——CURRENT-BASELINE §2.1 已自认，文档与代码自洽。

## 7. 阻塞项 / 非阻塞项分离总结

**阻塞项（应先进入修复 PR，走正常审计门禁）**：

| ID | 摘要 | 状态 |
|---|---|---|
| BUG-1 | history/rollback 数据根不对称（多用户数据一致性） | 并行修复任务进行中（工作区未提交），本审计确认 HEAD 存在该 bug |
| BUG-2 | TurnCommit 锁死无产品级逃生 | 并行任务 `session_recovery` 方向进行中；在修复落地前，本条维持阻塞级候选 |

本审计自身改动（README×2 端口、tools.rs 注释）**不引入**阻塞项：docs + 注释级，`cargo fmt --check` exit 0、`cargo test -p airp-core --lib tools` 80 passed，webui 零改动。

**非阻塞项（PR 合并后按 AGENTS.md「审计遗留项处理」建 issue，本任务不建）**：BUG-3/4/5（Tauri SSE 漂移，桌面恢复前置）、BUG-6（widget 沙箱默认反转）、BUG-7（window.confirm 统一化）、BUG-8（webui 错误信封结构化字段映射，建议修法：`api-client.js` dispatch 对 `payload.error` 解嵌套传入 AirpStreamError）、次级项 W-02/D2/D3/D4/D5/D6/D7、RR-007 webui 契约守卫（建议：从路由表导出端点 golden fixture 进 CI，可复用 `webui/tests/extract-v1-endpoints` 思路）、RR-008 SBOM release 门、C3 `--webui-dir` 边界条件补文档、`webui/README.md:10-13` 的 `--port 8765` 示例建议加注"引擎默认 8000"（本次未改，保持 deploy 口径稳定）。

## 8. 本审计的文档漂移修正清单（同批提交）

| 文件 | 改动 | 依据 |
|---|---|---|
| `README.md:57` | dev 默认 URL 8765 → 8000，注明引擎默认 `daemon_port=8000` 与 deploy 脚本显式传参口径 | `config.rs:138`、`config.json:6`；deploy 脚本显式 8765 保留不动 |
| `README.en.md:57` | 同上（英文版） | 同上 |
| `engine/src/agent/tools.rs:214` 注释 | "世界事件触发器 family 2 工具" → "family 4 工具（trigger_world_event / list_world_events / advance_clock / get_clock）" | `agent/tools/world_event.rs:48,168,233,315` 实注册 4 工具 |

**保留不改（核验后确认不冲突）**：`docs/SECURITY.md:26,30`（便携 launcher 实际绑 8765，表述正确）；`deploy/windows-webui/Start-AIRP.cmd:32`、`deploy/linux-webui/start-airp.sh:39,63`、`deploy/windows-webui/smoke-package.ps1`（显式传参）；`docs/audits/2026-07-20-PR-255-*.md`（历史审计记录）；`webui/README.md:10-13`（命令自带 `--port 8765`，自洽，仅建议加注，见 §7）；`ui/smoke-windows-installer.ps1`（18765，独立测试端口）。

## 9. 与研究报告结论不符之处（以本审计复核为准）

| # | 研究结论 | 本审计复核 |
|---|---|---|
| 1 | Alex：BUG-1 证据行号 `handlers/chat.rs:30-48,89-92`；"RollbackRequest 干脆没有 user_id 字段（types.rs:106-114）" | **结论方向在 HEAD 成立**，但 HEAD 精确行号为 `chat.rs:27-44`（history）、`chat.rs:76-86`（rollback，"intentionally"注释在 :83-85）；HEAD `types.rs` RollbackRequest 确无 user_id。注意：审计中途工作区出现并行修复（DX-1），若复核工作区会误判"已修"——HEAD 未修 |
| 2 | Tina：webui "76 通过 / 0 失败"、Rust "约 1381" | 本审计重跑**精确复现**：76/0（HEAD 跟踪测试面）、1381/0。但 Tina 若把并行任务未提交的 `sse-contract.test.mjs` 计入则会见到 2 失败（§1 基线外发现） |
| 3 | Eric B2："webui 完整保留 commit_state（api-client.js:17-25）" | **不成立（引擎错误路径）**：HEAD `api-client.js:97` 传顶层 payload，嵌套 `error.{code,retryable,commit_state}` 丢失 → BUG-8。webui 只在客户端自造错误上有 commitState |
| 4 | Alex/Eric 多处行号：`daemon/mod.rs:377-716`（路由）、`737-760`（CSP）、conversations `517-577`、backups `692-708`；Eric：`stream.rs:229-244` | HEAD 实际：v1_routes `342-676`、runtime-config/CSP `695-744`、conversations `481-539`、backups `653-668`、done 帧 `:240`、错误信封 `:223-238`。研究行号来自含并行改动的工作区快照或更早树，漂移 +9~+35 行；结论本身不受影响 |
| 5 | Alex："webui/assets 42 个文件" | HEAD 实际 23 个 `.js`（assets 目录另有 css 等；Tina 的"42 个文件"口径未复现） |
| 6 | Alex：`RollbackRequest` handler 注释承认 "intentionally"（chat.rs:89-92） | HEAD 注释在 `chat.rs:83-85`，措辞为 "RollbackRequest intentionally has no user_id"——**是刻意设计而非疏忽**，但这恰是 BUG-1 的多用户不一致根源，设计上应重新评估 |
| 7 | Tina：`turn_commit.rs:195-201` | HEAD 一致（注释块 195-201，`recover_completed_turn` :202），无误 |

## 10. 验证命令（本审计实际执行）

```text
git rev-parse HEAD                                   # 1b14a7c959ea71b8f5ad7841575b200af31894a9
cargo test --workspace --no-fail-fast                # 1381 passed / 0 failed（含 airp-core lib 1324+5 ignored）
cargo clippy --workspace --all-targets --no-deps --locked -- -D warnings   # 干净
node --check webui/assets/*.js                       # 23/23 通过
node --test <8 个被跟踪的 *.test.mjs>                 # 76 passed / 0 failed
cargo fmt --all -- --check                           # exit 0（注释修正后）
cargo test -p airp-core --lib tools                  # 80 passed / 0 failed（注释修正后）
gh run list --branch main / gh run view --json jobs  # main@1b14a7c 5 job success
git show HEAD:<path>                                 # 全部 HEAD 行号取证方式（规避工作区污染）
```

## 11. 裁决

**基线健康**：`main@1b14a7c` 测试真绿（1381/0 + 76/0）、clippy/fmt 干净、CI 与宣称一致、14 篇文档诚实度高（25 条宣称仅 1 条过时且已修正）。**必须优先投入**：BUG-1（多用户数据根，阻塞级）与 BUG-2（会话锁死逃生，阻塞级）——两者均已有并行修复在途；BUG-8 为本审计新增的中优先级契约漂移。**桌面线**维持暂停定性，恢复前以 BUG-3/4/5 + RR-006 packaged smoke 为前置。非阻塞遗留项在本 PR 合并后按 AGENTS.md 规则整理建 issue。
