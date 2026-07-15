# `data/` 持久化目录规范

`data/` 是引擎的数据根，不是一个可以随意堆放资源的共享目录。角色相关资产和运行状态应聚合在同一个稳定的角色目录下；命名会话再聚合在该角色的 `sessions/` 下。

实际根目录由 `engine/src/data_dir/paths.rs` 的 `resolve_data_root()` 决定，优先级如下：

1. `AIRP_DATA_DIR` 环境变量；
2. 开发模式下的 `cwd/data`；
3. 打包程序使用的 OS per-user data 目录下的 `airp/`。

仓库只提交本文件、默认 `settings.json` 和示例叙事风格，不提交任何玩家角色卡、世界书或会话数据。

## 核心归属模型

```text
data/
├── settings.json
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
│       │       ├── history/
│       │       │   ├── chat_log.jsonl
│       │       │   └── chat_log_meta.json
│       │       └── memory/
│       │           ├── current.md
│       │           ├── index.md
│       │           └── volumes/
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
└── exports/
    └── context-bundles/{character_id}/
```

目录按需创建，因此一个实际角色目录不一定包含树中的每一项。

### 角色目录名

`{character_id}` 是经过路径校验的稳定标识，不是角色卡的显示名。显示名可能重复、被修改或含有不适合作为路径的字符；它保存在角色卡内容中。重命名角色不应隐式移动持久化目录。

### 会话

新建的命名会话使用 UUID `{session_id}`，路径固定为 `characters/{character_id}/sessions/{session_id}/`。对话历史写入 `history/`，该会话的封卷记忆写入 `memory/`。

未提供 `session_id` 的旧调用仍使用角色级 `history/` 和 `memory/`，这是向后兼容范围，不是新客户端优先采用的布局。不要自行用会话标题作为文件夹名。

### 角色自带世界书

角色自带世界书的规范路径是 `characters/{character_id}/world/lorebook.json`。它必须保留关键词、优先级、`constant` 和扩展元数据等结构化语义，因此不使用 `world.md` 代替。

`world/extra/` 可存放额外导入的世界书 sidecar。旧版 `characters/{character_id}/worldbooks/` 仅保留扫描兼容；引擎不再为新角色创建该目录。

### 其他顶层目录

- `presets/`：用户导入的预设，按稳定 `preset_id` 分目录。
- `scenes/`：多角色场景；场景自身的共享世界书位于 `{scene_id}/world/lorebook.json`。
- `users/`：多用户隔离根；请求带 `user_id` 时，其下可再出现同样的 `characters/`、`presets/` 和 `scenes/`。
- `exports/`：引擎生成的上下文导出，不是输入资产源。
- `quota.json`：单用户模式的运行计数；多用户模式位于对应用户根。

`third_party/` 或 `third-party/` 当前没有运行时合同，不能仅凭目录名获得加载、隔离或许可证处理语义。未来若实现第三方资产管理，应先定义 schema、安全边界和 provenance，再统一采用一个名称。

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
