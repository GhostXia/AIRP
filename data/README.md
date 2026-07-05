# `data/` 目录说明

引擎数据根。`resolve_data_root()` 三层 fallback 决定真落点（见 `engine/src/data_dir/paths.rs`）：

1. `AIRP_DATA_DIR` 环境变量（最高优先）
2. `cwd/data`（开发模式：cwd 含 `Cargo.toml`）
3. `dirs::data_dir().join("airp")`（打包 .exe 双击：OS per-user 位）

本目录入仓的仅引擎默认配置与示例风格，**不含任何玩家会话产物**。

## 入仓文件（引擎默认配置）

| 文件 | 用途 | 性质 |
|---|---|---|
| `settings.json` | provider/endpoint/model/volume 默认配置；`api_key` 留空（用户自填或设 `AIRP_API_KEY` env） | 模板 |
| `styles/profiles/default.md` | 默认叙事风格（语气/句式/感官细节），引擎启动时读 | 默认配置 |

## 运行产物（不入仓，`.gitignore` 已排除）

| 路径 pattern | 用途 |
|---|---|
| `data/characters/` | 用户导入的角色卡 + history + sessions + state + memory |
| `data/presets/` | 用户导入的预设 |
| `data/items.md` | 玩家物品追踪清单（RP 运行时生成） |
| `data/world.md` | 世界观/场景/NPC 关系状态（RP 运行时生成） |
| `data/quota.json` | 封卷 quota 计数器 |
| `config.json` | daemon 启动自动写的运行配置 |
| `data/**/history/` | chat 历史 jsonl |
| `data/**/sessions/` | named session 命名空间 |
| `data/**/memory/` | 封卷记忆 |
| `data/**/gating/` | gating 决策记录 |
| `data/**/state/` | session 衍生状态 |
| `data/**/turn_counter*` | 会话计数器 |
| `data/**/*.lock` | 迁移/进程锁 |

## 验收

- `git ls-files data/` 仅含 `README.md` + `settings.json` + `styles/profiles/default.md`
- 新生成的 `data/characters/*/history/chat_log.jsonl` 等运行产物不被 `git add` 跟踪（`.gitignore` 通配生效）
- 仓库中无玩家个人 RP 内容（角色卡/历史/记忆/物品/世界状态）
