# Talemate 深度审计报告

> 审计对象：[vegu-ai/talemate](https://github.com/vegu-ai/talemate) v0.38.0（AGPL-3.0，Python ≥3.11）
> 审计日期：2026-08-04
> 审计依据：[AGENTS.md](../../AGENTS.md) 审计守则三原则（独立审计 / 可提己见 / 可质疑历史并查证）+ AIRP 当前源码 `main@4f3f792`
> 合规边界：仅吸收理念、需求洞察、公开行为与互操作性经验；**不复制任何 talemate 代码、prompt、模板、数据或视觉资产**（AGPL-3.0 比 MIT 更严，更需谨慎）
> 归档原因：本审计文件随 PR 同分支提交、合并到 main，是仓库惯例（见 [AGENTS.md](../../AGENTS.md) 审计文件归档立规 2026-07-20）

---

## 1. 项目本质判断

| 维度 | 结论 | 证据 |
|---|---|---|
| 定位 | 重 Python 单体 RP 客户端，事件驱动 + 多 Agent 协作 | [`pyproject.toml`](https://github.com/vegu-ai/talemate/blob/main/pyproject.toml) |
| 架构 | 单进程塞入 LLM 客户端 + ChromaDB + torch + transformers + 节点图引擎 + TTS | 同上 |
| 合规风险 | **AGPL-3.0**：若 AIRP 复用任何代码/prompt/模板/数据/视觉资产，将触发 GPL 网络效应，污染整个 AIRP | [`LICENSE`](https://github.com/vegu-ai/talemate/blob/main/LICENSE) |
| 依赖风险 | `torch + chromadb + sentence_transformers` 的重量级依赖是 AIRP Rust + WebUI 拆分架构的**反面教材** | `pyproject.toml` |
| 设计成熟度 | 域模型与 Agent 协作模式成熟；持久化与并发控制薄弱（无锁序、无 revision 合同） | 见 §3 |

**审计意见 1（合规）**：AGPL-3.0 比 AIRP 已吸收的 SillyTavern（AGPL）同等严格，网络效应条款适用。**仅做理念吸收，禁止任何代码/prompt/模板/数据复用**，且必须在 [docs/ACKNOWLEDGEMENTS.md](../ACKNOWLEDGEMENTS.md) 记录 talemate v0.38.0、AGPL-3.0、调研日期与吸收边界（本 PR 已完成）。

---

## 2. talemate 关键设计摘要（用于后续对照）

### 2.1 整体架构

- **技术栈**：Python 3.11–3.13、pydantic v2、asyncio + `nest_asyncio` + `blinker` 信号 + 自研 `async_signals`、Jinja2、ChromaDB + sentence_transformers + torch、tiktoken + nltk、uvicorn（ASGI）、sseclient-py、RestrictedPython>7.1、diff-match-patch + deepdiff。
- **顶层结构**（`src/talemate/`）：`tale_mate.py`（Scene/Actor/Player 聚合根 + 主循环信号）、`character.py`、`history.py`、`instance.py`（全局 `AGENTS`/`CLIENTS` 注册表）、`scene_message.py`（7 种消息 + 版本栈）、`shared_context.py`、`changelog.py`、`agents/`（11 个 agent）、`client/`（12 个 provider）、`prompts/`、`world_state/`、`game/`、`scene/`、`commands/`、`config/`、`emit/`、`load/`。
- **主循环**：事件驱动（非 while 循环），`tale_mate.py:68-78` 注册 8 个 async_signals：`scene_init`、`scene_init_after`、`game_loop_start`、`game_loop`、`game_loop_actor_iter`、`game_loop_new_message`、`player_turn_start`、`push_history`、`push_history.after`。

### 2.2 核心域模型

- **Scene**（`tale_mate.py:99`）：聚合根，持 `actors`、`history`（三层：active + archived + layered）、`ts = "PT0S"`（**ISO 8601 duration**，依赖 `isodate`）、`perspectives`、`intent_state`、`nodegraph_state`、`shared_context`、`commands.Manager`、`assets`、`voice_library`、`episodes`、`agent_persona_templates`、`writing_style_template`、`visual_style_template`。
- **Character**（`character.py`，25KB）：`CharacterDetails`（`world_state/manager.py:49` 反推）含 `base_attributes`、`details`、`reinforcements`（per-character 强化）、`actor`（`dialogue_examples` + `dialogue_instructions`）、`cover_image`/`avatar`/`current_avatar`。
- **WorldState**（`world_state/__init__.py`）：三类实体 `CharacterState` / `ObjectState` / `PlaceState`；`WorldStateResponse` = `{ world_state: dict, anchor_message_ids: list[int] }`；`InsertionMode` ∈ `{sequential, conversation-context, all-context, never}`。
- **Reinforcement**（`world_state/__init__.py:49`）：

  ```python
  class Reinforcement:
      question: str
      answer: str | None      # LLM 周期重新生成
      interval: int = 10      # 每 N 轮重新强化
      due: int = 0             # 倒计时
      character: str | None
      instructions: str | None
      insert: str = "sequential"
      require_active: bool = True
  ```

  这是 talemate 最核心的创新：**Q&A 式的周期性真相校准机制**——每个 reinforcement 是带 `interval`/`due` 倒计时的问题，到期由 LLM 重新生成答案作为「当前世界真相」注入 context。WorldState 提供 `add_reinforcement`/`find_reinforcement`/`reinforcements_for_character`/`reinforcements_for_world`/`filter_reinforcements`/`commit_to_memory`。
- **GameState**（`game/state.py`，2.5KB）：`variables: dict[str, Any]` + `goals: list[Goal]` + `instructions: Instructions`（per-character）；`set_var(key, value, commit=False)` 当 `commit=True` 时**写入 memory agent**（`memory.add(value, uid=f"game_state.{key}")`），即游戏状态可持久化到长期记忆。
- **与 WorldState 分工**：WorldState = 场景内实体当前快照（谁在场、物品、地点、情感）；GameState = 游戏逻辑变量（目标、变量、指令）。**双层分离**。

### 2.3 Agent 系统

- **11 个 Agent**（`agents/__init__.py` + `agents/registry.py`）：Conversation、Creator、Director、Editor、Memory（ChromaDBMemoryAgent）、Narrator、Summarize、TTS、Visual、World State。
- **注册**：`@register(condition=None)` 装饰器按 `agent_type` 存入全局 `AGENT_CLASSES`，支持条件注册（用于可选依赖如 ChromaDB）。
- **ActiveAgent 调用栈**（`agents/context.py`，~2KB）：`active_agent = contextvars.ContextVar`；`ActiveAgentContext` 持 `agent`、`fn`、`fn_args`、`fn_kwargs`、`agent_stack`、`agent_stack_uid`、`state: dict`、`state_params`、`previous`（链表）；`@property first` 取栈底、`@property fingerprint` 基于 `state_params` 算指纹（跨调用结果缓存）。`ActiveAgent` 是上下文管理器：进入压栈（继承 previous 的 state），退出恢复。**Agent A 调用 B 时 B 能看到 A 的 state，形成有状态调用链**。
- **per-agent client 分配**（`instance.py`）：`ensure_agent_llm_client()` 遍历 `AGENTS`，每个 agent 按其 config 指定的 `client` 名绑定；若未指定或该 client 不可用（disabled），fallback 到 `get_active_client()`。配置变更/客户端启停通过信号触发重分配。
- **Prompt 系统**（`prompts/`，`base.py` 65KB）：Jinja2 模板；`groups.py` 22KB 管理 prompt 分组；`overrides.py` 支持覆盖；`AgentActionConfig`（`agents/base.py:54`）声明 per-action 配置，类型：`autocomplete/blob/bool/flags/number/text/vector2/weights/wstemplate/password/unified_api_key`。
- **ClientContext**（`client/context.py`，~3KB）：`ContextModel` 含 `nuke_repetition: float`、`conversation: ConversationContext`（talking_character + other_characters）、`length: int`、`inference_preset`、`data_format`、`requires_active_scene: bool`、`disable_reasoning: bool`；用 `ContextVar` + deepcopy 实现非侵入式 per-call 配置覆盖。

### 2.4 Provider 集成（`client/`）

12 个 provider：OpenAI、Anthropic、Google（Gemini）、Mistral、Cohere、Groq、DeepSeek、KoboldCpp、llama.cpp、LM Studio、Ollama、`openai_compat`（OpenRouter/TabbyAPI/text-generation-webui 兼容层）；另有 `custom/` 子目录支持自定义 provider。流式用 `sseclient-py`；`model_prompts.py` 处理不同 provider 的 prompt 模板差异。

### 2.5 关键差异点（vs SillyTavern-style）

1. **双层状态**（WorldState + GameState）；SillyTavern 仅有静态 lorebook，无动态状态跟踪。
2. **Reinforcement 周期校准**：带 `interval`/`due` 的 Q&A 周期重新询问 LLM 获取当前真相，对抗长对话设定漂移。
3. **消息版本栈**（`scene_message.py`）：`VersionSource = Literal["original", "revision", "regenerate", "continue", "custom"]`；每条消息 `versions: list[MessageVersion]` + `active_version: int`，regenerate 保留旧版本，非破坏性。
4. **Passage of Time**：`Scene.ts = "PT0S"` ISO 8601 duration；`TimePassageMessage`；时间跳跃触发 **world state snapshot 重建**（文档：»When you move to a new point in time (a time jump), the snapshot is treated as a clean scene cut and rebuilt fresh«）；`EpisodesManager`（`scene/episodes.py`）。
5. **节点图脚本引擎**（`game/engine/nodes/`）：`GraphState` + `load_graph` + `FunctionWrapper` + `RestrictedPython` 沙箱执行；Director agent 通过节点图实现「scoped api scripting」——可视化场景逻辑编程。
6. **Durable Snapshot + auto-evict**：World State snapshot 默认增量更新而非重建；`Max items tracked` + `Auto-evict stale entries`（连续 N 次刷新未触达的实体自动驱逐）；时间跳跃才完全重建。

---

## 3. AIRP 源码对照（独立审计纠偏）

按审计守则「不附和子代理结论」，对 talemate 调研子代理报告中 4 处「AIRP 推测」做源码级核实纠正：

| 子代理报告 | AIRP 源码实际 | 修正结论 |
|---|---|---|
| 「AIRP 推测为单一 state」 | `engine/src/domain/state.rs` + `engine/src/domain/world_event.rs` + `engine/src/orchestrator/gating.rs`（时槽/关卡 CP-1/2/3）+ `engine/src/memory/decay.rs`（`reinforce_entry`） | AIRP 已有多层状态：per-character KV + WorldClock/Event + 时槽/关卡 + 记忆衰减/reinforce；但**确实缺「场景内所有实体的当前快照」层** |
| 「AIRP 推测为覆盖/单次 regenerate」 | `engine/src/chat_store.rs:56-87`（`message_parents` + `active_leaf` + `message_swipe_index`） | AIRP 已有**完整的分支对话树**，比 talemate 的线性 `MessageVersion` 更强 |
| 「AIRP 推测为全局单一 provider」 | `engine/src/provider_routing.rs:20-102`（`RouteContext` 5 级回退：character > scene_role > task_kind > default > first_default） | AIRP 已有**更精细的多维度路由**，比 talemate 的 per-agent client 是降级 |
| 「AIRP 推测为函数调用」 | `engine/src/agent/mod.rs` + `engine/src/agent/council.rs` + `engine/src/agent/director.rs` | AIRP 有**双平面隔离 AgentLoop + Council + Director**，但确实无 talemate 的 ActiveAgent 调用栈（也不应吸收，见 §5） |

### 3.1 AIRP 已有且更强的能力（不应照搬 talemate）

| 能力 | AIRP 实现 | talemate 实现 | 优势归属 |
|---|---|---|---|
| 消息版本 | 分支树 + swipe + parent + active_leaf | 单消息线性版本列表 | **AIRP 更强**（树 vs 线性） |
| Provider 路由 | `RouteContext` 5 级回退 | per-agent client | **AIRP 更强**（多维度 vs 单维度） |
| Agent 隔离 | 纯净 subagent + 控制平面双平面隔离（神圣不变式 `subagent_context_has_no_orchestrator_noise`） | `ActiveAgent` 共享 `state: dict` | **AIRP 更严谨**（talemate 的设计直接违反 AIRP 戒律 #6） |
| 时槽+关卡 | `gating.rs`：timeline.md + checkpoints.md + CP-1/2/3 | 无等价机制 | **AIRP 独有** |
| Revision 合同 | 每次写盘产生不可变 revision snapshot | 覆盖式持久化 | **AIRP 更严谨** |
| Lock order | R1/R2 `debug_assert` + RAII Guard | 无锁序概念 | **AIRP 更严谨** |
| 类型安全 ID | newtype（CharacterId/SceneId/PersonaId/UserId/SessionId） | pydantic 字符串 ID | **AIRP 更严谨** |

---

## 4. AIRP 真正缺失的能力（可吸收，按优先级）

### P0 — 高价值，AIRP 源码确认无等价

| # | 能力 | AIRP 缺失证据 | talemate 出处 | 吸收建议 |
|---|---|---|---|---|
| 1 | **Episode/Chapter 管理** | `Grep "episode\|chapter"` 在 `engine/src` 返回 No matches | `src/talemate/scene/episodes.py` | AIRP 的 ChatLog 是线性消息 + 分支树，缺章节切割抽象。可设计 `Episode` domain（章节边界 + 时间锚 + 摘要） |
| 2 | **双层状态分离**：WorldState（实体快照）+ GameState（游戏变量） | `engine/src/domain/state.rs` 是单角色 KV；无「场景内所有实体（characters/objects/places）的当前快照」层 | `src/talemate/world_state/__init__.py` + `src/talemate/game/state.py` | AIRP 应在 scene 级引入 `WorldState`（实体快照）与现有 per-character state 分离 |
| 3 | **周期性 Q&A 真相校准**（与 AIRP `reinforce_entry` 不同维度） | AIRP 的 `reinforce_entry` 是显式标记强化，**不重新询问 LLM**；talemate 是 interval 倒计时到期后用 LLM 重新生成答案 | `src/talemate/world_state/__init__.py:49` `Reinforcement` | 两种范式可结合：interval 触发校准，但答案优先用历史值或显式确认（避免 LLM 编造），仅在用户/导演显式触发时才调 LLM |

### P1 — 中价值，AIRP 部分有但可改进

| # | 能力 | AIRP 现状 | talemate 做法 | 吸收建议 |
|---|---|---|---|---|
| 4 | **InsertionMode 四态注入策略** | lorebook trigger_keywords 单态 | sequential / conversation-context / all-context / never | 在 AIRP 的 `engine/src/orchestrator/lorebook.rs` 上加 `insertion_mode` 字段 |
| 5 | **3 种长期记忆检索策略**（recent-context / AI-query / AI-Q&A） | resident + decay + FTS 单一策略 | World State Agent 设置中三选一 | 在 `engine/src/memory/fts.rs` 上加 `retrieval_strategy` 切换 |
| 6 | **角色进展提议（Suggestion）** | Agent tool 直接写 state | LLM 提议 → 玩家审批 | 在 `engine/src/agent/tools/state_lorebook.rs` 上加 `propose` 动作（pending → accept/reject） |
| 7 | **条件上下文钉（基于条件动态激活）** | lorebook trigger_keywords 静态匹配 | `AnnotatedContextPin` + `ConditionGroup` 基于 GameState/WorldState 条件 | 在 `LorebookEntry` 上加 `condition` 字段 |
| 8 | **Summarizer 双触发**（时间推进 + token 阈值） | `gating.rs` 有时槽推进，但摘要未联动 | `SummarizeAgent` 双触发 | 在 `engine/src/memory/compress.rs` 上接 gating trigger |
| 9 | **信号驱动 provider fallback**（client disabled/enabled 触发重分配） | `provider_routing.rs` 5 级回退是 request-time 静态 | `instance.py:ensure_agent_llm_client` 信号驱动重分配 | 在 `RouteContext` 之上加 client lifecycle 信号订阅 |

### P2 — 探索性，需独立设计

| # | 能力 | 风险 | 吸收建议 |
|---|---|---|---|
| 10 | **节点图脚本引擎** | talemate 用 RestrictedPython（Python 沙箱），AIRP 是 Rust；如要做应用 WASM 沙箱独立设计 | 暂不吸收，等 AIRP 有「Game Master 模式」需求时再独立设计 |
| 11 | **ISO 8601 duration 时间表达** | AIRP 的 u64 抽象时间表达力弱，但迁移成本高 | 评估 `WorldClock` 是否升级为 ISO 8601 duration |
| 12 | **per-scene agent overrides** | AIRP `SceneConfig` 无 `agent_settings` 字段 | 在 `SceneConfig` 上加可选 `agent_overrides` |
| 13 | **角色肖像自动选择/生成** | AIRP 无视觉生成 | World State Agent 设置 | 视 AIRP 视觉能力路线决定 |
| 14 | **agent_persona_templates + writing_style_template** | AIRP `SceneConfig` 无 | `tale_mate.py:137` | per-agent 绑定 persona 模板 + 场景级写作风格模板 |

---

## 5. 不应吸收的能力（含理由）

| 能力 | 不吸收理由 |
|---|---|
| **ActiveAgent 调用栈 + state 共享** | 直接违反 AIRP 神圣不变式 #6「干净提示词」（`subagent_context_has_no_orchestrator_noise`）。talemate 把 `state: dict` 在 agent 间传递，会让控制平面噪声污染角色平面。AIRP 的双平面隔离更严谨。 |
| **per-agent client 选择（作为主路由）** | AIRP 的 `RouteContext` 已有更精细的 5 级回退路由，talemate 的 per-agent 是降级。但 talemate 的「**信号驱动 fallback**」（client disabled/enabled 触发重分配）值得吸收（见 P1#9）。 |
| **MessageVersion 单消息版本栈（作为消息模型）** | AIRP 的分支树更强，但 `VersionSource` 分类（original/revision/regenerate/continue/custom）可作为元数据补充到 AIRP 的 `message_candidates` 上，丰富版本来源追踪 |
| **重 Python 单体依赖**（torch + chromadb + transformers） | 反面教材。AIRP 的 Rust + WebUI 拆分架构在生产部署、内存占用、冷启动上更优 |
| **覆盖式持久化** | AIRP 的 revision 合同 + lock_order 运行时强制更严谨，talemate 的覆盖式写盘在崩溃恢复上有风险 |

---

## 6. 对 talemate 设计的批判性意见

按审计守则「可提己见」，对 talemate 几处设计提出独立质疑：

1. **Reinforcement 依赖 LLM 重新生成答案**：`answer: str | None` 字段表明到期时由 LLM 重新生成——这会引入新的不可靠性（LLM 可能编造新答案覆盖原真相）。AIRP 若引入此机制，应优先用历史值或显式确认，仅在用户/导演显式触发时才调 LLM。
2. **WorldState durable snapshot + auto-evict**：实际是「LLM 周期性重建实体快照」，与 AIRP 的 revision 合同 + lock_order 严谨写盘理念冲突。AIRP 若引入 WorldStateManager，应保留 revision/lock 合同，不能用 LLM 重建覆盖显式写盘。
3. **ActiveAgent 共享 state**：违反了 RP 客户端的基本纪律——角色平面不能被控制平面污染。这是 talemate 的设计缺陷，不是优势。
4. **AGPL-3.0 + 重依赖**：talemate 的开源策略与依赖选型使其难以被商业项目深度参考。AIRP 应严格仅做理念吸收。

---

## 7. 落地建议

1. **本 PR 已完成**：在 [docs/ACKNOWLEDGEMENTS.md](../ACKNOWLEDGEMENTS.md) §2 表格追加 talemate v0.38.0 (AGPL-3.0) 条目，归档本审计报告。
2. **P0 项进入设计阶段**（不立即实施，符合 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md) §5 当前优先级——v0.0.3 P1 验收窗口不扩面）：
   - Episode/Chapter 管理 → 新建 `docs/plans/2026-08-04-episode-management.md`
   - 双层状态分离 → 新建 `docs/plans/2026-08-04-world-state-separation.md`
   - 周期性 Q&A 真相校准 → 新建 `docs/plans/2026-08-04-truth-calibration.md`（与 AIRP 现有 `reinforce_entry` 结合设计）
3. **P1 项作为 [#381](https://github.com/GhostXia/AIRP/issues/381) 审计 umbrella 的候选 follow-up**，不单独开 issue。
4. **不应吸收项**：在 [RISK-REGISTER.md](../RISK-REGISTER.md) 追加「talemate ActiveAgent state 共享设计违反 AIRP 神圣不变式 #6」作为决策记录（**本 PR 未做，留作 follow-up**）。
5. **审计遗留项**：按 [AGENTS.md](../../AGENTS.md) 审计遗留项处理立规 2026-07-06，本 PR 合并后应将「P0 项进入设计阶段」「不应吸收项记入 RISK-REGISTER」整理为 GitHub issue（**待 PR 合并后执行**）。

---

## 8. 总结

Talemate 对 AIRP **确实有极大帮助**，但帮助在于 **理念启发** 而非代码复用。其核心价值是：

1. **Episode/Chapter 管理**（AIRP 完全缺失）
2. **双层状态分离**（AIRP 只有单层 per-character state）
3. **周期性 Q&A 真相校准**（AIRP 有 reinforce 但不同维度）

但 talemate 的多项设计（ActiveAgent state 共享、覆盖式持久化、单消息版本栈、per-agent client）在 AIRP 已有更严谨的等价或更强能力面前是**降级**，不应吸收。

AIRP 的 v0.0.3 P1 验收窗口（[#130](https://github.com/GhostXia/AIRP/issues/130)）仍是当前唯一 P1 阻塞，不应被新能力吸收分散注意力。talemate 的 P0 项应作为 v0.0.4+ 的设计输入，不进入当前执行队列。
