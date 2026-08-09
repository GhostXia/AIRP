# 用户自定义流程（User-Defined Flow）设计抉择

> ⚠️ **Superseded for current status**：本文是历史设计留痕，不是当前交付承诺；当前边界以 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md) 为准。
>
> 关联代码：[`engine/src/agent/mod.rs`](../../engine/src/agent/mod.rs) · [`engine/src/agent/director.rs`](../../engine/src/agent/director.rs) · [`engine/src/plugin_tool.rs`](../../engine/src/plugin_tool.rs)
> 基线：`main@1b14a7c`（PR #454 合并后）
> 范围定位：**仅设计留痕**，不进入 0.0.3 路线。实现归入 0.0.4+ epic。
> 状态：设计已定，未实施。

## 0. 起源

评估第三方项目 `chuspeeism/dashi-taskboard` 时，用户提出："内嵌体系（AIRP 的 `AgentLoop` 协调器）= 默认安排的流程；如果用户想要自定义这个流程，AIRP 提供了接口来实现吗？"

经查证：

- AIRP 在 [plugin_tool.rs](../../engine/src/plugin_tool.rs) 已提供 Phase 5.3 Plugin Tools（用户可热更新工具集），但**工具调用顺序仍由协调器自主决策**。
- 流程编排层（"下一步选什么动作"）当前是编译期固定的 Rust 协调器逻辑（[agent/mod.rs::run_loop](../../engine/src/agent/mod.rs)），用户无法重写。
- 可配置的部分（Director / Council / `max_steps`）都是**参数化开关**，不是用户可定义的流程图。

dashi-taskboard 实际没有"用户自定义流程"的能力——它的 `manage-taskboard` skill 是把 `todo → in_progress → in_review → done` 状态机硬编码在 skill prompt 文本里，由外部编码 agent（Codex）按指令改 issue 字段。既无"用户自定义流程图"，也无"面板上编辑流程"。本设计是 AIRP 原生设计，**不吸收 dashi 代码、不写入 [`docs/ACKNOWLEDGEMENTS.md`](../ACKNOWLEDGEMENTS.md)**。

用户决定：**并存**——既保留 AIRP 设计好的两种机制，也允许用户自定义流程，三者互不干扰。理由与 AGENTS.md "更开放、更透明、在未来更易修正、且更易迭代更新" 的项目取向一致。

## 1. 现状

| 维度 | 当前事实 |
|---|---|
| 协调器层 | `run_loop` 每步调用 `decide_action`（控制平面 LLM）选 `Generate / CallTool / Finish`；`max_steps == 1` 退化为单回合 |
| 角色平面层 | `prepare_pipeline` 装配**全新纯净** subagent 上下文（card / lorebook / preset / 卷 / state） |
| Director | [agent/director.rs](../../engine/src/agent/director.rs) 周期性写 `director_directive.md`，prepare 阶段读取后**注入 subagent 的 system prompt** |
| Council | [agent/council.rs](../../engine/src/agent/council.rs) 多角色发言顺序（round_robin / random / relevance） |
| 戒律#6 | `subagent_context_has_no_orchestrator_noise` 测试守护：协调器状态不进 subagent 上下文 |
| Plugin Tools | Phase 5.3 用户可热更新工具集，但调用时机由协调器决定 |

## 2. 关键架构认知：两个正交维度（非"三种模式"并列）

**这是本设计的核心**。AIRP 的"流程"实际跨两个正交维度，不能简化成"三种模式互斥"：

```
维度1（协调器层 — 决定"下一步做什么动作"）
├─ A. AI 自主决策（decide_action）        ← 现状默认
└─ C-action. 用户流程的 action 字段决定   ← 新增

维度2（角色平面层 — 决定"是否注入内容指令"）
├─ B. Director 内置叙事                   ← 现状
├─ C-directive. 用户流程的 directive 字段 ← 新增
└─ 无注入                                 ← 现状（Director 关闭时）
```

**现状**：A 和 B 可以同时工作（协调器自主决策动作 + Director 周期性注入叙事）。它们不是"二选一"。

**C 方案**：跨两个维度，在每个维度上分别与 A / B 互斥。

## 3. 设计决策

### 决策 1：C 方案的"互不干扰"= 字段级正交，不是模式级互斥

不是"开了 C 就关 A 和 B"，而是字段级组合：

| 用户流程步骤填法 | 协调器层走 | 角色平面层走 |
|---|---|---|
| 不带 `flow` 字段（默认） | A（自主决策） | B（Director）或无 |
| `flow` 存在，步骤只填 `action` | C-action | B 仍可工作 |
| `flow` 存在，步骤只填 `directive` | A 仍可工作 | C-directive（B 该轮跳过） |
| `flow` 存在，步骤两者都填 | C-action | C-directive（B 该轮跳过） |
| `flow` 存在，步骤两者都不填 | 退化回 A | 退化回 B 或无 |

**理由**：如果把 C 当成整体开关，会导致"开了 C 就关 A 和 B"的错误实现，丧失灵活性。字段级正交才是"互不干扰"的准确含义。

### 决策 2：`UserFlowStep` 字段分层

```rust
pub struct UserFlowStep {
    // ── 控制平面（与戒律#6 兼容，不进 subagent 上下文）──
    pub action: PlanAction,                // Generate | CallTool { tool, params } | Finish
    pub condition: Option<ObsCondition>,   // 基于 observations 的分支（可选）

    // ── 角色平面（经 Director 通道注入 subagent 上下文）──
    pub directive: Option<UserDirective>,  // "这一轮表现悲伤" 类内容指令（可选）
}
```

`action` 走协调器（与 `decide_action` 同层），`directive` 走 `director_directive.md` 通道。两个字段的边界在数据结构层面分开，**不能混在一个 String 里**。

### 决策 3：与 Director 的协调规则（角色平面层互斥）

现状：Director 周期性写 `director_directive.md`。C 方案的 `directive` 字段也要写同一文件。

**冲突规则**：

- 用户流程步骤带 `directive` 时，**该轮 Director 评估跳过**（用户显式指令优先于内置导演）
- 用户流程步骤不带 `directive` 时，Director 正常工作
- 文件写入用**原子替换**（复用 `revision/atomic.rs` 的 staging→rename 模式），不允许追加（避免用户指令与导演指令叠加）

这条规则要在 doc 里写死，并在测试中守护。

### 决策 4：协调器层的互斥规则

`req.flow.is_some()` 时走 C-action，否则走 A。**不混合**——同一步骤不会既走 `decide_action` 又走用户流程的 `action`。

### 决策 5：新增两条不变式测试

```
invariant_1: user_flow_action_field_never_enters_subagent_context
  // action / condition 字段不出现在 prepare_pipeline 装配的 system prompt / messages

invariant_2: user_flow_directive_field_only_via_director_channel
  // directive 字段只通过 director_directive.md 注入，不进 req.base
```

**第二条是对戒律#6 的有意、有界违反**——必须在测试注释里明确标注"这是 Director 同款通道的扩展，非 bug"。戒律#6 的本意是"协调器多步状态不污染角色平面"，而 Director 通道是**已存在、已审查**的"有意注入点"，C-directive 复用此通道不引入新的注入路径。

### 决策 6：capability 门控不绕过

用户流程里的 `CallTool` 仍受 `AgentRunRequest.capabilities` + `allowed_tools` + `confirm_tools` 门控。用户流程定义可以列出任何工具名，但实际调用时若无 capability，按现有的 `tool not granted for this run` 拒绝。**用户流程定义不绕过权限模型**。

### 决策 7：`AgentRunRequest` 扩展

`AgentRunRequest` 已经是 `ChatCompletionRequest` 的超集，再加一个可选字段不破坏现有合同：

```rust
pub struct AgentRunRequest {
    #[serde(flatten)]
    pub base: ChatCompletionRequest,
    #[serde(default = "default_max_steps")]
    pub max_steps: u32,
    // ... 现有字段 ...
    /// 用户自定义流程。None → 走 A（AI 自主决策）。Some → 走 C-action。
    #[serde(default)]
    pub flow: Option<UserFlow>,
}
```

### 决策 8：持久化复用 plugin_tools 模式

- `data/user_flows.json` — 流程配置（不含密钥）
- HTTP 端点（建议）：
  - `GET /v1/user-flows` — 列出
  - `POST /v1/user-flows` — upsert（按 name）
  - `DELETE /v1/user-flows/:name` — 删除
  - `POST /v1/user-flows/:name/test` — dry-run 测试
- 命名空间校验：`^[a-z0-9_]{1,64}$`，不与内建保留前缀冲突（与 [plugin_tool.rs::validate_tool_name](../../engine/src/plugin_tool.rs) 同构）

### 决策 9：WebUI 编辑入口

AIRP 已有 webui + Tauri UI。在 webui 加一个"流程编辑"screen 即可，**无需注入外部应用**（这是与 dashi 的本质差异——dashi 靠 CDP 注入 Codex 侧栏，AIRP 自有 UI 表面）。

形态建议：**有序步骤数组 + 条件分支**，**不做图编辑器**（图编辑器是过度设计，与 AIRP 当前 UI 形态不匹配）。

## 4. 与戒律#6 的关系

戒律#6（[agent/mod.rs#L7-L13](../../engine/src/agent/mod.rs)）要求**协调器状态不进 subagent 的 system prompt / messages**。

| C 方案字段 | 与戒律#6 关系 |
|---|---|
| `action` / `condition` | ✅ 完全兼容。描述协调器层面的动作选择，与 `decide_action` 同层，subagent 上下文仍由 `prepare_pipeline` 装配 |
| `directive` | ⚠️ 有意、有界违反。复用 Director 已审查的 `director_directive.md` 通道，不引入新注入路径。需在测试中明确标注 |

**关键不变式**：用户流程定义的 `action` / `condition` 字段**不得包含可注入 subagent 上下文的内容**（只能描述"协调器下一步选什么动作"，不能描述"subagent 的 system prompt 加什么"）。这条新不变式需要测试守护。

## 5. 安全约束（与 plugin_tools 同构）

| 约束 | 值 |
|---|---|
| 流程名规则 | `^[a-z0-9_]{1,64}$`，不以数字开头 |
| 步骤数上限 | 建议≤32（避免 runaway） |
| `directive` 字段长度 | ≤4096 chars |
| 工具调用 | 仍受 capability + allowed_tools + confirm_tools 门控 |
| 持久化 | atomic write（复用 `revision/atomic.rs`） |
| 鉴权 | 复用 `access_api_key` 门控（与 Director 同） |

## 6. 实现路线（0.0.4+，非本 PR 范围）

1. **Phase 1：数据结构与持久化**
   - `UserFlow` / `UserFlowStep` / `UserDirective` / `ObsCondition` 类型
   - `data/user_flows.json` + atomic persist
   - HTTP 端点（CRUD + test）

2. **Phase 2：协调器层接入（C-action）**
   - `run_loop` 增加 `req.flow` 分支
   - 不变式测试 1：`user_flow_action_field_never_enters_subagent_context`

3. **Phase 3：角色平面层接入（C-directive）**
   - 复用 `director_directive.md` 通道
   - Director 协调规则实现 + 测试
   - 不变式测试 2：`user_flow_directive_field_only_via_director_channel`

4. **Phase 4：WebUI 编辑 screen**
   - 有序步骤数组编辑器
   - dry-run 预览

5. **Phase 5：集成测试**
   - 字段级正交的 5 种组合（决策 1 表格）
   - capability 门控不绕过

## 7. 不做的事

- **不做图编辑器**：有序步骤数组 + 条件分支足够，图编辑器是过度设计
- **不做流程分享市场**：用户流程是本地配置，不引入云市场
- **不做流程版本管理**：复用 `revision` 体系即可，不单独造
- **不绕过戒律#6**：`action` / `condition` 字段严格不进 subagent 上下文
- **不引入外部应用注入**：WebUI 自有表面，不学 dashi 的 CDP 注入模式
- **不写入 ACKNOWLEDGEMENTS.md**：本设计是 AIRP 原生，dashi 没有可吸收的"更好的处理"

## 8. 与 0.0.3 路线的关系

当前 0.0.3 阻塞在 #130（P1，maintainer 验收准备），W-02~W-05 在 issue 队列。**本设计只留痕，不实现**。实现归入 0.0.4+ epic，待 0.0.3 release 后启动。

## 9. 决策来源

- 用户指示（2026-08-04）："并行存在如何？这样既能给予 AI 自主决定权，还能让高级用户手动定义流程"
- 用户指示（2026-08-04）："我选择并存。不能因为'最小工作量'，而限制本项目的扩展能力"
- 用户指示（2026-08-04）："三种模式...这样就能做到互不干扰了"→ 经修正为"两个正交维度、字段级正交"（见 §2）
- AGENTS.md 项目取向："更开放、更透明、在未来更易修正、且更易迭代更新"
- AGENTS.md 第三方经验吸收规则：本设计是 AIRP 原生，不吸收 dashi 代码
