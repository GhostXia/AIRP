# AIRP 当前开发基线

> 基线日期：2026-07-13
>
> Git 基线：`main` / `6b145dc`（PR #127 已合并）
>
> 用途：新开发 session 的第一事实入口。若与旧计划、dated audit 或聊天记录冲突，以源码、测试和本文为准。

## 1. 当前可用能力

- `engine/` 是唯一 RP/Agent 内核：OpenAI-compatible/Anthropic 流式调用、角色卡、命名会话、state、基础 lorebook、preset、scene、volume、decompose/analysis、HTTP/SSE 与有界 structured tool-call Agent loop。
- 默认 Agent registry 有 19 个工具；运行时真相由 `GET /v1/agent/tools` 提供，执行受 capability、allowlist 和 destructive confirmation 约束。
- Chat history 已具备 session-scoped durable message ID、完整 JSONL 保留、`limit`/`before` cursor window、legacy deterministic ID 和 rollback-by-ID；旧 history/rollback 请求仍兼容。
- WebUI 已完成 provider 配置、角色导入、单默认 Persona、Preset 选择/JSON 导入、session 生命周期、流式聊天、Agent Run、非敏感恢复，以及 50 条首屏窗口、加载更早、durable-ID DOM 复用和按消息回滚。
- WebUI 已成为当前正式产品交付主面；近期目标从“基础验证面”升级为“可安全部署、可持续日用、可升级恢复的单实例自托管正式版”。生产边界与 release gates 见 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)。
- PR #127 已交付 schema v2 多 Persona 存储、revision、角色/会话绑定、legacy/default 协调和路径校验；现有 WebUI/HTTP 仍主要使用单 default Persona，不能宣称多 Persona 产品闭环完成。
- Tauri/Vue UI 已通过 `TauriBus`/`BusRelay` 直连 engine，具有 id-keyed chat、Blueprint/widget、RFC6902 patch、guard、sandbox/consent 和 sidecar 生命周期基础。
- Provider redirect 统一 fail-closed；CORS 使用内建 WebUI/Tauri origins，并只追加 `AIRP_CORS_ORIGINS` 的合法精确来源。

## 2. 尚不能宣称的能力

- 当前 AIRP-Dev Windows 安装包尚缺真实 artifact 的安装、启动、sidecar ready、简单对话和退出证据。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、多 Persona 生命周期与绑定、Preset migration report、PromptAssemblyTrace、SillyTavern 高级 worldbook 语义仍未完成。
- WebUI 已窗口分页，但不是虚拟列表；Tauri/Vue 的 10k/100k 性能、内存上界和虚拟滚动验收仍未完成，issue #122 因此保持开放。
- #37 的 branch/swipe/edit、per-user isolation 和长期记忆仍开放；durable ID、cursor pagination 与 rollback-by-ID 已交付，不应再列为缺口。
- 可配置多 Agent 编排尚未交付；[AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md) 是产品原则与待实现规范，不得把示例 profile 写成现有 runtime 能力。
- 当前没有首方生产部署 artifact、同源 HTTPS 反代、强制生产鉴权、备份/恢复、升级回滚或 production smoke；`webui/start.bat`、`serve.js` 和 `cargo run` 只属于开发启动路径。

## 3. 下一阶段优先顺序

1. WebUI production P0：同源 HTTPS 入口、私有 engine、强制生产 access key、remote import policy、首方部署产物和 production smoke；
2. RP 正式使用面：完成 #114/#115/#126 的 Persona/Preset/Worldbook 管理、有效配置、迁移报告和 trace 摘要；
3. 数据可靠性：版本化 migration、备份/恢复、可恢复删除、readiness、脱敏日志和运维 runbook；
4. 发布候选：真实浏览器、安全负向、升级/恢复、长会话 soak 和发布 artifact 门禁；
5. #117 ChangeInbox、#87 Agent-first 工作台、#116 Style Review 后移到 WebUI 正式版主链之后；
6. 桌面 #98/#29 继续保留，但不再阻塞 WebUI 首发。

## 4. 当前开放风险/issue 分组

- RP Profile/诊断：#114、#115、#116、#117；#114 已有基础 Preset 与多 Persona 存储地基，但多 Persona HTTP/WebUI 生命周期和完整绑定闭环仍未交付。
- Session/长期使用：#35、#37、#122；durable ID、cursor 与 WebUI window 已完成，剩余是 branch/swipe/edit、per-user/长期记忆和产品 UI 性能。
- Agent/extension：#32、#87。
- Desktop：#29、#98。
- Process/docs：#69、#70、#99、#104、#113。

## 5. 最近验证证据

PR #124（backend long-history contract）：

- `cargo test -p airp-core --lib`：447 passed，1 ignored；
- `cargo clippy -p airp-core --lib --tests -- -D warnings`、fmt 与神圣不变式通过；
- GitHub `Rust workspace`、`UI and WebUI`、CodeRabbit 全绿。

PR #125（WebUI history window）：

- `node webui/smoke.mjs`：64 checks / 0 failures，新增 durable IDs、cursor pages 与 rollback-by-ID 断言；
- 真实浏览器：首屏 50/54，加载更早后 54/54，prepend 后视口保持；键盘 Enter 可选择 durable rollback target；
- GitHub `Rust workspace`（含 workspace tests、Clippy、神圣不变式）与 `UI and WebUI` 全绿。

PR #127（multi Persona storage）：

- 本地最终验证：454 passed，1 ignored；fmt、Clippy 与神圣不变式通过；
- GitHub `Rust workspace`、`UI and WebUI`、CodeRabbit 全绿；
- 审计修复覆盖 nested lock 死锁、bindings 丢失、路径遍历、损坏 JSON fail-closed、canonical/legacy 部分提交回滚和降级编辑协调。

数字是 2026-07-13 的证据快照，不是永久质量承诺；修改后必须重新运行相关验证。

## 6. 文档阅读顺序

1. 本文：当前事实、缺口、下一步；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：实现不变式、工具链和开发流程；
3. [PLAN.md](PLAN.md)：长期产品原则与当前执行方向；
4. [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)：当前正式上线目标、release gates 与推进顺序；
5. [LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)：已交付的 durable history 合同与剩余性能边界；
6. [WEBUI-MVP-PLAN.md](WEBUI-MVP-PLAN.md)：已完成的基础验收合同；
7. [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)：第一方源仓吸收边界；
8. [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md)：第三方理念参考与独立实现 provenance；
9. dated audits/plans：只用于历史追溯，不作为当前状态。
