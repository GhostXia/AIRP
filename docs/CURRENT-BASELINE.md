# AIRP 当前开发基线

> 基线日期：2026-07-12
>
> Git 基线：PR #123 验收分支（合并后以 `main` 为准）
>
> 用途：新开发 session 的第一事实入口。若与旧计划、dated audit 或聊天记录冲突，以源码、测试和本文为准。

## 1. 当前可用能力

- `engine/` 是唯一 RP/Agent 内核：OpenAI-compatible/Anthropic 流式调用、角色卡、命名会话、state、基础 lorebook、preset、scene、volume、decompose/analysis、HTTP/SSE 与有界 structured tool-call Agent loop。
- 默认 Agent registry 有 19 个工具；运行时真相由 `GET /v1/agent/tools` 提供，执行受 capability、allowlist 和 destructive confirmation 约束。
- WebUI 已完成 provider 配置、角色导入、单默认 Persona、Preset 选择/JSON 导入、session 创建/切换/删除、session-scoped history/regen/rollback、流式聊天、Agent Run 与非敏感工作区恢复。
- Tauri/Vue UI 已通过 `TauriBus`/`BusRelay` 直连 engine，具有 id-keyed chat、Blueprint/widget、RFC6902 patch、guard、sandbox/consent 和 sidecar 生命周期基础。
- Provider redirect 统一 fail-closed；CORS 使用内建 WebUI/Tauri origins，并只追加 `AIRP_CORS_ORIGINS` 的合法精确来源。

## 2. 尚不能宣称的能力

- WebUI 已通过零密钥 mock provider 验收：自动 engine 真相 harness 为 56/56，真实浏览器完成连接、数据恢复、发送消息与 24-chunk SSE 渲染。它现在是基本可用的轻量 RP 客户端，但仍不是长期 Tauri/Vue 产品 UI。
- 当前 AIRP-Dev Windows 安装包尚缺真实 artifact 的安装、启动、sidecar ready、简单对话和退出证据。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、完整 Persona 生命周期、Preset migration/trace、SillyTavern 高级 worldbook 语义、长会话分页/虚拟化仍未完成。
- WebUI `loadHistory` 仍全量重建消息 DOM，已由 issue #122 跟踪；不得把该路径复制到产品 UI。

## 3. 下一阶段优先顺序

1. 桌面 artifact 安装/启动/sidecar/简单对话/退出验收；
2. 长会话性能与 durable message ID（#37/#122）；
3. 完整 Persona/Preset 生命周期与 trace（#114/#115）；
4. 继续保持零密钥 WebUI smoke 与神圣提示词不变式为回归门禁。

## 4. 当前开放风险/issue 分组

- WebUI/MVP：#105、#114、#115、#116、#117、#122；其中 #114/#117 已完成最小子集，issue 保留的是扩展范围。
- Session/长期使用：#35、#37；delete 与 session-scoped 操作已落地，per-user isolation、durable message ID、branch/swipe/pagination 仍开放。
- Desktop：#29、#98。
- Extension/security：#32、#87。
- Process/docs：#69、#70、#99、#104、#113。

## 5. 最近验证证据

PR #123 合并前新增验证：

- `node webui/smoke.mjs`：56 checks / 0 failures，覆盖三轮持久化、session 隔离、rollback/regen/delete 与 typed errors；
- 真实浏览器：WebUI 连接成功，恢复角色/会话/预设，页面发送消息后收到 24 个 SSE chunks 并渲染 user/assistant；
- rate-limit 回归确认 100ms/token（10 req/s）与 burst 20；
- 完整 Rust/UI 门禁以 PR #123 GitHub checks 为准。

PR #119/#121 的上一轮证据：

- `cargo fmt --all -- --check`；
- `cargo clippy --workspace --all-targets -- -D warnings`；
- `cargo test --workspace`：426 passed，1 ignored；集成 suites 4/11/5 passed；
- `cargo test -p airp-core subagent_context_has_no_orchestrator_noise`；
- `npm --prefix ui run build`；
- `npm --prefix ui test -- --run`：98 passed；
- `node --check webui/app.js`。

数字是 2026-07-12 的快照，不应永久硬编码为质量承诺；新 session 修改代码后必须重新运行相关验证。

## 6. 文档阅读顺序

1. 本文：当前事实、缺口、下一步；
2. [DEV-GUIDE.md](DEV-GUIDE.md)：实现不变式、工具链和开发流程；
3. [WEBUI-MVP-PLAN.md](WEBUI-MVP-PLAN.md)：当前验收门槛；
4. [PLAN.md](PLAN.md)：长期产品原则；
5. [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)：第一方源仓吸收边界；
6. [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md)：第三方理念参考与独立实现 provenance；
7. dated audits/plans：只用于历史追溯，不作为当前状态。
