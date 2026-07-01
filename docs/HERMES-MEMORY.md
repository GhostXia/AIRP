# Hermes Agent 记忆机制学习 → 用到我们 RP 客户端

> 研究对象：Hermes Agent（Nous Research 开源自进化 agent）。"随使用时长能力提升"的核心=持久记忆+技能自建+用户建模的自我进化闭环。
> 来源：hermes-agent.nousresearch.com/docs、mindstudio 5-支柱解析、glukhov 记忆系统技术贴（2026-07 实读）。
> 结论先行：这套机制**极适合 RP**，且**主要靠扩展我们已有的件即可实现**，是相对酒馆（每轮重灌静态卡+世界书、无跨会话学习）的**核心差异化**。
> 最后更新：2026-07-01

---

## 一、Hermes 五支柱（机制）

### 1. Memory（记忆）—— 两层
- **常驻层（always-injected，有界 markdown）**：
  - `MEMORY.md`（agent 笔记，~2200 字符/~800 token）：环境/项目/工作事实。
  - `USER.md`（用户画像，~1375 字符/~500 token）：偏好、沟通风格、行为约束。
  - **frozen snapshot**：会话开始时载入 system prompt 静态块；会话中改动落盘但**本轮不进 prompt，下轮才生效**——防模型对自己的记忆更新反应，且**稳定前缀=命中前缀缓存**。
  - **有界+主动整理**：超 80% 容量，agent 合并相关条目/删旧/压缩。"无限记忆是负债——鼓励乱塞、永不整理、终成噪声。"
  - **`memory` 工具**：add/replace/remove，target=memory|user，`old_text` 做外科式子串替换。**无 read**——记忆一直在 prompt 里。
- **历史层（按需检索）**：SQLite FTS5 全文检索（`~/.hermes/state.db`）存全部跨会话对话；`session_search` 工具 = FTS + LLM 摘要（Gemini Flash）做历史回忆。**非向量 RAG，轻量**。
- **可选外部记忆 provider**（Honcho/Mem0/Hindsight…）：prefetch（用户消息到达先搜）/sync（生成后存对话块）/queue-prefetch（后台备下一轮）。

### 2. Skills（技能）
- markdown + YAML front matter，**progressive disclosure**（元数据决定相关性，不载全文进上下文）。
- agent 从**可重复工作流**中提议建技能；**用户给反馈则更新技能**。"写一次的专家 playbook"，不是通用插件。
- 91 内置 + 520+ 社区，URL 一键装。

### 3. Soul（灵魂）
- `soul.md`=人格/语气当基础设施。同模型多 agent 靠不同 soul 表现不同。
- **动态**：反馈（"太啰嗦""语气不对"）→ agent 更新 soul。防"语气漂移"。

### 4. Crons（定时）
- 自然语言排程；隔离会话（不继承上下文）；cron 不能递归建 cron。

### 5. 自进化闭环
- 前四支柱的涌现属性：用户动作 → agent 学习 → 更新记忆/技能 → 相关时检索旧会话 → 精度提升。
- **"自动 ≠ 魔法"**：被动用有点改善；**主动纠正+显式存记忆+复杂任务后建技能 = 复利式提升**。

> 备注：这套（有界 md 记忆 + frozen snapshot + 容量整理 + skills md+YAML + session FTS 检索）**几乎就是 Claude Code 自己的记忆+技能系统**。

---

## 二、映射到我们的件（大部分靠扩展现有）

| Hermes 支柱 | 我们已有 | 差距/要加 |
|---|---|---|
| MEMORY.md 常驻 agent 笔记 | 🔧 Core 封卷 volumes + `memory/index.md` + world facts | 有归档卷，但**缺"有界+always-injected+自动整理"的常驻 RP 记忆**。要加 |
| USER.md 用户画像（自动学） | 🔧 Core User Persona（M_UP base+drift） | 有双层模型，但 drift 偏手动。**缺从对话自动抽取用户偏好/文风**——这是"越用越懂你"的魔法。要加 |
| Skills 经验技能 | ✅ 生态已有 SKILL.md + skills-vs-mcp + Claude-Code 式技能 | RP 技能（"怎么写角色 X""某类场景套路""你偏好的文风"）从经验自建。框架在，接进 loop |
| Soul 动态人格 | 🔧 角色卡/persona | 卡是**作者写死**的；Hermes soul 会随反馈演化。RP 里→给角色一个**学习式人格 overlay**（类似 state drift 但作用于文风/性格深度），或作用于"agent 的书写风格" |
| session_search（FTS5+摘要） | 🆕 我们 RAG 暂缓 | **Hermes 证明 FTS5+LLM 摘要是非向量的轻量长程记忆**，正合"先简单检索、RAG 后置"。"回忆三个月前那段剧情"对长 RP 极有用 |
| frozen snapshot 稳定前缀 | ✅ 我们的载荷按可变性排序/`[[CACHE_BREAK]]`（§3.5） | **同一原理**——记忆当稳定前缀跨轮字节稳定。Hermes 坐实了我们的缓存纪律 |
| 有界+80% 整理 | 🆕 | 我们封卷有阈值信号，但缺"常驻记忆超限自动合并压缩"。要加 |

---

## 三、为什么这是相对酒馆的核心差异化

- **酒馆是"每轮重灌静态卡+世界书"**——无跨会话学习，角色不会因为你玩得久而更懂你、更有深度。世界书是死的关键词库。
- **我们+Hermes 式记忆 = RP 会复利**：玩得越久，agent 积累①情节记忆（发生过什么）②用户模型（你的偏好/文风/雷点）③技能（怎么写这个角色/这类场景）④角色深度演化。**角色真的"活"起来、记得过往、适应你的口味。** 这是酒馆架构做不到的。
- 且与我们**最终形态（Claude Code 式完整 Agent，§0）完全同源**——Hermes 就是个自进化 agent，我们本就要做 agent 客户端，记忆+技能+自进化是 agent 的原生能力，不是 RP 特供外挂。

---

## 四、落到设计（建议，供讨论）

**RP 记忆分三层（借 Hermes 两层 + 我们封卷）：**
1. **常驻有界记忆**（新加）：每角色/每存档一份有界 markdown（RP 版 MEMORY.md=关键情节/关系/世界事实 + USER.md=用户文风偏好），always-injected 当稳定前缀，超限自动整理。**从对话自动抽取更新**（frozen snapshot：本轮落盘、下轮生效）。
2. **归档卷**（已有）：封卷 volumes，长会话压缩归档。
3. **历史检索**（新加）：SQLite FTS5 全文 + LLM 摘要的 `session_search`，回忆任意历史片段。非 RAG，轻量。

**技能层**：RP 技能（角色书写 playbook/场景套路/用户文风）从经验自建、反馈更新——接进 agent loop 的工具/技能注册表（与 §3.8 扩展面共用底座）。

**自进化闭环**：每轮/每会话，agent 抽事实→更新常驻记忆+用户模型+技能；下会话当 frozen 前缀注入。守 §1 干净提示词（记忆进角色平面是 RP 数据，正当；抽取/整理的控制逻辑走控制平面，不脏化角色 prompt）。

**Soul 动态人格演化（已确认加入，第二档优先级 —— 用户 2026-07-01）**：
- **调和"角色卡作者写死"的张力**：采 **base + drift overlay 双层**（复用 Core User Persona M_UP 的成熟模式）——原角色卡=**作者写死的不可变 base**（像 `persona.lock` 契约），**soul-drift=学习式人格 overlay**，随对话/反馈演化，注入时叠加于 base 之上、**不改原卡**。
- **演化什么**：角色的性格深度/说话习惯/关系态度随剧情累积（"这个角色跟你熟了之后更放松"），以及 agent 的书写风格贴合用户口味。可读可审（对标 Hermes"read me your soul file"）、可回滚。
- **守干净提示词**：soul-drift 是 RP 数据，进角色平面正当；演化/抽取的控制逻辑走控制平面。
- **优先级：第二档**（MVP 后、与常驻记忆/用户模型同批推进）。

**优先级**：常驻有界记忆 + 用户模型自动抽取 = "越用越懂你"的最小魔法，**MVP 后紧接着做**（是核心卖点，不宜太后）；session_search FTS5 中期；Soul 演化待议。

---

## 五、补漏（官方文档 + GitHub 一手核对，2026-07-01）

前面基于二手博客，官方核对后补以下**缺漏项**（对我们有价值的加粗）：

- **Honcho 辩证用户建模（dialectic user modeling）**：官方把它列为**核心用户建模机制**（非仅可选外部 provider）——"跨会话不断加深'你是谁'的模型"。是"越用越懂你"的官方主力实现。（注：前文引第三方博客的 `MEMORY.md ~2200字符 / USER.md ~1375字符` 具体数字是第三方分析，官方未固化，量级参考即可。）
- **程序性记忆 + 周期性 nudge**：agent 周期性"提醒自己"把知识持久化（procedural memory with periodic nudges）——不只被动抽取，有主动固化机制。
- **agentskills.io 开放技能标准（🌟对我们重要）**：Hermes 技能**兼容 agentskills.io 开放标准**，可移植/共享，社区 Skills Hub。**对我们"无缝支持第三方"是现成标准**——若兼容它，即接入已有技能生态，不必自造标准。
- **subagent + RPC 脚本调工具（零上下文成本，🌟对我们重要）**："派生隔离 subagent 跑并行工作流；写 Python 脚本经 RPC 调工具，把多步管线压成零上下文成本的一轮。" **这正是 Core"loop=纯净 subagent 编排器"的印证与延伸**——多步工具调用不堆进主上下文，压成脚本一次调。合我们干净提示词 + agent 脊柱。
- **MCP server 接入**："连任意 MCP server 扩能力"——**再次印证我们 MCP-client 当扩展底座**。60+/40+ 内置工具 + `execute_code` 程序化调用。
- **Headless + 多平台投递（🌟印证架构）**：Hermes 是**无头引擎**，经统一 gateway 投递到 20+ 平台（Telegram/Discord/Slack/WhatsApp/Signal/Email/CLI…）+ 多终端后端（Local/Docker/SSH/Modal/Daytona）。**强印证我们"引擎=独立 service、UI 无关"**——我们的"Tauri 先/web 后"只是最小版，Hermes 展示了天花板（任意前端/平台都是客户端）。
- **Slash 命令面**：`/personality`（切人格）、`/compress`（压上下文）、`/usage`、`/insights`、`/skills`、`/retry`、`/undo`、`/model`（切模型）、`/new`、`/reset`——对标我们 slash 命令扩展面（§3.8）。
- **安全模型**：命令审批（command approval）、DM 配对认证、容器隔离、命令 allowlist——对标我们 capability + 沙箱 + agent 戒律"破坏性工具默认 dry-run"。
- **RL 训练闭环（超出我们范围，记录）**：trajectory 导出 + Atropos RL 训练下一代工具调用模型——Hermes"变强"的最深层不止记忆，还产训练数据。我们暂不做，但记着"用户交互轨迹可留作将来训练资产"。
- **OpenClaw 迁移**：一键导入 settings/memories/skills/keys/personas/TTS/workspace——对标我们酒馆导入思路（迁移旧资产）。

**核对结论**：我的映射（三层记忆 + 用户自动抽取 + FTS 检索 + 技能自建）方向无误；主要补两条**对我们尤其重要**——① 兼容 agentskills.io 开放技能标准（白捡第三方生态）；② subagent+RPC 零上下文成本工具调用（印证干净 prompt + agent 脊柱，值得设计时纳入）。多平台 headless 也强印证引擎架构。
