# 学习：NeuroBook（notnotype/neuro-book）

> **研究参考**：本文不代表相关 long-form memory、角色知识视角或 authoring workflow 已在 AIRP 实现。AIRP 当前仅将其作为理念参考，不复制代码或资产；任何未来复用必须重新核验许可证兼容性。当前落地状态见 [PROJECT-AUDIT-2026-07-10.md](PROJECT-AUDIT-2026-07-10.md)。

> 对象：https://github.com/notnotype/neuro-book —— 本地 AI 工作台 IDE，长篇小说写作 + AI RP。Nuxt/Vue/Bun/SQLite/Prisma。许可证记录已于 2026-07-11 复核为 AGPL-3.0；此前 PolyForm Noncommercial 的记录已经过时。本文仍仅参考理念，不搬代码。
> topics 直接含 `airp` / `rp` / `sillytavern` / `harness` / `agent`——与我们**高度同域**，且它**独立收敛出与我们相同的多个核心设计**，佐证方向、并提供可借鉴的具体形态。
> 性质：学习参考。一切以我们实际需求为准（[[PLAN.md §0]]）。
> 最后更新：2026-07-11

---

## 一句话
它跟我们做的是同一类东西（RP + agent harness + 酒馆导入 + 干净上下文），但偏"小说写作 IDE"。它在**干净提示词 / 世界书解耦 / 记忆分层**上的做法，和我们的设计**独立吻合**——说明我们方向对，且它给出了值得借鉴的具体形态。

## 该学的（按对我们价值排序）

### 1. 🌟 TSX Profile 系统 —— prompt 是类型化组件树，不是字符串拼接
- NeuroBook 把 prompt 做成 **TSX 组件树**，结构显式、可测；分三层：
  - `HistorySet` → 长期稳定上下文（身份、规则）
  - `DynamicSet` → 本轮临时状态
  - `AppendingSet` → 工作区提醒、激活的技能、用户输入
- 每个 profile 声明 `key/kind/name/inputSchema/outputSchema/allowedToolKeys/buildPrompt(ctx)` → "一个明确的**运行时合同**"。
- **对我们的意义（强相关）**：这正是我们 §1 干净提示词 + §3.5 载荷按可变性排序的**一种可落地实现形态**。我们的 orchestrator 现在是字符串装配；可借鉴"把角色平面装配做成**结构化、声明式、带 schema 的组件树**"——稳定/动态/追加三层天然对上我们的"稳定前缀在前、易变在后"（缓存纪律 + Hermes frozen-snapshot）。`allowedToolKeys` 也天然是我们控制平面 + capability 的落点。**建议：Phase 2 立 orchestrator 装配的结构化 profile 抽象时，参考这套分层 + 运行时合同。**

### 2. 🌟 Per-subject 知识隔离 —— 角色只知道"自己该知道的"，非全知世界书
- NeuroBook 规定：**Subject-facing Knowledge 不得直接从完整的 Canonical Lorebook Entry 拷贝**，防上下文泄露。角色知识单独存 `simulation/subjects/{id}/knowledge.md`，只含该角色"已知/被告知/观察/推断/误解"的内容。
- **对我们的意义（强相关，RP 沉浸命门）**：这比酒馆世界书懒加载更进一步——把"角色知道什么"做成**一等的 per-角色知识模型**，而非给角色喂全知世界书。直接解决"角色知道太多/破沉浸"。**建议：世界书引擎（Task 1.3）+ Soul/记忆里，引入"角色视角知识"层——注入角色平面的是该角色该知道的子集，不是全量世界书。守干净提示词的同时提升演绎可信度。**

### 3. 内容节点 retrieval / inject / refs 语义 —— 印证并细化我们 §5 世界书解耦
- NeuroBook 的引用分四类：普通链接（作者导航）、`refs`（结构关系）、`retrieval`（信号：agent 何时该回忆此节点）、`inject`（信号：何时直接注入上下文）。
- **对我们的意义**：这**独立印证了我们 §5 的"解耦重组"决定**——把酒馆的机械 position/depth 降为"给 agent 的建议元数据 + 检索工具"。NeuroBook 让每个节点**自带"该被回忆还是该被注入"的信号**，agent 据此决策。**建议：Task 1.3 世界书条目建模时，给条目加 `retrieval`/`inject` 语义标签（agent 判定），而非硬编插入位置。**

### 4. Agent Dialogue 内容边界 —— 印证我们两平面
- NeuroBook 显式规定：**Agent Dialogue Content 排除工具调用、工具结果、思考内容**，且**排除 harness 提醒、profile/model-context 注入的消息**。
- AGENTS.md：**"做提示词工程时不要把当前对话用户提到的要求带进提示词"** + **"不要假定对方拥有和你一样的知识"**（子 agent 上下文纯净纪律）。
- **对我们的意义**：这跟我们 §1 两平面（角色平面/控制平面）+ 戒律#6 **逐条吻合**——独立验证我们的红线是对的。可借鉴：把"Agent Dialogue Content 排除项"做成**显式内容边界规则 + 测试**（我们已有 `subagent_context_has_no_orchestrator_noise`，可扩展成一张"排除项清单"断言）。

### 5. 记忆：重建非累积 + index.md/state.md 稳定动态分离
- **Summarizer 重建模式**：不累积上下文，而是"每次运行从源会话当前活跃路径**重建** Agent Dialogue Content"，防无界增长、保新鲜。
- **`index.md + state.md` 模式**：把稳定设定和动态状态拆开，防长篇写作的信息纠缠。结构化数据进 **Project SQLite**（`.nbook`）。
- **对我们的意义**：跟 Hermes 记忆（MEMORY.md 稳定 / USER.md + drift 动态、frozen-snapshot、超限整理）+ 我们封卷**同源**。可借鉴"重建而非累积"作为长会话上下文构建原则；SQLite 存结构化状态呼应我们 session_search FTS5（Task Phase 2）。

### 6. Leader/subagent 编排 + walkthrough 可观测 + 中途问人
- Leader thread（编排/批计划/调工具/聚合）+ Retrieval subagent + Writer subagent。每个 subagent 产出 **"walkthrough"**（执行摘要，展示给作者）。subagent 可**中途向用户提问、等明确答复再续**。
- **对我们的意义**：对上我们"loop = 纯净 subagent 编排器"（§3.1）+ 可观测戒律。可借鉴：① retrieval/writer 分工（取数据的 subagent 与写 RP 的纯净 subagent 分开）；② **walkthrough** = 每步执行摘要推给 UI（我们 SSE 的 plan/tool_call/tool_result 事件可包装成用户可读的 walkthrough）；③ 中途问人（RP 里"要不要推进到下一幕"这类可暂停问用户）。

### 7. 酒馆卡三段式导入：inspect → unpack → import
- NeuroBook 导入酒馆卡走"检查 → 拆包 → 导入"三段（landing 页所述）。
- **对我们的意义（Task 1.1）**：不盲导——先 **inspect**（校验格式/未知宏，对应我们 `validate_card`）→ **unpack**（拆成结构化字段/分解，对应 decompose）→ **import** 入库。建议 Task 1.1 导入 UI 采这种分段，给用户可见的校验反馈。

## 与自有项目比对（防重合/冲突 —— 用户 2026-07-02 要求）

> 核对对象：AIRP-MCP-Server（`D:\airp-mcp-server`，我们自己的数据层仓）+ Core/engine 现有设计。目的：NeuroBook 的"学习点"里，凡我们**本来就有**的，标为"已有·仅佐证"，**不当新东西造、更不造平行系统**；只有**净新**的才纳入路线。

| # | NeuroBook 点 | 我们现状 | 结论 |
|---|---|---|---|
| 1 | TSX Profile 类型化组件树 + 三层 | **分层概念已有**（MCP `prompt-caching.md` 按可变性排序 + 我们 §3.5 稳定前缀）；类型化组件树**形态**没有 | **半新**：只学"形态"，且是 engine orchestrator 的**重构参考**，**不新增平行装配系统**（否则跟 Core orchestrator 冲突） |
| 2 | Per-subject 知识隔离（角色只知该知道的） | **目标已有**（SKILL.md:75 防"角色知道太多"+ `apply_lorebook` 关键词懒加载）；**持久化 per-角色知识模型没有**（`视角` 仅指多角色主视角标签，非同概念） | **净新·真值得做**：懒加载是"取时过滤"，NeuroBook 是"持久化角色视角知识层"，更进一步 |
| 3 | 节点 retrieval/inject/refs 语义 | **已有**：MCP lorebook 有 `constant`(常驻注入)/`selective`(条件) + 我们 §5 已决定"位置降为建议元数据+检索工具" | **重合·仅佐证**：NeuroBook 独立走到同一处，不新增 |
| 4 | Agent Dialogue 内容边界（排工具/思考/harness） | **已有**：Core 两平面 + `subagent_context_has_no_orchestrator_noise` CI 不变式 | **重合·仅佐证** |
| 5 | 重建非累积 + index/state 分离 | **已有**：MCP `memory/current.md`+`index.md`、`state/live.json`+`history.jsonl`；Core `prepare_pipeline` 每轮重建；封卷；已定采 Hermes 记忆 | **重合·仅佐证** |
| 6 | Leader/subagent + walkthrough + 问人 | **subagent 编排+可观测已有**（Core M_AGENT loop + SSE `plan`/`tool_call`/`tool_result`）；**walkthrough 用户可读摘要 + 中途问人 没有** | **半新**：只学 walkthrough 包装 + pause-for-human |
| 7 | 酒馆卡三段导入 inspect→unpack→import | **完全已有**：`validate_card`(检查) + `decompose_character`/`analyze_card`(拆解) + `import_card`(导入) | **纯重合·删此学习点**：我们已有，Task 1.1 复用现有三工具，**别按 NeuroBook 重造导入管线** |

**净提炼——真正值得纳入的只有：**
- **点 2（净新）**：持久化 per-角色视角知识层——超出我们现有"关键词懒加载"。→ Task 1.3 世界书 + 角色模型。
- **点 1（半新·形态）**：orchestrator 结构化 profile 抽象的**形态**参考——**重构** Core orchestrator 时借鉴，**不建平行系统**。
- **点 6（半新·小）**：walkthrough（SSE 事件包装成用户可读执行摘要）+ 中途问人。
- 点 3/4/5/7 **我们已有**，NeuroBook 仅**外部佐证方向**，**不新增、不重造、不建平行实现**。

## 不学 / 差异
- 技术栈 Nuxt/Bun/SQLite/Prisma vs 我们 Tauri/Rust——不搬代码。
- 定位偏"小说写作 IDE"（Thread/Scene/Plot 剧情结构、手稿分卷），我们偏"RP agent 客户端"——剧情结构那套按需借鉴，非照搬。
- License 非商用——只学理念，不复制其代码。

## 落地建议（**只列净新，去掉与自有重合的**）
- **Task 1.3 世界书/角色模型**：per-角色视角知识层（点 2·净新）**不做成独立静态隔离，而是并入"角色成长模型"的知识维**（用户 2026-07-02）——角色随剧情成长、非一成不变，知识只是成长的一个维度。**复用已有 User Persona M_UP 的 base+drift 模式套到角色上**（不建平行系统），与 Soul 演化（人格维）、state（关系维）、gating（进度维）统一。详见 [PLAN.md](PLAN.md) §3.4「角色成长模型」。
- **Phase 2 orchestrator 重构（非新增）**：借鉴 **TSX 三层 + 运行时合同的形态**（点 1·半新）重构 Core orchestrator 装配——**不建平行装配系统**，避免与现有 orchestrator 冲突。
- **可观测（小）**：SSE 事件包装成用户可读 **walkthrough** + 支持 loop 中途 pause 问人（点 6·半新）。
- **不做**：三段导入（点 7 已有 `validate_card`/`decompose_character`/`import_card`，Task 1.1 直接复用）、内容边界断言（点 4 已有 CI 不变式）、重建/分层记忆（点 5 已有 + Hermes 已定）、retrieval/inject 语义（点 3 §5 已定）——**这些 NeuroBook 只作外部佐证，不重造。**
