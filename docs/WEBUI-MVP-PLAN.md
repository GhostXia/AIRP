# WebUI 最快可用推进计划

> 状态：PR #123 已完成；作为基础可用验收合同与历史实施记录保留，不再是近期执行入口
>
> 基线日期：2026-07-13
>
> 目标：以最短路径让普通用户通过浏览器完成一次可持续的基础 RP 使用闭环，而不是继续扩张候选能力。

> 完成记录：PR #118、#119、#121 交付 Persona/Preset/session/outbound 契约、恢复与 busy-state；PR #123 完成基础零密钥验收；PR #124/#125 将 harness 扩到 64/64，并交付 durable history 与 WebUI window。当前方向见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。

## 1. “基本可用”的唯一判据

在全新 data root 上，用户能够：

1. 启动 engine 与 `webui/`，连接并看到明确健康状态；
2. 填写 provider endpoint/model/API key，并通过真实 `/v1/models` 验证；
3. 导入并选择角色；
4. 创建、选择和删除会话，切换后不会串流、串历史或串配置；
5. 编辑一个持久化的基础 User Persona，并选择一个 Preset；
6. 连续完成至少三轮流式 RP，对话刷新页面后仍可从历史恢复；
7. 停止生成、regen 和 rollback；破坏性操作有确认；
8. 可选运行一次 Agent Run，并看懂 plan/tool/result/final 事件；
9. provider、HTTP、SSE 或数据错误在 UI 中给出可行动提示，不以静默失败或控制台日志代替；
10. 一条自动化浏览器 smoke 在无真实密钥的 mock provider 下覆盖上述主路径。

不满足以上十项，就不能称 WebUI 基本可用。视觉精修、完整酒馆 parity 和高级 Agent 能力不计入本里程碑。

## 2. 当前已有，不重复建设

- provider settings、`/v1/models` 验证和 bearer 连接；
- 角色列表、JSON/PNG base64 导入和头像；
- 会话 list/create/select；
- `/v1/chat/completions` 流式回复、停止、history、regen、rollback；
- Agent Run、运行时 tool catalog、allowlist 与 destructive confirm；
- 角色卡、lorebook、state 和 decompose/analysis 的开发工作台；
- character/session 切换时中止旧 stream，并阻止旧请求回写新视图。

这些能力应修补和串成闭环，不再重写第三套 WebUI，也不从参考项目复用 UI 或代码。

## 3. 已交付范围（PR #118/#119/#121）

### 3.1 必做：最小 RP Profile

#### User Persona

- 复用现有 `UserId`、effective user root 与 persona 路径，先只实现**每个用户一个默认 Persona**；
- 最小字段：`name`、`description`、`variables`、`revision`；
- 增加 GET/PUT API，写入走 shared service、原子替换和 revision conflict；
- WebUI 提供基础编辑、保存状态和当前生效摘要；
- chat/agent 由 engine 根据 user identity 解析有效 Persona，逐步停止让 WebUI 每轮硬编码 `{name:"User"}`；
- 多 Persona、头像、角色/会话绑定、drift/history/rollback 继续留在 #114 后续阶段。

#### Preset

- WebUI 聊天 header 增加现有 `/v1/presets` 的选择器，并将 `preset_id` 带入 chat/agent request；
- 增加最小 JSON 导入 API，保留 raw sidecar，拒绝脚本执行和路径输入；
- UI 展示当前 preset 名称及模型参数建议是否被 request/provider 覆盖；
- rename/duplicate/export、字段级迁移报告和 PromptAssemblyTrace 留在 #115。

### 3.2 必做：会话生命周期与隔离

- 完成 #35 的最小子集：session delete endpoint、用户隔离、删除当前会话后的确定性选择；
- history、regen、rollback、并发诊断全部必须携带当前 `session_id`；
- 修复当前并发诊断读取 history 时只传 `character_id` 的旁路；
- URL/hash 或本地状态只恢复仍存在的 character/session；不存在时回到安全空态；
- #37 的 durable ID、cursor pagination 与 WebUI window 已在 MVP 后由 PR #124/#125 实现；swipe、分支、编辑和产品 UI 性能仍后移。

### 3.3 必做：provider 请求安全

- 先完成 #117 A：所有携带凭据的 chat/agent/models 请求使用统一 outbound client policy；
- cross-origin、scheme/port 变化或 downgrade redirect 不得携带 Bearer、`x-api-key` 或自定义 secret header；
- 保持现有 timeout、脱敏日志和 typed error；安全修复不得引入第二套 HTTP client 配置真相。

### 3.4 必做：可行动错误与恢复

- 连接失败、401/403、provider 4xx/5xx、timeout、stream interrupt、revision conflict 和资源不存在分别显示；
- 所有进行中按钮有 disabled/busy 状态，完成或失败后恢复；
- character/session/preset/persona 切换会中止旧请求并清除不再有效的视图状态；
- API key 只存在 engine runtime memory 与密码输入框的当次提交，不进入 localStorage、URL、event log 或复制摘要。

## 4. 实施记录与当前验收阶段

### PR A：WebUI RP MVP 纵向闭环（已完成）

一个 PR 同时交付 shared domain → HTTP → WebUI → tests：

1. credential-safe outbound client policy；
2. 默认 Persona GET/PUT shared service；
3. Preset 最小 JSON import + 现有 list/get 接线；
4. session delete 与 session-scoped history/regen/rollback/concurrent smoke；
5. WebUI Persona/Preset selector、当前有效配置摘要和会话删除；
6. chat 与 Agent Run 使用同一有效 RP profile；
7. Rust/JS 单元与集成测试。

该 PR 可以较大，但不得夹带 Style Review、ChangeInbox、Tauri UI 重构、MCP/skills/plugin、世界书高级语义或视觉重做。

### PR B：真实验收、回归修复与文档收口（PR #123 已完成）

1. 启动真实 engine + 本地 mock OpenAI-compatible provider + `webui/serve.js`；
2. 浏览器自动执行连接→配置→导入角色→Persona/Preset→建会话→三轮聊天→刷新恢复→regen/rollback→删会话；
3. 覆盖 401、provider error、SSE 中断和切换会话时旧响应不回填；
4. 可选手动真实 provider smoke，只记录脱敏的 provider/model/status/SSE 序列；
5. 同步 README、WebUI README、SECURITY、DEV-GUIDE、DOC-AUDIT 与对应 issue 状态。

PR B 只修验收暴露的阻塞 bug；非阻塞建议按 AGENTS.md 在合并后进入 issue，不扩张范围。

## 5. Issue 排序（完成项只作追溯）

### 本里程碑消费

| Issue | 本次只取 |
|---|---|
| #117 | A：credential redirect policy；ChangeInbox/Prompt sections 后移 |
| #114 | 单默认 Persona + Preset 最小选择/导入；多 Persona 和完整资产生命周期后移 |
| #35 | session delete、用户隔离和确定性生命周期 |
| #37 | session-scoped 操作与 durable ID 兼容；swipe/branch/pagination 后移 |
| #105 | 复核 PR #106 已落地的 V2 运行态，关闭已完成 A0；只修仍影响 MVP 的遗留项 |

### MVP 后处理

- #87 Agent-first 完整编辑工作台；
- #115 migration report、PromptAssemblyTrace、payload capture；
- #116 Style Review；
- #117 ChangeInbox 与 prompt section contract；
- #29/#98 Tauri sidecar 与 Windows 安装包闭环；
- #32 widget capability 扩展面；
- #113 rustdoc policy；
- #104 git 历史美化；
- 世界书高级语义、长期记忆、MCP/skills/plugin 和完整 SillyTavern parity。

后移不等于关闭或否定，只表示它们不应阻塞浏览器基础 RP 使用。

## 6. 验证门

实现阶段已经执行并通过以下质量门；验收修复不得降低这些门禁：

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p airp-core subagent_context_has_no_orchestrator_noise
```

PR #123 已执行基础零密钥验收；PR #125 的自动 harness 为 64/64，并另有 50/54 → 54/54、滚动保持和键盘选择 rollback target 的真实浏览器证据。后续改动不得降低这些门禁。

## 7. 完成后的产品边界

完成本计划后，`webui/` 是**可日常完成基础 RP 的轻量浏览器客户端，同时是当前后端能力孵化与合同验证主开发面**；它仍不是最终视觉产品，也不替代长期 Tauri/Vue 产品面。后续顺序已在 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 校准。
