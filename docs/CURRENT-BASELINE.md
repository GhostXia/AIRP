# AIRP 当前开发基线

> 基线日期：2026-07-20（PR #268 合并后校准）
>
> 前置实现基线：`main@7895f8c`；本次校准吸收 PR #233–#268 的全部变更
>
> 用途：新开发 session 的第一事实入口。源码、manifest、测试和可重复运行证据高于本文；GitHub issues 是未完成工作的实时追踪面。

## 1. 产品与仓库边界

AIRP 是专精 Role Play 的 AI Agent 客户端，当前采用“无头 engine + 可换 UI”结构：

- `engine/`（`airp-core`）：唯一 RP/Agent 内核，负责数据、prompt 装配、LLM adapter、Agent loop 与 HTTP/SSE；
- `webui/`：当前正式产品交付主面；
- `ui/` + `ui/src-tauri/`（`airp-ui`）：保留的 Tauri/Vue 桌面客户端，属于长期开发线，不是近期交付载体；
- `protocol/`（`airp-state-protocol`）：UI/engine 共用线协议类型；
- `deploy/windows-webui/`：当前优先的 Windows 便携 WebUI artifact；
- `deploy/production/`：保留的单实例自托管 P0 preview 资产，不是当前落地前置条件；
- `data/`：运行时数据根规范与仓库内安全样例，不是共享素材目录。

AIRPCLI、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 原仓库是作者自己的第一方前序项目。统一按“吸收资产，不继承产品北极星”处理，不是当前 runtime 依赖或必须逐项复刻的清单。

## 2. 当前已交付能力

### Engine 与 Agent

- OpenAI-compatible / Anthropic 流式 adapter，`/v1/chat/completions` SSE，以及有 step/token/wall-clock/cancel 边界的 `/v1/agent/run`。
- 默认 Agent registry 有 21 个工具；运行时目录由 `GET /v1/agent/tools` 提供，执行受 capability、allowlist 与 destructive confirmation 约束。
- Agent tool 已按 session/character、state/lorebook、volume/context、analysis、preset 等职责拆分；`get_preset` 与 `update_preset` 已注册，后者支持 dry-run 且实际写入要求确认。daemon handler 已按 settings、presets、scenes、models、sessions、personas、chat、agent、characters、state、lorebook 拆分。
- daemon HTTP 合同测试已按 catalog、chat、health/settings、persona、security、sessions、state/scene 分组，覆盖主要 route、校验和安全边界。
- settings 更新以专用异步事务锁串行化“候选配置 → 原子落盘 → live config 提交”；校验/写入失败不产生部分内存更新，并发提交结束后磁盘与运行态来自同一提交。Rustdoc 坏链接、无效 HTML 与其他 warning 已纳入 `RUSTDOCFLAGS=-D warnings` 的 workspace CI 门禁。
- 干净提示词不变式 `subagent_context_has_no_orchestrator_noise` 仍是阻塞门禁：Agent 编排脚手架不得进入 RP 角色平面。
- 关键持久化路径已加固（PR #219/#227/#232）：`quota::check_and_increment` / `record_tokens` 用进程级 `Mutex` 串行化 load-mutate-save，poison 时统一 `into_inner()` 恢复；`chat_store::append_message` 加 `sync_data`，`fs::write(chat.jsonl)` 改用 `replace_file` 实现 tmp+sync_all+rename+parent-dir sync 的崩溃原子性；`replace_file` 自身补 parent-dir fsync（Unix），tmp/backup 扩展名保留原后缀；`volume_store::next_volume_number` 用 `saturating_add(1)` 防 u32 溢出回卷；`extract_card_assets` 在新卡 `character_book` 缺失/空/规范化失败时**保留旧 lorebook**，仅在显式无 `character_book` 字段时删除；`update_character_card` 在存在性检查前获取 `character_lock(cid).write()`，与 `LorebookService::write` / `StateService::write` 锁纪律对齐。chat pipeline 现在先持久化 user message、再推进时间线；assistant 的 live state、ChatLog 与 `current.md` 任一关键写入失败都会硬失败，不再 warn 后继续或发送虚假 `done`。SSE 错误携带稳定 code、retryability 与 commit state，只有 engine 明确确认未提交时 WebUI 才允许重发。
- 工程治理基础设施已落地（PR #218）：`tools/dep-governance/` 提供 Cargo workspace + npm package-lock.json v3 依赖发现、纯函数审计路由（auto-pass/audit-required/block + 五类升级路由）、SPDX-2.3 与 CycloneDX 1.5 SBOM 生成器和人类可读第三方声明；当前 SBOM 快照存于 `docs/sbom/`。GitHub Actions 已升级到 `actions/checkout@v7` / `setup-node@v6` / `upload-artifact@v7`（这些 v6/v7 action wrapper 自身跑在 Node 24 上；workflow step 显式 `node-version: 20.19.0` 不变，仍由 setup-node 在 runner 上配置 Node 20.19 给 UI/Vitest/WebUI 测试使用）。
- chat pipeline 已模块化（PR #253）：原 1799 行单文件拆分为 `chat_pipeline/` 下 10 个子模块（finalize、generation_step、helpers、prepare、prepare_scene、state_extract、stdout_runner、stream、trace、types），保持外部 API 不变。
- Agent `update_preset` 在变更 context bundle 前先验证 preset 合法性（PR #260），拒绝无效 preset 写入。
- Docstring 覆盖政策已落地（PR #261/#267）：rustdoc `missing-docs` 为维护清单（仅 public item），CodeRabbit docstring coverage 为参考信号（含 private/test），两者分层处理，均非 CI 门禁。

### RP 数据与会话

- 角色卡 JSON/PNG 导入、角色 CRUD、preset、scene、state、volume、decompose/analysis 和基础 worldbook 已有共享服务或 HTTP 能力。
- Preset 导入已输出规范化报告并保留 BOM 清理后的原始输入 sidecar；Agent 更新把 canonical/raw 写入不可变版本目录，再以单一原子 `current` 指针切换活动版本。`decompose_preset` 优先读取规范化版本，并兼容 legacy 布局。
- `PromptAssemblyTrace` 已接入真实 single/scene chat 装配路径，按 provider payload 顺序记录 card/persona/lorebook/state/preset/scene/memory/history/user 的显式 provenance；Phase 2h (#215) 把 `EffectiveIds` 的 6 类 `*_revision` 字段（character/persona/preset/lorebook/state/memory）全部接到统一 `content_revision` 合同，新数据填充实际 u64，旧数据或读取失败时推送 `*_revision_unavailable` 诊断；scene 模式下 character/lorebook/state/memory revision 留 `None`（多角色无单一 revision），不推送诊断。scene 合并角色与场景 worldbook 后仍按命中条目保留逻辑来源和源文档条目序号，不暴露本机路径。`POST /v1/chat/preview` 复用该路径但不推进时间线、不创建会话或修复 metadata，且不返回 prompt 正文、API key 或 endpoint 明文。
- 命名 session 使用外层目录 UUID 作为 session 目录、history 响应和 `chat_log_meta.json` 的唯一规范身份；旧双 UUID metadata 会 best-effort 原子修复，损坏 metadata 不阻塞仍可读取的历史。
- Chat history 已有 session-scoped durable message ID、完整 JSONL 保留、`limit`/`before` cursor、legacy deterministic ID 与 rollback-by-ID。非法 rollback index 在 service/API 和 `ChatLog` 持久化边界均被拒绝。
- Persona 删除先验证 `persona_id` 再构造删除路径，revision 清理只忽略 `NotFound`；路径穿越、权限或其他 I/O 失败均 fail-closed，并保留工作副本。
- 新数据根不再创建根级 `world.md`/`items.md`；新角色不再创建 legacy `worldbooks/`。角色默认世界书的规范位置是 `characters/{character_id}/world/lorebook.json`。
- 自包含 session、角色卡/世界书工作副本、统一 `content_revision`、`AIRP-TREE-SHA256-v1`、JSONL 崩溃恢复与完整导出已经形成接受合同，但除命名 session 身份、history/memory 隔离和 Phase 2 (#115) 6 类 revision 合同外仍需分阶段实现。详见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)。
- 统一 revision 合同已覆盖 Preset（Phase 2b/#202）、Character/Worldbook/State/Memory/Persona（Phase 2c-2g/#203）与 trace 收口（Phase 2h/#215）：`commit_revision` helper + `AssetKind` enum + 单调 u64 `content_revision` + 不可变 `revisions/{N}/` 快照 + 原子 `current_revision` 指针；`next_content_revision` (#206) 在 orphan revision_dir（提交第 5 步后、第 8 步前崩溃留下的快照）场景下取 `max(pointer, 磁盘最大 revision_dir) + 1` 跳过 orphan，避免 asset 永久不可写。所有 service 保留 legacy 工作副本，revision 作为增量叠加；lazy migration 在首次写入时从 revision 1 起。

### Persona、Worldbook 与 WebUI

- 多 Persona 存储、revision、默认/角色/session 绑定、plural HTTP CRUD、legacy singular API 兼容，以及 chat pipeline 的 explicit → binding → default 激活顺序已交付。
- WebUI 已有「自动（跟随绑定/默认）」与显式 Persona 选择、effective source/双 scope owner 展示、角色/session 绑定与解绑，并始终在聊天 payload 传当前 `user_id`；显式选择才传 `persona_id`。服务端在同一 per-user snapshot 解析 owner，并在原子保存边界拒绝同 scope 多 owner。
- Swipe 多候选 + Smooth Streaming 已交付（PR #249/#251）：用户可在多个候选回复之间滑动切换，流式输出采用平滑渲染（按句子/段落边界刷新而非每 token 重建 DOM）。
- 每消息操作与对话控制已交付（PR #250）：auto-regen（SSE 断流自动重连）、continue（继续生成）、per-message actions（复制/重试/编辑/删除）、单条消息删除。
- Windows 便携 WebUI 已交付（PR #238/#243）：`Start-AIRP.cmd` 一键启动、provider key 写入包内 `data/secrets.json`、PowerShell 启动器已移除、环境变量清理与 `AIRP_ALLOW_LOCAL_PATH` 显式禁用。
- Worldbook v4 已实现 `enabled && (constant || (primary_match && (!selective || no_valid_secondary_keys || any_secondary_match)))`；`secondary_keys` 使用 OR/any-match，空集合退化为 primary-only，`constant` 跳过 selective gate。
- shared normalizer 统一 PNG、PUT API、Agent tool 三入口，将 top-level 或 v3 `extensions.selective` presence-aware 提升为 canonical 字段，并保留 `case_sensitive`、position/depth/probability/递归等尚未执行的 advisory metadata；保留字段不等于执行兼容。
- Worldbook 编辑器已迁入 character-scoped 普通用户主面板，可编辑 v4 `selective`/`secondary_keys`，只读展示 advisory 字段，并有未保存修改确认、异步响应防串角色和 429 可恢复提示。production browser smoke 覆盖容器静态资产、字段切换、只读展示和 malformed/legacy 响应；V3 PNG/JSON character_book 的 `constant` 条目已有导入到最终 system prompt 的端到端回归。
- WebUI 已有 provider 配置、角色导入、Preset 选择/JSON 导入、session 生命周期、流式聊天、Agent Run、非敏感恢复、50 条首屏窗口、加载更早、durable-ID DOM 复用与按消息回滚；对话主面板现可查看本轮实际 Persona/Preset/Provider/Model，以及有序装配材料、稳定性和估算规模。本轮装配 chips 已从 5 项扩展到 10 项（card/persona/preset/lorebook/state/memory + 模型/服务 + 温度/最大 tokens），身份/模型/服务/温度/最大 tokens chip 附带来源后缀（如 `· 显式`/`· 预设`/`· 请求`），在对应 asset 有 `*_revision_unavailable` 诊断时显示 `unavailable` 标识，未激活 asset 显示「未启用」。#114 统一有效配置摘要已交付：Persona 激活来源（explicit/session_binding/character_binding/default/absent）与参数来源（request/preset/snapshot）由 engine 侧 `PersonaService::resolve_effective_persona` 与 `resolve_param_sources` 填充，与 HTTP effective 端点同源。
- WebUI 已有首次启动 onboarding wizard（[#209](https://github.com/GhostXia/AIRP/issues/209) / PR #212）：6-stage 状态机（部署健康检查 → provider 配置 → 模型验证 → 角色导入 → Persona/Preset 选择 → 首轮对话），Port 合同 + 动态 import 边界 + fail-open 降级（F1–F4，F5–F6 为向导内重试），desync 时可重触发；现有 `webui/app.js` 仍是 M1 backend validation harness，向导是面向 RP 重度用户的引导层而非替代。Shadow DOM 隔离与 Port 版本协商跟踪于 [#210](https://github.com/GhostXia/AIRP/issues/210) / [#211](https://github.com/GhostXia/AIRP/issues/211)。PR #256 修复了 Stage 1 prod timer 泄漏（W-05）、preset_id 随机后缀加固（W-07）和 disposed guard。
- PR #232 收口 onboarding 首聊错误边界：流式错误保留机器可读 code/retryability/commit state，提前 EOF 或提交状态不明时不盲目重发；“稍后聊天”会清除旧 session，开发连接值只恢复非敏感字段；WebUI 错误摘要会递归脱敏普通字段、quoted JSON credential、URL userinfo 与 query secret。HTTP `PathEscape` 和内部/上游失败只返回稳定公开消息，详细信息留在服务端日志。
- `ui/` 开发工具链已升级到 Vite `8.1.4`、Vitest `4.1.10` 与 `@vitejs/plugin-vue` `6.0.8`；manifest 使用不跨主版本的有界范围，lockfile 固定实际解析版本，Node 合同为 `^20.19.0 || >=22.12.0`。PR #191 的 `npm audit` 为 0 项。

### Production P0

- engine production mode 在读写配置和监听前 fail-closed 校验 deployment mode、32-byte base64url access key、canonical HTTPS public origin、绝对且已存在可写的数据目录，并禁止 local-path import。
- `deploy/production/` 已提供 digest-pinned engine/Caddy images、版本化 Compose、私有 engine 网络、secret mounts、显式 TLS 模式、安全 headers、同源 WebUI runtime config 与 operator bootstrap。
- production topology CI 会启动真实 HTTPS 栈，覆盖 perimeter auth、私有 engine、CSP/headers、content-only import、SSE、浏览器注入/取消、重启持久化和 secret scan。
- Production 重启连续性已验证（PR #234/#236/#246）：服务重启后聊天恢复 smoke 已纳入 CI，Caddy upstream warmup 竞态已修复，浏览器刷新恢复与服务重启恢复分别覆盖。
- P1 已提供人工冷备份与回滚逃生路径：备份归档持久化 SHA-256 sidecar，恢复前先校验哈希；回滚使用独立卷并在启动前验证 `.env` 指向已存在的目标卷；验证期间 gateway 只绑定 loopback，健康、资产、session 与历史只读核验通过后才重新开放公网监听。该流程不是 P2 自动 backup/restore、migration 或完整恢复演练。
- 这是已实现的 P0 preview，不是正式发布；P1–P3 仍由 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) 管理。

## 3. 尚不能宣称

- 当前已经形成 **P1 有限试用代码候选**，但不是 P1 已通过，更不是正式发布：首聊黄金路径仍需继续开发并以真实 provider、真实浏览器、生产拓扑及自动化/人工验收建立可重复证据；页面刷新恢复和服务重启恢复必须分别通过，常见失败必须可重试，且 secret 不泄露、关键资产不被静默损坏。完整 RP 资产生命周期、base lock / drift / rollback、版本化 migration、自动备份/恢复、可恢复删除、升级回滚、运维 runbook、浏览器矩阵与长会话 soak 仍属于 P2/P3 或正式发布门禁。
- Persona 的 base lock、drift/history/rollback、头像、导入导出与备份恢复仍未闭合；完整 Preset 生命周期、HTTP/UI 受控 dry-run、revision/collision/overwrite/provenance 合同仍不完整，版本保留/清理策略也尚未定稿。
- Worldbook v4 runtime、普通用户主面板管理和 advisory 字段只读可见性已交付；受控大型上传、更多 advisory 字段的运行时语义与完整资产生命周期仍未完成。
- `PromptAssemblyTrace` 的 pipeline → 脱敏 HTTP preview → WebUI 用户摘要已闭环。Phase 2 (#115) 6 类 asset revision 合同已落地（角色卡 / Persona / Preset / Worldbook / State / Memory）：新数据上 `EffectiveIds` 全部 6 个 `*_revision` 字段填充实际 u64，旧数据上推送 `*_revision_unavailable` 诊断并在 WebUI 显示 `unavailable`，禁止用文件时间冒充版本；#114 统一有效配置摘要已交付 Persona 激活来源与参数来源 chips；但 base lock / drift / rollback / 受控 dry-run / 完整 provenance 审计仍未交付（详见上行）。
- PR #219/#227/#232 已加固单资源持久化边界（quota race / chat_store 原子性 / replace_file fsync/扩展名 / character_lock / lorebook 保留 / volume 溢出 / chat finalization 失败关闭），但跨资源事务、`AIRP-TREE-SHA256-v1` 完整性校验、版本化 migration registry、自动备份恢复与可恢复删除仍是合同而非 runtime 能力；`character_lock` poison 恢复与 `record_tokens` `spawn_blocking` 性能优化已记为 #220 deferred 项。
- session 当前还不是完整自包含、逐轮可复现的存档；revision manifest、工作副本、完整性加载、派生角色卡导出和完整 session archive 仍是合同而非 runtime 能力。
- 工程治理基础设施（PR #218）已提供 Cargo/npm 锁文件的离线 inventory、纯函数审计路由、SBOM 生成与 GitHub Actions `actions/checkout@v7` / `setup-node@v6` / `upload-artifact@v7` 升级（workflow step Node 版本仍为 20.19.0，仅 action wrapper 自身跑在 Node 24）。它当前不枚举 Dockerfile 基础镜像，不查询上游最新稳定版本或安全公告，也不自动创建/去重依赖 PR/issue；发布签名、许可证/proof 自动核验和 SBOM release gate 均未交付。P1 有限试用不扩建该工具链。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、可配置多 Agent 编排和长期自进化记忆尚未交付。
- WebUI 的 50 条窗口不是虚拟列表；Tauri/Vue 的 10k/100k 性能与内存上界仍属长期路线。Windows 便携 WebUI 已有本机 artifact/Chrome 自动 smoke，但尚需 GitHub artifact 门禁与真实 provider 人工验收。Swipe 多候选已交付，但 branch（分支对话树）和 edit（编辑已发送消息）仍未交付。
- `card_path` 只适用于可信本地桌面调用；远端/Web 调用必须使用受控 content upload，不能把服务端任意路径读取暴露给不可信调用方。

# 4. 当前执行顺序

> 2026-07-20 PR #268 合并后校准：Swipe/Smooth Streaming、每消息操作、Windows 便携包、production 重启连续性和 P1 审计跟进批次已落地。项目最大风险仍是工程治理远超产品成熟度、首聊迟迟未有真实用户。以下顺序体现“先让产品活过来，再让它健康”的原则。

**P1 有限试用代码候选已形成；当前唯一近期目标是让真实用户尽快完成第一次 RP 对话。**按以下顺序执行：

1. **跑通首聊黄金路径（最高优先）**：用真实 provider（OpenAI / Anthropic / DeepSeek 等）贯通 onboarding → 首轮流式对话 → 页面刷新后继续 → engine 重启后继续。过程中遇到的**实际阻塞 bug** 立即修复；审计 #248 中未实际触发的理论风险不阻塞本步。

   > **暂时认为默认实现**（2026-07-20 本地验证 `main@7895f8c`）：以下路径已在无真实 API key 条件下贯通，pipeline 到 upstream 调用前全部正常，标记为默认实现基线：
   > - `GET /health` → `engine: ok`, `data_root_writable: true`；配置 provider 后 `provider_configured: true`
   > - `POST /v1/settings` → provider/endpoint/model/api_key 写入，`api_key_set: true`，secret 不回显
   > - `GET /v1/characters` → 正确返回已有角色列表
   > - `POST /v1/chat/completions`（正确 `user_profile` + `message` 格式）→ pipeline 完成角色加载、prompt 装配、user message 持久化后到达 upstream 调用；placeholder key 下返回结构化 SSE 错误（`code: upstream`, `commit_state: partially_committed`, `retryable: false`）
   > - `POST /v1/chat/history` → user message 已持久化，durable ID 稳定
   > - 页面刷新恢复：history 端点返回同一 session/message ID
   > - Engine 重启恢复：stop → restart → history 完整，session ID / message ID / 内容不变
   >
   > **首聊黄金路径已贯通**（2026-07-20 真实 provider 验证）：使用 GitCode API（DeepSeek-V3）完成完整流式对话，确认 SSE `body_chunk` 流式输出 → assistant message 持久化 → 多轮对话上下文连贯均正常。history 端点返回 5 条消息、durable ID 稳定、session 一致。结合前序验证（health / settings / characters / 刷新恢复 / 重启恢复），首聊黄金路径全链路已贯通，标记为默认实现基线。
2. **让 1-3 个真实用户用起来**：Windows 便携包（`Start-AIRP.cmd`）交给目标 RP 用户试用，收集体验痛点。用户反馈的优先级高于审计遗留项。试用不要求 P1 全部门禁通过，但必须提供 data 备份说明和"有限试用"标注。
3. **提升对话体验与 Agent RP 差异化**：在黄金路径稳定后，优先推进直接影响 RP 体验的能力——流式输出流畅度、角色/会话切换顺滑度、错误提示可行动性、Agent 驱动的 RP 增强（多角色编排、世界事件触发、情感状态追踪）。这是 AIRP 相对 SillyTavern 的真正差异化，不能继续让 95% 精力留在数据管理基础设施上。
4. **按需加固（审计 #248 选择性修补）**：以下项在真实使用中实际触发或分发前必须处理时才修：
   - secrets.json OS 级文件权限（H-01）：分发超过 3 人前修；
   - 依赖漏洞扫描 CI 门禁（H-04）：首次公开 release 前加；
   - WebUI 类型安全（B-01）：app.js 超过 5000 行或第二人接手前引入；
   - 其余审计项（H-02/H-03/M-01~M-05/L-01~L-04）进入 P2 或按需处理。
5. **保持 P1 取舍**：不新增 asset revision 类型、revision chip、provenance 展示或 dependency-governance 自动化。Persona/Preset 高级生命周期、完整 revision/collision/overwrite/provenance 审计及 #220 性能/重构项后移到 P2，除非真实用户反馈证明其直接阻塞日用。
6. **P1 验收后进入 P2/P3**：P2 选择一种高价值资产完成端到端 vertical slice；P3 执行浏览器兼容、安全负向、备份恢复、长会话 soak、artifact 签名与回滚演练。

开放 issue 还包括审计/流程/文档遗留项；不要在本文复制完整 issue 列表。开始工作前使用 `gh issue list --state open` 获取实时状态。

## 5. 最近验证证据

PR #268 最终 head `7895f8c` 在 [PR gate run 29746252864](https://github.com/GhostXia/AIRP/actions/runs/29746252864) 中通过全部代码门禁：

- `Rust workspace`：通过；
- `UI and WebUI`：通过；
- `Production topology`：通过。

本次校准在 `main@7895f8c` 上本地复算：

| 分桶 | 命令 | 结果 |
|---|---|---|
| Engine lib | `cargo test -p airp-core --lib` | 760（759 pass + 1 ignored） |
| Engine integration | `cargo test -p airp-core --test '*'` | 29（4+4+11+5+5） |
| Protocol lib | `cargo test -p airp-state-protocol` | 6 |
| UI lib | `cargo test -p airp-ui` | 9 |
| WebUI | `node --test webui/tests/*.test.mjs` | 125 |
| UI Vitest | `npx vitest run`（ui/） | 98 across 13 files |
| dep-governance | `node --test tools/dep-governance/tests/*.test.mjs` | 91 |

这些证据覆盖 workspace fmt/build/严格 Clippy/tests、warning-free rustdoc、干净提示词不变式 `subagent_context_has_no_orchestrator_noise`、Node 20.19 workflow step runtime 下的 UI typecheck 与 Vitest、125 项 WebUI 回归、依赖发现/路由/SBOM 测试，以及真实 HTTPS/Chrome production topology smoke。新增回归覆盖 Swipe 多候选切换、Smooth Streaming 平滑渲染、auto-regen/continue/per-message actions/单条删除、preset 变更前校验、onboarding disposed guard 与 preset_id 加固、production 重启后聊天恢复。该结果证明 `main@7895f8c` 的代码树；后续变更不得沿用为新结果。

本基线吸收的最近合并序列（`2a14b7e → 7895f8c`）：PR #233（P1 基线校准）→ PR #234/#236/#246（production 重启连续性）→ PR #238/#243（Windows 便携 WebUI）→ PR #249/#251（Swipe + Smooth Streaming）→ PR #250（auto-regen/continue/per-message actions/单条删除）→ PR #253（chat pipeline 模块化）→ PR #254/#255/#256/#257（onboarding/launcher/baseline 修复）→ PR #258/#260（SmoothStreamer 收口/preset 校验）→ PR #261/#267（docstring 政策）→ PR #268（P1 审计跟进批次 #186/#262/#264/#252）。

## 6. 最短阅读顺序

1. 本文：当前能力、缺口、顺序与证据；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：工程不变式、目录边界、验证和交付流程；
3. [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)：近期 release gates；
4. 与任务直接相关的合同：例如 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)；
5. [PLAN.md](PLAN.md)：长期产品原则，不用于证明能力已交付。
