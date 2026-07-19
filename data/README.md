# `data/` 持久化目录规范

> 当前/目标边界最后在 2026-07-19 按 Windows 便携 WebUI 优先路线复核。

`data/` 是引擎的数据根，不是一个可以随意堆放资源的共享目录。角色相关资产按稳定角色 ID 聚合；每个命名 session 是一个独立开局/存档槽位。完整的目标合同见 [`docs/SESSION-DATA-DESIGN.md`](../docs/SESSION-DATA-DESIGN.md)。

实际根目录由 `engine/src/data_dir/paths.rs` 的 `resolve_data_root()` 决定，优先级如下：

1. `AIRP_DATA_DIR` 环境变量；
2. 开发模式下的 `cwd/data`；
3. 其它打包程序使用的 OS per-user data 目录下的 `airp/`。

当前 Windows 便携 WebUI 启动器显式设置第一优先级，将 `AIRP_DATA_DIR` 固定到解压目录内的 `data/`，因此不会走第 3 项。用户升级或移动目录时必须一起迁移此目录。

仓库只提交本文件、默认 `settings.json` 和示例叙事风格，不提交任何玩家角色卡、世界书或会话数据。

## 核心归属模型

```text
data/
├── settings.json
├── secrets.json
├── styles/
│   └── profiles/default.md
├── characters/
│   └── {character_id}/
│       ├── card/
│       │   ├── card.json
│       │   ├── card.png
│       │   ├── raw.json
│       │   └── greetings/
│       ├── world/
│       │   ├── lorebook.json
│       │   └── extra/
│       ├── sessions/
│       │   └── {session_id}/
│       │       ├── meta.json
│       │       ├── history/
│       │       │   ├── chat_log.jsonl
│       │       │   └── chat_log_meta.json
│       │       ├── memory/
│       │       │   ├── current.md
│       │       │   ├── index.md
│       │       │   └── volumes/
│       │       ├── state/
│       │       ├── character/
│       │       │   ├── card.json
│       │       │   ├── card.png
│       │       │   ├── greetings/
│       │       │   └── provenance.json
│       │       ├── worldbooks/
│       │       └── revisions/
│       │           └── {revision_id}/
│       │               ├── manifest.json
│       │               ├── character/
│       │               └── worldbooks/
│       ├── state/
│       ├── gating/
│       ├── analysis/
│       └── memory/
├── presets/
│   └── {preset_id}/
├── scenes/
│   └── {scene_id}/
├── users/
│   └── {user_id}/
├── third_party/
│   └── worldbooks/
└── exports/
    └── context-bundles/{character_id}/
```

`secrets.json` 仅由 Windows 便携 WebUI 在显式开启 `AIRP_PERSIST_PROVIDER_KEY` 时创建。它是带版本字段的明文 provider key 单文件，API/UI 默认不回显；能读取 `data/` 的主体也能读取 key，因此不得提交、共享或收入支持包。

这是目标归属模型，目录按需创建。当前代码已经隔离命名 session 的 history 与 memory，并落地 Phase 2 (#115) 6 类 asset（character/persona/preset/lorebook/state/memory）的统一 `content_revision` 合同（asset 级 `revisions/{N}/` 快照 + `current_revision` 指针，PR #201/#202/#203/#206/#215）；但 `meta.json`、session state、角色卡/世界书工作副本物化、第三方素材库、世界书 manifest 与覆盖二者的 session 级统一 revision 仍待分阶段实现，不能把本树全部视为已交付能力。

### 角色目录名

`{character_id}` 是经过路径校验的稳定标识，不是角色卡的显示名。显示名可能重复、被修改或含有不适合作为路径的字符；它保存在角色卡内容中。重命名角色不应隐式移动持久化目录。

### 会话

新建的命名会话使用 UUID `{session_id}`，路径固定为 `characters/{character_id}/sessions/{session_id}/`。它可以简单理解为一个独立“开局”或“存档槽位”：对话历史写入 `history/`，该会话的封卷记忆写入 `memory/`；目标状态还包括独立 state、角色卡与世界书工作副本，以及覆盖二者的统一 revision。用户可修改的“开局 1”“二周目”等标题保存在未来的 `meta.json`，不进入目录名。

外层 `{session_id}` 同时是该开局、聊天历史和 `chat_log_meta.json` 的唯一规范 UUID。`history/` 与 `memory/` 已位于这个 UUID 目录内，不再嵌套或生成第二个对话 UUID。

未提供 `session_id` 的旧调用仍使用角色级 `history/` 和 `memory/`，这是向后兼容范围，不是新客户端优先采用的布局。不要自行用会话标题作为文件夹名。

### 角色自带世界书

角色默认世界书当前位于 `characters/{character_id}/world/lorebook.json`。它必须保留关键词、优先级、`constant` 和扩展元数据等结构化语义，因此不使用 `world.md` 代替。目标实现会在创建开局时把它物化为 session 工作副本；运行中的 session 不再把角色级文件当作动态唯一真值。

`world/extra/` 可存放额外导入的世界书 sidecar。旧版 `characters/{character_id}/worldbooks/` 仅保留扫描兼容；引擎不再为新角色创建该目录。

### 其他顶层目录

- `presets/`：用户导入的预设，按稳定 `preset_id` 分目录。
- `scenes/`：多角色场景；场景自身的共享世界书位于 `{scene_id}/world/lorebook.json`。
- `users/`：多用户隔离根；请求带 `user_id` 时，其下可再出现同样的 `characters/`、`presets/` 和 `scenes/`。
- `exports/`：引擎生成的上下文导出，不是输入资产源。
- `quota.json`：单用户模式的运行计数；多用户模式位于对应用户根。

第三方世界书统一使用 `third_party/worldbooks/{source_id}/{package_id}/{version}/`，并区分原始文件、AIRP 归一化素材和 provenance。该目录是素材库，不是活跃 session 的动态依赖；采用时必须复制到 session 的 `worldbooks/` 并由 manifest 明确启用、顺序、来源和 hash。此合同已经确定，但运行时尚未实现。

## 遗留兼容与迁移

- 根级 `world.md`、`items.md` 不属于角色自带世界书，也不再由新数据根自动生成。
- 升级不会删除用户已有的根级 `world.md`、`items.md` 或旧 `worldbooks/`；备份确认后再由用户清理。
- 迁移必须遵守“不覆盖新位置已有内容”的原则。发生新旧位置冲突时保留两者并报告，不能静默覆盖。
- 不要直接手工移动活跃会话目录；应通过受测试的迁移流程处理历史、记忆、状态和删除标记。

## 入仓与验收

- `git ls-files data/` 只应包含 `README.md`、`settings.json` 和 `styles/profiles/default.md`。
- `data/characters/`、`data/presets/` 及所有会话、状态、记忆、导出和迁移锁均由 `.gitignore` 排除。
- 仓库不得包含真实玩家的角色卡、历史、记忆、物品或世界状态。
- 新建数据根不应产生根级 `world.md`、`items.md`；新建角色不应产生 legacy `worldbooks/`。
