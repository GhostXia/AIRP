# AIRP 当前开发基线

> 基线日期：2026-07-12  
> Git 基线：`main` / `2a96dea`（PR #121 已合并）  
> 用途：新开发 session 的第一事实入口。若与旧计划、dated audit 或聊天记录冲突，以源码、测试和本文为准。

## 1. 当前可用能力

- `engine/` 是唯一 RP/Agent 内核：OpenAI-compatible/Anthropic 流式调用、角色卡、命名会话、state、基础 lorebook、preset、scene、volume、decompose/analysis、HTTP/SSE 与有界 structured tool-call Agent loop。
- 默认 Agent registry 有 19 个工具；运行时真相由 `GET /v1/agent/tools` 提供，执行受 capability、allowlist 和 destructive confirmation 约束。
- WebUI 已完成 provider 配置、角色导入、单默认 Persona、Preset 选择/JSON 导入、session 创建/切换/删除、session-scoped history/regen/rollback、流式聊天、Agent Run 与非敏感工作区恢复。
- Tauri/Vue UI 已通过 `TauriBus`/`BusRelay` 直连 engine，具有 id-keyed chat、Blueprint/widget、RFC6902 patch、guard、sandbox/consent 和 sidecar 生命周期基础。
- Provider redirect 统一 fail-closed；CORS 使用内建 WebUI/Tauri origins，并只追加 `AIRP_CORS_ORIGINS` 的合法精确来源。

## 2. 尚不能宣称的能力

- WebUI 尚未通过“零密钥 mock provider → 三轮流式 RP → 刷新恢复 → regen/rollback → 删除 session”的自动化全链路验收，因此仍是 MVP candidate，不宣称正式基本可用。
- 当前 AIRP-Dev Windows 安装包尚缺真实 artifact 的安装、启动、sidecar ready、简单对话和退出证据。
- MCP upstream client、skills/plugin runtime、完整 ChangeInbox、完整 Persona 生命周期、Preset migration/trace、SillyTavern 高级 worldbook 语义、长会话分页/虚拟化仍未完成。
- WebUI `loadHistory` 仍全量重建消息 DOM，已由 issue #122 跟踪；不得把该路径复制到产品 UI。

## 3. 下一阶段唯一优先顺序

1. 建立本地零密钥 OpenAI-compatible mock provider；
2. 自动执行 WebUI 连接、provider 配置、角色/Persona/Preset、建 session、三轮流式聊天、刷新恢复、regen/rollback、删 session；
3. 断言 engine 端真实 history、有效 Persona/Preset/session ID、错误类型与无跨 session 串扰；
4. 只修验收暴露的阻塞问题，开 PR、独立审计、CI 全绿后合并；
5. 基本可用门槛通过后，再在桌面 artifact 验收、长会话（#37/#122）和完整 Persona/Preset（#114/#115）之间重排。

## 4. 当前开放风险/issue 分组

- WebUI/MVP：#105、#114、#115、#116、#117、#122；其中 #114/#117 已完成最小子集，issue 保留的是扩展范围。
- Session/长期使用：#35、#37；delete 与 session-scoped 操作已落地，per-user isolation、durable message ID、branch/swipe/pagination 仍开放。
- Desktop：#29、#98。
- Extension/security：#32、#87。
- Process/docs：#69、#70、#99、#104、#113。

## 5. 最近验证证据

PR #119/#121 合并前在本地与 GitHub PR gate 验证：

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
