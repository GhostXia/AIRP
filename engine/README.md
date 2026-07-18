# AIRP Engine (`airp-core`)

AIRP Engine 是 AIRP 产品内的无头 RP 引擎。它负责角色卡/世界书/会话/状态/场景/卷数据、上下文装配、上游 LLM 流式调用、Agent loop 骨架和 HTTP/SSE API。它与 `ui/` 和 `protocol/` 一起构成当前 AIRP workspace；AIRP-MCP-Server、AIRP-Gateway 和 AIRP-State-Protocol 原仓库只是资产来源，不是本 crate 的运行时依赖或产品边界。

当前状态、缺口与下一步以 [当前基线](../docs/CURRENT-BASELINE.md) 为准；本页最后在 2026-07-18 的 `main@63f1c5b` 复核。

## 当前能力

- OpenAI-compatible 与 Anthropic provider；响应头阶段可配置超时；
- 单回合 `/v1/chat/completions` SSE 对话；
- `/v1/agent/run` 有 step/token/wall-clock/cancel 闸和 typed SSE 事件；
- Tavern Card JSON/PNG 导入、canonical/sidecar 落盘和角色 CRUD；
- 会话创建/列表、history、append、rollback、regen；
- rollback 在 service/API 与 `ChatLog` 持久化边界都拒绝非法 index；空日志 `index=0` 保留兼容；
- 多 Persona 存储、revision、HTTP CRUD/绑定，以及 chat pipeline 的显式/绑定/default 激活；
- lorebook CRUD、OR-key 触发、`enabled`、`priority`、`constant` 常驻注入、v4 `selective`/`secondary_keys` gate 与 shared normalizer/导入诊断；
- live state/history/schema 读取与模型 `<state>` 提取；
- scene、多角色 prompt、preset、regex、volume sealing；
- character/preset deterministic decompose、analysis preview/apply；
- preset 规范化导入报告、原始输入 sidecar、版本目录与原子 current 指针；
- `PromptAssemblyTrace` 已接入真实 single/scene chat 装配路径，按 provider payload 顺序记录材料 provenance；`POST /v1/chat/preview` 复用该路径，且不创建会话、不推进 history/memory/state 或修复 metadata；
- Phase 2 (#115) 6 类 asset（character/persona/preset/lorebook/state/memory）统一 revision 合同已落地：`commit_revision` + 单调 u64 `content_revision` + 不可变 `revisions/{N}/` 快照 + 原子 `current_revision` 指针；旧数据推送 `*_revision_unavailable` 诊断；`next_content_revision` 在 orphan revision_dir 场景下跳过取号，避免 asset 永久不可写；base lock / drift / rollback / 受控 dry-run / 完整 provenance 审计仍未交付；
- settings/models/version/health 和 rate limit；默认 daemon 只适合 loopback 本地开发，desktop 使用进程级 bearer；development CORS 保留 WebUI/Tauri 精确来源，production CORS 只允许 `AIRP_PUBLIC_ORIGIN`。
- settings 更新在专用异步事务边界内完成校验、原子持久化和 live commit；失败不产生部分更新，并发提交保持运行态与磁盘一致。

## 必须诚实区分的边界

### Agent loop 已可动态规划和调用工具

当前 planner 使用 OpenAI/Anthropic 原生 structured tool call，在 step/token/wall-clock/cancel 边界内进行 plan-act-observe；工具执行受 capability、allowlist 和 destructive confirm 三层门控。finalizer 只接收整理后的 observation，并保持角色平面不含协调器噪声。它仍不是完整 MCP/skills/plugin runtime。

### 默认 Agent 工具恰为 21 个

| 分组 | 工具 |
|---|---|
| 基础 | `echo` |
| 会话 | `list_sessions`、`start_session`、`append_message`、`get_recent_context`、`rollback_messages` |
| 角色 | `list_characters`、`get_character`、`delete_character` |
| 状态 | `get_character_state`、`update_character_state` |
| 世界书 | `get_lorebook`、`update_lorebook`、`apply_lorebook`、`merge_lorebooks` |
| 记忆/导出 | `seal_volume`、`export_context_bundle` |
| Analysis | `enhance_analysis`、`apply_enhanced_analysis` |
| Preset | `get_preset`、`update_preset` |

目录由 `GET /v1/agent/tools` 从实际 registry 生成。`update_preset` 支持 dry-run，实际写入受 destructive confirmation 门控。底层模块或 HTTP route 存在仍不等于 Agent registry 已注册；Persona 已有 domain/HTTP/pipeline，但没有 Persona Agent tool；plugin data、MCP client/server、skills、完整记忆 runtime 也尚未实现。

### Worldbook 与 state 都是部分实现

- Worldbook 已有 v4 `constant` + `selective`/`secondary_keys` 合同；probability、sticky/cooldown/delay、group、position/depth、递归等仍只是 advisory/unsupported runtime；
- state schema 在写入前强制 required/type/range/additionalProperties，并以 revisioned atomic replace 更新 live/history；
- Chat/State/Lorebook 的 HTTP、pipeline 与 Agent tools 已复用共享 domain services；更广泛的跨资源事务仍需逐项设计。

### Production P0 已实现，正式发布仍未完成

默认 daemon 绑定 loopback；只有隔离容器网络中的首方部署才显式传 `daemon --host 0.0.0.0`，且 Compose 不发布 engine 端口。development CORS 使用 WebUI/Tauri 内置精确来源并允许 `AIRP_CORS_ORIGINS` 追加可信来源；`AIRP_ACCESS_KEY` 可启用 Bearer 保护。`AIRP_DEPLOYMENT_MODE=production` 已实现监听前 fail-closed 配置/数据目录校验、单一 HTTPS origin CORS、local-path import 禁用和 bearer 热更禁用；`deploy/production/` 已提供 OCI/Compose + Caddy artifact，并由真实 HTTPS topology CI 验证。P1-P3 发布门禁仍未完成，且无论何种部署都不能把 engine 直接暴露给局域网、互联网或不可信浏览器 origin。

## 快速开始

AIRP 不限制 Windows 工具链的安装盘符。确保 `cargo` 可从当前 shell 的 `PATH` 找到后运行：

```powershell
cargo run -p airp-core -- daemon --port 8000
```

维护者本机因 `C:` 盘空间不足使用 `D:` 盘工具链；这是 [AGENTS.md](../AGENTS.md) 记录的本地覆盖，不是项目要求。

默认配置由程序默认值、`data/settings.json`、环境变量按顺序合并。运行时也可通过 `POST /v1/settings` 更新。provider/access secrets 仅在进程内或环境变量中存在，序列化会跳过它们并忽略旧明文字段。

CLI 调试单次流式输出：

```powershell
cargo run -p airp-core -- run --message "hello"
```

## HTTP API

### 对话与 Agent

| Method | Path | 说明 |
|---|---|---|
| POST | `/v1/chat/completions` | 单回合 RP SSE |
| POST | `/v1/chat/preview` | 无写副作用的脱敏 prompt 装配摘要；不返回 prompt 正文、API key 或 endpoint |
| GET | `/v1/agent/tools` | 排序后的实际工具目录与副作用等级 |
| POST | `/v1/agent/run` | 动态 structured tool-call Agent loop SSE |
| POST | `/v1/chat/history` | 读取历史 |
| POST | `/v1/chat/rollback` | 回滚到消息位置 |
| POST | `/v1/chat/regen` | 删除最后 assistant 消息以便重生成 |

### 角色、世界书与状态

| Method | Path | 说明 |
|---|---|---|
| GET | `/v1/characters` | 角色列表 |
| POST | `/v1/characters/import` | 上传 JSON/PNG；`card_path` 仅可在进程以 `AIRP_ALLOW_LOCAL_PATH=1` 启动时使用，当前不验证调用方身份，Web/远端不得使用 |
| GET/PUT/DELETE | `/v1/characters/:character_id` | 卡读取、更新、删除 |
| POST | `/v1/characters/:character_id/reextract` | 重解 sidecar |
| GET | `/v1/characters/:character_id/avatar` | 头像 |
| GET/PUT | `/v1/characters/:character_id/lorebook` | 基础世界书 |
| GET | `/v1/characters/:character_id/state` | live state |
| GET | `/v1/characters/:character_id/state/history` | state history |
| GET | `/v1/characters/:character_id/state/schema` | state schema |

### 其他数据与诊断

| Method | Path | 说明 |
|---|---|---|
| GET/POST | `/v1/sessions/:character_id` | 列表/新建会话 |
| DELETE | `/v1/sessions/:character_id/:session_id` | 删除命名会话 |
| GET/POST | `/v1/scenes` | 列表/创建场景 |
| GET | `/v1/scenes/:scene_id` | 场景详情 |
| POST | `/v1/scenes/:scene_id/characters` | 加入场景角色 |
| GET | `/v1/presets` | 预设列表 |
| GET | `/v1/presets/:preset_id` | 预设详情 |
| POST | `/v1/presets/import` | 校验并导入预设 |
| GET/PUT | `/v1/users/:user_id/persona` | legacy 默认 Persona |
| GET | `/v1/users/:user_id/persona/effective` | binding→default 生效 Persona、来源与 `bindings.character_persona_id` / `bindings.session_persona_id` |
| GET/POST | `/v1/users/:user_id/personas` | 多 Persona 列表/创建 |
| GET/PUT/DELETE | `/v1/users/:user_id/personas/:persona_id` | 多 Persona CRUD |
| POST/DELETE | `/v1/users/:user_id/personas/:persona_id/bindings` | 角色/session 绑定 |
| GET | `/v1/models` | 上游模型代理 |
| GET/POST | `/v1/settings` | 读取/更新配置 |
| GET | `/version` | 无鉴权版本信息 |
| GET | `/health` | 无鉴权健康信息 |

### Decompose / analysis

| Method | Path | 说明 |
|---|---|---|
| POST | `/v1/characters/:character_id/decompose` | 生成角色 analysis sidecar |
| POST | `/v1/presets/:preset_id/decompose` | 生成预设 analysis sidecar |
| GET | `/v1/characters/:character_id/analysis` | 文件列表 |
| GET/POST | `/v1/characters/:character_id/analysis/*filename` | 读取、preview 或 apply |

没有 `/mcp/v1`。`rmcp` 目前仅出现在依赖中，源码未实现 MCP transport/client。

## 主要模块

| 路径 | 职责 |
|---|---|
| `src/adapter.rs` | provider 请求与 SSE 解析 |
| `src/chat_pipeline.rs` | prepare/stream/finalize 单回合管线 |
| `src/agent/` | loop 骨架、Tool trait/registry、内置工具 |
| `src/agent/tools/` | 按 session/character、state/lorebook、volume/context、analysis、preset 拆分的工具 family |
| `src/daemon/` | axum routes、auth、rate limit 与 adapter facade |
| `src/daemon/handlers/` | 按资源职责拆分的 HTTP handler family |
| `src/daemon/tests/` | catalog/chat/settings/persona/security/session/state 等 route 合同测试 |
| `src/orchestrator/` | card/lorebook/state/preset 上下文装配 |
| `src/chat_store.rs` | JSONL 会话持久化 |
| `src/data_dir/` | 路径、迁移、沙箱与 session 布局 |
| `src/decompose.rs` | deterministic Markdown analysis sidecar |
| `src/volume_*` | 长会话卷与维护 |

## 验证

从仓库根目录执行：

```powershell
cargo test -p airp-core --locked
cargo test -p airp-core --lib subagent_context_has_no_orchestrator_noise --locked -- --nocapture
cargo fmt --all -- --check
cargo clippy -p airp-core --lib --tests --locked -- -D warnings
$env:RUSTDOCFLAGS = "-D warnings"
cargo doc --workspace --no-deps --locked
Remove-Item Env:RUSTDOCFLAGS
```

`main@63f1c5b` 的 [GitHub run `29631048229`](https://github.com/GhostXia/AIRP/actions/runs/29631048229) 中 Rust workspace（含 warning-free rustdoc 与干净提示词不变式 `subagent_context_has_no_orchestrator_noise`）、UI and WebUI、Production topology 全绿；本地复算 `cargo test --workspace --locked` = 735 lib（734 pass + 1 ignored）+ 40 integration tests，并覆盖 Phase 2h 6 类 revision 字段填充、`*_revision_unavailable` 诊断、orphan revision_dir 恢复、prompt preview 的 engine/HTTP/WebUI 与生产浏览器路径、#114 effective config summary（PR #217 Persona 激活来源 + 参数来源）、PR #219 高影响缺陷修复回归（quota 并发 / chat_store 原子替换 / character_lock 串行化 / replace_file parent-dir fsync / extract_card_assets 空 entries 保留旧 lorebook / next_volume_number u32::MAX saturating）与 PR #227 `replace_file` 扩展名保留回归。这些结果只属于该 commit，后续修改必须重跑并记录新结果。

## License

MIT OR Apache-2.0
