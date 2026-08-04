# 学习：LoopX（huangruiteng/loopx）

> **研究参考**：本文记录 AIRP 借鉴 LoopX 公开设计理念的方向与边界，不代表相关能力已在 AIRP 实现。AIRP 当前仅将其作为理念参考，不复制代码、prompt、测试或资产；任何未来依赖必须重新核验许可证兼容性。当前落地状态见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。

> 对象：https://github.com/huangruiteng/loopx —— 面向长程 AI agent 与 peer agent team 的「轻量状态内核 + 本地控制面」。Python 3.11+，零运行时依赖（仅标准库）。许可证记录已于 2026-07-27 复核为 MIT。
> 一句话定位：**LoopX 不替代任何 agent runtime（Codex / Claude Code / Cursor / shell），而把目标、工作、权限、证据、配额、交接等跨 turn 才需稳定存在的事实外置成可恢复、可审计的状态**。它的隐喻是 *agent-native Kanban*：卡片携带 identity / authority / evidence / continuation，移动是经校验的算子（claim / gate / monitor / writeback），不是 UI 手势；看板是投影，状态内核才是真相。
> 性质：理念参考。一切以 AIRP 实际需求与 [PLAN.md](PLAN.md) §1 为准。
> 研究更新：2026-08-04；审查基线：`main` 分支 commit `f4bf2684a903e5e204fb46bc1a7d281643a41b0f`（2026-07-27），覆盖 `README.md`、`docs/architecture.md`、`docs/development/control-plane-course/00-concept-primer.md`、`docs/public-private-boundary.md`、`AGENTS.md`、`docs/product/vision.md`、`docs/product/release-readiness.md`。

---

## 一句话

它跟我们做的是不同领域（它面向工程/研究/recurring 监控类长程任务，我们面向 RP 客户端），但它在「**长程任务状态外置**」上给出了完整的概念框架——把 AIRP 审计守则中"危险权限归人""Merged≠Runtime-active""公私边界""PR 门禁"等零散直觉，提炼成了系统化的控制面分层。这种理念收益远高于直接复用代码。

## 该学的（按对我们价值排序）

### 1. 🌟 状态外置分类法 —— 模型上下文是工作内存，不是长期状态

LoopX 的核心工程预设：长程任务不能建立在"模型应该还记得"上。完整 transcript 既可能超窗，也混合了已验证事实、临时猜测、过期观察和未提交计划。真正需要跨 turn 延续的内容必须被提炼成**有身份、有作用域、可验证**的外置状态：

| 必须延续的问题 | 不能只留在聊天里的原因 | LoopX 外置形态 |
| --- | --- | --- |
| 最终要达到什么，什么算完成？ | 摘要可能丢失约束，局部任务可能替代最终目标 | goal、vision、acceptance、boundary |
| 现在有哪些工作，谁正在做？ | 多个 session 会重复领取或遗漏 successor | todo、frontier、claim、lease |
| 哪些动作允许执行？ | 能力、工作归属和外部写权限不是同一件事 | authority、gate、capability、workspace guard |
| 哪些判断已经被证明？ | "我运行过"不等于结果可信或 effect 已发生 | evidence、effect receipt、fresh observation |
| 何时再看，何时停止？ | 周期唤醒会热轮询，空 todo 也可能误停 | monitor、scheduler hint、backoff、terminal audit |
| 失败或换人后从哪里继续？ | transcript 不能提供稳定 identity 和幂等边界 | event、run history、lineage、replay、repair |

**对 AIRP 的意义**：角色卡、世界书、会话、记忆这些跨 session 资产，正是"不能只留在聊天里"的典型场景。AIRP 已有封卷记忆、session 自包含快照、`subagent_context_has_no_orchestrator_noise` 不变式，但缺乏这样一份系统化的"状态外置分类法"总览。可借鉴此表为 AIRP 起草一份《AIRP 状态外置分类》。

### 2. 🌟 4 责任模型 —— Agent / Provider / Capability / Kernel

LoopX 把运行时责任切成四份，每份有明确的 owns 与 must-not-own：

| 责任 | Owns | Must not own |
| --- | --- | --- |
| **Agent** | 规划、分析、工具使用、一次有界执行 | 持久 goal 生命周期或越界 effect 权限 |
| **Provider** | 外部调用、有界观察、effect 结果、readback | 领域 transition 策略或 LoopX todo 状态 |
| **Capability** | 调用者结果合同、领域策略、验证、typed transition 提议 | 持久调度、claims、gates、直接生命周期写入 |
| **Kernel** | goal、todo、claim、gate、monitor、quota、accepted writeback、recovery、调度 | 领域推理或 provider 实现细节 |

数据流方向相反：
```
Agent -> Capability -> Provider -> 外部系统
外部观察 / effect readback -> Provider -> Capability
typed transition proposal -> Kernel -> 下一个 todo / gate / monitor / turn
```

**关键洞察**：观察 ≠ transition；provider receipt ≠ 已接受的进度。capability 必须验证，Kernel 才提交状态变更。

**对 AIRP 的意义**：AIRP 已有 AIRP-Core / AIRPCLI / AIRP-MCP-Server / AIRP-Gateway / AIRP-State-Protocol 的第一方分层，但缺少"runtime 责任四分法"这样的边界守则。可借鉴为 AIRP 架构文档的边界审查清单——尤其对"Provider 不能维护第二套状态机""Agent 不能持有持久 lifecycle 权限"这两条，AIRP 在 MCP 工具与 engine 推理的边界上需保持同样纪律。

### 3. 🌟 Authority 四分法 —— 能看见 / 能提议 / 能执行 effect / 能提交 terminal

LoopX 把"权限"细分成四种独立粒度：
- **能看见**（see）：能读到某状态
- **能提议**（propose）：能提出 transition 建议
- **能执行 effect**（effect）：能调外部系统做事
- **能提交 terminal**（commit terminal）：能宣告 goal 完成

> 一个 Agent 可以有能力分析发布风险，却没有权执行发布；host 可以执行 API 调用，却不能自行扩大 proposal scope。

**对 AIRP 的意义**：直接对应 AIRP 审计守则的"危险权限、发布、生产写入归人"原则。AIRP 当前在 daemon / webui / MCP 工具 / 插件数据面的权限边界上，可借鉴这种四分法做更细的分层——例如插件能"提议"角色卡修改，但"执行写入"归 engine；webui 能"看见"会话状态，但"提交封卷"归 daemon 权限。

### 4. Claim / Lease / Gate 三分法 —— 锁的范围与责任范围不匹配是常见陷阱

| 概念 | 回答的问题 | 典型生命周期 |
| --- | --- | --- |
| Claim | 这项工作由哪个 Agent lane 接手？ | 可交接、释放或完成 |
| Lease | 当前执行窗口是否被占用，何时过期？ | 短期、可续租、可回收 |
| Gate | 哪个 scoped decision 尚未满足？ | 由对应 authority 明确解决 |

**关键约束**：
- Claim 不自动授予生产写权限
- Lease 不代表长期 ownership
- 普通 user todo 不冻结整个 goal
- Gate 必须带清晰 scope；若 scope 缺失，应修复投影或状态，而不是猜测它约束所有 Agent

**对 AIRP 的意义**：AIRP 已完成 R1 lock order convergence（见 [LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md)），但 Lock 与 Claim / Lease / Gate 是不同维度的概念。AIRP 当前主要解决"锁的获取顺序"，而 LoopX 这套区分解决"锁的范围与责任范围匹配"。两者互补——AIRP 可在 R1 基础上补充一份"锁的范围与责任范围匹配"审查清单。

### 5. 🌟 Evidence + Effect receipt + readback —— 操作完成 ≠ 业务效果

LoopX 的完整 transition 链路：
```
proposal -> authority check -> provider effect -> readback -> receipt -> state commit
```

并明确：
- 「命令返回 0」通常只证明进程退出成功，**不能证明业务效果已发生**
- 若 effect 成功后、receipt 写回前崩溃，稳定 identity + 幂等规则让恢复逻辑先 reconcile，而非盲目重试

**对 AIRP 的意义**：直接映射 AIRP 已交付的 #342 备份/恢复闭环与审计门禁。AIRP 的备份操作"返回 0"不等于"备份可用"——需要 readback（恢复测试）验证。这正好印证 AIRP W-01/W-06 修复方向。可借鉴 LoopX 的链路命名（proposal → authority → effect → readback → receipt → commit）为 AIRP 备份/恢复文档补充术语。

### 6. 🌟 Canonical state vs Projection —— 单一真相原则

LoopX 的硬规约：
- **Canonical state**：可重放的生命周期事实
- **Projection**：从事实生成的 status / dashboard / 报告 / 通知
- Projection 可为不同角色重组信息，**但不能反向成为第二套真相**
- Dashboard 与 CLI 状态矛盾时：**修 projection 或 source contract，不要手工同步两处**

**对 AIRP 的意义**：这是 AIRP webui 与 engine 状态一致性问题的答案——webui 是 projection，engine 是 canonical。两者冲突时是契约修复，不是双写。AIRP 当前已有 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) 与 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)，可把这条"projection 不可反向成为第二套真相"作为 webui 开发指南的最高原则显式写入。

### 7. 🌟 "Merged Is Not Runtime-Active" —— 发布纪律

LoopX 的发布就绪文档明确：
> A post-merge check proves behavior on the tested source commit. It does not prove that an installed LoopX runtime contains that commit.

PR 合并 ≠ 用户已用上；package version 匹配 ≠ installed source commit 是最新。

**对 AIRP 的意义**：这是 release readiness 的深刻洞察，直接对应 AIRP #130 maintainer 验收（real provider onboarding + production Compose + real browser smoke testing）。AIRP 0.0.3 的 P1 阻塞项正是同一种意识——PR 合并不等于用户能跑通。可借鉴 LoopX 的"merged is not runtime-active"作为 AIRP release checklist 的固定项。

### 8. Capability / Provider / Extension 正交分离

LoopX 的边界规则：
- **Capability**：调用者结果合同，**按调用者结果命名，不按实现机制命名**（例："判断 issue 是否适合形成修复 PR" 是 capability；"调用 Git CLI" 只是 provider 机制）
- **Provider**：连接真实系统的实现者，可替换但保留合同
- **Extension**：独立交付/升级/启停的 provider 包，**不为让 extension 可安装就虚构 capability**
- Capability / Extension 正交：capability 是产品合同，extension 是独立交付单元

**关键守则**：不要为了让一个 extension 可安装就虚构 capability。只有当 LoopX 调用者真的需要一个 provider-neutral 的稳定结果合同时，才应把它提升为公共 capability。

**对 AIRP 的意义**：AIRP 已有 CAPABILITY-ABSORPTION.md 与 MCP-SERVER-ABSORPTION.md，可借鉴 LoopX 的"capability 命名规则"和"extension 不变成第五种 runtime 责任"作为新增模块时的审查清单——尤其当 AIRP 引入第三方 provider（如 LLM provider、图像 provider）时。

### 9. Public / Private boundary 三句话规约

LoopX 的公私边界三句话原则极具操作性：
- **Public repo** 回答「loopx 怎么工作？」（schemas、runtime 目录约定、通用 CLI、adapter 生命周期规则、脱敏示例）
- **Project repo** 回答「这个具体 goal 当前在做什么？」（local 路径、内部 repo 名、raw 日志、task id、credentials、活跃 goal state）
- **Runtime root** 回答「近期 goal tick 发生了什么？」

并明确：
- Public artifact 应保持：schema names、role names、脱敏 work-scope 例子、lifecycle states、通用 merge 规则
- Private project state 应保持：raw child prompts、raw trajectories、local task evidence、非公开 repo 名、含项目特定上下文的命令输出
- Run summaries 仅在脱敏后才可发布

**对 AIRP 的意义**：对应 AIRP 审计守则的"流程现状"与"审计遗留项处理"中的公私边界意识。LoopX 的 boundary scan 做法可直接借鉴成 AIRP 的 PR boundary lint——尤其 AIRP 的 webui 截图、烟测输出、审计报告中可能含本地路径或私有上下文，可按这三句话规约做扫描。

### 10. Lifetime Goal Invariant —— 长期稳定 + 单步有界

LoopX 的产品不变式：
> LoopX should optimize for **lifetime goals**: durable intentions that may outlive a single thread, executor, project phase, or plan.

并约束：
> A goal can live for years, but every agent turn still has to pass through current authority, boundary, quota, validation, and writeback before it can count as progress.

即"preserve continuity without claiming open-ended autonomy"。

**对 AIRP 的意义**：对应 AIRP AGENTS.md 的"代际重构特例"——允许大跨度演进，但每步仍有门禁。这正是"长期稳定 + 单步有界"的同一哲学。AIRP 的角色卡、世界书、会话记忆正是"lifetime asset"，而每个 turn 的 engine 推理仍须穿过干净提示词、不变式、审计门禁。两者理念高度共鸣。

### 11. Agent-native Kanban 投影 —— 看板是图，控制面是合同

LoopX 的核心隐喻：
> Kanban is the picture; the control plane is the contract.

- Card = Todo（带 stable todo_id、owner、依赖、scope、evidence、continuation）
- Column = 从 canonical state 派生的 lane / view（列由 lifecycle、task class、routing、proof/time 等维度组合，不是单个字符串）
- Move card = Typed transition（每次移动都检查 authority、前置条件、验证结果和 writeback）
- WIP limit = Claim / lease / quota / workspace guard
- Done = Accepted writeback + receipt + terminal audit

**对 AIRP 的意义**：AIRP 的会话编排、Agent loop、工具调用序列可视化为 webui 时，可借鉴这种"投影而非真相"的视角。webui 的会话流、工具调用展示是 projection，背后的 engine state machine 才是合同。

### 12. CLI packet 过程协议 —— 每 turn 编译当前状态为可执行 packet

LoopX 把每 turn 的执行流程外置成：
```
canonical state + fresh environment
  -> status / quota decision
  -> interaction contract + evidence refs + next CLI actions
  -> host task body / visible packet
  -> bounded Turn
```

Packet 要足够薄，只携带本轮 identity、允许动作、必要 gate、验证条件和 writeback 命令；但又不能薄到遗漏 required proof、scope 或 terminal gap。

**对 AIRP 的意义**：AIRP 是否需要这种"过程协议"外置，取决于 AIRP 的 agent runtime 拓扑——如果 AIRP 是单进程 engine + webui，可能不需要；如果有 MCP / Gateway 多 agent 接力，则很有价值。**当前 AIRP 不建议引入**，但保留为未来多 agent 场景的候选模式。

## 与自有项目比对（防重合 / 冲突）

> 核对对象：AIRP 已有审计守则、[SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)、[LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md)、[CURRENT-BASELINE.md](CURRENT-BASELINE.md)。目的：LoopX 的学习点里，凡 AIRP 本来就有的，标为"已有·仅佐证"，不当新能力重复建设；只有净新增量才进入候选路线。

| # | LoopX 点 | AIRP 现状 | 结论 |
|---|---|---|---|
| 1 | 状态外置分类法 | 已有封卷记忆、session 自包含快照、`subagent_context_has_no_orchestrator_noise` 不变式；缺系统化分类总览 | **半新**：补一份《AIRP 状态外置分类》文档，不新增平行状态系统 |
| 2 | 4 责任模型 | 已有 AIRP-Core / AIRPCLI / AIRP-MCP-Server / AIRP-Gateway / AIRP-State-Protocol 分层；缺 runtime 责任四分法边界守则 | **半新**：补 runtime 责任清单，不重构现有模块 |
| 3 | Authority 四分法 | 审计守则有"危险权限归人"原则；缺四分法细化 | **净新·真值得做**：补权限分层清单 |
| 4 | Claim / Lease / Gate 三分法 | 已有 R1 lock order convergence；缺锁的范围与责任范围匹配审查 | **半新**：在 R1 基础上补"范围匹配"审查清单 |
| 5 | Evidence + Effect receipt + readback | 已有 #342 备份/恢复闭环 + W-01/W-06 修复；缺链路命名标准化 | **半新·形态**：补链路术语，不重造闭环 |
| 6 | Canonical vs Projection | 已有 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) 与 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)；缺"projection 不可反向成为第二套真相"的显式原则 | **半新·小**：把原则显式写入 webui 开发指南 |
| 7 | Merged ≠ Runtime-active | #130 maintainer 验收正是此意识；缺 release checklist 标准化 | **半新·小**：补 release checklist 固定项 |
| 8 | Capability / Provider / Extension 正交 | 已有 [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md) 与 [MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)；缺"capability 命名规则"与"extension 不变第五责任"守则 | **半新·小**：补命名审查清单 |
| 9 | Public / Private boundary 三句话 | 审计守则有公私边界意识；缺三句话规约与 boundary scan | **净新·真值得做**：补 PR boundary lint 规约 |
| 10 | Lifetime Goal Invariant | 代际重构特例已体现此哲学；缺产品级不变式表述 | **重合·仅佐证**：AIRP 已有此意识，LoopX 仅外部印证 |
| 11 | Agent-native Kanban 投影 | webui 已是 projection；缺 Kanban 隐喻形式化 | **重合·仅佐证**：不新增 Kanban 抽象 |
| 12 | CLI packet 过程协议 | AIRP 是单进程 engine + webui，非多 agent 接力拓扑 | **不适用·暂不引入**：保留为未来多 agent 场景候选 |

**净提炼——真正值得纳入 AIRP 路线的只有：**
- **点 3（净新）**：Authority 四分法权限分层清单 → 未来权限/安全文档。
- **点 9（净新）**：Public / Private boundary 三句话规约 + PR boundary scan → PR 门禁增强。
- **点 1/2/4/5/6/7/8（半新·形态/小）**：补术语、清单、原则显式化，不新增平行实现。
- **点 10/11（重合·仅佐证）**：AIRP 已有此意识，LoopX 仅外部印证方向。
- **点 12（不适用）**：当前不引入，保留为未来候选。

## 不学 / 差异

- **技术栈**：LoopX 是 Python 3.11+ 零依赖；AIRP 主语言是 Rust（见 [AGENTS.md](../AGENTS.md) 工具链路径）。不复制任何实现。
- **领域差异**：LoopX 面向工程/研究/recurring 监控类长程任务（Issue-Fix / Auto ML / Auto Research capability pack）；AIRP 面向 RP 客户端，领域泳道是角色卡生成 / 世界书维护 / 会话编排等。LoopX 的 capability pack 实现不能照搬，但其"capability / domain state / kernel 三分"结构可借鉴。
- **runtime 拓扑差异**：LoopX 是 CLI + 可选 host adapter（Codex / Claude Code / Cursor）；AIRP 是单进程 engine + webui + 可选 MCP / Gateway。LoopX 的 CLI packet / heartbeat scheduler 模式适合多 host 接力，AIRP 当前不需要。
- **License**：MIT 表面允许，但按 AGENTS.md「第三方经验吸收与独立实现」规则，AIRP 不得复制、翻译、改写或移植其源码、prompt、测试、fixtures、HTML/CSS、图标及视觉资产。仅吸收理念。
- **产品边界**：LoopX 自身定位为"local coordination substrate"，明确"dangerous permissions, publishing, production writes, and final ownership stay with the human/operator"。AIRP 的 RP 产品闭环（角色卡 / 世界书 / 会话 / 记忆 / webui）是 LoopX 不涉及的方向，AIRP 不应让 LoopX 的控制面叙事替 AIRP 决定产品边界。

## 落地建议（只列净新与半新，去掉重合与不适用）

- **Authority 四分法权限分层（净新·点 3）**：不建独立静态平行系统；并入 AIRP 未来权限/安全文档，并与现有 daemon / webui / MCP 工具 / 插件数据面边界统一。当前不立即实施，进入 issue 队列评估。
- **Public / Private boundary 三句话规约 + PR boundary scan（净新·点 9）**：可立即作为 PR 门禁增强候选。建议在 `.github/` 或 `docs/` 下加一份 boundary lint 规约，扫描 PR 改动中的本地路径、内部 repo 名、raw 日志、credentials、私有上下文。进入 issue 队列评估。
- **状态外置分类总览（半新·点 1）**：补一份《AIRP 状态外置分类》文档，把 AIRP 已有的封卷记忆、session 快照、不变式、`data/` 目录布局整理成类似 LoopX 的"必须延续的问题 → 外置形态"表。不新增平行状态系统，仅整理已有事实。
- **Runtime 责任四分法清单（半新·点 2）**：在 AIRP 架构文档中写下 Agent / Provider / Capability / Kernel 的"owns / must-not-own"表，作为新增模块时的边界审查清单。
- **Effect receipt + readback 链路命名（半新·点 5）**：在 [BACKUP-RESTORE.md](BACKUP-RESTORE.md) 与审计文档中补"proposal → authority → effect → readback → receipt → commit"链路术语，统一 AIRP 已有的备份/恢复闭环描述。
- **Canonical vs Projection 原则显式化（半新·点 6）**：把"projection 不可反向成为第二套真相；webui 与 engine 状态矛盾时修 projection 或 source contract，不双写"作为最高原则显式写入 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md) 或 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)。
- **Merged ≠ Runtime-active release checklist（半新·点 7）**：在 AIRP release 文档中加入"PR 合并 ≠ 用户已用上；package version 匹配 ≠ installed source commit 最新"作为固定 checklist 项，对应 #130 maintainer 验收。
- **Capability 命名规则（半新·点 8）**：在 [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md) 中补"capability 按调用者结果命名，不按实现机制命名""extension 不变成第五种 runtime 责任"作为新增模块审查清单。
- **不做**：CLI packet 过程协议（点 12，当前不适用）、Agent-native Kanban 抽象（点 11，已有 webui projection）、Lifetime Goal Invariant 表述（点 10，已有代际重构特例）——这些 LoopX 仅作外部佐证，不重造。

## 验证与归档

本文研究后已按 AGENTS.md「第三方经验吸收与独立实现」规则，在 [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) 第 2 节"第三方设计参考"表中登记 LoopX 条目，记录项目、吸收经验、当前关系、审查基线（commit `f4bf2684a903e5e204fb46bc1a7d281643a41b0f` / 2026-07-27）、许可证核验（MIT）与 AIRP 记录（本文）。

后续如落地某条建议，应：
1. 在 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 记录落地状态；
2. 在 [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) 第 4 节"维护待办"勾选相应项；
3. 若形成运行时或发布依赖，进入 provenance、notices 与 SBOM，不再列作"理念参考"。
