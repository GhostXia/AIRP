# AIRP 当前开发基线

> 基线日期：2026-07-15
>
> 实现基线：`main@1f3e6ed` / PR #169
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
- 默认 Agent registry 有 19 个工具；运行时目录由 `GET /v1/agent/tools` 提供，执行受 capability、allowlist 与 destructive confirmation 约束。
- Agent tool 已按 session/character、state/lorebook、volume/context、analysis 等职责拆分；daemon handler 已按 settings、presets、scenes、models、sessions、personas、chat、agent、characters、state、lorebook 拆分。此次拆分不改变 HTTP 或工具合同。
- daemon HTTP 合同测试已按 catalog、chat、health/settings、persona、security、sessions、state/scene 分组，覆盖主要 route、校验和安全边界。
- 干净提示词不变式 `subagent_context_has_no_orchestrator_noise` 仍是阻塞门禁：Agent 编排脚手架不得进入 RP 角色平面。

### RP 数据与会话

- 角色卡 JSON/PNG 导入、角色 CRUD、preset、scene、state、volume、decompose/analysis 和基础 worldbook 已有共享服务或 HTTP 能力。
- 命名 session 使用外层目录 UUID 作为 session 目录、history 响应和 `chat_log_meta.json` 的唯一规范身份；旧双 UUID metadata 会 best-effort 原子修复，损坏 metadata 不阻塞仍可读取的历史。
- Chat history 已有 session-scoped durable message ID、完整 JSONL 保留、`limit`/`before` cursor、legacy deterministic ID 与 rollback-by-ID。非法 rollback index 在 service/API 和 `ChatLog` 持久化边界均被拒绝。
- 新数据根不再创建根级 `world.md`/`items.md`；新角色不再创建 legacy `worldbooks/`。角色默认世界书的规范位置是 `characters/{character_id}/world/lorebook.json`。
- 自包含 session、角色卡/世界书工作副本、统一 `content_revision`、`AIRP-TREE-SHA256-v1`、JSONL 崩溃恢复与完整导出已经形成接受合同，但除命名 session 身份和 history/memory 隔离外仍需分阶段实现。详见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)。

### Persona、Worldbook 与 WebUI

- 多 Persona 存储、revision、默认/角色/session 绑定、plural HTTP CRUD、legacy singular API 兼容，以及 chat pipeline 的 explicit → binding → default 激活顺序已交付。
- WebUI 已有多 Persona 列表、创建、编辑、删除与本地选择状态；绑定/解绑和聊天请求显式 persona 切换仍未形成完整产品闭环。
- Worldbook v2 已实现 `enabled && (constant || primary_keyword_match)`；v3 shared normalizer 统一 PNG、PUT API、Agent tool 三入口，保留 `secondary_keys`、`case_sensitive` 与其他 ST advisory metadata，并输出导入诊断。
- `selective`、secondary-key 组合、position/depth、probability、递归等 SillyTavern 高级运行时语义尚未实现；保留字段不等于执行兼容。
- WebUI 已有 provider 配置、角色导入、Preset 选择/JSON 导入、session 生命周期、流式聊天、Agent Run、非敏感恢复、50 条首屏窗口、加载更早、durable-ID DOM 复用与按消息回滚。

### Production P0

- engine production mode 在读写配置和监听前 fail-closed 校验 deployment mode、32-byte base64url access key、canonical HTTPS public origin、绝对且已存在可写的数据目录，并禁止 local-path import。
- `deploy/production/` 已提供 digest-pinned engine/Caddy images、版本化 Compose、私有 engine 网络、secret mounts、显式 TLS 模式、安全 headers、同源 WebUI runtime config 与 operator bootstrap。
- production topology CI 会启动真实 HTTPS 栈，覆盖 perimeter auth、私有 engine、CSP/headers、content-only import、SSE、浏览器注入/取消、重启持久化和 secret scan。
- 这是已实现的 P0 preview，不是正式发布；P1–P3 仍由 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) 管理。

## 3. 尚不能宣称

- WebUI 尚未达到“正式发布”：完整 RP 资产管理、有效配置可见性、版本化 migration、备份/恢复、可恢复删除、升级回滚、运维 runbook、浏览器矩阵与长会话 soak 仍不完整。
- 多 Persona 的 WebUI 绑定/解绑、聊天时激活切换、完整 Preset 生命周期、PromptAssemblyTrace 与迁移报告仍未闭合。
- session 当前还不是完整自包含、逐轮可复现的存档；revision manifest、工作副本、完整性加载、派生角色卡导出和完整 session archive 仍是合同而非 runtime 能力。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、可配置多 Agent 编排和长期自进化记忆尚未交付。
- WebUI 的 50 条窗口不是虚拟列表；Tauri/Vue 的 10k/100k 性能、内存上界和真实 Windows artifact 验收仍暂停。
- `card_path` 只适用于可信本地桌面调用；远端/Web 调用必须使用受控 content upload，不能把服务端任意路径读取暴露给不可信调用方。

## 4. 当前执行顺序

1. 处理 #137 的 Vite/Vitest 安全升级，并重跑 UI/WebUI、production browser smoke 与 Tauri 配置验证；
2. 完成 #114/#115/#126 的剩余 P1 闭环：Persona/Preset/Worldbook 管理、绑定、有效配置、迁移/诊断与 trace；已交付子项不得重复实现；
3. 按 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md) 分阶段实现 session 自包含边界、revision 与恢复导出，并补版本化 migration、备份/恢复、可恢复删除、readiness、日志和 runbook；
4. 建立 P3 发布候选门禁：浏览器兼容、安全负向、旧数据升级、备份恢复、长会话 soak、artifact 与回滚演练；
5. #117 ChangeInbox、#87 Agent-first 工作台、#116 Style Review、#163 受控扩展设计后移；桌面 #29/#98/#122 仅保留追踪。

开放 issue 还包括审计/流程/文档遗留项；不要在本文复制完整 issue 列表。开始工作前使用 `gh issue list --state open` 获取实时状态。

## 5. 最近验证证据

`main@1f3e6ed` 的 push run [29390516900](https://github.com/GhostXia/AIRP/actions/runs/29390516900) 全绿：

- `Rust workspace`：通过；
- `UI and WebUI`：通过；
- `Production topology`：通过。

PR #169 的最终本地证据包括 `cargo test -p airp-core --lib` 568 passed / 1 ignored、workspace suites 通过、严格 Clippy、fmt、相对链接检查与 `git diff --check`；PR gate 的 Rust/UI/production topology 和审计状态均通过。该数字只对应 PR #169 快照，后续变更不得沿用为新结果。

## 6. 最短阅读顺序

1. 本文：当前能力、缺口、顺序与证据；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：工程不变式、目录边界、验证和交付流程；
3. [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)：近期 release gates；
4. 与任务直接相关的合同：例如 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)；
5. [PLAN.md](PLAN.md)：长期产品原则，不用于证明能力已交付。
