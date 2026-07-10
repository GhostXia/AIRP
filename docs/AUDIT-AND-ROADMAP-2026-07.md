# 项目审计与后续开发计划（2026-07-05）

> **历史快照**：本文基于 `54350e4`，已被 PR #77–#100 和 [2026-07-10 全项目独立审计](PROJECT-AUDIT-2026-07-10.md) 取代。保留本文用于追溯当时发现；不得用其中的测试数字、缺口比例或优先级判断当前状态。

> 基于 origin/main `54350e4` 的全仓审计（engine / protocol / ui / webui / docs / data / CI），
> 以及据此制定的后续开发计划。计划与 PLAN.md / DEV-GUIDE.md 的 Phase 划分对齐，
> 本文是执行层的优先级排序与任务拆解，不取代上述两份权威文档。

---

## 第一部分：审计结论

### 1. 总体成熟度

| 维度 | 评估 | 说明 |
|---|---|---|
| 架构设计 | ★★★★☆ | 无头引擎 + State-Protocol + 可换 UI 的两盒方案已拍板且落地一致；六条不变式清晰 |
| engine 后端 | ★★★★☆ | 14 个 HTTP/SSE 端点、双 provider adapter、FSM 流过滤、封卷系统均可用；安全防护（路径遍历/时序攻击/常量时间比较）完善 |
| Agent 内核 | ★★☆☆☆ | loop 骨架 + 工具注册在（M_AGENT-1/2 部分），但无 ReAct 规划、无工具结果回灌、无 memory/skill/hook/macro/subagent 能力面 |
| protocol crate | ★★★★★ | v1 稳定，Envelope/Blueprint/RFC6902 patch/guard 完整，Rust+TS 双端镜像 |
| ui（Tauri/Vue 产品线） | ★★★☆☆ | 框架层（widget 注册表/沙箱/consent/虚拟滚动/state store）接近 100%，但内容空心：TauriBus↔engine IPC 未接通，7 个 widget 仅 3 个有逻辑，无 markdown 渲染 |
| webui（验证 harness） | ★★★★☆ | 定位明确（临时诊断面），M1/M2 已达成，错误处理与 SSE 处理细致；按计划最终降级为 dev-only |
| 测试 | ★★★☆☆ | engine 单测好、神圣不变式测试在；缺 HTTP 集成测试；ui 前端 95 测试通过但 IPC 路径无真实覆盖 |
| CI/工程卫生 | ★★☆☆☆ | 仅 manual-build.yml（非 PR gate）；无 lint CI；无数据纪律自动门；data/ 目录污染 |

### 2. 关键发现（按严重度）

**🔴 P0 — 阻塞产品目标**

1. **Agent 内核能力面为空**。loop 只有固定计划骨架，无基于模型 tool_calls 的 ReAct 规划，无工具结果回灌（M_AGENT-4），memory/skill/hook/macro/subagent 全部未实现。这是「最终形态 = RP 特化 Agent 客户端」愿景的主干，当前兑现度约 0-30%。
   - 现状锚点：`engine/src/agent/mod.rs`（~480 行，固定计划）、`engine/src/agent/tools.rs`（~1224 行，注册框架 + character 工具）
2. **世界书（Lorebook）引擎完全未做**（Task 1.3）。position/depth/selective 等插入语义为零实现，是 Phase 1 验收条件，也是酒馆对标的最大缺口。
3. **TauriBus ↔ engine 的 IPC 绑定未完成**（`ui/src/protocol/tauri-bus.ts`）。产品 UI 目前只能跑 MockBus，与真实 engine 的联调链路断裂——首要目标「可执行文件双击启动 + 真实配置 + 对话闭环」因此未验收。
4. **运行时验收缺失**：Perf Spike（10 万条虚拟滚动）从未真跑；打包产物的真实启动闭环未验证。
5. **engine 并发安全**：ChatLog 滚动截断整文重写、无文件级锁；HTTP handler 层无集成测试（14 个端点零覆盖）。

**🟡 P1 — 重要债务**

6. **data/ 目录污染**：`data/characters/*/history/chat_log.jsonl`、`memory/*.md`、`gating/*.md`、`quota.json`、`migration_done.lock` 等运行时/私有数据已 track 进 repo。鉴于此前已发生过一次泄露清理（2026-07-02 filter-repo），这是复发风险。
7. **扩展接口引擎侧空白**：事件总线 / 工具注册开放 / 宏 / slash 命令 / prompt 拦截器全部未建。UI 侧 widget 系统很强，但「zero-barrier 第三方扩展」是硬需求，且接口越晚开越难改（DEV-GUIDE §3.8 已警示）。
8. **capability 引擎侧不强制**：UI consent gate 完整，但 engine 不校验 widget capability 声明，扩展可越权（issue #32）。
9. **CI 非 PR gate**：无自动测试门、无 clippy/eslint、无数据纪律（不变式 #6）扫描——DEV-GUIDE 要求将其升为 CI gate。
10. **大函数**：`import_card_to_disk` ~600 行；token 估算仅字符计数（误差 20-30%）；卡/预设/lorebook 无缓存。

**🟢 P2 — 后续优化**

11. ChatWidget 无 markdown 渲染、无头像/时间戳/消息交互；其余 4 个 widget 为骨架。
12. state store 无版本号、patch 失败静默（`ui/src/state/store.ts:64`）、跨 patch 无事务。
13. MCP 集成（M_AGENT-3）、MCP-Server 38 工具融入、Hermes 三层记忆（常驻+归档+检索，当前仅封卷一层）。

### 3. 文档承诺 vs 代码兑现对照（摘要）

| 承诺 | 兑现度 |
|---|---|
| 多 provider 流式（OpenAI + Anthropic） | ✅ 100% |
| 无头引擎 HTTP/SSE 服务 | ✅ 100% |
| id-keyed chat / RFC6902 test-op 预校验 | ✅ 100% |
| 干净提示词两平面隔离 | 🟡 80%（实现在，仅本地测试守护，非 CI gate） |
| 可执行文件双击启动 | 🟡 50%（打包链路通，运行时验收缺） |
| 三层记忆 | 🟡 33%（仅封卷） |
| 第三方扩展接口 | 🟡 30%（UI 强 / engine 空） |
| Agent 内核（loop+工具+记忆+技能+钩子+subagent） | 🔴 ~15% |
| 世界书引擎 | 🔴 0% |
| Perf Spike 验证 | 🔴 0% |

---

## 第二部分：后续开发计划

### 排序原则

1. **先闭环、后铺面**：优先打通「双击启动 → 真实配置 → 与真实 engine 对话」的端到端闭环（用户 2026-07-03 定的首要目标），再横向扩能力。
2. **接口先行**：扩展接口骨架（事件总线/工具注册/宏）在 Phase 2 尽早立，避免生态接口后期难改。
3. **卫生债随行**：每个 Sprint 附带 1-2 项工程卫生任务，不单独攒大清理。
4. 遵守工作流纪律：一任务一分支一 PR；本地测试是唯一 gate（直至 CI gate 建成）。

### Sprint 1（约 1-2 周）：端到端闭环 + 止血

| # | 任务 | 内容 | 验收标准 |
|---|---|---|---|
| 1.1 | **data/ 污染清理** | 补 `.gitignore`（`data/characters/*/history/`、`memory/`、`gating/`、`quota.json`、`*.lock`）；`git rm --cached` 已 track 的私有文件；示例数据加 README 说明；评估是否需再次 filter-repo | repo 中无运行时/私有数据；新生成文件不再被 track |
| 1.2 | **TauriBus ↔ engine IPC 打通** | 完成 `ui/src/protocol/tauri-bus.ts` 实现 + src-tauri 侧桥接 engine sidecar 的 HTTP/SSE；intent（chat.send / characters.import）→ engine → state patch 回流 | Tauri dev 模式下用真实 engine 完成一轮对话，ChatWidget 显示流式 delta |
| 1.3 | **可执行文件运行时验收** | 用 manual-build.yml 产物做真实验收：双击启动 → 配置 provider → 导入角色卡 → 对话闭环；记录证据（截图/日志）回填 DEV-GUIDE §8.2 | 验收记录进 docs；发现的启动缺陷开 issue |
| 1.4 | **ChatLog 并发安全** | 文件级锁或原子写（temp + rename）修复滚动截断竞态 | 并发写测试通过 |

### Sprint 2（约 2 周）：Agent 内核主干（M_AGENT-4/5）

| # | 任务 | 内容 | 验收标准 |
|---|---|---|---|
| 2.1 | **工具结果回灌**（M_AGENT-4） | 扩展 adapter wire format 支持 assistant tool_calls + tool role 消息；loop 内多轮工具调用 | 一次 agent run 中模型可基于工具结果继续推理 |
| 2.2 | **ReAct 规划实装** | 移除固定计划骨架，改为模型驱动的 tool_calls 决策；保留 step cap / token budget / wall clock 有界性（戒律 #1） | agent 能自主选择并串联 character 工具完成任务 |
| 2.3 | **取消与确认流**（M_AGENT-5） | CancellationToken 绑定 SSE 连接生命周期；破坏性工具走确认事件（webui 已有确认 UI 可复用） | 断开 SSE 即中止 run；破坏性操作需显式确认 |
| 2.4 | **HTTP 集成测试** | 对 14 个 `/v1/*` handler 建集成测试（wiremock 模拟上游） | 全端点至少 happy path + 1 错误路径覆盖 |

### Sprint 3（约 2-3 周）：世界书引擎（Task 1.3）

Phase 1 验收条件，最大新建件，独立成 Sprint。

| # | 任务 | 内容 |
|---|---|---|
| 3.1 | 世界书数据模型 + 导入 | 酒馆 lorebook JSON（含角色卡内嵌 character_book）解析、存储到 `data/characters/*/worldbooks/` |
| 3.2 | 插入引擎 | position（before/after char、@depth）、depth、order、selective/secondary keys、递归扫描、token 预算裁剪 |
| 3.3 | 装配集成 | orchestrator 装配路径接入，守两平面隔离不变式（扩展 `subagent_context_has_no_orchestrator_noise` 类测试） |
| 3.4 | UI 呈现 | webui 诊断面先行（命中条目展示）；ui 侧 lorebook widget 骨架填充 |

验收：导入一张带世界书的真实酒馆卡，关键词触发条目正确插入 prompt，且 agent 平面无泄漏。

### Sprint 4（约 2 周）：扩展接口地基（Phase 2 起点）

| # | 任务 | 内容 |
|---|---|---|
| 4.1 | **事件总线** | engine 全生命周期事件（msg/gen-start/gen-end/stream-token/world-hit/tool-call/state-change）对内广播 + 对外 SSE 订阅端点 |
| 4.2 | **工具注册开放** | 第三方经 MCP client（M_AGENT-3）或声明式注册接入 agent 工具集；capability 声明进 engine 侧校验（issue #32） |
| 4.3 | **宏系统** | `{{char}}/{{user}}/{{time}}/{{roll}}` + 自定义宏，装配层展开 |
| 4.4 | **CI 升级为 PR gate** | `cargo test --all`（含神圣不变式测试）+ clippy -D warnings + typecheck + vitest + 数据纪律扫描（base64 blob / payload 体积检测，兑现不变式 #6 的 CI gate 要求） |

### Sprint 5+（Phase 2 中后期，按需排期）

- **RP 体验补全**：ChatWidget markdown 渲染、头像/时间戳、消息编辑/删除端点、会话管理 UI、预设编辑 UI
- **三层记忆**：常驻有界 md + 用户自动抽取 + FTS5 检索（HERMES-MEMORY 落地），与现有封卷系统合并
- **MCP-Server 38 工具融入**（MCP-SERVER-ABSORPTION.md 映射表执行）
- **多角色场景**（/v1/scenes 实装）、slash 命令、prompt 拦截器、消息格式化 hook
- **Perf Spike**：10 万条消息真实压测（虚拟滚动代码已备，只欠验证）
- **质量债**：`import_card_to_disk` 拆函数、token 估算换 tokenizer、卡/预设缓存、state store 版本号 + patch 失败日志
- **多平台打包**：macOS/Linux sidecar builder

### 里程碑视图

```
Sprint 1  ──► 闭环达成：双击可用的真实对话产品（首要目标兑现）
Sprint 2  ──► Agent 内核可用：模型驱动工具循环 + 安全边界
Sprint 3  ──► Phase 1 验收：世界书 = 酒馆替代的最后一块核心拼图
Sprint 4  ──► Phase 2 开启：扩展生态接口冻结 v1 + CI gate 上线
Sprint 5+ ──► 体验打磨 + 记忆系统 + MCP 生态
```

### 风险与依赖提示

- **RR-001**（card_path 任意路径读）：engine 对外网暴露之前必须硬化；Sprint 4 事件总线对外开 SSE 订阅时重新评估。
- Sprint 2 的工具回灌是 Sprint 4 工具注册开放的前置；Sprint 3 世界书与 Sprint 2 可并行（不同分支）。
- webui 在 Sprint 1-3 期间继续作为诊断面服役，Phase 2 CI gate 建成、ui 产品线联调稳定后按计划降级为 dev-only。

### 执行进展（滚动更新）

- 2026-07-05：审计当日修复批次——PR #51（/v1/models proxy 硬化，关 #34/#40/#42）、
  PR #52（吸收 catalog 层级修正，关 #23）、PR #53（神圣不变式补强 + AgentLoop 集成覆盖，关 #26/#30）。

---

*本文档为一次性审计快照；执行中的状态更新请回填 DEV-GUIDE.md §8 与各任务 issue，不在本文档维护进度。*
