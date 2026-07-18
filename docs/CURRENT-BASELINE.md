# AIRP 当前开发基线

> 基线日期：2026-07-17
>
> 实现基线：`main@15cb6c0` / PR #215
>
> 用途：新开发 session 的第一事实入口。源码、manifest、测试和可重复运行证据高于本文；GitHub issues 是未完成工作的实时追踪面。

## 1. 产品与仓库边界

AIRP 是专精 Role Play 的 AI Agent 客户端，当前采用“无头 engine + 可换 UI”结构：

- `engine/`（`airp-core`）：唯一 RP/Agent 内核，负责数据、prompt 装配、LLM adapter、Agent loop 与 HTTP/SSE；
- `webui/`：当前正式产品交付主面；
- `ui/` + `ui/src-tauri/`（`airp-ui`）：保留的 Tauri/Vue 桌面客户端，近期开发与打包验收暂停；
- `protocol/`（`airp-state-protocol`）：UI/engine 共用线协议类型；
- `deploy/production/`：单实例、自托管、单用户 WebUI 的 P0 preview 拓扑；
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

### RP 数据与会话

- 角色卡 JSON/PNG 导入、角色 CRUD、preset、scene、state、volume、decompose/analysis 和基础 worldbook 已有共享服务或 HTTP 能力。
- Preset 导入已输出规范化报告并保留 BOM 清理后的原始输入 sidecar；Agent 更新把 canonical/raw 写入不可变版本目录，再以单一原子 `current` 指针切换活动版本。`decompose_preset` 优先读取规范化版本，并兼容 legacy 布局。
- `PromptAssemblyTrace` 已接入真实 single/scene chat 装配路径，按 provider payload 顺序记录 card/persona/lorebook/state/preset/scene/memory/history/user 的显式 provenance；Phase 2h (#215) 把 `EffectiveIds` 的 6 类 `*_revision` 字段（character/persona/preset/lorebook/state/memory）全部接到统一 `content_revision` 合同，新数据填充实际 u64，旧数据或读取失败时推送 `*_revision_unavailable` 诊断；scene 模式下 character/lorebook/state/memory revision 留 `None`（多角色无单一 revision），不推送诊断。scene 合并角色与场景 worldbook 后仍按命中条目保留逻辑来源和源文档条目序号，不暴露本机路径。`POST /v1/chat/preview` 复用该路径但不推进时间线、不创建会话或修复 metadata，且不返回 prompt 正文、API key 或 endpoint 明文。
- 命名 session 使用外层目录 UUID 作为 session 目录、history 响应和 `chat_log_meta.json` 的唯一规范身份；旧双 UUID metadata 会 best-effort 原子修复，损坏 metadata 不阻塞仍可读取的历史。
- Chat history 已有 session-scoped durable message ID、完整 JSONL 保留、`limit`/`before` cursor、legacy deterministic ID 与 rollback-by-ID。非法 rollback index 在 service/API 和 `ChatLog` 持久化边界均被拒绝。
- 新数据根不再创建根级 `world.md`/`items.md`；新角色不再创建 legacy `worldbooks/`。角色默认世界书的规范位置是 `characters/{character_id}/world/lorebook.json`。
- 自包含 session、角色卡/世界书工作副本、统一 `content_revision`、`AIRP-TREE-SHA256-v1`、JSONL 崩溃恢复与完整导出已经形成接受合同，但除命名 session 身份、history/memory 隔离和 Phase 2 (#115) 6 类 revision 合同外仍需分阶段实现。详见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)。
- 统一 revision 合同已覆盖 Preset（Phase 2b/#202）、Character/Worldbook/State/Memory/Persona（Phase 2c-2g/#203）与 trace 收口（Phase 2h/#215）：`commit_revision` helper + `AssetKind` enum + 单调 u64 `content_revision` + 不可变 `revisions/{N}/` 快照 + 原子 `current_revision` 指针；`next_content_revision` (#206) 在 orphan revision_dir（提交第 5 步后、第 8 步前崩溃留下的快照）场景下取 `max(pointer, 磁盘最大 revision_dir) + 1` 跳过 orphan，避免 asset 永久不可写。所有 service 保留 legacy 工作副本，revision 作为增量叠加；lazy migration 在首次写入时从 revision 1 起。

### Persona、Worldbook 与 WebUI

- 多 Persona 存储、revision、默认/角色/session 绑定、plural HTTP CRUD、legacy singular API 兼容，以及 chat pipeline 的 explicit → binding → default 激活顺序已交付。
- WebUI 已有「自动（跟随绑定/默认）」与显式 Persona 选择、effective source/双 scope owner 展示、角色/session 绑定与解绑，并始终在聊天 payload 传当前 `user_id`；显式选择才传 `persona_id`。服务端在同一 per-user snapshot 解析 owner，并在原子保存边界拒绝同 scope 多 owner。
- Worldbook v4 已实现 `enabled && (constant || (primary_match && (!selective || no_valid_secondary_keys || any_secondary_match)))`；`secondary_keys` 使用 OR/any-match，空集合退化为 primary-only，`constant` 跳过 selective gate。
- shared normalizer 统一 PNG、PUT API、Agent tool 三入口，将 top-level 或 v3 `extensions.selective` presence-aware 提升为 canonical 字段，并保留 `case_sensitive`、position/depth/probability/递归等尚未执行的 advisory metadata；保留字段不等于执行兼容。
- Worldbook 编辑器已迁入 character-scoped 普通用户主面板，可编辑 v4 `selective`/`secondary_keys`，只读展示 advisory 字段，并有未保存修改确认、异步响应防串角色和 429 可恢复提示。production browser smoke 覆盖容器静态资产、字段切换、只读展示和 malformed/legacy 响应；V3 PNG/JSON character_book 的 `constant` 条目已有导入到最终 system prompt 的端到端回归。
- WebUI 已有 provider 配置、角色导入、Preset 选择/JSON 导入、session 生命周期、流式聊天、Agent Run、非敏感恢复、50 条首屏窗口、加载更早、durable-ID DOM 复用与按消息回滚；对话主面板现可查看本轮实际 Persona/Preset/Provider/Model，以及有序装配材料、稳定性和估算规模。本轮装配 chips 已从 5 项扩展到 10 项（card/persona/preset/lorebook/state/memory + 模型/服务 + 温度/最大 tokens），身份/模型/服务/温度/最大 tokens chip 附带来源后缀（如 `· 显式`/`· 预设`/`· 请求`），在对应 asset 有 `*_revision_unavailable` 诊断时显示 `unavailable` 标识，未激活 asset 显示「未启用」。#114 统一有效配置摘要已交付：Persona 激活来源（explicit/session_binding/character_binding/default/absent）与参数来源（request/preset/snapshot）由 engine 侧 `PersonaService::resolve_effective_persona` 与 `resolve_param_sources` 填充，与 HTTP effective 端点同源。
- WebUI 已有首次启动 onboarding wizard（[#209](https://github.com/GhostXia/AIRP/issues/209) / PR #212）：6-stage 状态机（部署健康检查 → provider 配置 → 模型验证 → 角色导入 → Persona/Preset 选择 → 首轮对话），Port 合同 + 动态 import 边界 + fail-open 降级（F1–F4，F5–F6 为向导内重试），desync 时可重触发；现有 `webui/app.js` 仍是 M1 backend validation harness，向导是面向 RP 重度用户的引导层而非替代。Shadow DOM 隔离与 Port 版本协商跟踪于 [#210](https://github.com/GhostXia/AIRP/issues/210) / [#211](https://github.com/GhostXia/AIRP/issues/211)。
- `ui/` 开发工具链已升级到 Vite `8.1.4`、Vitest `4.1.10` 与 `@vitejs/plugin-vue` `6.0.8`；manifest 使用不跨主版本的有界范围，lockfile 固定实际解析版本，Node 合同为 `^20.19.0 || >=22.12.0`。PR #191 的 `npm audit` 为 0 项。

### Production P0

- engine production mode 在读写配置和监听前 fail-closed 校验 deployment mode、32-byte base64url access key、canonical HTTPS public origin、绝对且已存在可写的数据目录，并禁止 local-path import。
- `deploy/production/` 已提供 digest-pinned engine/Caddy images、版本化 Compose、私有 engine 网络、secret mounts、显式 TLS 模式、安全 headers、同源 WebUI runtime config 与 operator bootstrap。
- production topology CI 会启动真实 HTTPS 栈，覆盖 perimeter auth、私有 engine、CSP/headers、content-only import、SSE、浏览器注入/取消、重启持久化和 secret scan。
- 这是已实现的 P0 preview，不是正式发布；P1–P3 仍由 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) 管理。

## 3. 尚不能宣称

- WebUI 尚未达到“正式发布”：完整 RP 资产管理、完整有效配置可见性、配置控制与审计（base lock / drift / rollback / 受控 dry-run / 完整 provenance 审计）、版本化 migration、备份/恢复、可恢复删除、升级回滚、运维 runbook、浏览器矩阵与长会话 soak 仍不完整。
- Persona 的 base lock、drift/history/rollback、头像、导入导出与备份恢复仍未闭合；完整 Preset 生命周期、HTTP/UI 受控 dry-run、revision/collision/overwrite/provenance 合同仍不完整，版本保留/清理策略也尚未定稿。
- Worldbook v4 runtime、普通用户主面板管理和 advisory 字段只读可见性已交付；受控大型上传、更多 advisory 字段的运行时语义与完整资产生命周期仍未完成。
- `PromptAssemblyTrace` 的 pipeline → 脱敏 HTTP preview → WebUI 用户摘要已闭环。Phase 2 (#115) 6 类 asset revision 合同已落地（角色卡 / Persona / Preset / Worldbook / State / Memory）：新数据上 `EffectiveIds` 全部 6 个 `*_revision` 字段填充实际 u64，旧数据上推送 `*_revision_unavailable` 诊断并在 WebUI 显示 `unavailable`，禁止用文件时间冒充版本；#114 统一有效配置摘要已交付 Persona 激活来源与参数来源 chips；但 base lock / drift / rollback / 受控 dry-run / 完整 provenance 审计仍未交付（详见上行）。
- session 当前还不是完整自包含、逐轮可复现的存档；revision manifest、工作副本、完整性加载、派生角色卡导出和完整 session archive 仍是合同而非 runtime 能力。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、可配置多 Agent 编排和长期自进化记忆尚未交付。
- WebUI 的 50 条窗口不是虚拟列表；Tauri/Vue 的 10k/100k 性能、内存上界和真实 Windows artifact 验收仍暂停。
- `card_path` 只适用于可信本地桌面调用；远端/Web 调用必须使用受控 content upload，不能把服务端任意路径读取暴露给不可信调用方。

## 4. 当前执行顺序

1. Phase 2 (#115) 6 类 asset revision 合同与 `PromptAssemblyTrace` 收口已落地（角色卡 / Persona / Preset / Worldbook / State / Memory，PR #201/#202/#203/#206/#215），#114 统一有效配置摘要已交付（Persona 激活来源 + 参数来源 chips）；下一步在 #114 已交付 Persona effective/绑定能力之上完成 Persona/Preset 高级生命周期（base lock、drift/history/rollback、头像、导入导出/备份恢复）、完整 Preset 生命周期、HTTP/UI 受控 dry-run、revision/collision/overwrite/provenance 审计；#126 的 v4 runtime、shared normalizer、主面板编辑和 PNG/JSON 端到端回归已交付，不得重复实现；
2. 按 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md) 分阶段实现 session 自包含边界、revision manifest、工作副本、完整性加载、派生角色卡导出与完整 session archive，并补版本化 migration、备份/恢复、可恢复删除、readiness、日志和 runbook；
3. 建立 P3 发布候选门禁：浏览器兼容、安全负向、旧数据升级、备份恢复、长会话 soak、artifact 与回滚演练；§2.4 干净提示词 + Agent loop 价值验证 L0/L1 全部满足；
4. #192 依赖发现自动化与 #128 Actions runtime 升级作为工程治理项跟踪，但不抢占 #114 产品主链；#117、#87、#116、#163 后移，桌面 #29/#98/#122 仅保留追踪。

开放 issue 还包括审计/流程/文档遗留项；不要在本文复制完整 issue 列表。开始工作前使用 `gh issue list --state open` 获取实时状态。

## 5. 最近验证证据

`main@15cb6c0` 在 [push gate run 29590129817](https://github.com/GhostXia/AIRP/actions/runs/29590129817) 中通过全部代码门禁：

- `Rust workspace`：通过；
- `UI and WebUI`：通过；
- `Production topology`：通过。

这些远端证据覆盖 workspace fmt/build/严格 Clippy/tests、warning-free rustdoc、干净提示词不变式 `subagent_context_has_no_orchestrator_noise`、Node 20.19 下的 UI typecheck 与 Vitest、WebUI 脚本/纯函数测试与 onboarding wizard 22 项 L1/L2 + 43 项 L4 回归，以及真实 HTTPS/Chrome production topology smoke；Rust workspace 还覆盖 Phase 2h 6 类 revision 字段填充、`*_revision_unavailable` 诊断推送、scene 模式留 `None`、orphan revision_dir 跳过与 next_content_revision 边界测试。该结果证明 `15cb6c0` 与 PR #215 head；后续变更不得沿用为新结果。

基线之后合并序列（c38e7ec → 15cb6c0）：PR #201（Phase 2a 统一 revision 基础 + 2h 诊断）→ #202（Phase 2b Preset）→ #203（Phase 2c-2g Character/Worldbook/State/Memory/Persona）→ #206（orphan revision_dir 恢复）→ #208（采纳 #207 3+1 launch-validation targets）→ #212（onboarding wizard Phase 1，#209）→ #215（Phase 2h trace 完整性收口）。

## 6. 最短阅读顺序

1. 本文：当前能力、缺口、顺序与证据；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：工程不变式、目录边界、验证和交付流程；
3. [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)：近期 release gates；
4. 与任务直接相关的合同：例如 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)；
5. [PLAN.md](PLAN.md)：长期产品原则，不用于证明能力已交付。
