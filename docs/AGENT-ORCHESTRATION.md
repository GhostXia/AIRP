# Agent 编排与升级策略

> 状态：产品原则与待实现规范草案，不代表当前 engine 已交付可配置多 Agent runtime。
>
> 日期：2026-07-12
>
> 目标：允许 AIRP 提供安全、可观测的参考编排，同时让用户组合自己的角色、模型、并发、验收与升级规则。

## 1. 定位

AIRP 不把某一种“强模型 orchestrator + 若干低成本 executor”组合固化为唯一方案。模型能力、价格、额度、用户目标和任务结构都会变化；真正稳定的产品能力应是**可插拔的编排策略层**。

系统提供：

- 有界执行内核；
- 角色、任务、依赖和结果的通用模型；
- capability、allowlist 与破坏性确认；
- 结构化事件、trace、预算与取消；
- 确定性 validator 和升级闸门；
- 少量可复制、可修改的参考 profile。

用户决定：

- 有哪些角色，以及角色是否由不同模型、provider 或本地工具承担；
- 单 agent、串行 handoff、并行 fan-out、评审循环或自定义图；
- 每个任务的输入、输出、依赖、并发和预算；
- 哪些 validator 必须通过；
- 失败、证据冲突或风险升高时如何重试、降级、升级或交给人。

“用户自定义”不等于绕过系统安全边界。profile 可以收紧权限，不能扩大 host 未授予的 capability，也不能取消硬预算、审计、角色平面纯净度或破坏性确认。

## 2. 强制内核与可配置策略

### 2.1 系统强制内核

所有编排方案都必须遵守：

1. **角色平面纯净**：RP 模型上下文只包含 RP 数据；任务调度、工具说明、validator 和升级规则留在控制平面。
2. **有界**：每次 run 和每个 node 都有 step、token、成本、并发、墙钟和重试上限，并可取消。
3. **能力受控**：每个角色只获得任务所需的最小 capability；破坏性动作仍需精确确认。
4. **输入输出明确**：node 声明输入、输出 schema、依赖和验收条件；不能靠共享聊天上下文隐式传递关键状态。
5. **证据可观测**：记录路由原因、模型/provider、预算消耗、工具事件、validator 结果、重试与升级原因；默认对 secret 脱敏。
6. **写入可仲裁**：并行 worker 不直接争写同一资源；通过独立产物、patch、ChangeInbox 或单写者合并。
7. **失败可收敛**：禁止无界“模型审模型”循环；每次重试或升级必须有新证据、不同策略或明确终止条件。

### 2.2 用户可配置策略

profile 至少可以表达：

- `roles`：角色名称、职责、候选模型/provider、reasoning 档位、capability 和预算；
- `graph`：node、依赖、串并行关系、最大 fan-out；
- `routing`：按风险、复杂度、数据敏感度、工具需求或用户规则选角色；
- `validators`：测试、schema、lint、静态分析、smoke、人工确认或领域判据；
- `gates`：接受、重试、回退、升级、仲裁或停止条件；
- `merge`：单写者、patch 合并、投票、裁判模型或人工 review；
- `fallbacks`：模型/provider 不可用、超预算或 validator 失败时的替代路径。

模型名称只是 profile 数据，不进入 engine 的固定 domain model。系统应依据 capability metadata 和 eval 结果路由，而不是在代码中写死“强/弱模型品牌表”。

## 3. 参考编排，不是唯一答案

### Profile A：单 Agent 直接执行

适用：低风险、小范围、强耦合任务。

```text
agent → validator → accept / retry once / human
```

这是默认起点。不能因为支持多 Agent 就为简单任务增加协调成本。

### Profile B：规划者 + 执行者 + 升级闸门

适用：合同明确、可以拆成低耦合模块的工程任务。

```text
planner
  → executors (bounded fan-out)
  → cheap deterministic validators
  → evidence sufficient: merge
  → conflict or insufficient evidence: planner/arbitrator
```

强模型/高预算角色负责拆解、合同和冲突裁决；低成本角色执行批量工作；测试优先拦截返工。并发数由任务图和资源预算决定，不固定为 3–5。

### Profile C：独立候选 + 仲裁

适用：方案探索、诊断或高不确定性决策。

```text
candidate A ─┐
candidate B ─┼→ evidence comparison → arbitrator / human
candidate C ─┘
```

候选必须独立形成假设和证据，避免共享结论造成附和。仲裁比较可验证差异，而不是按多数票代替事实。

### Profile D：流水线与专业 handoff

适用：顺序依赖明显、角色专业化的任务。

```text
research → contract → implementation → verification → release
```

每一步只消费前一步的结构化产物；失败回到最近能改变证据的 node，不默认从头重跑。

### Profile E：用户自定义图

用户可以组合条件分支、并行、循环、人工节点和外部 validator。例如：

```text
classify
  ├─ low risk → local executor → unit tests
  ├─ security → two independent reviewers → security gate
  └─ destructive → dry run → human approval → executor → audit
```

AIRP 应允许用户保存、复制、版本化、导入和导出 profile，并在执行前展示展开后的任务图、权限与预算上限。

## 4. 升级闸门

升级不是“低成本模型失败就把全部上下文交给最强模型”。gate 应先收集最小充分证据，再决定下一步。

建议 gate 顺序：

1. schema、解析、权限和前置条件检查；
2. 单元/契约测试、lint、静态分析或领域脚本；
3. 集成测试、真实 smoke 或交叉证据比较；
4. 仅将失败断言、相关 diff、最小日志和冲突摘要交给仲裁角色；
5. 高风险、不可逆或证据仍冲突时交给人。

典型升级原因：

- validator 对同一事实给出冲突结果；
- 任务合同缺失或 executor 必须越界才能继续；
- 修改触及安全、数据迁移、协议兼容或不可逆写入；
- 重试无法产生新证据；
- 预算或时间将触顶。

## 5. 与 AIRP 现有原语的关系

- **Tool / MCP**：执行能力，不承担最终权限判断；
- **Skill**：可复用工作方法，可作为 node implementation；
- **Memory**：提供受控背景，不替代显式任务输入；
- **Hook / Event**：触发 validator、trace、预算和外部集成；
- **Subagent**：隔离执行上下文，不天然等于并行或低成本模型；
- **ChangeInbox**：承接 worker 建议、patch、preview 与 accept/reject，避免多 writer 直接改同一真相；
- **PromptAssemblyTrace**：证明实际进入模型的内容和来源；
- **Capability**：host 授权上限，profile 只能在其内分配。

## 6. 实施顺序

本规范不要求当前立即建设完整多 Agent runtime。建议在现有路线中逐步落地：

1. #37/#122 的 durable message、cursor pagination 与 WebUI window 已由 PR #124/#125 建立；剩余事件身份/branch 与产品 UI 性能沿该合同演进；
2. 当前用 #114/#115 建 RP Profile、migration report、PromptAssemblyTrace 和可验证输入来源；
3. 用 #117 建立 revisioned ChangeInbox 和单写者仲裁；
4. 再定义 versioned orchestration profile schema、运行 trace 与 validator registry；
5. 最后接入多角色路由、并行 fan-out、升级 gate 和 WebUI 图形化编排。

在 schema 与 eval 稳定前，参考 profile 只作为可替换配置和验收样例，不应硬编码成唯一运行路径。

## 7. 验收标准

未来宣称“用户可编排”至少需要证明：

- 同一任务能在单 Agent与至少两种多 Agent profile 间切换；
- 用户可以复制并修改 profile，而不改 engine 代码；
- 权限、预算和并发上限在服务端强制；
- validator 能直接接受结果，且不会无条件调用仲裁模型；
- 冲突时只升级最小相关证据；
- 多 worker 不会无仲裁地覆盖同一资源；
- run 可取消、可恢复或明确终止，trace 能解释每次路由和升级；
- 神圣不变式继续证明 RP 角色平面没有 orchestrator 噪声。
