# 酒馆（SillyTavern）功能对标 + 扩展接口需求

> **需求对标，不是兼容性声明**：表内 ✅/🔧 表示 2026-07-01 源项目资产状态，不证明当前 AIRP-Dev 已交付；“AIRP-Dev 落点/缺口”列必须以当前源码复核。Worldbook 当前为基础 CRUD/关键词触发；单默认 Persona 已有 API/WebUI 与 chat 注入，多 Persona/base+drift 和 plugin/完整扩展 API 尚未完成。见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)。
> 目的：(1) 列全酒馆功能，标出候选能力；(2) 落实硬需求——**充分暴露接口，无门槛无缝支持第三方扩展**。来源：docs.sillytavern.app（2026-07 实读）。图例：✅ 源项目已有 ｜ 🔧 源项目部分有需补 ｜ 🆕 源项目皆无，需新加 ｜ ➖ 暂不做/低优先。
> 最后更新：2026-07-01

---

## 第一部分：酒馆功能全集 → 我们的缺口

### 1. 角色 / 人设
| 酒馆功能 | 源项目资产状态 | AIRP-Dev 落点/缺口 |
|---|---|---|
| 角色卡全字段（desc/personality/scenario/first_mes/mes_example/alt_greetings/creator_notes/system_prompt/post_history_instructions/tags/creator/version/embedded character_book） | ✅ | Core `TavernCardV2` 有全字段 + png_parser 正确解析 |
| Character's Note（按深度注入的角色级 prompt，可配 depth + role） | 🆕 | 角色卡里的深度注入，orchestrator 需支持 |
| Main Prompt / Post-History 覆盖（`{{original}}` 插入） | 🔧 | 字段在，装配时的覆盖+`{{original}}` 展开需做 |
| User Personas（用户人设，AI 理解个人信息） | 🔧 | 单默认 Persona 的 API/WebUI、名称与变量注入已实现；多 Persona、角色/会话绑定、base+drift、历史与迁移仍未实现 |
| Talkativeness（群聊发言概率 0-100%） | 🆕 | 群聊角色轮转权重 |
| 收藏/标签/hotswap | 🆕 | 角色管理 UI |
| Expression Images（情绪立绘） | ➖ | 扩展类，后期 |

### 2. 会话 / 消息
| 酒馆功能 | 源项目资产状态 | AIRP-Dev 落点/缺口 |
|---|---|---|
| Swipes（同一轮多个候选回复，左右切） | 🆕 | RP 招牌功能、粘性高。Core 有 regen 但无 swipe 候选管理 |
| Branches / Checkpoints（对话树、存档点分叉） | 🆕 | 从某消息分叉出平行线 |
| 消息编辑 / 重生成 / 继续 / 删除 / 隐藏 | 🔧 | Core 有 history/rollback/regen 端点；编辑/继续/隐藏需补 |
| Impersonate（让 AI 替用户写一条） | 🆕 | |
| 群聊机制（多 bot 互相对话、轮转顺序） | 🔧 | Core `scene.rs` 多角色装配在，轮转/talkativeness 调度需补 |
| 流式渲染 | ✅ | Core SSE + UI 流式追加 |
| Reasoning / thinking 块（思维链显示） | 🔧 | Core `xml_unpacker` 拆 immersive/action/state；reasoning 块显示+折叠需补 |

### 3. 世界书 / World Info
| 酒馆功能 | 源项目资产状态 | AIRP-Dev 落点/缺口 |
|---|---|---|
| 全字段 + 插入引擎（position/depth/order/probability/selective/secondary_keys/constant/sticky/cooldown/delay/递归/group） | 🆕 | **最大新建件**（见 PARTS.md F）。四仓皆残缺 |
| 关键词触发扫描 | 🔧 | Core aho-corasick 扫描在，但触发语义（selective/递归）缺 |
| 向量化/RAG 注入 | ➖ | Data Bank，后期 |

### 4. Prompt / 预设系统
| 酒馆功能 | 源项目资产状态 | AIRP-Dev 落点/缺口 |
|---|---|---|
| 采样参数 + 社区预设 | ✅ | 作建议素材，Agent 适配（见 PLAN §3.3） |
| Chat Completion 的 Prompt Manager（可重排 prompt 块） | 🔧 | 我们 orchestrator 有默认序；用户可重排的管理面需做（但按"建议素材"哲学，非机械回放） |
| Instruct Mode（Alpaca/ChatML/Llama2 指令模板） | 🆕 | 文本补全模型的指令格式包装 |
| Context Template（上下文模板） | 🆕 | |
| Author's Note（任意位置/深度/频率注入文本） | 🆕 | 比角色 note 更灵活，用户级。`/note /interval /depth /position` |
| Reasoning Formatting（思维块格式） | 🔧 | |
| Connection Profiles（API+模型+模板+设置一键切换打包） | 🆕 | 多配置切换，实用 |
| Start Reply With / 自定义停止串 / prompt 后处理 | 🔧 | 部分在 adapter |

### 5. API 连接
| 酒馆功能 | 源项目资产状态 | AIRP-Dev 落点/缺口 |
|---|---|---|
| OpenAI 兼容 / Anthropic | ✅ | Core adapter 双 provider |
| KoboldAI / Tabby / AI Horde / DreamGen / 本地等多后端 | ➖ | 按需扩 BackendEngine |

### 6. 内置扩展（酒馆随附，用户高频用）
| 酒馆内置扩展 | 源项目资产状态 | AIRP-Dev 落点/缺口 |
|---|---|---|
| TTS（文字转语音） / STT | 🆕➖ | 后期，走扩展接口 |
| Image Generation（SD/FLUX/DALL-E） | 🆕➖ | 后期，走扩展接口 |
| Image Captioning（图片描述） | ➖ | |
| Expression Images（立绘） | 🆕➖ | widget 可做 |
| Summarize（自动摘要） | 🔧 | 对应封卷，可强化 |
| Chat Vectorization（向量记忆） | ➖ | RAG 后期 |
| Chat Translation（翻译） | ➖ | |
| Web Search（联网搜索，函数工具） | 🆕➖ | 走 agent 工具 |
| Token Counter | 🔧 | Core estimate_tokens ±30%，可加真 tokenizer |
| Quick Reply（可脚本化快捷按钮） | 🆕 | 见第二部分 |
| Regex（用户可配的输入/输出 find-replace） | 🔧 | Core preset_regex 在，用户级 UI + 全字段需补 |

### 7. 其他
| 酒馆功能 | 源项目资产状态 | AIRP-Dev 落点/缺口 |
|---|---|---|
| Data Bank / RAG（文档引用检索） | ➖ | 后期 |
| Macros（`{{char}}/{{user}}/{{random}}/{{roll}}/{{time}}` + 自定义宏） | 🆕 | 见第二部分。装配层高频用 |
| STscript（脚本自动化语言） | 🆕 | 见第二部分 |
| 富 UI 定制（主题/背景/自定义 CSS） | 🔧 | State-Protocol theme tokens 在，CSS 注入需补 |
| 多用户模式 | ➖ | 本地单用户优先 |

---

## 第二部分：扩展接口需求（硬需求 —— 无门槛无缝支持第三方）

> 酒馆的扩展性是它的护城河，也是"无缝兼容第三方"的标杆。**我们必须暴露对标或更好的接口面。**

### 酒馆扩展模型（我们要对标的标杆）
- **两类**：UI 扩展（浏览器内 JS，DOM 全权访问）+ 服务端插件（放密钥/后端逻辑）。
- **manifest.json** + 生命周期钩子（activate/install/update/delete/enable/disable/clean）。
- **`getContext()` 全局 API**：chat/characters/settings/metadata + 工具函数（saveSettings/saveMetadata/writeExtensionField 写进卡/renderTemplate…）。
- **生成 API**：`generateQuietPrompt`（后台生成）、`generateRaw`（无上下文全控，支持 `jsonSchema` 结构化输出）。
- **事件系统**：`eventSource.on/emit` + 丰富 `event_types`（MESSAGE_SENT/RECEIVED/EDITED/DELETED/SWIPED、GENERATION_STARTED/ENDED、STREAM_TOKEN_RECEIVED、CHAT_CHANGED、WORLDINFO_UPDATED、TOOL_CALLS_PERFORMED…）。
- **Slash 命令**：`SlashCommandParser.addCommandObject`（带命名/位置参数），驱动 STscript。
- **Prompt 拦截器**：`generate_interceptor` —— 生成前拿到可变 chat 数组，可改可 abort。**这是最强的 prompt 干预点。**
- **消息格式化钩子**：`messageFormatter.addHook`（beforeRegex/afterRegex/afterMarkdown 三阶段管线）。
- **函数工具注册**：`registerFunctionTool`（LLM function calling）。
- **宏系统**：`macros.register/registerAlias`。
- **数据源抓取**：`registerDataBankScraper`。
- **状态持久化**：extensionSettings / chatMetadata / writeExtensionField（存进角色卡 V2 extensions）。
- **共享库**（lodash/DOMPurify/Handlebars/…）、i18n、Popup/loader UI 助手。

### 我们的现状 vs 缺口
- **UI 侧扩展：State-Protocol 已有开放系统**（widget manifest/命名空间/esm 动态加载/capability/iframe 沙箱/同意闸门）——**比酒馆更安全**（酒馆是无限制 DOM 访问，我们是声明式+沙箱+能力强制）。✅ 这块是优势，但目前只覆盖 **UI widget**。
- **引擎侧扩展：基本空白**。酒馆的 event/prompt-interceptor/slash-command/function-tool/macro 这些**引擎级钩子，我们没有对等物**。这是"无缝支持第三方"的最大缺口。

### 需要暴露的接口面（新建，🆕）
1. **事件总线**：引擎发全生命周期事件（消息收发/编辑/swipe、生成起止、流式 token、世界书命中、工具调用、state 变更…），第三方可订阅。对标 `eventSource`。
2. **Prompt 拦截钩子**：生成前把**装配好的角色平面上下文**交给已授权扩展过目/修改（但守 §1 干净提示词——拦截器改的是 RP 数据层，不能偷塞 agent 脚手架；且经 capability 门）。对标 `generate_interceptor`。
3. **函数工具注册**：第三方注册 LLM 可调的工具，接进 agent loop 的工具注册表。对标 `registerFunctionTool` —— **天然契合我们的 agent loop + MCP client**（第三方工具可走 MCP，比酒馆的 JS 更标准）。
4. **Slash 命令 / 脚本**：命令注册 + 一套脚本语言（STscript 对标）驱动自动化 + Quick Replies 可脚本化按钮。
5. **宏系统**：`{{char}}/{{user}}/{{roll}}/{{random}}/{{time}}` + 第三方注册自定义宏，装配层展开。
6. **消息格式化管线钩子**：输出渲染前的多阶段 transform（对标 messageFormatter + regex），用户/第三方可插。
7. **生成 API 暴露**：`generateRaw`/`generateQuietPrompt` 等价物，让扩展能后台调 LLM（结构化输出支持）。
8. **扩展态持久化**：扩展可存自己的设置 + 往角色卡/会话 metadata 写数据（对标 writeExtensionField/chatMetadata）——我们的插件零 schema 数据（plugin_kv/jsonl/blob）已是现成底座 ✅。

### 关键设计张力（需你拍板）
**酒馆"零门槛"= 无限制 JS + DOM 全访问**（它自己承认有安全风险）。**我们的 State-Protocol 刻意反向**——声明式 Blueprint + 沙箱 iframe + capability 强制，就是为躲这个风险。二者冲突：
- 选 A **酒馆式无限制**：门槛最低、生态最快，但把"Agent/第三方执行任意代码"的风险敞开——**直接违背 State-Protocol 的安全立仓之本**。
- 选 B **能力受控开放**（推荐）：暴露同样丰富的**结构化钩子**（事件/工具/拦截/宏/命令），但都过 capability 门 + 沙箱，第三方声明所需权限、用户授权。门槛略高于酒馆（要声明能力），但**无门槛 ≠ 无边界**——用标准化接口（MCP 工具 / 声明式 widget / 事件订阅）换取安全，且对第三方仍是"发个 manifest 就接入"。
- **我们的结构性优势**：agent loop + MCP client 让"第三方工具"可以是标准 MCP server（跨语言、进程隔离），比酒馆的同进程 JS **既更无缝又更安全**——这可能是相对酒馆扩展生态的差异化卖点。

---

## 第四部分：酒馆功能 → agent 框架原语的解耦重组（用户 2026-07-01 硬要求）

> **原则**：酒馆是"固定 prompt 装配管线 + 外挂插件"架构；我们是"agent 自主决策 + 能力以工具/钩子暴露"的 agent 框架。**照搬酒馆机械管线塞不进我们的框架。** 每个酒馆功能必须拆成"底层用户能力"，再用我们的原语（工具 / 记忆 / 技能 / 事件钩子 / prompt 装配规则 / 宏 / 子agent）重新表达。
> 我们的 agent 框架原语：**Tool（内置+MCP）· Memory（三层，§3.4）· Skill（agentskills.io 兼容）· Event Hook · Prompt-Interceptor · 装配规则（orchestrator）· Macro · Subagent**。

| 酒馆功能（机械形态） | 底层用户能力 | 重组为我们的 agent 原语 |
|---|---|---|
| **World Info**（固定关键词→按位置深度机械注入管线） | "相关背景按需进上下文" | **检索 Tool**（agent 生成中按需调 `lorebook_lookup`）+ 装配规则（触发条进角色平面）。位置/深度/selective 从"机械插入参数"降为"给 agent 的建议元数据"，agent 决定用不用（呼应 §3.2 待议）。**非硬编注入器** |
| **Author's Note / Character's Note**（固定深度注入文本） | "持久指令/提醒在某位置反复生效" | **Memory 常驻层条目**（§3.4）或**装配规则指令**，不是机械 depth-injector |
| **预设**（机械回放 prompt 结构） | "文风/参数/后处理打包移植" | **建议素材 + Agent 适配**（§3.3 已定）+ 正则→消息格式化 **Hook** + 采样参数→adapter 建议值 |
| **Quick Replies / STscript**（脚本按钮） | "自动化重复动作/自定义命令" | **Slash 命令注册 + Skill**（可脚本化）——即 agent 的命令/技能面 |
| **Regex 脚本**（输入输出 find-replace） | "收发文本转换" | **消息格式化 Hook**（多阶段管线，§3.8 扩展面），用户/第三方可插 |
| **Macros**（`{{char}}/{{roll}}`） | "模板变量展开" | **Macro 原语**（装配层展开 + 第三方注册自定义宏） |
| **Swipes / Branches / Checkpoints** | "多候选 + 对话树分叉" | **Session/State 管理**（agent 编排的会话操作 Tool + 存储分叉） |
| **Summarize / Vectorization**（内置扩展） | "长会话记忆" | **三层记忆 §3.4**（封卷 + session_search FTS5）——归内核记忆，非外挂 |
| **Expression Images / TTS / Image Gen / Web Search**（内置扩展） | "多模态/联网能力" | **Tool / MCP server / Widget**——走扩展接口，agent 按需调 |
| **Data Bank / RAG** | "文档引用检索" | **检索 Tool + 记忆外部 provider**（Hermes 式，可选） |
| **Connection Profiles** | "配置整组切换" | 引擎配置项（adapter 层），非 prompt 管线 |
| **Instruct Mode / Context Template** | "指令模型的格式包装" | adapter/装配层的**输出格式化**，agent 无关 |

**一句话**：酒馆把这些做成"管线里的固定环节 + 插件"；我们做成"agent 可调的能力（工具/技能/钩子/记忆）"。用户拿到的功能等价甚至更强，但底层是 agent 自主编排，不是死管线。

---

## 第三部分：优先级建议（供讨论）

- **MVP 必需**：角色卡/世界书/预设导入（含世界书引擎🆕）、基础会话（swipe🆕/编辑/regen）、干净 prompt 装配、单 provider 对话跑通。
- **紧随其后（扩展地基）**：事件总线 + 函数工具注册（走 MCP）+ 宏系统 —— 尽早立"接口暴露"骨架，避免后期返工（第三方生态越晚开越难改接口）。
- **中期**：Author's Note/Character's Note/Instruct Mode/Connection Profiles、Quick Replies/slash 命令/脚本、消息格式化管线、reasoning 显示、群聊调度。
- **后期/扩展态**：TTS/STT/图像生成/翻译/Web搜索/Data Bank-RAG/立绘 —— 全部走扩展接口，不进内核。
