# AIRP-Core

> **独立、开源、乐高式的 Agent 后端。** 自调 LLM，装配角色卡 / 世界书 / 预设 /
> 卷 / 状态上下文，在有界戒律内跑 server-side agent loop（`src/agent/`），流式
> 输出经 FSM 过滤 + XML 拆包，落库 + 状态持久化 + 封卷。
>
> **乐高式定位：** Core 是生态的「推理大脑」——AIRP 四块里一直空着的那个框。
> 底座纯度（不调 LLM 的数据工具面）移交 AIRP-MCP-Server 守护；Core 升入 runtime 层。
>
> | 想要 | 用 |
> |---|---|
> | 纯 MCP 数据工具面（角色卡/世界书/会话/状态，不调 LLM） | [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server) |
> | 协议桥 / AgentBus（HTTP/SSE ↔ MCP 翻译） | [AIRP-Gateway](https://github.com/GhostXia/AIRP-Gateway) |
> | UI + State Protocol 契约（Tauri+Vue，Blueprint 渲染） | [AIRP-State-Protocol](https://github.com/GhostXia/AIRP-State-Protocol) |
> | **自调 LLM 的 Agent 后端 + 流式 RP（本仓）** | **AIRP-Core** |

---

**AIRP-Core 是自调 LLM 的独立 Agent 后端。** 两条入口：`POST /v1/agent/run`（多步 loop，M_AGENT-1 已落地）与 `POST /v1/chat/completions`（单回合退化，向后兼容）。loop 协调器把单回合流式管线当库复用，在有界戒律内派生纯净 subagent + 调工具 + 收敛。

**Core 自身调用 LLM 且跑 loop。** 底座纯度（不调 LLM、不跑 loop）由 AIRP-MCP-Server 守护；Core 升入 runtime 层补位生态空框——这是 `AGENT_BACKEND_PLAN.md` 的转向。

---

## 设计理念

- **License**：MIT OR Apache-2.0，商用 / fork / 集成无限制
- **独立 Agent 后端**：自调 LLM + 有界 server-side loop，单 Rust 二进制
- **乐高式**：不假设上游有 MCP-Server、下游有 Gateway；可独立跑
- **协议标准**：对外 OpenAI 兼容 + 结构化 tool-calling；不自造闭源协议
- **数据格式**：SillyTavern V2 角色卡 / lorebook / preset 直读，与 AIRP-MCP-Server 共享 `data/` 目录布局，可互换

---

## 架构戒律

Core 已从「单回合流式后端」演进为「独立 Agent 后端」（M_AGENT 系列，见 `AGENT_BACKEND_PLAN.md`）。底座纯度移交 [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server) 守护；Core 升入 runtime 层，受**「有界 Agent 戒律」**约束：

1. **有界** — loop 必须有 step 上限 + token/成本预算 + 墙钟超时，任一触顶即停。无限循环 = bug。
2. **可取消** — 任何在跑的 agent run 必须能被客户端单次请求中止，已派生子任务随之收敛。
3. **可观测** — 每一步（规划 / 工具调用 / 工具结果 / 生成）都流式可见，不做黑箱。
4. **工具最小授权** — 工具走 allowlist；破坏性工具默认 dry-run，需显式确认才真执行。
5. **幂等与隔离** — 带幂等键的工具重试不重复副作用；同角色/quota root 并发写串行化。
6. **上下文纯净（RP 命门）** — agent 脚手架（工具定义 / 规划指令 / 观测回灌）走结构化通道，**不混入角色 system prompt**。进角色上下文的 token 由 RP 数据决定。

> 一句话：底座戒律守"server 永不自醒"；Agent 戒律守"自醒也得在笼子里、且不弄脏角色上下文"。

**M_AGENT 进度：** M_AGENT-0 定位落档 ✅ · **M_AGENT-1 Loop 骨架 ✅**（`POST /v1/agent/run`，`src/agent/`）· M_AGENT-2~7 待推。

---

## 当前状态

| 项 | 值 |
|---|---|
| 测试 | 本地当前基线：`cargo test -p airp-core` 通过（302 unit passed + 10 integration passed，1 ignored） |
| Clippy `--lib --bins -- -D warnings` | 历史记录为 0 warning；当前轮未复跑 |
| CLI 子命令 | `daemon`（SSE 网关）、`run`（单次流式到 stdout） |
| HTTP 入口 | `/v1/chat/completions`（单回合）· `/v1/agent/run`（多步 loop，M_AGENT-1） |
| Agent 进度 | M_AGENT-0 ✅ · **M_AGENT-1 ✅** · M_AGENT-2~7 待推（见 `AGENT_BACKEND_PLAN.md`） |

---

## 快速开始

### 启动 daemon（SSE 网关）

```powershell
cargo run -- daemon --port 8000
# 或 release 二进制
airp-core.exe daemon --port 8000
```

随后前端经 `POST http://127.0.0.1:8000/v1/chat/completions`（OpenAI 兼容）发请求，Core 装配上下文、自调上游 LLM、流式回 SSE（`UnpackedChunk`：`immersive` / `<action>` 拆包），并自动落库 + 封卷。

配置（上游 endpoint / api_key / model 等）走 `config.json` + `data/settings.json` + 环境变量三层合并，运行时可经 `POST /v1/settings` 热重载。注意 `config.json` 是本地运行时配置，已被 `.gitignore` 忽略。

### 单次流式到 stdout（CLI 调试）

```powershell
cargo run -- run --character alice --message "你好" --filters "<thought>[\s\S]*?<\/thought>"
```

### Windows 构建环境

```powershell
$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
```

目标三元组 `x86_64-pc-windows-gnu`（见 `.cargo/config.toml`）。Linux CI 通过 `CARGO_BUILD_TARGET=x86_64-unknown-linux-gnu` 覆盖。

### 构建

```powershell
cargo build --release
```

### 使用模式

Core 两条入口，向后兼容：

```powershell
# 1. daemon SSE 网关（生产用）
cargo run -- daemon --port 8000
#   前端打 POST /v1/chat/completions（单回合，OpenAI 兼容）
#   或    POST /v1/agent/run（多步 loop，M_AGENT-1；max_steps>1 启用）

# 2. 单次流式到 stdout（脚本 / 调试）
cargo run -- run --character alice --message "你好" --filters "<thought>[\s\S]*?<\/thought>"
```

`/v1/agent/run` 入参 = `/v1/chat/completions` 超集：加 `max_steps`（缺省=1 退化单回合）、`token_budget`、`wall_clock_secs`。SSE 事件：`plan` / `tool_call` / `tool_result` / `delta` / `done`。

本仓当前未跟踪 `run_daemon.bat` / `run_tests.bat`。请使用上面的 `cargo run` / `cargo test` 命令。

---

## HTTP API

| Method | Path | 说明 |
|---|---|---|
| POST | `/v1/chat/completions` | OpenAI 兼容流式入口；限流 10 req/s、burst 20/IP |
| POST | `/v1/agent/run` | M_AGENT-1：多步 agent loop（SSE）；`max_steps=1` ≡ chat/completions |
| POST | `/v1/chat/history` / `/v1/chat/rollback` / `/v1/chat/regen` | 历史 / 回滚 / 重生 |
| GET | `/v1/characters` / POST `/v1/characters/import` | 角色列表 / 导入 |
| GET/POST | `/v1/sessions/:character_id` | 多会话 |
| GET/POST | `/v1/scenes` · `/v1/scenes/:id` · `/v1/scenes/:id/characters` | 多角色场景 |
| GET | `/v1/characters/:id/avatar` · `/state` · `/state/schema` · `/state/history` | 角色资产 / 状态 |
| GET/POST | `/v1/settings` | 运行时配置热重载 |
| GET | `/v1/models` | 透传上游 provider 的 /models |
| GET | `/version` | 构建元数据（name + version） |

**注：** `/mcp/v1`（MCP Streamable HTTP）已随 MCP 工具面剥离移除——如需 MCP 协议入口，用 [AIRP-Gateway](https://github.com/GhostXia/AIRP-Gateway) 或 [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server)。

---

## 模块地图（剥离后）

| 模块 | 职责 |
|---|---|
| `adapter.rs` | OpenAI / Anthropic 双 provider 流式调用 + 引擎分发（Direct / AnthropicMessages / ClaudeCodeSdk stub） |
| `chat_pipeline.rs` | 三相流：prepare（上下文装配）→ stream（FSM 过滤 + XML 拆包 + SSE）→ finalize（落库 + 状态 + 封卷） |
| `orchestrator/` | prompt 装配：card / lorebook / preset / gating / volume_inject / 多角色场景 |
| `daemon/` | axum 路由、HTTP handlers、`DaemonState`（`RwLock<MutableConfig>`）、鉴权 + 限流 |
| `chat_store.rs` | ChatLog JSONL 持久化，O(1) append |
| `volume_store.rs` / `volume_manager.rs` | current.md / vol_XXX.md / index.md I/O + 封卷工作流 |
| `fsm.rs` / `xml_unpacker.rs` | 字符级流式过滤 + `immersive`/`<action>`/`<state>` 拆包（`pub(crate)`） |
| `config.rs` | 三层合并：default → settings.json → env → 请求 body |
| `types.rs` | Newtype IDs：`CharacterId` / `PresetId` / `SessionId` / `SceneId` |
| `data_dir/` | 路径解析 + 安全原语 |
| `scene.rs` | 多角色场景配置（SceneConfig） |
| `png_parser.rs` | SillyTavern V2 PNG 角色卡解析 |
| `get_state_history` | 读状态快照（newest-first） | readonly |
| `write_preset_artifact` | Agent 写预设分析产物 | mutate / idempotent |
| `write_character_artifact` | Agent 写角色卡分析产物 | mutate / idempotent |
| `list_preset_regex_scripts` | 列预设正则脚本 | readonly |
| `remove_preset_regex_script` | 删一条正则脚本 | destructive |
| `set_preset_regex_enabled` | 启/禁用正则脚本 | mutate / idempotent |
| `import_user_persona` | 导入用户人设元设定（可封存） | mutate / idempotent |
| `lock_user_persona` | 封存用户 persona（写 persona.lock） | mutate / idempotent |
| `get_user_persona` | 读 base + state + drift_keys | readonly |
| `update_user_state` | 更新用户变量设定（drift overlay） | mutate |
| `get_user_state_history` | 用户状态历史快照 | readonly |
| `list_characters` | 列出全部角色 ID | readonly |
| `list_users` | 列出全部用户 ID | readonly |
| `get_character` | 取角色卡 + 元数据 | readonly |
| `get_live_state` | 读角色当前 state/live.json | readonly |
| `delete_character` | 删除整个角色目录（默认 dry-run） | destructive |
| `list_scenes` | 列出全部场景 ID | readonly |
| `get_scene` | 读场景完整配置 | readonly |
| `create_scene` | 从 JSON 创建/覆盖场景 | mutate / idempotent |
| `add_scene_character` | 向场景追加角色 | mutate |
| `list_volumes` | 列出角色已封存卷 | readonly |
| `read_volume` | 读取指定编号卷内容 | readonly |
| `seal_volume` | 封存 current.md 为下一卷（纯文件操作，不调 LLM） | mutate / idempotent |
| `plugin_kv_get` | 读插件 KV（plugins/{name}/{key}.json） | readonly |
| `plugin_kv_set` | 写插件 KV（任意 JSON 值，零 schema） | mutate / idempotent |
| `plugin_jsonl_append` | 插件 JSONL 追加（O(1) append） | append |
| `plugin_jsonl_read` | 插件 JSONL 分页读取 | readonly |
| `plugin_blob_write` | 插件任意文件写入（base64 / UTF-8 文本） | mutate / idempotent |
| `plugin_blob_read` | 插件任意文件读取（上限 4 MiB） | readonly |

**M_PLUGIN_DATA 零 schema 插件数据（戒律 4）：**
- 任何语言的 MCP client 取一个 `plugin_name` 命名空间即可存取自己的数据 — 无 manifest、无注册、无 schema 强制
- 数据落地 `data/plugins/{plugin_name}/`，完全任意文件树，AIRP 不解析语义
- 三个写工具均推送 `airp://plugins/{name}/data/{path}` 资源变更通知 — 可把 AIRP 当零代码事件总线

**User persona 双层模型（M_UP）：**
- **元设定 / Base**（`users/{id}/persona.json`）：初始人设，可通过 `persona.lock` 封存为只读契约
- **变量设定 / Drift**（`users/{id}/state/live.json`）：剧情推进中累积的变化（学会新技能、心情变化等）
- Server **不判定语义冲突**（戒律 1）— `get_user_persona` 返回完整 base + drift + drift_keys，Agent 自行推断「不会打篮球（base）vs 学会了打篮球（drift）」这类冲突

---

**ID 类型契约**：`character_id` / `preset_id` 反序列化即 `validate_id_segment`（拒路径分隔符、`..`、空字节、`.` 开头）；`session_id` 必须合法 UUID v4。

可选 API key 鉴权：env `AIRP_ACCESS_KEY` 设置后所有 `/v1/*` 路径要求 `Authorization: Bearer <key>`。

---

## 架构

```
前端 (HTTP/SSE) → POST /v1/agent/run (多步) 或 /v1/chat/completions (单回合)
  → daemon::agent_run / chat_completion handler
  → chat_pipeline::prepare_pipeline (装配上下文 + 持久化 user 消息)
      ├─ 校验 ID newtype（CharacterId / PresetId / SessionId / SceneId）
      ├─ 加载角色卡 + Orchestrator 装配 system prompt
      │    (card → preset → checkpoint gating → known context → 卷 → lorebook)
      └─ 持久化 user 消息（JSONL append）
  → [单回合] adapter::call_streaming_api_auto → fsm + xml_unpacker → finalize
  → [多步]   AgentLoop::run (src/agent/) 协调器：
      ├─ 每步：派生纯净 subagent (run_generation_step) / 调工具 / 收敛
      ├─ 四道闸：step cap + token 预算 + 墙钟 + CancellationToken
      └─ SSE 事件：plan / tool_call / tool_result / delta / done
```

多角色场景走 `prepare_scene_pipeline` 分支：加载 SceneConfig + 所有角色卡 + 合并 lorebook → `build_multi_char_system_prompt`。

### 关键模块

| 模块 | 职责 |
|---|---|
| `agent/mod.rs` | M_AGENT-1：`AgentLoop::run` 协调器（有界 loop + 四道闸 + SSE 事件） |
| `agent/tools.rs` | `Tool` trait + `ToolRegistry` + mock `echo`（M_AGENT-2 将加 built-in） |
| `daemon/mod.rs` | axum router + HTTP handlers + `RwLock<MutableConfig>` + 鉴权 + 限流 |
| `chat_pipeline.rs` | 三阶段流：prepare → stream → finalize |
| `orchestrator/` | 提示词装配（card / lorebook / preset / gating / volume_inject / 多角色场景） |
| `adapter.rs` | `Provider` enum、`ProviderConfig`、`GenerationParams`、OpenAI/Anthropic 双格式 SSE |
| `chat_store.rs` | ChatLog JSONL 持久化（O(1) append） |
| `fsm.rs` | char 级流过滤 FSM（`pub(crate)`） |
| `xml_unpacker.rs` | `immersive` / `<action>` / `<state>` 拆包（`pub(crate)`） |
| `volume_store.rs` / `volume_manager.rs` | 卷 I/O + 封卷工作流（`pub(crate)`） |
| `config.rs` | 三层合并：default → settings.json → env → request |
| `types.rs` | newtype ID（serde 反序列化时校验） |
| `data_dir/` | 路径解析 + 安全原语 |
| `scene.rs` | 多角色场景（SceneConfig） |
| `png_parser.rs` | SillyTavern V2 PNG 角色卡解析 |

### 工程不变式

- **`pub(crate)` 内部模块** — `fsm` / `xml_unpacker` / `volume_store` / `volume_manager` / `index_parser` / `auto_converter` 不对外暴露
- **热路径无 `Arc<Mutex>`** — `MutableConfig` 用 `std::sync::RwLock`
- **JSONL chat logs** — `OpenOptions::append` 唯一写路径，O(1)
- **newtype ID** — 反序列化时校验，下游免重复 `validate_id_segment`
- **`estimate_tokens` ±30% 近似** — 非真实 tiktoken；卷阈值容忍此精度

### Rust 原生加速点

| 路径 | 技术 | 效果 |
|---|---|---|
| 关键词扫描（`lorebook` + `volume_inject`） | `aho-corasick` 单次 DFA | **11.37× 实测加速**（500 entries × 3 keys × 4 KiB） |
| 流式 FSM（`fsm.rs`） | char-level 状态机 + `special_first_chars` HashSet 快进 + `mem::take/replace` 零 clone | 消除 N 次 `String::from(c)` 分配 |
| XML 拆包（`xml_unpacker.rs`） | 本地 buf 批量 + `mem::take` flush | 消除每字符 `Vec::push` |
| HTTP client | `reqwest::Client` 共享于 `DaemonState`（`Arc<ConnectionPool>`） | 跨请求复用，免 TLS 握手 |
| 流任务管理 | `tokio::task::JoinSet` | finalize await 全部子任务，无遗弃 JoinHandle |
| ChatLog | `chat_log.jsonl` 行式 + `OpenOptions::append` | O(1) 追加，仅滚动/回滚整体重写 |

---

## 数据目录

```
data/
├── settings.json
├── characters/{character_id}/
│   ├── card/                     (CF-1 文件夹形态：card.json + card.png)
│   ├── greetings/                (greetings 文件夹)
│   ├── world/lorebook.json       (CF-8 自动发现)
│   ├── analysis/                 (analyze_character_card 产物)
│   ├── state/
│   │   ├── live.json             (当前实时状态)
│   │   ├── history.jsonl         (状态快照时序，newest-last append)
│   │   └── schema.json           (M_LS-7 可选 schema)
│   ├── gating/checkpoints.json
│   ├── memory/                   (legacy session 卷系统：current.md / index.md / volumes/vol_*.md)
│   └── sessions/{session_id}/    (M5.1 显式 SessionId)
│       ├── meta.json
│       ├── chat.jsonl
│       └── memory/               (同 legacy memory/ 结构)
├── presets/{preset_id}/
│   ├── preset.json               (M_PR 目录化)
│   ├── preset.md
│   ├── regex/*.json              (PR-4 SillyTavern 正则脚本)
│   └── analysis/                 (analyze_preset 产物)
├── scenes/{scene_id}/            (M_MS 多角色场景)
│   ├── scene.json
│   ├── memory/                   (场景级独立卷系统)
│   └── world/lorebook.json       (场景级世界书)
└── plugins/{plugin_name}/        (M_PLUGIN_DATA 零 schema 插件数据)
    └── {arbitrary_file_tree}     (完全任意结构，AIRP 不解析)
```

---

## 测试基础设施

- **单元测试** — 当前 `cargo test -p airp-core` 覆盖配置三层合并 / 卷系统隔离 / FSM 状态转换 / Orchestrator 装配 / ChatLog 持久化 / 场景多角色装配等，共 302 个 unit test 通过，1 个 ignored
- **集成测试** — `tests/sse_wiremock.rs` + `tests/openai_compat.rs` 用 `wiremock` mock 上游 SSE，5 端到端场景
- **Property test** — `fsm.rs` proptest 验证 chunk 边界独立性 / 任意 UTF-8 不 panic / 变量替换 chunk 独立 / `<卷评估/>` 自闭合标签 chunk 独立
- **CI** — 当前 AIRP 仓没有项目级 `.github` CI；本地测试 + 人工 review 是主要门禁

---

## 配置三层合并

优先级：`default → data/settings.json → AIRP_* env → request body`

| 字段 | env 变量 |
|---|---|
| `provider` | `AIRP_PROVIDER` |
| `endpoint` | `AIRP_ENDPOINT` |
| `api_key` | `AIRP_API_KEY` |
| `model` | `AIRP_MODEL` |
| `daemon_port` | `AIRP_DAEMON_PORT` |
| `access_api_key`（鉴权） | `AIRP_ACCESS_KEY` |

合并完成后 `AppConfig::validate()` fast-fail（如 `VolumeConfig.soft >= hard` 拦截）。

---

## 部署

Dockerfile / docker-compose 部署文件当前未在本仓跟踪。部署文档待重新整理。

---

## 路线图与决策

**已完成里程碑：**
- M0–M3：Rust 质量审计 + 安全 + 错误统一 + 流管线 + 三层配置
- M_CF：角色卡文件夹分层
- M_PR：预设目录化 + SillyTavern 正则脚本（PR-1~10）
- M_MS：多角色场景
- M_MCP：MCP 协议全量集成（33 工具 + 资源 + 提示词 + stdio + HTTP）
- M_DX：API key 鉴权 + Docker 部署
- M_LS：实时状态系统 + schema 推断
- M_CA：Agent-driven 分析提示词
- **M_HARDEN：13/13 子任务全部完成**（鉴权扩展到 /v1/*、SceneId newtype 全量 retrofit、tool side_effect 元数据、resource subscribe emit、idempotency keys、stdio 优雅停机、/version 端点、rmcp pin、卷封存/跨卷维护软提示、list-tools CLI、safe_resolve property test、RwLock 决策验证）
- **M_PLUGIN_DATA：零 schema 三原语**（plugin_kv_get/set + plugin_jsonl_append/read + plugin_blob_write/read，6 工具 + 3 资源 URI + 订阅推送；戒律 4 开放接入落地）

**预留里程碑：**
- M_HELPERS（airp-mcp-helpers Rust crate，生态杠杆）
- M_ARTIFACTS_UNIFIED（通用 artifact 工具组）
- M_MODES（三档 prompt mode：compat / enhanced / bare）
- M_REGEN / M_MEMORY_ENTRIES / M_AUDIT_LOG / M_WORLD_EVENTS

---

## 已知限制

- **`estimate_tokens` ±30% 偏差** — 启发式而非真实 tokenizer。卷阈值容忍
- **Windows-GNU 本地覆盖率不可跑** — 历史记录显示 `profiler_builtins` runtime 缺失；当前仓无 CI 覆盖率门禁
- **错误响应中文为主** — 跨语言 API client 解析不便（未来 M_I18N 规划）
- **角色卡仅支持 PNG / JSON** — PNG 覆盖 `tEXt` / `zTXt` / `iTXt`（含 zlib 压缩），`ccv3`(V3) 优先回退 `chara`(V2)，v1 平铺卡自动归一化为 v2。WEBP / JPEG 非 SillyTavern 标准导出格式，暂不支持（未来扩展：EXIF/XMP 字节扫描解卡）

---

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or <https://opensource.org/licenses/MIT>)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
