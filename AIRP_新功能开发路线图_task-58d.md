# AIRP 新功能开发路线图

基线：`main@d53acd1`（2026-07-24）。Engine 27 个 Agent 工具 + WebUI 33 屏已交付。以下按阶段排列，每阶段可独立发版。

---

## Phase 1: 快速见效（1-2 周）

低成本高感知，全部基于现有 Engine 能力，主要是 WebUI 补全 + 小 Engine 扩展。

| # | 功能 | 工作量 | 涉及文件 |
|---|---|---|---|
| 1.1 | **对话导出**（Markdown/JSON 一键下载） | S | `webui/assets/chat-space.js` + Engine 无需改动（history 已完整） |
| 1.2 | **Drift 回滚按钮** | S | `webui/assets/console-runtime.js` renderStyle() + `POST /v1/characters/:id/drift/rollback` 已有 |
| 1.3 | **场景添加角色 UI** | S | `webui/assets/console-runtime.js` renderScenes() + `POST /v1/scenes/:id/characters` 已有 |
| 1.4 | **角色关系图谱**（力导向图可视化） | M | 新页面 `34-relationship-graph.html` + 从 state/relationship 数据渲染 |
| 1.5 | **角色情感状态 HUD**（聊天侧栏状态条） | M | `webui/assets/chat-space.js` 侧栏 + `GET /v1/characters/:id/state` 轮询 |
| 1.6 | **Decompose/Analysis 入口** | S | 角色工作台加按钮，调用已有端点 |

---

## Phase 2: Agent 智能化 — 核心差异化（2-4 周）

AIRP 相对 SillyTavern 的真正护城河。Engine 侧新增编排层。

| # | 功能 | 工作量 | 设计要点 |
|---|---|---|---|
| 2.1 | **导演 Agent 编排** | L | 新增 `DirectorAgent`，持有 plot_status + npc_registry + world_clock；每轮用户消息后决定是否介入（引入冲突/切换场景/推进时间线）；角色 Agent 只负责扮演。Engine: `engine/src/agent/director.rs` |
| 2.2 | **NPC 自主行动轮** | M | 导演 Agent 可在用户消息间插入 NPC 行动（`npc_action` 已有）；WebUI 以不同气泡样式展示"世界推进"消息 |
| 2.3 | **世界时钟与定时事件** | M | `world_clock` 字段加入 session state；每轮自动 +N 时间单位；world_event 增加 `time_trigger` 条件。Engine: `engine/src/agent/tools/world_event.rs` 扩展 |
| 2.4 | **剧情弧编辑器** | M | 用户预设"起承转合"大纲 JSON；`advance_plot` 按大纲节奏推进；WebUI 新页面 `35-plot-arc.html` 可视化进度 |
| 2.5 | **长期记忆遗忘曲线** | M | `memory/resident.rs` 增加 decay 权重：每轮 compress 时按 importance * recency 衰减；低于阈值的记忆标记为 faded 不再注入 prompt |
| 2.6 | **多 Agent 辩论/会议模式** | L | scene 内多角色 Agent 就议题各自生成回复，用户可选择介入或旁听；需要并发调用 + 发言顺序调度 |

---

## Phase 3: RP 沉浸体验（2-3 周）

让对话"活起来"，偏前端 + 外部 API 集成。

| # | 功能 | 工作量 | 设计要点 |
|---|---|---|---|
| 3.1 | **多角色群聊 UI** | L | 基于 scene 系统，一个 session 内多角色轮流发言；WebUI `18-group-chat.html` 激活；气泡按角色着色 |
| 3.2 | **TTS 朗读** | M | Web Speech API 或外部 TTS（Azure/Edge TTS）；每角色绑定音色；聊天页加朗读按钮 |
| 3.3 | **场景插图生成** | M | 新 adapter 接图片生成 API（DALL-E / Stable Diffusion）；关键剧情节点自动或手动触发；图片存入 session 资产 |
| 3.4 | **氛围 BGM 建议** | S | 根据 scene tag / 情绪 state 推荐 YouTube/本地音乐链接；纯前端 + 可选嵌入播放器 |
| 3.5 | **打字机 + 表情动画** | S | 流式输出时角色头像微动画；消息完成时轻微弹跳；纯 CSS/JS |
| 3.6 | **对话片段分享卡片** | M | 选中消息 → 脱敏 → 渲染为图片/HTML 卡片 → 下载或复制；基于 #307 脱敏逻辑 |

---

## Phase 4: 创作工具（2-3 周）

面向 RP 作者/角色卡创作者的生产力工具。

| # | 功能 | 工作量 | 设计要点 |
|---|---|---|---|
| 4.1 | **角色卡模板库** | M | 内置 5-10 个模板（奇幻/科幻/日常/悬疑）；一键导入 → 微调；WebUI 新页面或导入页扩展 |
| 4.2 | **风格迁移** | M | 用户粘贴一段文本 → Engine 调用 LLM 提取风格特征 → 写入 drift/style profile；`POST /v1/style/learn` 新端点 |
| 4.3 | **对话示例生成器** | S | 给角色卡 + 场景描述 → Agent 生成 3-5 组对话示例 → 用户确认后写入角色卡 `mes_example` |
| 4.4 | **世界书知识图谱** | M | 可视化条目间 key 重叠/引用关系；力导向图；辅助发现设定冲突 |
| 4.5 | **剧情时间线导出** | M | 从 chat history + world_events 生成结构化时间线 → EPUB/Markdown/PDF |
| 4.6 | **角色卡版本对比** | S | 选两个 revision → JSON diff 高亮展示；基于已有 revision 系统 |

---

## Phase 5: 平台化与技术扩展（3-4 周）

扩展 AIRP 的适用场景和生态。

| # | 功能 | 工作量 | 设计要点 |
|---|---|---|---|
| 5.1 | **多 Provider 路由** | L | settings 扩展为 provider 数组；按角色/场景/任务类型路由（旁白→便宜模型，主角→旗舰）；Engine: `config.rs` + `adapter/mod.rs` |
| 5.2 | **本地模型支持 (Ollama)** | M | 新增 `BackendEngine::Ollama`；OpenAI-compatible 端点直连；settings UI 加 Ollama 选项 |
| 5.3 | **插件/自定义工具** | L | 用户通过 HTTP webhook 或本地脚本注册 Agent 工具；`agent/tools/plugin.rs` + 安全沙箱 |
| 5.4 | **MCP 服务器集成** | L | 连接外部 MCP 服务器，将其 tools 注册到 Agent registry；`engine/src/mcp/client.rs` |
| 5.5 | **多语言 UI** | M | i18n 字典 + 语言切换；优先英/日；#308 |
| 5.6 | **自动备份/恢复** | M | 定时 tar.gz data_root → 保留 N 份；WebUI 22 屏激活；`POST /v1/backup/create` + `POST /v1/backup/restore` |
| 5.7 | **跨设备同步 (WebDAV/S3)** | L | 增量同步角色/会话/记忆；冲突解决策略；P2/P3 |

---

## 依赖关系与建议顺序

```text
Phase 1 (快速见效) ─── 无依赖，立即可做
    │
    ├──→ Phase 2 (Agent 智能化) ─── 2.1 导演 Agent 是 2.2/2.3/2.6 的前置
    │         │
    │         └──→ Phase 3.1 (多角色群聊) 依赖 2.1 导演编排
    │
    ├──→ Phase 3 (沉浸体验) ─── 大部分独立，3.3 需新 adapter
    │
    ├──→ Phase 4 (创作工具) ─── 4.2 风格迁移依赖 style 系统（已有）
    │
    └──→ Phase 5 (平台化) ─── 5.1 多 Provider 是 5.2 的前置；5.3/5.4 独立
```

## 工作量图例

- S (Small): 1-2 天，单文件或少量文件改动
- M (Medium): 3-5 天，跨 Engine+WebUI
- L (Large): 1-2 周，需要设计文档 + 新模块

## 总计

- 5 个阶段，约 30 个功能点
- 预估总工期：10-16 周（按单人全职）
- 每阶段可独立发版，不必等全部完成
