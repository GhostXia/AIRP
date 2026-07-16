# AIRP 当前开发基线

> 基线日期：2026-07-16
>
> 实现基线：`main@c47585b` / PR #191
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
- `PromptAssemblyTrace` 已有显式数据模型骨架，稳定性按小写 `stable` / `volatile` 序列化；调用方必须明确提供 provenance，禁止从最终 prompt marker 反向猜测来源。
- 命名 session 使用外层目录 UUID 作为 session 目录、history 响应和 `chat_log_meta.json` 的唯一规范身份；旧双 UUID metadata 会 best-effort 原子修复，损坏 metadata 不阻塞仍可读取的历史。
- Chat history 已有 session-scoped durable message ID、完整 JSONL 保留、`limit`/`before` cursor、legacy deterministic ID 与 rollback-by-ID。非法 rollback index 在 service/API 和 `ChatLog` 持久化边界均被拒绝。
- 新数据根不再创建根级 `world.md`/`items.md`；新角色不再创建 legacy `worldbooks/`。角色默认世界书的规范位置是 `characters/{character_id}/world/lorebook.json`。
- 自包含 session、角色卡/世界书工作副本、统一 `content_revision`、`AIRP-TREE-SHA256-v1`、JSONL 崩溃恢复与完整导出已经形成接受合同，但除命名 session 身份和 history/memory 隔离外仍需分阶段实现。详见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)。

### Persona、Worldbook 与 WebUI

- 多 Persona 存储、revision、默认/角色/session 绑定、plural HTTP CRUD、legacy singular API 兼容，以及 chat pipeline 的 explicit → binding → default 激活顺序已交付。
- WebUI 已有「自动（跟随绑定/默认）」与显式 Persona 选择、effective source/双 scope owner 展示、角色/session 绑定与解绑，并始终在聊天 payload 传当前 `user_id`；显式选择才传 `persona_id`。服务端在同一 per-user snapshot 解析 owner，并在原子保存边界拒绝同 scope 多 owner。
- Worldbook v4 已实现 `enabled && (constant || (primary_match && (!selective || no_valid_secondary_keys || any_secondary_match)))`；`secondary_keys` 使用 OR/any-match，空集合退化为 primary-only，`constant` 跳过 selective gate。
- shared normalizer 统一 PNG、PUT API、Agent tool 三入口，将 top-level 或 v3 `extensions.selective` presence-aware 提升为 canonical 字段，并保留 `case_sensitive`、position/depth/probability/递归等尚未执行的 advisory metadata；保留字段不等于执行兼容。
- Worldbook 编辑器已迁入 character-scoped 普通用户主面板，可编辑 v4 `selective`/`secondary_keys`，只读展示 advisory 字段，并有未保存修改确认、异步响应防串角色和 429 可恢复提示。production browser smoke 覆盖容器静态资产、字段切换、只读展示和 malformed/legacy 响应；V3 PNG/JSON character_book 的 `constant` 条目已有导入到最终 system prompt 的端到端回归。
- WebUI 已有 provider 配置、角色导入、Preset 选择/JSON 导入、session 生命周期、流式聊天、Agent Run、非敏感恢复、50 条首屏窗口、加载更早、durable-ID DOM 复用与按消息回滚。
- `ui/` 开发工具链已升级到 Vite `8.1.4`、Vitest `4.1.10` 与 `@vitejs/plugin-vue` `6.0.8`；manifest 使用不跨主版本的有界范围，lockfile 固定实际解析版本，Node 合同为 `^20.19.0 || >=22.12.0`。PR #191 的 `npm audit` 为 0 项。

### Production P0

- engine production mode 在读写配置和监听前 fail-closed 校验 deployment mode、32-byte base64url access key、canonical HTTPS public origin、绝对且已存在可写的数据目录，并禁止 local-path import。
- `deploy/production/` 已提供 digest-pinned engine/Caddy images、版本化 Compose、私有 engine 网络、secret mounts、显式 TLS 模式、安全 headers、同源 WebUI runtime config 与 operator bootstrap。
- production topology CI 会启动真实 HTTPS 栈，覆盖 perimeter auth、私有 engine、CSP/headers、content-only import、SSE、浏览器注入/取消、重启持久化和 secret scan。
- 这是已实现的 P0 preview，不是正式发布；P1–P3 仍由 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) 管理。

## 3. 尚不能宣称

- WebUI 尚未达到“正式发布”：完整 RP 资产管理、有效配置可见性、版本化 migration、备份/恢复、可恢复删除、升级回滚、运维 runbook、浏览器矩阵与长会话 soak 仍不完整。
- Persona 的 base lock、drift/history/rollback、头像、导入导出与备份恢复仍未闭合；完整 Preset 生命周期、HTTP/UI 受控 dry-run、revision/collision/overwrite/provenance 合同仍不完整，版本保留/清理策略也尚未定稿。
- Worldbook v4 runtime、普通用户主面板管理和 advisory 字段只读可见性已交付；受控大型上传、更多 advisory 字段的运行时语义与完整资产生命周期仍未完成。
- `PromptAssemblyTrace` 目前只是显式模型和不变式：chat pipeline instrumentation、实际 revision 收集、HTTP/UI preview 与用户可读摘要尚未交付，不能宣称已有端到端 trace。
- session 当前还不是完整自包含、逐轮可复现的存档；revision manifest、工作副本、完整性加载、派生角色卡导出和完整 session archive 仍是合同而非 runtime 能力。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、可配置多 Agent 编排和长期自进化记忆尚未交付。
- WebUI 的 50 条窗口不是虚拟列表；Tauri/Vue 的 10k/100k 性能、内存上界和真实 Windows artifact 验收仍暂停。
- `card_path` 只适用于可信本地桌面调用；远端/Web 调用必须使用受控 content upload，不能把服务端任意路径读取暴露给不可信调用方。

## 4. 当前执行顺序

1. 先完成 #115 的产品纵向切片：把 `PromptAssemblyTrace` 接入真实 chat pipeline，收集实际 revision，提供有界脱敏 HTTP 摘要，并让普通用户在 WebUI 看见当前 Persona/Preset/Provider/Model、来源与装配顺序；
2. 在该可观察基础上完成 #114 的 Persona 高级生命周期、Preset 管理与 revision/provenance、统一有效配置摘要；#126 的 v4 runtime、shared normalizer、主面板编辑和 PNG/JSON 端到端回归已交付，不得重复实现；
3. 按 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md) 分阶段实现 session 自包含边界、revision 与恢复导出，并补版本化 migration、备份/恢复、可恢复删除、readiness、日志和 runbook；
4. 建立 P3 发布候选门禁：浏览器兼容、安全负向、旧数据升级、备份恢复、长会话 soak、artifact 与回滚演练；
5. #192 依赖发现自动化与 #128 Actions runtime 升级作为工程治理项跟踪，但不抢占 #115/#114 产品主链；#117、#87、#116、#163 后移，桌面 #29/#98/#122 仅保留追踪。

开放 issue 还包括审计/流程/文档遗留项；不要在本文复制完整 issue 列表。开始工作前使用 `gh issue list --state open` 获取实时状态。

## 5. 最近验证证据

合入 `main@c47585b` 的 PR #191 head tree 在 [PR gate run 29478836944](https://github.com/GhostXia/AIRP/actions/runs/29478836944) 中通过全部阻塞门禁：

- `Rust workspace`：通过；
- `UI and WebUI`：通过；
- `Production topology`：通过。
- `CodeRabbit`：通过。

这些远端证据覆盖 workspace fmt/build/严格 Clippy/tests、warning-free rustdoc、神圣提示词不变式、Node 20.19 下的 UI typecheck 与 98 项 Vitest、WebUI 脚本/纯函数测试，以及真实 HTTPS/Chrome production topology。该结果证明 PR #191 的合入树；merge commit 未增加额外内容，后续变更不得沿用为新结果。

## 6. 最短阅读顺序

1. 本文：当前能力、缺口、顺序与证据；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：工程不变式、目录边界、验证和交付流程；
3. [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)：近期 release gates；
4. 与任务直接相关的合同：例如 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)；
5. [PLAN.md](PLAN.md)：长期产品原则，不用于证明能力已交付。
