# AIRP Engine (`airp-core`)

AIRP Engine 是 AIRP 产品内的无头 RP 引擎。它负责角色卡/世界书/会话/状态/场景/卷数据、上下文装配、上游 LLM 流式调用、Agent loop 骨架和 HTTP/SSE API。它与 `ui/` 和 `protocol/` 一起构成当前 AIRP workspace；AIRP-MCP-Server、AIRP-Gateway 和 AIRP-State-Protocol 原仓库只是资产来源，不是本 crate 的运行时依赖或产品边界。

当前状态、缺口与下一步以 [当前基线](../docs/CURRENT-BASELINE.md) 为准；2026-07-10 全项目独立审计仅作历史证据。

## 当前能力

- OpenAI-compatible 与 Anthropic provider；响应头阶段可配置超时；
- 单回合 `/v1/chat/completions` SSE 对话；
- `/v1/agent/run` 有 step/token/wall-clock/cancel 闸和 typed SSE 事件；
- Tavern Card JSON/PNG 导入、canonical/sidecar 落盘和角色 CRUD；
- 会话创建/列表、history、append、rollback、regen；
- rollback 在 service/API 与 `ChatLog` 持久化边界都拒绝非法 index；空日志 `index=0` 保留兼容；
- lorebook CRUD、OR-key 触发、`enabled`、`priority` 与 v2 `constant` 常驻注入；
- live state/history/schema 读取与模型 `<state>` 提取；
- scene、多角色 prompt、preset、regex、volume sealing；
- character/preset deterministic decompose、analysis preview/apply；
- settings/models/version/health 和 rate limit；默认 daemon 只适合 loopback 本地开发，desktop 使用进程级 bearer；development CORS 保留 WebUI/Tauri 精确来源，production CORS 只允许 `AIRP_PUBLIC_ORIGIN`。

## 必须诚实区分的边界

### Agent loop 已可动态规划和调用工具

当前 planner 使用 OpenAI/Anthropic 原生 structured tool call，在 step/token/wall-clock/cancel 边界内进行 plan-act-observe；工具执行受 capability、allowlist 和 destructive confirm 三层门控。finalizer 只接收整理后的 observation，并保持角色平面不含协调器噪声。它仍不是完整 MCP/skills/plugin runtime。

### 默认 Agent 工具恰为 19 个

| 分组 | 工具 |
|---|---|
| 基础 | `echo` |
| 会话 | `list_sessions`、`start_session`、`append_message`、`get_recent_context`、`rollback_messages` |
| 角色 | `list_characters`、`get_character`、`delete_character` |
| 状态 | `get_character_state`、`update_character_state` |
| 世界书 | `get_lorebook`、`update_lorebook`、`apply_lorebook`、`merge_lorebooks` |
| 记忆/导出 | `seal_volume`、`export_context_bundle` |
| Analysis | `enhance_analysis`、`apply_enhanced_analysis` |

目录由 `GET /v1/agent/tools` 从实际 registry 生成。底层模块或 HTTP route 存在仍不等于 Agent registry 已注册；persona、plugin data、MCP client/server、skills、完整记忆 runtime 均尚未实现。

### Worldbook 与 state 都是部分实现

- Worldbook 已有 v2 `constant` 合同；仍无 selective/secondary、probability、sticky/cooldown/delay、group、position/depth 等高级语义的完整 AIRP 合同；
- state schema 在写入前强制 required/type/range/additionalProperties，并以 revisioned atomic replace 更新 live/history；
- Chat/State/Lorebook 的 HTTP、pipeline 与 Agent tools 已复用共享 domain services；更广泛的跨资源事务仍需逐项设计。

### 部署安全边界尚未产品化

默认 daemon 绑定 loopback；只有隔离容器网络中的首方部署才显式传 `daemon --host 0.0.0.0`，且 Compose 不发布 engine 端口。development CORS 使用 WebUI/Tauri 内置精确来源并允许 `AIRP_CORS_ORIGINS` 追加可信来源；`AIRP_ACCESS_KEY` 可启用 Bearer 保护。`AIRP_DEPLOYMENT_MODE=production` 已实现监听前 fail-closed 配置/数据目录校验、单一 HTTPS origin CORS、local-path import 禁用和 bearer 热更禁用；`deploy/production/` 已提供 OCI/Compose + Caddy artifact，并由真实 HTTPS topology CI 验证。P1-P3 发布门禁仍未完成，且无论何种部署都不能把 engine 直接暴露给局域网、互联网或不可信浏览器 origin。

## 快速开始

Windows 本地工具链必须使用 D 盘：

```powershell
$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH

cargo run -p airp-core -- daemon --port 8000
```

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
| GET/POST | `/v1/scenes` | 列表/创建场景 |
| GET | `/v1/scenes/:scene_id` | 场景详情 |
| POST | `/v1/scenes/:scene_id/characters` | 加入场景角色 |
| GET | `/v1/presets` | 预设列表 |
| GET | `/v1/presets/:preset_id` | 预设详情 |
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
| `src/daemon/` | axum routes、handlers、auth、rate limit |
| `src/orchestrator/` | card/lorebook/state/preset 上下文装配 |
| `src/chat_store.rs` | JSONL 会话持久化 |
| `src/data_dir/` | 路径、迁移、沙箱与 session 布局 |
| `src/decompose.rs` | deterministic Markdown analysis sidecar |
| `src/volume_*` | 长会话卷与维护 |

## 验证

2026-07-14 在仓库根目录按 D 盘工具链执行：

```powershell
cargo test -p airp-core --locked
cargo test -p airp-core rollback --locked
cargo fmt --all -- --check
cargo clippy -p airp-core --lib --tests --locked -- -D warnings
```

PR #139 的本地结果：engine lib 464 passed / 1 ignored，integration suites 全绿；回滚过滤测试 13 passed；`subagent_context_has_no_orchestrator_noise`、fmt 和严格 Clippy 均通过。GitHub run `29297782903` 的 Rust workspace、UI and WebUI、Production topology 全绿。数字是该提交的证据快照，后续修改必须重跑。

## License

MIT OR Apache-2.0
