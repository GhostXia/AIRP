# AIRP Chimera Blueprint: 同赛道竞品吸收清单 + 独立审计

> ⚠️ **战略备忘（未经采纳 / 已被再审计否定）**
>
> 本文档是 2026-08-04 的战略方向备忘，**未经用户采纳为路线图**，且 §5 独立审计已对多数建议提出否定或存疑意见。用户于 2026-08-05 裁决：**研究归档，不进路线**。本文保留仅为可追溯性，不构成对任何功能的承诺或计划。所有 P0–P3 优先级标记均为**建议备忘，非计划承诺**；任何项若要进入实施，必须重新经过独立设计、审计与用户显式批准。

> 状态: **战略备忘（已归档，不进路线）**（非已交付能力，非当前 sprint 承诺，非未来路线承诺）
> 适用范围: **0.0.4+ 战略方向备忘**，与当前 0.0.3 #130 maintainer 验收解耦
> 配套审计: 见本文 §5 独立审计视角
> 引用: [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md)（第一方源仓库吸收）· [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) §2 第三方设计参考 · [CURRENT-BASELINE.md](CURRENT-BASELINE.md)
>
> 立规依据: AGENTS.md "项目取向"（2026-07-03）· "第三方经验吸收与独立实现"（2026-07-11）· "周期性代际重构特例"（2026-07-16）· "审计 Agent 守则"（2026-07-03）

---

## 0. 摘要

AIRP 的"奇美拉"形态 =

> **用 SillyTavern 的资产生态做冷启动，用 Talemate 的状态精度做深度，用 Marinara 的产品体验做留存，用 Agnai 的协作能力做扩展，用 AI Dungeon / Friends & Fables 的商业化经验做变现，用 AIRP 自己的治理和原子提交做护城河。**

本文是战略方向备忘，不是已决策的路线图。所有 P0–P3 项均需独立设计与审计后才能进入实施；本文不构成对任何第三方代码、规则文本、prompt、测试、数据或视觉资产的复用授权（详见 §8 引用与归属）。

---

## 1. 背景与定位

- AIRP 当前 0.0.3 唯一 P1 阻塞是 [#130 maintainer 验收](https://github.com/GhostXia/AIRP/issues/130)（real provider onboarding + production Compose + real browser smoke）。Chimera 蓝图是 **0.0.4+ 战略方向**，不阻塞 0.0.3。
- 立规依据:
  - **项目取向**（2026-07-03 用户立）: 代码应当更开放、更透明、在未来更易修正、且更易迭代更新。
  - **第三方经验吸收与独立实现**（2026-07-11 用户立）: 只允许吸收理念、需求洞察、公开行为和互操作性经验；AIRP 实现必须从自身需求与架构边界出发，完全独立设计并重构，不复用第三方实现代码。
  - **周期性代际重构特例**（2026-07-16 用户立）: 允许通过显式启动的代际升级进行破坏式重构；本蓝图中如需破坏现有架构的项，必须按该特例流程执行（旗舰模型强制、双线并行、市场验证后才能替代等九条约束）。
- 蓝图与现行文档的关系:
  - **CAPABILITY-ABSORPTION.md** 管"第一方四源仓库"（AIRP-Core / AIRP-MCP-Server / AIRP-Gateway / AIRP-State-Protocol）的能力融入；
  - 本文管"第三方同赛道竞品"的理念吸收清单；
  - **ACKNOWLEDGEMENTS.md §2** 记录所有已研究过的第三方项目、固定版本与许可证核验状态。

---

## 2. 同赛道竞品全景（三层圈层）

按**项目与 AIRP 的交集程度**分为三个圈层。

### 2.1 第一圈层:直接竞争者（Agent + RP Engine）

这些项目在"Agent 驱动的角色扮演"方向上与 AIRP 存在直接重叠。

#### 2.1.1 Marinara Engine

三种聊天模式——Conversation（Discord 风格私聊）、Roleplay（沉浸式 RPG，带立绘和背景）、Game（AI Game Master，带队伍、任务和战斗）。25+ 内置 Agent，涵盖世界状态追踪、任务管理、战斗、表情检测、叙事导演、Spotify DJ、CYOA 选择等。

Tracker Agents 包括 World State、Expression Engine、Quest Tracker、Background、Character Tracker、Persona Stats、Custom Tracker 和 World Maps；Misc Agents 包括 Echo Chamber、Illustrator、Lorebook Keeper、Long-Term Memory、Combat、Immersive HTML、Music DJ、Haptic Feedback、CYOA Choices、Storyboard 等，甚至还有 UNO、Chess、Poker 等内置小游戏。

Agent 会消耗额外的 token 和模型调用，但 Marinara 会将共享同一连接的 Agent 合并为一次调用来节省开销。

角色卡浏览器可以从 Chub.ai、JannyAI、CharacterTavern、Pygmalion、Wyvern 等多站搜索和导入。

**AIRP 必须吸收的优势:**

| Marinara 优势 | AIRP 应吸收的形式 |
|---|---|
| **模式分层（Chat / RP / Game）** | AIRP 目前只有 RP Engine，应明确定义三种运行模式，让用户按需切换复杂度 |
| **Agent 可下载/可发现** | Marinara 有 Downloadable Agents 目录，带 Conversation/Roleplay/Game 兼容性徽章。AIRP 应建立类似的 Agent 市场 |
| **多站角色卡浏览器** | 内置 Bot Browser 直接搜 Chub/JannyAI/Pygmalion，而不是让用户先下载 PNG 再导入 |
| **Agent 负载可视化** | Agent 列表上方有负载估算，显示当前设置增加了多少 token 和多少额外调用，负载过高时变为警告色 |
| **视觉沉浸层** | 表情立绘自动切换、场景背景、天气叠层、插画生成、短视频场景——AIRP 作为无头 Engine 应定义标准化的视觉 Hook |
| **"装了就能玩"的产品哲学** | "你安装它，你运行它，它就能用"——不想花几个小时在配置上，只想玩 |

#### 2.1.2 Talemate

专注于强叙事和一致的世界/游戏状态追踪。拥有多个 Agent 分工:对话、叙事、摘要、导演、编辑、世界状态管理、角色/场景创建、TTS 和视觉生成。支持 Node 编辑器创建复杂场景和可复用模块，上下文管理涵盖角色细节、世界信息、过去事件和固定信息，所有 Prompt 使用 Jinja2 模板可定制。

使用 ChromaDB 实现长期记忆，支持自托管 API 如 KoboldCpp、text-generation-webui、LMStudio 和 TabbyAPI。

可单独选择重置的组件:context DB、历史记录（可保留静态条目）、intent state、每 Agent 缓存状态（可选择具体 key）和 reinforcements，取代了以前分散的重置命令。

时间流逝消息可直接在场景视图中插入、编辑和删除；世界状态管理器的历史视图显示可编辑时长字段的时间流逝条目。

Pin conditions 可以针对游戏状态变量；游戏状态变量可以在调试工具中查看和编辑。

**AIRP 必须吸收的优势:**

| Talemate 优势 | AIRP 应吸收的形式 |
|---|---|
| **Node 编辑器做场景/模块** | 可视化场景编排，让非编程用户也能编排 Agent 工作流 |
| **ChromaDB 向量长期记忆** | AIRP 有记忆系统但应支持可插拔的向量后端（ChromaDB / Qdrant / SQLite-vec）|
| **per-Agent API 选择** | Talemate 支持 per agent API selection。关键 Agent（Director）可以用强模型，追踪 Agent 用便宜模型 |
| **精细化状态重置** | 不是"全部清空"，而是可以单独重置历史、记忆、Agent 缓存、意图状态 |
| **时间系统的 UI 暴露** | 时间流逝不是隐藏在后台的数字，而是场景视图中可编辑的一等公民 |
| **Jinja2 Prompt 模板** | 让高级用户完全控制每个 Agent 的 Prompt 结构 |
| **游戏状态变量 + 条件 Pin** | 世界书条目可以根据游戏状态变量决定是否激活——这是 ST 没有的 |

### 2.2 第二圈层:资产生态中心（AIRP 必须兼容的平台）

这些不是 Agent Engine，但拥有 AIRP 必须寄生的资产生态。

#### 2.2.1 SillyTavern

一个免费、开源的聊天前端，可连接 OpenAI、Anthropic、KoboldAI、Oobabooga/Text Generation WebUI、LM Studio 等多种 LLM 后端。SillyTavern 仍然是市场上功能最丰富、可定制性最强的前端，开源、自托管、拥有庞大社区，持续产出扩展、主题和脚本。

World Info 功能允许创建详细的世界设定（位置、历史、规则、角色关系），当特定关键词出现在对话中时，系统自动将相关世界设定注入 AI 上下文。

扩展生态从向量存储长期记忆到图像生成集成，SillyTavern 几乎什么插件都有。项目始于 2023 年 TavernAI 的 fork，现有超过 200 位贡献者。

**AIRP 必须吸收的优势:**

| SillyTavern 优势 | AIRP 应吸收的形式 |
|---|---|
| **事实标准角色卡 V2/V3 规范** | SillyTavern 普及了角色卡格式，2026 年角色卡是 AI RP 的"通用货币"。AIRP 必须做到 100% 往返兼容 |
| **World Info 关键词触发机制** | `position`、`depth`、`probability`、`sticky`、`cooldown`、正则、递归——全部必须运行语义等价 |
| **扩展生态模式** | ST 的扩展是 JS 插件 + Server Plugin，AIRP 应定义等价的 Plugin SDK |
| **200+ 贡献者的社区惯性** | 需要让 ST 用户能"无痛迁移"——导入全部角色卡、World Info、Preset 并立刻可用 |
| **Prompt 排序系统** | ST 的 Prompt Order 拖拽排列是高级用户的核心工作流 |
| **Regex 脚本** | 正则替换输入/输出是 ST 用户大量使用的功能 |

#### 2.2.2 RisuAI

跨平台 AI 角色扮演聊天应用，桌面和 Web 双端，支持创意故事构建和角色交互，多 API 支持，内聊天资产，正则能力。RisuAI 带来视觉小说美学，具有富文本编辑器、分支对话树，支持带自定义背景和立绘的角色卡。内置 lorebook 系统特别强大，允许定义 AI 遵守的世界规则。

**AIRP 应吸收:**

| RisuAI 优势 | 吸收形式 |
|---|---|
| **视觉小说美学 + 分支对话树** | RP 体验不只是文字流——分支可视化、立绘表情切换、背景切换 |
| **移动端优先** | RisuAI 最适合移动端优先用户和休闲 RP 玩家。AIRP 作为 Engine 应定义 Mobile-friendly API |
| **轻量化定位** | RisuAI 在 SillyTavern 功能密度造成摩擦时是更轻的选择。AIRP 应有"简单模式"入口 |

#### 2.2.3 Agnai (Agnaistic)

免费开源 AI 角色扮演聊天平台，基于 PygmalionAI 的 Galatea-UI，支持 Kobold、NovelAI、AI Horde、OpenAI、Claude、Replicate、OpenRouter、Mancer 等多后端。

Agnai 是三者中唯一具有可信多用户支持的，适合共享服务器、协作角色扮演和小型团队项目。Agnai 定位为中间地带——像 SillyTavern 一样开源但有更干净的 UI 和更强的隐私功能，支持端到端加密，本地优先设计。

用户可以多人多 bot 同时群聊，通过 Memory 和 Lore book 构建持久世界观，支持 W++、SBF、Boostyle、纯文本等多种角色定义格式。

**AIRP 应吸收:**

| Agnai 优势 | 吸收形式 |
|---|---|
| **多用户协作 RP** | AIRP 目前是单用户 Engine，但多人协作 RP（共享世界、多人控角）是真实需求 |
| **端到端加密** | 隐私不只是"本地存储"，应支持加密备份和同步 |
| **多种角色定义格式** | W++、SBF、Boostyle 等全部支持，降低迁移门槛 |
| **群聊多 bot 架构** | 多 NPC 自主对话时的调度和上下文分配 |

### 2.3 第三圈层:商业化参考（产品体验 + 留存机制）

这些是封闭平台，AIRP 不会与之直接竞争，但必须学习其产品设计。

#### 2.3.1 AI Dungeon

健壮的 AI 记忆系统使用 Story Cards 和 Memory Banks 存储上下文相关信息，仅在相关时调出。自家 AI 研究团队构建了市场上最先进的 AI 原生冒险系统。定制微调模型意味着比其他任何产品都更好的角色扮演体验——真正的挑战、无陈词滥调。发现数千个其他玩家编写的场景，或创建并分享自己的场景；支持多人联机冒险。

**AIRP 应吸收:**

| AI Dungeon 优势 | 吸收形式 |
|---|---|
| **Story Cards + Memory Banks 机制** | 上下文相关记忆按需调出，不是全部塞进 prompt |
| **UGC 场景发现与分享** | 用户创建的场景/角色/世界可以发布、搜索、fork |
| **多人联机** | 朋友协作冒险——AIRP Engine 应预留多人会话协议 |
| **分级订阅模式** | 虽然 AIRP 是开源的，但可以为托管版设计分级方案 |

#### 2.3.2 Friends & Fables

首次将 AI Game Master、世界构建工具和虚拟桌面无缝集成——Franz（AI GM）叙述、裁决规则并实时响应世界变化。Franz 适应玩家选择并创建动态故事线，具有高级记忆和自定义能力。基于 D&D 5e 规则的战术回合制战斗。

**AIRP 应吸收:**

| Friends & Fables 优势 | 吸收形式 |
|---|---|
| **规则引擎（D&D 5e 等）** | AIRP 如果想进入 Game Mode，需要可插拔的规则系统 |
| **AI GM + 虚拟桌面集成** | 地图生成、战术战斗、位置发现——AIRP 的 Game Mode 路线图 |
| **UGC 世界浏览** | 数千个玩家制作的世界——社区驱动的内容发现 |

#### 2.3.3 KoboldAI / KoboldCpp

KoboldAI 不只是前端——它是运行本地模型的完整工具包。提供高度可定制的界面，具有记忆管理、世界信息和可编脚本动作等高级功能。

**AIRP 应吸收:**

| KoboldAI 优势 | 吸收形式 |
|---|---|
| **本地模型极致优化** | 采样参数精细控制（mirostat、rep penalty、tail-free 等），AIRP 的 Provider 层需要暴露等效控制面 |
| **可编脚本动作（Lua）** | 用户脚本不只是正则替换，而是可以介入生成流程 |
| **社区模型微调生态** | KoboldAI 社区围绕 RP 微调模型（如 Erebus、Nerys 等），AIRP 应考虑与微调模型社区对接 |

---

## 3. Chimera 蓝图（分层架构图）

把上述所有优势汇总，AIRP 的"奇美拉"形态应该是:

```
┌──────────────────────────────────────────────────────────────┐
│                     AIRP Chimera Engine                      │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌─ 资产层 ──────────────────────────────────────────────┐   │
│  │  ST V2/V3 角色卡 100% 往返兼容  ← SillyTavern        │   │
│  │  多格式角色定义 (W++/SBF/Boostyle) ← Agnai           │   │
│  │  多站角色卡浏览器 (Chub/Pygmalion/…) ← Marinara      │   │
│  │  UGC 场景发现 & 分享 & Fork ← AI Dungeon             │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌─ Agent 层 ────────────────────────────────────────────┐   │
│  │  Agent 可下载/可发现/带兼容性徽章 ← Marinara          │   │
│  │  per-Agent 模型选择 (强模型做Director) ← Talemate     │   │
│  │  Agent 负载估算 & token 成本可视化 ← Marinara         │   │
│  │  Node 编辑器编排 Agent 工作流 ← Talemate              │   │
│  │  Agent 批量合并调用节省开销 ← Marinara                │   │
│  │  有界 Agent 权限 + 原子提交 ← AIRP 原创               │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌─ 世界状态层 ──────────────────────────────────────────┐   │
│  │  World Info 关键词触发完整语义兼容 ← SillyTavern      │   │
│  │  游戏状态变量 + 条件 Pin ← Talemate                   │   │
│  │  时间系统 UI 暴露 & 可编辑 ← Talemate                 │   │
│  │  Story Cards + Memory Banks 按需调出 ← AI Dungeon     │   │
│  │  向量长期记忆 (ChromaDB/Qdrant) ← Talemate            │   │
│  │  精细化组件重置 ← Talemate                            │   │
│  │  可插拔规则引擎 (D&D 5e等) ← Friends & Fables         │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌─ 运行模式层 ──────────────────────────────────────────┐   │
│  │  Chat Mode (轻量对话) ← Marinara                      │   │
│  │  Roleplay Mode (沉浸叙事) ← AIRP 核心                 │   │
│  │  Game Mode (GM+规则+战斗) ← Marinara + F&F            │   │
│  │  简单模式入口 (降低门槛) ← RisuAI                     │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌─ 协作层 ──────────────────────────────────────────────┐   │
│  │  多用户协作 RP / 共享服务器 ← Agnai                   │   │
│  │  多人联机冒险 ← AI Dungeon                            │   │
│  │  端到端加密 ← Agnai                                   │   │
│  │  群聊多 bot 调度 ← Agnai                              │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌─ 视觉 & 沉浸层 ──────────────────────────────────────┐   │
│  │  表情立绘自动切换 ← Marinara + RisuAI                 │   │
│  │  场景背景 + 天气叠层 ← Marinara                       │   │
│  │  分支对话树可视化 ← RisuAI                            │   │
│  │  地图生成 + 战术战斗 ← Friends & Fables               │   │
│  │  插画/短视频生成 ← Marinara                           │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌─ 开发者 & 扩展层 ────────────────────────────────────┐   │
│  │  Plugin SDK (等价 ST 扩展生态) ← SillyTavern          │   │
│  │  Jinja2 Prompt 模板 ← Talemate                        │   │
│  │  Regex 脚本 ← SillyTavern                             │   │
│  │  可编脚本动作 (Lua/WASM) ← KoboldAI                   │   │
│  │  采样参数精细控制 ← KoboldAI                          │   │
│  │  MCP 工具接入 ← AIRP 原创                             │   │
│  └───────────────────────────────────────────────────────┘   │
│                                                              │
│  ┌─ 治理 & 恢复层 (AIRP 独有) ───────────────────────────┐  │
│  │  合同式不变式 + 基线文档                               │   │
│  │  TurnCommit 原子提交 + 崩溃恢复                       │   │
│  │  审计 Agent + CI 门禁                                  │   │
│  │  资产版本化 + 备份 + 回滚                              │   │
│  └───────────────────────────────────────────────────────┘   │
└──────────────────────────────────────────────────────────────┘
```

---

## 4. 优先级建议（冷启动价值排序）

AIRP 不可能同时吸收所有零件。按**冷启动价值**排序（**以下优先级均为建议备忘，非计划承诺**）:

| 优先级 | 吸收项 | 来源 | 理由 |
|---|---|---|---|
| **P0** ⚠️ | ST World Info 完整运行语义兼容 | SillyTavern | 没有这个，ST 用户不会迁移。**冲突标注**：此项与 [TAVERN-PARITY.md](TAVERN-PARITY.md) 已决策的 advisory 哲学矛盾——TAVERN-PARITY 定位为"advisory 对齐清单"而非"全字段等价"，AIRP 不追求 ST World Info 的完整运行语义等价。本行建议已被再审计否定，保留仅为记录原始战略思考。 |
| **P0** | per-Agent 模型选择 | Talemate | Director 用 Claude，Tracker 用本地小模型——直接影响成本和质量 |
| **P0** | "装了就能玩"的简单模式 | Marinara/RisuAI | 治理文档不应暴露给终端用户 |
| **P1** | Agent 负载估算 + token 成本可视化 | Marinara | Agent RP 最大痛点是"我不知道一轮要花多少钱" |
| **P1** | 向量长期记忆 | Talemate | 角色一致性的基础设施 |
| **P1** | 多站角色卡浏览器 | Marinara | 大幅降低新用户获取内容的摩擦 |
| **P2** | Node 编辑器 | Talemate | 高级用户的场景编排，但不阻塞 MVP |
| **P2** | 多用户协作 | Agnai | 真实需求但架构改动大 |
| **P2** | 视觉沉浸层（立绘/背景/天气） | Marinara/RisuAI | 必须有，但作为无头 Engine 可通过标准 Hook 委托给前端 |
| **P3** | Game Mode + 规则引擎 | Marinara/F&F | 远期路线图 |
| **P3** | UGC 场景发现 | AI Dungeon | 需要用户基数支撑 |

---

## 5. 独立审计视角

> 按 AGENTS.md "审计 Agent 守则"（2026-07-03 用户立）三原则执行: **独立审计**（不附和开发 agent 的结论）、**可以提出自己的想法**、**对历史决策产生质疑并主动查证**。本节是审计 agent 对 §3 蓝图与 §4 优先级的独立审计意见，不构成通过/阻塞裁决，但所有疑点应在 PR 合并后开 Issue 跟进（见 §6）。

### 5.1 蓝图本身的盲点

**疑点 A1 — "奇美拉"陷阱**: 吸收所有优势 = 什么都做一点、什么都平庸。真正的护城河是"显式拒绝做什么"。Marinara 的 UNO/Chess/Poker 内置游戏、Talemate 的 Spotify DJ、Haptic Feedback——大概率不该做。蓝图需要一张 **NOT 吸收清单**，明确边界。

**疑点 A2 — P0 排序与当前 0.0.3 阻塞项脱钩**: 当前唯一 P1 阻塞是 #130 maintainer 验收（real provider + production Compose + real browser smoke）。Chimera 蓝图里的 P0（ST World Info 兼容、per-agent 模型选择、简单模式）与 #130 没有交集。需明确: Chimera 蓝图是 0.0.4+ 战略还是 0.0.3 范围？若是 0.0.4+，应在 §1 显式声明并与 0.0.3 解耦。

**疑点 A3 — per-agent 模型选择 (P0) 与 token 成本可视化 (P1) 应耦合**: 没有成本可见性的 per-agent 选择是盲选——用户没法决定 Director 用 Claude Opus 还是 Sonnet。这两项要么同时 P0，要么都 P1。当前 P0/P1 拆分在逻辑上不成立。

### 5.2 架构层面的风险

**疑点 B1 — 多用户协作标 P2 是架构风险**: 单用户数据模型 → 后期加多用户 = 重写。即使实现晚做，**架构预留**必须是 P0/P1，否则数据模型一旦定型为纯单用户，后期改不动。Agnai 的端到端加密、群聊多 bot 调度同理。建议将"多用户架构 hook 预留"与"多用户实现"分开评级。

**疑点 B2 — Game Mode P3 关闭了战略门**: 如果 AIRP 真要和 Marinara/F&F 同赛道，Game Mode 不能等"远期"。规则引擎的架构 hook 至少应 P1 预留，否则 RP-only 数据模型定型后，加 Game Mode 等于半代际重构。建议将"规则引擎架构 hook 预留"与"Game Mode 完整实现"分开评级。

**疑点 B3 — 模型策略缺位**: AI Dungeon 的"定制微调模型"在 §2.3.1 原始分析里，但 §3 Chimera 蓝图里消失了。AIRP 是否推荐 RP 微调模型？是否对接 KoboldAI 社区模型（Erebus/Nerys 等）？是否自研或合作？这是战略级决策，不应被埋在"开发者层"的零散条目里。建议单列"模型策略"层并明确决策路径。

### 5.3 战略层面的质疑

**疑点 C1 — 护城河 claim 可疑**: §0 摘要里"AIRP 自己的治理和原子提交做护城河"。但这是技术护城河吗？竞品完全可以加（ST 加个 transaction log 不难）。真正的护城河通常是:
- **网络效应**（UGC 生态，AI Dungeon / F&F 的优势）；
- **切换成本**（资产锁定，这正是 ST 的优势，也是 AIRP 要兼容 ST 角色卡的原因）；
- **规模**（用户基数摊薄开发成本）。

技术优势在 AI 领域贬值极快——三年后"原子提交"可能是标配。建议重新审视护城河定位，把"治理 + 原子提交"定位为**工程基线**（must-have，不构成差异化），而非护城河。

**疑点 C2 — 竞品清单可能不全**: 缺失:
- **CharacterAI**（闭源但用户基数最大，定义了大众对 AI RP 的认知）；
- **NovelAI**（订阅 + 自家微调模型，商业模型参考）；
- **JanitorAI**（庞大用户基数，产品决策参考）；
- **Backyard.ai**（本地优先竞品）；
- **Faraday.dev**（桌面本地 LLM RP）。

闭源竞品的产品决策（尤其留存/变现）对 AIRP 同样有参考价值。建议补一轮闭源竞品研究。

### 5.4 与既有规则的衔接

**疑点 D1 — 代际重构特例的衔接路径未明确**: 蓝图里若要实现"破坏 ST 兼容性以重新设计角色卡 schema"这类破坏式重构，必须按 AGENTS.md "周期性代际重构特例"（2026-07-16 用户立）流程执行——旗舰模型强制、双线并行、市场验证后才能替代、不破坏用户资产。当前蓝图未指明哪些项属于"破坏式重构"、哪些属于"渐进改进"。建议在 §6 未决问题中列出。

**疑点 D2 — 第三方代码独立性规则**: AGENTS.md "第三方经验吸收与独立实现"（2026-07-11）明确禁止复制/翻译/改写第三方源码、规则文本、prompt、测试、数据集、HTML/CSS、图标。蓝图中"Jinja2 Prompt 模板 ← Talemate"、"Regex 脚本 ← SillyTavern"、"W++/SBF/Boostyle ← Agnai"等条目需要明确: AIRP 是**实现等价能力**，不是**移植第三方实现**。建议在 §8 引用与归属中显式声明独立实现边界。

---

## 6. 未决问题（PR 合并后开 Issue 跟进）

> 按 AGENTS.md "审计遗留项处理"（2026-07-06 用户立）与"时序约束": issue 提交**必须在 PR 合并之后**执行。本 PR 合并后，以下未决问题应分别开 Issue 跟进。

| # | 未决问题 | 来源审计疑点 | 建议优先级 | 建议 Issue 标签 |
|---|---|---|---|---|
| Q1 | NOT 吸收清单——明确显式拒绝做的功能边界（如内置小游戏、Spotify DJ、Haptic Feedback 等） | A1 | high | strategic, needs-design |
| Q2 | Chimera 蓝图与 0.0.3 #130 阻塞项的衔接路径（0.0.4+ 战略 vs 0.0.3 范围） | A2 | high | strategic, release |
| Q3 | per-agent 模型选择与 token 成本可视化的耦合关系（是否同时 P0） | A3 | high | strategic, agent, ux |
| Q4 | 多用户协作的架构预留等级（架构 P0/P1 vs 实现 P2） | B1 | high | architecture, strategic |
| Q5 | Game Mode / 规则引擎的架构 hook 预留等级 | B2 | medium | architecture, strategic |
| Q6 | 模型策略缺位——是否对接 RP 微调模型社区（KoboldAI Erebus/Nerys 等） | B3 | high | strategic, model |
| Q7 | 护城河定位重审——"治理 + 原子提交"是工程基线还是差异化？ | C1 | medium | strategic |
| Q8 | 竞品清单补全——CharacterAI / NovelAI / JanitorAI / Backyard.ai / Faraday.dev 研究 | C2 | medium | research, strategic |
| Q9 | 代际重构特例衔接——明确蓝图哪些项属于破坏式重构、需启动代际升级 | D1 | medium | strategic, remake |
| Q10 | 第三方独立性边界——在 §8 显式声明哪些是"实现等价能力"而非"移植第三方实现" | D2 | high | legal, strategic |

---

## 7. 与当前 release 的衔接

- **不阻塞 0.0.3**: 本蓝图是 0.0.4+ 战略方向，当前 0.0.3 唯一 P1 阻塞仍是 #130 maintainer 验收。
- **不自动启动代际升级**: 蓝图中如需破坏式重构的项，必须由用户显式启动"周期性代际重构特例"流程，半年/年度是允许的评估周期，不是到期自动改仓。
- **不豁免门禁**: 即使是战略级决策，仍受 §11.1 "PR → 审计 → 合并"门禁、第三方独立实现、许可证/provenance、安全、测试、神圣提示词不变式约束。
- **不影响 ACKNOWLEDGEMENTS**: 本文中提到的所有第三方项目，已同步更新 `docs/ACKNOWLEDGEMENTS.md` §2 第三方设计参考表，标记为"研究参考；未作为当前 capability 事实"。

---

## 8. 引用与归属

### 8.1 立规依据

- **AGENTS.md "项目取向"**（2026-07-03 用户立）: 代码应当更开放、更透明、在未来更易修正、且更易迭代更新。
- **AGENTS.md "审计 Agent 守则"**（2026-07-03 用户立）: 独立审计、可提己见、可质疑历史并查证。
- **AGENTS.md "第三方经验吸收与独立实现"**（2026-07-11 用户立）: 只允许吸收理念、需求洞察、公开行为和互操作性经验；AIRP 实现必须完全独立设计，不复用第三方实现代码。
- **AGENTS.md "周期性代际重构特例"**（2026-07-16 用户立）: 允许通过显式启动的代际升级进行破坏式重构。
- **AGENTS.md "审计遗留项处理"**（2026-07-06 用户立）: 未改动但写出来的修改意见应整理后写入 GitHub issue。
- **AGENTS.md "时序约束"**（2026-07-06 用户立）: issue 提交必须在 PR 合并之后执行。

### 8.2 第三方项目清单

本文研究的所有第三方项目均记录于 [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) §2 第三方设计参考表。本文中提到的功能描述、产品行为、互操作性经验均来自公开文档与产品行为研究，**不构成对第三方源码、规则文本、prompt、测试、数据集、HTML/CSS、图标或视觉资产的复用授权**。

按 §5.4 疑点 D2 要求，本文 §3 蓝图中所有"← 来源项目"标注应理解为"**实现等价能力的理念参考**"，而非"移植第三方实现"。具体独立实现边界由后续 Q10 Issue 跟进。

### 8.3 相关文档

- [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md): 第一方四源仓库（AIRP-Core / AIRP-MCP-Server / AIRP-Gateway / AIRP-State-Protocol）的能力融入通则。
- [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md): 第三方项目致谢与许可证核验。
- [CURRENT-BASELINE.md](CURRENT-BASELINE.md): 当前能力事实基线。
- [RISK-REGISTER.md](RISK-REGISTER.md): 项目风险登记。
- [TAVERN-PARITY.md](TAVERN-PARITY.md): SillyTavern 兼容性对齐清单。

---

## 9. 变更历史

| 日期 | 版本 | 变更 | 来源 |
|---|---|---|---|
| 2026-08-04 | v1 | 初版: 用户提出 Chimera 蓝图 + 审计 agent 独立审计视角 + 未决问题清单 | 本 PR |
