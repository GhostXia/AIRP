# MCP-Server 能力融入 engine（agent 内化 catalog）

> 用户 2026-07-02 定调：**把 AIRP-MCP-Server 的能力融进我们的 agent（engine），不是当外部 MCP 后端连**。MCP-Server 绝大部分内容是我们未来发展的**刚需**。
> 纠正此前误框：我曾因角色卡/世界书**解析有 bug**（属实）就把整个 MCP-Server 当"边缘零件库、可丢"——错。解析 bug 是局部要修的点；MCP-Server 的 **38 工具 / 12 工作流提示词 / 19 资源 + 数据模型**是完整 RP 数据管理面 = engine 的数据层 + agent 工具规格。
> 架构落点：**engine 原生内化**（拆解重组进 engine），**非**"engine 当 MCP client 连独立 MCP-Server"。这正是 Core 路线图 **M_AGENT-2**"把进程内数据操作包成 built-in 工具"的目标规格。
> 权威源：`D:\airp-mcp-server\src\mcp\{mod.rs,tools.rs,prompts.rs,resources.rs}`（本 catalog 从源码枚举）。
> 最后更新：2026-07-02

---

## 融入原则
- MCP-Server 的**工具面** → engine 的 **agent 内置工具注册表**（`agent/tools.rs`，M_AGENT-2+）。agent loop 直接调，不走 MCP 网络跳。
- MCP-Server 的**12 工作流提示词** → engine 的 **agent 工作流指南 / 技能**（与 §3.8 扩展面、agentskills.io 共底座）。
- MCP-Server 的**19 airp:// 资源** → engine 的**数据读 API / 资源面**（内部直读，或对 UI/扩展暴露）。
- MCP-Server 的**数据模型** → engine **数据层**的超集规格（engine 现有子集，需补齐）。
- **保留 MCP client 能力**：内化 ≠ 放弃 MCP。engine 仍作 MCP client 接**第三方** MCP server（§3.8 扩展生态）；只是我们**自己的** RP 数据能力内化为原生工具，不外置。

## 1. 工具面（38）→ engine 内置工具

状态：✅ engine 已有等价内部能力（M_AGENT-2 包成工具即可）｜🔧 engine 有部分/需补｜🆕 engine 无、需从 MCP-Server 移植

| 类 | MCP 工具 | engine 现状 | 融入动作 |
|---|---|---|---|
| 角色 | `import_card` | ✅ png_parser 正确 + `/v1/characters/import` | 包成 agent 工具（Task 1.1 已排） |
| 角色 | `list_characters` `get_character` `delete_character` | ✅ daemon 有 | 包成工具 |
| 角色 | `analyze_card`(4档) | 🆕 | 移植（+ `analyze_preset` 同族） |
| 角色 | `decompose_character` | 🆕 | 移植（拆 7 md，配 prompt） |
| 会话 | `start_session` `list_sessions` `append_message` `get_recent_context` `rollback_messages` | ✅ chat_store + daemon | 包成工具 |
| 世界书 | `apply_lorebook` `update_lorebook` | 🔧 orchestrator 有 aho-corasick 扫描；**解析结构错需修**（Task 1.3） | 修解析 + 包成工具 + §5 检索工具化 |
| 状态 | `get_live_state` `update_state`(RFC7386) | ✅ state live.json+history | 包成工具；补 schema clamp |
| 记忆 | `seal_volume` | ✅ volume_store/manager | 包成工具（阈值信号→loop 拍板） |
| 闸门 | `get_gating_status` | ✅ gating/checkpoints | 包成工具 |
| 预设 | `import_preset` `list_presets` `get_preset` | 🔧 orchestrator preset；解析需补字段 | 修 + 包 |
| 预设 | `write_preset_artifact` `list_preset_regex_scripts` `remove_preset_regex_script` `set_preset_regex_enabled` | 🔧 preset_regex 有骨架 | 移植正则脚本管理 + artifact 写 |
| 预设 | `decompose_preset` | 🆕 | 移植 |
| 场景 | `create_scene` `list_scenes` `get_scene` `add_character_to_scene` `merge_lorebooks` `build_scene_system_prompt` | ✅ scene.rs + orchestrator 多角色 | 包成工具 |
| 导出 | `export_context_bundle` | 🆕 | 移植（喂纯净 subagent 的成品上下文包，正合 loop=subagent 编排器；注意修 §3.5 载荷排序） |
| 插件 | `plugin_kv_get/set` `plugin_jsonl_append/read` `plugin_blob_write/read` | 🆕 | 移植——**零 schema 插件数据 = 扩展/记忆的数据底座**（§3.8 + Hermes 外部记忆 provider） |

> 汇总：engine 已有约 20 个的内部等价（包成工具即 M_AGENT-2）；🆕 需从 MCP-Server 移植的：analyze/decompose 族、preset 正则/artifact 全套、export_context_bundle、plugin 零schema 6 工具。

## 2. 工作流提示词（12）→ engine agent 工作流/技能

`build_system_prompt`（装配指南）· `filter_text`（预设正则过滤）· `state_update_instruction`（`<state>` 更新格式）· `decompose_character` · `enhance_analysis` · `build_session_context` · `seal_volume`（封卷指南）· `analyze_preset`（3步工作流）· `build_scene`（多角色5步）· `validate_card`（5类检查）· `tune_preset`（按模型热调）· `validate_preset`。

→ 这些是**教 agent 怎么用数据推进 RP 的工作流**，正是我们 agent 的原生工作流/技能。融入为 engine 的内置技能（agentskills.io 兼容，§3.8）。**尤其 `validate_card`+`decompose_character` = Task 1.1 的 inspect→unpack→import 三段，我们已有、直接用。**

## 3. 资源面（19 airp://）→ engine 读 API

characters（列表/card/greetings/world·lorebook/state·live/memory·{current,index,volumes/n}）· presets（列表/{id}/raw/artifacts/regex）· scenes（列表/{id}）· gating/{id}/checkpoints · plugins（列表/{name}/files/data/{path}）。

→ engine 内部直读 + 按需对 UI/扩展暴露（对 UI 走 State-Protocol，对第三方走 capability 门）。

## 4. 数据模型 → engine 数据层超集
`data/` 布局 engine 已基本一致（characters/sessions/presets/scenes/plugins + state/memory/gating）。MCP-Server 的域模型（character/session/lorebook/state/preset/scene/gating）是 engine 数据层的**完整规格**——engine 现有子集，随各 Task 补齐。路径沙箱（`safe_resolve_for_write`+`validate_id_segment`）一并移植。

## 5. 落地映射（并入路线）
- **M_AGENT-2（agent 内置工具）= 本融入的主载体**：把上表工具逐批包进 `agent/tools.rs`。engine 已有的先包，🆕 的从 `D:\airp-mcp-server` 移植。
- Task 1.1 导入：复用 `validate_card`+`decompose_character`+`import_card`（inspect→unpack→import）。
- Task 1.3 世界书：修 `apply_lorebook`/`update_lorebook` 解析 + §5 检索工具化。
- Task 1.5 预设：`import_preset` + 正则脚本全套 + `analyze/tune/decompose_preset`。
- Phase 2 扩展/记忆：plugin 零schema 6 工具（扩展数据底座 + Hermes 外部记忆）；`export_context_bundle`（喂纯净 subagent）。
- 12 工作流提示词 → engine 内置技能（agentskills.io 兼容）。
- **保留**：engine 作 MCP client 接第三方 MCP（§3.8）——内化自有能力，不放弃 MCP 生态接入。

## 6. 仍要修的解析 bug（局部，不影响"融入"定调）
角色卡 zTXt-only（用 engine png_parser 替换）、世界书 Vec 结构、预设 RegexScript 冲突、state 不 clamp、list 排序、import_preset 绕沙箱、错误码全 INTERNAL_ERROR——见 [PARTS.md](PARTS.md) §M。移植时一并修。
