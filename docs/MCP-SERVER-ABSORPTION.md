# MCP-Server 能力融入 engine（agent 内化 catalog）

> **路线 catalog，不是交付清单**：38/12/19 是源 AIRP-MCP-Server 的枚举，不是本仓完成度或必须逐项复制的目标。2026-07-15 本仓默认 Agent registry 为 19 个**已注册**工具，并由 `GET /v1/agent/tools` 公开实际目录；文中“约 20 个内部等价”仍只是历史 domain/data 能力估算。新增能力应先进入共享 domain service，再由 HTTP/Agent/MCP adapter 暴露。当前状态见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)，文档层级见 [README.md](README.md)。
> 用户 2026-07-02 定调：**把 AIRP-MCP-Server 的能力融进我们的 agent（engine），不是当外部 MCP 后端连**。MCP-Server 绝大部分内容是我们未来发展的**刚需**。
> 来源库存：AIRP-MCP-Server 暴露 38 工具、12 工作流提示词、19 资源与一套数据模型；这些数字只描述来源仓能力面，是 AIRP 需求审计的候选输入，不是本仓的数据层或 Agent 工具权威规格。
> 架构落点：经 AIRP 需求、domain、安全与测试复核后，采纳的自有 RP 能力以内建 shared service/Agent tool 方式进入 engine，而不是把独立 MCP-Server 当外部 runtime 后端。
> 来源基线：AIRP-MCP-Server 的 `src/mcp/{mod.rs,tools.rs,prompts.rs,resources.rs}`（本 catalog 从源码枚举）。
> 最后校准：2026-07-15（19-tool registry 不变；工具与 HTTP handler 已按职责拆分，外部合同不变）

---

## 融入原则
- MCP-Server 的**工具面** → engine 的 **agent 内置工具注册表**（`agent/tools.rs`，M_AGENT-2+）。agent loop 直接调，不走 MCP 网络跳。
- MCP-Server 的**12 工作流提示词** → 候选 Agent 工作流/技能需求；若采用，与 [PLAN.md](PLAN.md) §4.4 的扩展面共用底座并由 AIRP 独立编写。
- MCP-Server 的**19 airp:// 资源** → engine 的**数据读 API / 资源面**（内部直读，或对 UI/扩展暴露）。
- MCP-Server 的**数据模型** → 候选 domain/interop 输入；AIRP 的规范数据合同仍由本仓文档与 shared service 定义。
- **保留 MCP client 方向**：内化不等于放弃 MCP。未来 engine 可作 MCP client 接第三方 server（见 [PLAN.md](PLAN.md) §4.4）；自有 RP 数据能力则保持原生 shared service/工具，不外置成平行真相源。

## 1. 工具面（38）→ engine 内置工具

状态：✅ engine 已有等价内部能力（接现有 shared service）｜🔧 engine 有部分/需补｜🆕 engine 无、仅表示候选需求

> **层级约定（#23）**："engine 现状"必须区分三个层级，不可混称——
> **data 层**（`data_dir`/store 函数）、**agent 工具**（`agent/tools.rs` 注册表，仅 agent loop 内可调）、
> **HTTP 路由**（daemon 对 WebUI/API 直接暴露）。agent 工具 ≠ HTTP 端点：当前 `/v1/agent/run`
> 当前已由模型原生 structured tool call 动态选择，但这里只列候选映射；未注册或未授权的工具仍不能当作 HTTP 能力的实际替代。

| 类 | MCP 工具 | engine 现状 | 融入动作 |
|---|---|---|---|
| 角色 | `import_card` | ✅ png_parser 正确 + `/v1/characters/import` | 包成 agent 工具（Task 1.1 已排） |
| 角色 | `list_characters` `get_character` `delete_character` | 🔧 list：data 层 + agent 工具 + HTTP `GET /v1/characters` 全有；get/delete：**data 层 + agent 工具已有（PR #20），无 HTTP 路由** | 工具已包（M_AGENT-2 batch 2）；HTTP get/delete 端点待 WebUI 实际需要时再加（含 dry-run/confirm 纪律，背景见 #23） |
| 角色 | `analyze_card`(4档) | 🆕 | 按 AIRP analysis 合同独立设计（与 `analyze_preset` 同族） |
| 角色 | `decompose_character` | 🔧 decompose service + HTTP 已有，Agent tool 未注册 | 复用现有 CharacterDecomposer/HTTP 合同；出现真实 Agent 工作流后再补 adapter |
| 会话 | `start_session` `list_sessions` `append_message` `get_recent_context` `rollback_messages` | ✅ chat_store + daemon | 包成工具 |
| 世界书 | `apply_lorebook` `update_lorebook` | ✅ 共享 lorebook service + 关键词扫描 + Agent tools | 高级 SillyTavern 语义仍按需扩展 |
| 状态 | `get_live_state` `update_state`(RFC7386) | ✅ state live.json+history | 包成工具；补 schema clamp |
| 记忆 | `seal_volume` | ✅ volume_store/manager + destructive confirm Agent tool | 阈值信号仍由 loop/调用方拍板 |
| 闸门 | `get_gating_status` | ✅ gating/checkpoints | 包成工具 |
| 预设 | `import_preset` `list_presets` `get_preset` | 🔧 orchestrator preset；解析需补字段 | 修 + 包 |
| 预设 | `write_preset_artifact` `list_preset_regex_scripts` `remove_preset_regex_script` `set_preset_regex_enabled` | 🔧 preset_regex 有骨架 | 按 PresetService/revision 合同补正则管理与 artifact 写入 |
| 预设 | `decompose_preset` | 🔧 decompose service + HTTP 已有，Agent tool 未注册 | 复用现有 preset decompose/HTTP 合同；出现真实 Agent 工作流后再补 adapter |
| 场景 | `create_scene` `list_scenes` `get_scene` `add_character_to_scene` `merge_lorebooks` `build_scene_system_prompt` | 🔧 scene.rs + orchestrator 多角色；`merge_lorebooks` 已注册 | 其余只在出现真实 Agent 工作流时暴露 |
| 导出 | `export_context_bundle` | ✅ 已注册；固定安全目录、稳定块在前、live state 在后、UTF-8 安全截断 | 保持 generic Markdown 与隔离 subagent 不变式 |
| 插件 | `plugin_kv_get/set` `plugin_jsonl_append/read` `plugin_blob_write/read` | 🆕 | 候选需求；须先完成 #163 的扩展身份、配额、沙箱与迁移合同 |

> 汇总（历史 catalog）：约 20 个指底层 domain/data 等价能力的估算，并非当前已注册工具数；当前 registry 的确切数是 19。未注册的 analyze/decompose 族、preset 正则/artifact、plugin 零 schema 仍须按当前源码与真实工作流重新核验后再决定是否暴露。

## 2. 工作流提示词（12）→ engine agent 工作流/技能

`build_system_prompt`（装配指南）· `filter_text`（预设正则过滤）· `state_update_instruction`（`<state>` 更新格式）· `decompose_character` · `enhance_analysis` · `build_session_context` · `seal_volume`（封卷指南）· `analyze_preset`（3步工作流）· `build_scene`（多角色5步）· `validate_card`（5类检查）· `tune_preset`（按模型热调）· `validate_preset`。

→ 这些是“如何用数据推进 RP”的候选工作流需求。采用时按 AIRP 自己的工具合同和安全边界编写，不复制来源 prompt；`validate_card` / `decompose_character` 需求应优先复用本仓既有 import/decompose 服务。

## 3. 资源面（19 airp://）→ engine 读 API

characters（列表/card/greetings/world·lorebook/state·live/memory·{current,index,volumes/n}）· presets（列表/{id}/raw/artifacts/regex）· scenes（列表/{id}）· gating/{id}/checkpoints · plugins（列表/{name}/files/data/{path}）。

→ engine 内部直读 + 按需对 UI/扩展暴露（对 UI 走 State-Protocol，对第三方走 capability 门）。

## 4. 数据模型 → engine 数据层超集
两仓的 `data/` 布局在 characters/sessions/presets/scenes/state/memory/gating 等概念上有重合，但 AIRP 当前规范只由本仓 shared services、[data/README.md](../data/README.md) 和专题合同定义。MCP-Server 域模型与路径沙箱经验是候选输入，不是 AIRP 的“完整规格”；缺口按本仓 ID、revision、事务与安全边界独立实现。

## 5. 落地映射（并入路线）
- **M_AGENT-2（agent 内置工具）= 本融入的主载体**：把上表能力按 AIRP 当前 domain/service 边界逐批独立重构为 `agent/tools/` 工具；engine 已有共享服务的先接 adapter，缺失能力按本仓需求新建，不移植来源实现。
- Task 1.1 导入：复用 `validate_card`+`decompose_character`+`import_card`（inspect→unpack→import）。
- Task 1.3 世界书：基础 `apply_lorebook`/`update_lorebook`/`merge_lorebooks` 已工具化；高级语义继续增量实现。
- Task 1.5 预设：`import_preset` + 正则脚本全套 + `analyze/tune/decompose_preset`。
- Phase 2 扩展/记忆：`seal_volume` 与 `export_context_bundle` 已注册；plugin 零 schema 6 工具仍待真实扩展工作流驱动。
- 12 工作流提示词 → engine 内置技能（agentskills.io 兼容）。
- **保留方向**：engine 作 MCP client 接第三方 MCP；内化自有能力不妨碍未来生态接入。

## 6. 仍要修的解析 bug（局部，不影响"融入"定调）
来源项目的角色卡 zTXt-only、世界书 Vec 结构、Preset RegexScript 冲突、state 不 clamp、list 排序、import_preset 绕沙箱、错误码过度归类等问题见 [PARTS.md](PARTS.md) §M。吸收需求时不得继承这些实现缺陷。
