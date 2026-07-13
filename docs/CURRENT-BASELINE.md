# AIRP 当前开发基线

> 基线日期：2026-07-13
>
> Git 基线：`main` / `7fb766e`（PR #124、#125 已合并）
>
> 用途：新开发 session 的第一事实入口。若与旧计划、dated audit 或聊天记录冲突，以源码、测试和本文为准。

## 1. 当前可用能力

- `engine/` 是唯一 RP/Agent 内核：OpenAI-compatible/Anthropic 流式调用、角色卡、命名会话、state、基础 lorebook、preset、scene、volume、decompose/analysis、HTTP/SSE 与有界 structured tool-call Agent loop。
- 默认 Agent registry 有 19 个工具；运行时真相由 `GET /v1/agent/tools` 提供，执行受 capability、allowlist 和 destructive confirmation 约束。
- Chat history 已具备 session-scoped durable message ID、完整 JSONL 保留、`limit`/`before` cursor window、legacy deterministic ID 和 rollback-by-ID；旧 history/rollback 请求仍兼容。
- WebUI 已完成 provider 配置、角色导入、单默认 Persona、Preset 选择/JSON 导入、session 生命周期、流式聊天、Agent Run、非敏感恢复，以及 50 条首屏窗口、加载更早、durable-ID DOM 复用和按消息回滚。
- WebUI 是当前后端能力孵化、API/数据合同验证与基础 RP 使用的主开发面；新增能力优先贯通 engine shared service → HTTP/SSE → WebUI → tests，再把稳定合同接入桌面端。
- Tauri/Vue UI 已通过 `TauriBus`/`BusRelay` 直连 engine，具有 id-keyed chat、Blueprint/widget、RFC6902 patch、guard、sandbox/consent 和 sidecar 生命周期基础。
- Provider redirect 统一 fail-closed；CORS 使用内建 WebUI/Tauri origins，并只追加 `AIRP_CORS_ORIGINS` 的合法精确来源。

## 2. 尚不能宣称的能力

- 当前 AIRP-Dev Windows 安装包尚缺真实 artifact 的安装、启动、sidecar ready、简单对话和退出证据。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、多 Persona 生命周期与绑定、Preset migration report、PromptAssemblyTrace、SillyTavern 高级 worldbook 语义仍未完成。
- WebUI 已窗口分页，但不是虚拟列表；Tauri/Vue 的 10k/100k 性能、内存上界和虚拟滚动验收仍未完成，issue #122 因此保持开放。
- #37 的 branch/swipe/edit、per-user isolation 和长期记忆仍开放；durable ID、cursor pagination 与 rollback-by-ID 已交付，不应再列为缺口。
- 可配置多 Agent 编排尚未交付；[AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md) 是产品原则与待实现规范，不得把示例 profile 写成现有 runtime 能力。

## 3. 下一阶段优先顺序

1. RP Profile 地基（#114/#115）：先收敛 shared `PresetService`、无损 raw sidecar、`PresetImportReport`/dry-run/revision，再补多 Persona 与有效配置绑定；
2. `PromptAssemblyTrace`（#115）：让有效 character/persona/preset/provider/model、segment 来源和截断事实可观察；
3. revisioned ChangeInbox（#117），再推进 Agent-first 工作台（#87）；
4. Style Review（#116）在 trace/change 合同稳定后接入，只产出建议；
5. 长历史剩余性能门（#122）和 branch/swipe/edit（#37）按需要纵向补齐；
6. 桌面 artifact 与 sidecar 可靠性（#98/#29）保留为阶段性 release gate。

## 4. 当前开放风险/issue 分组

- RP Profile/诊断：#114、#115、#116、#117；#114 仅完成单默认 Persona 与基础 Preset 导入/选择。
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

数字是 2026-07-13 的证据快照，不是永久质量承诺；修改后必须重新运行相关验证。

## 6. 文档阅读顺序

1. 本文：当前事实、缺口、下一步；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：实现不变式、工具链和开发流程；
3. [PLAN.md](PLAN.md)：长期产品原则与当前执行方向；
4. [LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)：已交付的 durable history 合同与剩余性能边界；
5. [WEBUI-MVP-PLAN.md](WEBUI-MVP-PLAN.md)：已完成的基础验收合同；
6. [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)：第一方源仓吸收边界；
7. [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md)：第三方理念参考与独立实现 provenance；
8. dated audits/plans：只用于历史追溯，不作为当前状态。
