# AIRP 客户端 —— 拆件清单（Parts Inventory）

> 目的：把四个原仓库拆成**功能零件**，脱离原仓库的模块边界，供"当新项目重组"时按需取用。
> 状态图例：✅ 可原样复用 ｜ 🔧 有基础但需修/补 ｜ 🆕 四仓皆无，需新建 ｜ 📖 仅作参考/思路（代码不直接搬）
> 来源仓库：C=AIRP-Core(D:\AIRPCLI) · M=AIRP-MCP-Server(D:\airp-mcp-server) · S=AIRP-State-Protocol · G=AIRP-Gateway。行内 file:line 指原仓库路径。
> 最后更新：2026-07-11

> **使用限制（2026-07-12 校准）**：本文是源项目候选零件目录，不是当前 AIRP capability inventory。✅/🔧 表示源资产可参考或可吸收，不代表本仓已存在 HTTP route、Agent tool 或产品 UI。当前实现状态见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。

> UI 协议拍板：S 的 Blueprint/Widget/patch/guard/虚拟滚动/consent/sandbox 是必须吸收的成熟资产；S 的"通用 Agent UI 标准优先"与"乐高优先"不是 AIRP 产品主线。见 [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)。
> 源项目总拍板：C/M/G/S 都是资产来源，统一按"吸收资产，不继承产品北极星"处理。见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)。

---

## A. 干净提示词内核（产品命根子 —— 最该完整保留的资产）

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| 两平面物理隔离机制 | C `AGENT_BACKEND_PLAN.md:187-201` + `chat_pipeline.rs` | ✅ | 角色平面(RP数据)/控制平面(结构化 tool-calling)分离；orchestrator 只装 RP 数据，工具走 API 原生字段 |
| 戒律#6 本地/PR CI 不变式 | C `tests::subagent_context_has_no_orchestrator_noise` | ✅ | 断言角色平面 prompt 无脚手架标记，违反即红。**必须随内核一起保留**；自动 PR gate + 人工 review 承接 |
| 六条有界 Agent 戒律 | C `README.md:39-46` | 📖 | 有界/可取消/可观测/最小授权/幂等隔离/上下文纯净。作我们引擎的设计律采纳 |
| orchestrator 装配 | C `orchestrator/mod.rs` | ✅ | card→preset→gating→known→卷→lorebook 默认序；多角色 `build_multi_char_system_prompt`；schema min/max 渲染 `:289-302` |
| chat_pipeline 三段式 | C `chat_pipeline.rs`（prepare `:296`/stream `:596`/finalize `:694`） | ✅ | 单回合流水线，被 agent loop 当库复用。AGENT_CLIENT_ASSESSMENT 认证"80% 后端已在此" |
| 载荷按可变性排序（缓存友好） | C orchestrator + M `prompt-caching.md` §4 | ✅ | AIRP-Dev `export_context_bundle` 保持稳定 card/preset/extensions/lorebook 在前、易变 live state 在后 |

## B. LLM 连接层

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| 双 provider 流式 adapter | C `adapter.rs`（OpenAI `:96` / Anthropic `:186` / 分发 `:292`） | ✅ | OpenAI 兼容 + Anthropic `/v1/messages` 双格式 SSE，跨包行缓冲、断连取消后仍 finalize |
| BackendEngine 抽象 | C `adapter.rs:18` | ✅ | Direct/AnthropicMessages/ClaudeCodeSdk(stub) 可插拔 |
| ClaudeCodeSdk 引擎 | C `adapter.rs:315` | 🆕 | stub "not yet implemented"。要"外部 agent 当可选生成 engine"才需写；**绝不当 loop owner**（会污染） |
| `[[CACHE_BREAK]]`→`cache_control` 翻译 | M `prompt-caching.md` | 🆕 | 边缘层活（客户端/引擎 adapter），按标记切结构块给稳定前缀附 cache_control。Anthropic TTL 默认 5min |
| 三层配置合并 | C `config.rs`（default→settings.json→env→request） | ✅ | 运行时热重载 `POST /v1/settings` |

## C. Agent Loop（多步/工具/多角色）

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| AgentLoop | C `agent/mod.rs`（M_AGENT-1） | ✅ | 双 provider structured planner + 有界 plan-act-observe + finalizer + SSE 事件 |
| Tool trait + ToolRegistry | C `agent/tools.rs` | ✅ | 19 个 built-in 工具；capability/allowlist/confirm 门控；HTTP runtime catalog |
| loop=纯净 subagent 编排器 | C `AGENT_BACKEND_PLAN.md:130-149` | 📖 | 每步派生全新纯净上下文 subagent，协调器噪声不进 subagent。设计思路 |
| 四道闸计量基建 | C `quota.rs` + `tokio JoinSet` | ✅ | 预算计量 + 子任务收敛现成 |
| SSE 多步事件协议 | C `/v1/agent/run`：plan/tool_call/tool_result/delta/done | ✅ | 已定义 |

## D. 流式处理

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| 字符级流过滤 FSM | C `fsm.rs`（`pub(crate)`） | ✅ | 剥 `<state>`/`<卷评估>`/preset 正则；proptest 验 chunk 边界独立 |
| XML 拆包 | C `xml_unpacker.rs` | ✅ | `immersive`/`<action>`/`<state>` 拆包，未闭合优雅降级。`<action>` 是工具调用回退种子 |

## E. 数据层（角色卡/会话/世界书/state/记忆/预设/场景/插件）

> 两处并存：C 自带 batteries-included（正确），M 是数据管家（框架全但酒馆兼容有假）。拆散后**熔成引擎单一数据层**——从两边取优。

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| `data/` 目录布局 | C `README.md:264-293` / M | ✅ | 两仓兼容可互换。characters/{id}/{card,greetings,world,state,gating,memory,sessions}、presets、scenes、plugins |
| 会话 JSONL 存储 | C `chat_store.rs`（O(1) append） | ✅ | ChatLog 行式追加，仅滚动/回滚整体重写 |
| Newtype ID 校验 | C `types.rs` | ✅ | 反序列化即校验，拒路径穿越/`..`/空字节 |
| 路径沙箱 | M `safe_resolve_for_write`+`validate_id_segment`（`storage/mod.rs`） | ✅ | 组件级校验拒 `..`/绝对路径/符号链接。⚠️ M 的 `import_preset` 未走沙箱(E.2)，搬时统一 |
| 结构化 state（HP/好感度/物品） | C `state/live.json`+`history.jsonl`；M `update_state`(RFC7386 merge) | 🔧 | live+history 存储在。**schema min/max 不在写入路径钳制**(模型可写 999)，需补 clamp(C ASSESS §5.1) |
| 场景多角色 | C `scene.rs` / M `create_scene`+`merge_lorebooks` | ✅ | SceneConfig + 合并世界书(union/primary_only) |
| 插件零 schema 数据 | C/M `plugin_kv/jsonl/blob`（6 工具 + 订阅推送） | ✅ | 任意命名空间存任意数据，可当零代码事件总线。戒律4 |
| list 输出稳定排序 | M `character_store.rs:97`/`preset_store.rs:46` | 🔧 | FS 顺序漂移害下游 diff/缓存，需补 `sort`(E.1) |

## F. 酒馆格式导入（硬需求 —— 手动导入 + 兼容酒馆）

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| 角色卡 PNG 解析（正确版） | C `png_parser.rs`(262行) | ✅ | tEXt/zTXt/iTXt + ccv3(V3)优先 chara(V2)回退 + v1平铺归一化。**这是正确实现** |
| 角色卡 TavernCardV2 模型 | C `types.rs:37-62` | ✅ | spec/data 封装 + system_prompt/alternate_greetings/character_book + `normalize_v1_to_v2()` |
| 角色卡解析（坏版，勿用） | M `character_store.rs:217`+`character.rs:20-32` | 📖 | zTXt-only 读不到真卡 + 摊平 struct 解析失败。**反面教材，用 C 的替换** |
| 世界书解析+插入引擎 | 皆无 | 🆕 | 酒馆是 uid-keyed object，M 用 Vec(解析失败)、C 也残缺。position/depth/selective/constant/probability/递归**全缺，需新建** |
| 预设正则脚本（正确骨架） | M `preset_regex.rs` `SillyTavernRegexScript` | 🔧 | findRegex/replaceString/placement 映射对，但缺 trimStrings/minDepth/maxDepth/markdownOnly 等字段 |
| 预设正则（冲突坏版） | M `preset.rs:50-56` `RegexScript` | 📖 | 瞎起名字段，与上面冲突。**杀掉留一个真相源** |
| 预设分析/热调工具 | M `analyze_preset`/`tune_preset`/`decompose_preset` | 📖 | 提示词工作流：读懂 prompt 块用途、按当前模型热调。对应"预设是建议素材，Agent 适配"哲学 |
| 卡验证（未知宏/破损） | M `validate_card`/`validate_preset` | 🔧 | `[UNKNOWN_ORIGIN]` 内容不删只标记提示。导入不可信卡前用 |

## G. 记忆 / 长期上下文

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| 封卷系统 | C `volume_store.rs`+`volume_manager.rs`（`pub(crate)`） | ✅ | current.md/vol_XXX.md/index.md I/O + 封卷工作流。**永不自动——阈值信号给 loop 拍板** |
| gating/checkpoints/timeline | C `gating/checkpoints.json` / M `get_gating_status` | ✅ | 长篇 RP 进度锚点 |
| 长程语义检索(RAG) | 皆无 | 🆕 | 暂缓——先 volume+简单检索顶。真撞瓶颈再起(C §9) |

## H. UI 前端（State-Protocol —— 四块里代码最成熟）

> 采纳方式：把 S 当作 AIRP UI 资产库，而不是当作产品定位。默认目标是强 AIRP 客户端：先打通并验收 UI→engine→patch→widget 渲染闭环，再谈第三方生态和公共协议。

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| Tauri+Vue 壳 + 打包 | S `src-tauri/` + `tauri-build.yml` | ✅ | AIRP-State-Protocol 原项目最早已验证打包后的 `airp-ui.exe` 可正常启动并做简单交互；未做深度功能/性能测试。签名二进制分发，不运行时编译 |
| Widget Registry | S `src/registry/registry.ts` | ✅ | vue/module/esm 三类注册，命名空间 `namespace.name`，`core.*` 保留 |
| BlueprintRenderer + WidgetHost | S `src/components/` | ✅ | 渲染声明式 Blueprint，错误隔离(onErrorCaptured) |
| RFC6902 状态 store | S `src/state/store.ts` | ✅ | 全量 patch(add/remove/replace/move/copy/test)。`test` op 已改为整包预校验，失败时不会半应用前置 op |
| 首方 widget 集 | S `src/widgets/` | ✅ | chat/emotion/memory/inventory/quest/map/card + clock(module) |
| 虚拟滚动 | S `src/widgets/virtual-window.ts`(`computeWindow`) | 🔧 | 定高窗口化纯函数，代码在。**perf spike(10万条)从没真跑过**，必验 |
| AgentBus 接口 + 环境工厂 | S `src/protocol/{bus,bus-factory,tauri-bus}.ts` | ✅ | `dispatch`/`subscribe` 抽象，按 `__TAURI_INTERNALS__` 选 bus。**换后端只换 bus 实现** |
| 边界 guard | S `src/protocol/guard.ts` | ✅ | Envelope 进 store 前结构校验，非法回 error，fail-closed |
| BusRelay（engine live link 已落地） | 当前 `ui/src-tauri/src/bus.rs` | ✅ | Phase 0 已从 mock 改为直连 engine `/v1/chat/completions` 并消费 SSE。Task 1.2 已改为 id-keyed chat、移除 `chat_lock`，每次 `chat.send` 用单 patch envelope 原子创建 user/assistant 行。GUI runtime 验收与 Perf Spike 待补 |

## I. 线协议 / 状态渲染契约

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| Envelope/Body 协议 v1 | S `schema/` + `docs/spec/protocol.md` | ✅ | 下行 blueprint/state/manifest/event/error；上行 intent/subscribe/hello/ack。作为 AIRP 内部线协议复用，非当前公共标准化目标 |
| Blueprint/Widget/Capability 类型 | S `schema/*.schema.json` | ✅ | 半永久 Blueprint、WidgetDef manifest、capability 闭集。Blueprint/Widget 必须吸收；capability 需补 engine 侧强制 |
| Rust + TS 双绑定 | S `bindings/rust`(+`AgentBus` trait)、`bindings/typescript` | ✅ | schema 真相 + 两端对齐 |
| ⚠️ 推理路由缺口 | S/G 全部文档 | 🆕 | 现有 intent `chat.send`→**MCP 数据工具**(只存取)。我们需→**引擎推理**(生成)。**无现成方案，需定** |

## J. 安全 / 沙箱 / 权限

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| iframe 沙箱桥 | S `src/registry/sandbox-bridge.ts` | 🔧 | opaque-origin iframe(allow-scripts 无 allow-same-origin) + postMessage。**真远程 import 待浏览器验证** |
| 同意闸门 + 持久化 | S `src/registry/consent.ts` | ✅ | 授权绑 `{type,version,source}`，localStorage 持久化，换源/升版重授 |
| capability 强制 | S 声明在，Gateway 侧强制 | 🆕 | **现只有 UI 侧单边限制，真强制不存在**。引擎侧要做 |
| 责任边界模型 | S `docs/SECURITY.md` | 📖 | 宿主守自身/不审插件/用户自担。三信任级(builtin/esm/esm+sandbox) |
| 注入放大警告 | M `deployment-tavern-agent.md:74-85` | 📖 | 不可信卡 + 有 shell 权限的 agent = 注入放大。引擎侧数据沙箱已有，agent shell 隔离另论 |

## K. 性能契约（防重蹈酒馆覆辙 —— 硬约束）

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| 7 条性能硬约束 | S `背景整理.md:248-256` | 📖 | 虚拟滚动/窗口分页/patch优先/稳定ID/重计算入Rust/流式增量/内存卫生。**任何实现者不许破** |
| 根因诊断 | S `背景整理.md:241-246` | 📖 | 酒馆崩=无界DOM+单线程阻塞+内存泄漏，非算力。WebView2 就是 Chromium 不多吃硬件 |
| Perf Spike 验证门 | S `背景整理.md:266-275` | 🆕 | 开发前灌 10万假消息验 60fps+内存封顶。**代码有、从没跑过，必做** |

## L. HTTP / 传输 / 桥接（Gateway + Core daemon）

| 零件 | 来源 | 状态 | 说明 |
|---|---|---|---|
| Core daemon HTTP 层 | C `daemon/`（axum + 鉴权 + 限流 10req/s） | ✅ | 完整 `/v1/*` API(chat/agent-run/characters/sessions/scenes/state/settings/models/history/rollback/regen) |
| 常数时间 bearer 鉴权 | C `daemon/` / G `middleware.rs` | ✅ | `AIRP_ACCESS_KEY`；默认 loopback + WebUI/Tauri CORS 白名单，自定义来源走 `AIRP_CORS_ORIGINS`。默认无 bearer，仅限本机可信拓扑 |
| MCP client（stdio+HTTP transport） | G `mcp/{client,transport/{stdio,http}}.rs` | ✅ | 要"引擎接第三方 MCP 工具"时复用。initialize 握手/版本协商/连接池 |
| 纯桥路由/分发 | G `bridge/mod.rs` + `RouteRule` | 📖 | 声明式 path→tool/resource。**纯 MCP 桥、接不到非 MCP 的 Core**——对单客户端价值有限 |
| Gateway 安全硬化批 | G ADR-009（SSRF/请求体上限/错误脱敏/优雅关机/OOM 防护） | 📖 | 若引擎 HTTP 要对外，思路可借 |
| agentbus SSE 适配 | G `agentbus/`（`/airp/dispatch`+`/airp/stream`） | 🔧 | 现存最接近的 UI↔后端桥，但**只桥到 MCP 工具、不桥推理**；自重写了一套 Envelope(与 S 重复) |
| Streaming（Gateway） | G Stage 2 | 🆕 | 返回 Unimplemented 的桩。唯一明确功能缺口 |

## M. 已知代码问题（若搬对应零件，一并修）

| 问题 | 位置 | 说明 |
|---|---|---|
| 角色卡 zTXt-only | M `character_store.rs:217` | 读不到真酒馆卡。用 C 的 png_parser 替换 |
| 世界书 Vec 结构 | M `lorebook.rs:8` | 酒馆是 object，解析失败 |
| 预设两套 RegexScript 冲突 | M `preset.rs:50-56` vs `preset_regex.rs` | 杀坏的那套 |
| state 写入不 clamp | C `chat_pipeline.rs:972` | 模型可写越界数值 |
| list 排序漂移 | M `character_store.rs:97`/`preset_store.rs:46` | 补 sort(E.1) |
| import_preset 绕沙箱 | M `tools.rs:1371`(E.2) | 统一走 safe_resolve |
| constant_time_eq 长度侧信道 | M `http.rs:137`(E.3) | 改 HMAC 定长比较 |
| 错误码全归 INTERNAL_ERROR | M `error.rs:41`(E.4) | 客户端错误用 -32600/-32602 |
| 并发写无文件锁 | C deploy §并发 | 多 agent 写同角色会漂移。per-character 串行化待建 |
| RFC6902 test 非事务 | S `store.ts` | 已修：patch 前预校验所有 `test`，失败不半应用 |
| Gateway pending oneshot 泄漏 | G R11.2 | 超时后 pending 表条目不清 |

---

## 汇总：拆件的性质分布

- **✅ 可原样复用（引擎心脏 + UI 主体）**：干净 prompt 内核、双 provider adapter、流式 FSM/拆包、封卷、Core 数据层与 daemon、整套 Tauri+Vue UI + Blueprint/widget + 协议契约。**这些是白捡的成熟资产。**
- **🔧 有基础需修/补**：酒馆预设正则字段、世界书高级语义、虚拟滚动验证、BusRelay 后续债务与数据层若干 E 系列。Agent 真工具、state schema 边界和 context bundle 载荷排序已完成。
- **🆕 必须新建**：**世界书完整解析+插入引擎**（最大工作量）、capability 引擎侧强制、Perf Spike 实跑、（可选）RAG/ClaudeCodeSdk/第三方 MCP 接入。UI→引擎聊天推理路由已先行落地，后续要把它从最小直连接成更完整的 State-Protocol/Blueprint 流。
- **📖 仅参考**：六戒律/两平面/性能契约/责任边界等设计律，Gateway 纯桥路由（对单客户端价值有限），State-Protocol 的公共标准化/通用 Agent 浏览器定位。
- **不继承的北极星**：Core 的 standalone 乐高 Agent 后端叙事、MCP-Server 的纯 MCP 数据层边界、Gateway 的纯协议桥目标、State-Protocol 的通用 Agent UI 标准化目标，均不作为 AIRP-Dev 主线。
