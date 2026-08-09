# Widget 与 Trusted Plugin 双层扩展架构设计

> ⚠️ **Superseded for current status**：本文保留设计/审计轨迹；当前扩展能力、授权和安全边界以 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md) 与源码为准。

**类型**: 设计文档 / 架构备份
**状态**: 已设计，待实现
**日期**: 2026-08-07
**来源**: 与用户讨论 pi-rp / SillyTavern server plugins 后的独立判断

## 1. 背景与动机

### 1.1 起因

讨论 [pi-rp](https://github.com/2722550596/pi-rp/blob/main/README.zh-CN.md) 项目时提出："胶水层确实可以存在，但还是要要有我们自己的核心内容。"

### 1.2 对 pi-rp 的独立判断

pi-rp 自我定位是"把 RP 功能写进 agent 核心"，反对做成扩展。架构判断本身正确，但落地方式是 fork `pi-coding-agent` 在别人核心里打 patch——本质是**高级胶水**，没有自己的 agent runtime / 协议 / 数据格式。AIRP 已经拥有自己的核心（protocol / domain / revision / memory / orchestrator），不需要走这条路。

### 1.3 对 SillyTavern server plugins 的独立判断

ST 文档里的"侵入式"其实是**安全缺陷的描述**（"not sandboxed"），不是设计能力。ST 真正的设计只有两条：HTTP 路由注册 + init/exit 生命周期。其"侵入"能力来自 Node.js / JS 的动态性（monkey-patch），在 Rust 里**物理上不可实现**。

ST 模型给 AIRP 的可借鉴部分：**trusted plugin 生命周期（init/exit + 自有路由 + 启动加载）**。不可借鉴部分：无沙箱 + 同进程 monkey-patch。

## 2. AIRP 已有的核心（护城河）

对照代码事实，AIRP 自有的、不可被外部项目抽走的能力：

| 层 | 路径 | 性质 |
|---|---|---|
| 协议 | `protocol/` + sse-events.json / widget-intents.json / wire-discriminants.json | 自有 wire 协议 |
| 领域模型 | `engine/src/domain/` (chat / lorebook / state / plot / world_event / persona / lock_order) | 自有 model |
| 会话/分支树 | `engine/src/revision/` + `engine/src/conversation.rs` | 自有 revision tree |
| 记忆 | `engine/src/memory/` (compress / decay / extract / fts / resident / user_model) | 自研 |
| 编排/注入 | `engine/src/orchestrator/` | 自有注入管线 |
| Widget 沙箱 | `ui/src/registry/` | 自有 consent + sandbox |
| 备份/恢复 | `engine/src/backup/` | 自有格式 |
| 状态机 | `engine/src/fsm.rs` + `engine/src/chat_pipeline/` | 自有对话生命周期 |

AGENTS.md「第三方经验吸收与独立实现（2026-07-11）」守护的就是这一层——吸收理念，不复用实现。

## 3. 三档模型

```
┌─────────────────────────────────────────────────────┐
│  Dev Mode（最高，本期不动）                          │
│  阶段替换式 hook，常驻"开发者模式"提示                 │
├──────────────────┬──────────────────────────────────┤
│  Widget 层       │  Trusted Plugin 层              │
│  前端 iframe 沙箱 │  后端子进程                      │
│  manifest +      │  manifest + 显式信任              │
│  capability grant│  HTTP 反代                       │
│  widget_intent   │  /api/plugins/:id/*             │
│  → engine         │  → 127.0.0.1:<port>             │
├──────────────────┴──────────────────────────────────┤
│  Engine Core (Rust, 不可侵入)                       │
└─────────────────────────────────────────────────────┘
```

**Widget 和 Trusted Plugin 并列**，权限模型不同：
- Widget：零信任（沙箱 + capability + 逐调用 grant）
- Trusted Plugin：显式信任（用户装的、跑在本机的子进程）

**Dev Mode 在两者之上**，本期不实现。

## 4. 判别标准

hook 点的粒度 = engine 的演进自由度：

- **粗粒度 hook**（阶段边界：prepare / finalize / generation_step / memory_compress）→ 内部可重构，可以做（dev mode 专属）
- **细粒度 hook**（内部函数 hook）→ 焊死内部，不做

判别一个能力该不该做胶水层：
> "禁用这个扩展，RP 基础工作流会崩吗？会 → 必须在核心；不会 → 可以做胶水层接口。"

## 5. Widget Executor 计划（C-P4）

### 5.1 现状

`engine/src/extensions/api.rs:516` 注释明写：
> "C-P3 无执行器，授权通过即视为 intent 被接受并留痕。C-P4 接入真实执行器时，此分支改为派发到 executor。"

授权已闭环（manifest → capability → consent → grant 检查），缺最后一步 dispatch。

### 5.2 已有可复用资产

**KNOWN_CAPABILITIES 6 个**（`engine/src/extensions/mod.rs:649`，文档锁，与 `docs/WIDGET-DEVELOPMENT.md §5` 一致）：

```
read:memory | write:memory | read:worldbook | read:state | write:state | call:tool
```

已有 handler 内联了 read 逻辑：
- `read:memory` → `engine/src/daemon/handlers/memory.rs:66` `get_resident_memory`（`data_dir::resolve_session_dir` + 读 resident memory 文件）
- `read:state` → `engine/src/daemon/handlers/state.rs:40` `get_character_state`（`data_dir::char_state_dir(...).join("live.json")`）
- `read:worldbook` → `engine/src/daemon/handlers/lorebook.rs:21` `get_character_lorebook`（`LorebookService::new(...).read(...)`）

### 5.3 C-P4.1：read 三件套（先做）

在 `widget_intent` handler 的授权通过分支（`api.rs:526` return 之前）加 `match capability`，对三种 read capability 直接调已有 service/fs helper。

**不抽 executor trait、不引新模块**。`envelope.args` 带 `character_id` / `session_id`，校验后传给已有函数。

**改动文件**：
- `engine/src/extensions/api.rs`（widget_intent handler 加 dispatch）

**新增测试**：
- `engine/src/daemon/tests/extensions.rs`（read 路径端到端）

**不改**：`KNOWN_CAPABILITIES`、manifest schema、host_api、capability grant 流程——全复用。

### 5.4 C-P4.2：write + call:tool（read 跑通后再看）

- `write:memory` / `write:state`：写操作要留 audit log，校验是否破坏 chat pipeline 不变式
- `call:tool`：复用 `engine/src/plugin_tool.rs` 的完整沙箱（canonical 路径校验、env_clear、超时 kill、stdin/stdout/stderr pipe、MAX_INPUT/MAX_OUTPUT）

**判断**：C-P4.1 跑通后看真实 widget 用不用得上 read。没人用就不做 C-P4.2——YAGNI。

## 6. Trusted Plugin 计划

### 6.1 现状

AIRP 无此层。但 `engine/src/plugin_tool.rs:723-820` 已有完整 subprocess 模式（canonical 路径校验、env_clear、超时 kill、stdin/stdout/stderr pipe、MAX_INPUT/MAX_OUTPUT）——这是 trusted plugin 的现成基础设施，不重新设计。

### 6.2 Manifest

`data/plugins/manifests/*.json`：

```json
{
  "id": "com.example.tts",
  "version": "1.0.0",
  "command": "./tts-server",
  "args": ["--port", "${AIRP_PLUGIN_PORT}"],
  "port": 8765,
  "host_api": ">=0.1"
}
```

- `command` 必须在 `data/plugins/<id>/` 目录下（canonical 校验，复用 plugin_tool 越界检查）
- `host_api` semver 与 widget 一致（major 钉死）
- `port` 是 trusted plugin 自己监听的 loopback 端口，engine 不分配

### 6.3 生命周期

daemon 启动 → 扫 `data/plugins/manifests/*.json` → 逐个 spawn 子进程。

env 注入：
- `AIRP_PLUGIN_PORT`（manifest 指定）
- `AIRP_DATA_ROOT`（daemon data_root）
- `AIRP_PLUGIN_ID`

**不 env_clear**——trusted plugin 是用户显式装的，允许它读自己的环境（区别于 plugin_tool 的零信任脚本）。

daemon 退出 → kill 所有子进程（SIGTERM → 等 5s → SIGKILL）。

### 6.4 路由反代

`GET/POST /api/plugins/:id/*path` → 反向代理到 `127.0.0.1:<port>/*path`。

- 仅 loopback（与 daemon HTTP 一致）
- 超时 30s（复用 plugin_tool 超时常量）
- **不做 capability grant**——trusted plugin 是显式信任模型，不是 widget 的零信任
- **不做 caller 限制**——loopback 上任何进程都能调（trusted plugin 之间也能通过此路由互调，见 §6.6）

### 6.5 端口冲突（有意选择）

两个 trusted plugin 声明同一 `port` → daemon 不做事前检测 → 第二个 spawn 时端口被占 → 子进程启动失败 → daemon 记日志。

`ponytail:` 不做事前端口分配表。冲突在 spawn 时暴露，用户看到日志后改 manifest 重启即可。事前检测要维护端口注册表 + 处理释放竞争，是策略复杂度，不是基础设施。

### 6.6 Plugin 间通信（明确允许，不保证语义）

trusted plugin 子进程在 loopback 上，**技术上能访问** `127.0.0.1:<daemon_port>/api/plugins/<其他_id>/...`。daemon 不限制 caller。

**明确允许** plugin 间通过反代路由互调，但 engine **不保证**：
- 调用顺序（A 启动时 B 可能还没起）
- 依赖图（engine 不解析 plugin 间依赖）
- 健康检查（engine 不探活 plugin，A 调 B 失败 = A 自己处理）

plugin 需要依赖另一个 plugin 时，应在自己的 manifest 注明（供用户阅读），但 engine 不强制。

### 6.7 崩溃处理（最小）

子进程退出 → daemon 记录日志，**不自动重启**。
`ponytail:` 不做自动重启，等真有人需要长时间跑的服务再做。重启策略是策略问题，不是基础设施问题。

### 6.8 改动文件清单

| 文件 | 改动 |
|---|---|
| `engine/src/plugins/mod.rs` | 新增模块（manifest struct + DaemonState 字段） |
| `engine/src/plugins/spawn.rs` | spawn/kill 逻辑（复用 plugin_tool 的 canonical/env 模式） |
| `engine/src/daemon/routes.rs` | `/api/plugins/:id/*path` 反代路由 |
| `engine/src/daemon/mod.rs` | DaemonState 加 `plugin_children: Arc<Mutex<HashMap<PluginId, Child>>>` |
| `engine/src/main.rs` | daemon 启动 spawn、shutdown kill |
| `docs/TRUSTED-PLUGINS.md`（新增）或 `docs/WIDGET-DEVELOPMENT.md`（追加） | 文档 |

## 7. 混合架构：Widget + Trusted Plugin

一个第三方插件要做成混合架构，实际上是**两个独立的包**：

| 包 | 安装位置 | 信任模型 | 通信 |
|---|---|---|---|
| Widget 包 | digest-pinned 静态包（`data/extensions/<digest>/`） | manifest + capability grant | iframe 沙箱 |
| Trusted Plugin 包 | `data/plugins/<id>/` + manifest | 用户显式装 | 独立子进程 + HTTP |

两个包**独立安装、独立版本、独立 digest**。engine 不做绑定。

### 7.1 Widget manifest 软依赖字段

`WidgetManifest` 加**可选**字段：

```json
{
  "id": "com.example.tts-ui",
  "host_api": ">=0.1",
  "trusted_plugins": [
    {"id": "com.example.tts", "min_host_api": ">=0.1"}
  ]
}
```

- 软依赖：声明了但 trusted plugin 不存在 → widget 仍可加载，自己处理降级
- `min_host_api` 复用 widget-engine 的 semver 模型
- engine 不强制匹配，只负责回答 webui 的查询，由 webui 决定怎么提示

### 7.2 通信路径

```
Widget iframe
    │
    │ fetch('/api/plugins/com.example.tts/speak', {body: ...})
    ▼
Daemon (axum reverse proxy)
    │
    │ HTTP → 127.0.0.1:<port>
    ▼
Trusted Plugin subprocess
```

**关键**：widget 调 trusted plugin 走**普通 HTTP fetch**，**不走 widget_intent**。

- widget_intent = widget → engine（走 capability grant）
- widget → trusted plugin = 普通 HTTP（trusted plugin 自己校验请求）

engine 不替 trusted plugin 做安全策略。trusted plugin 应自己校验 Origin、要求 auth token、校验 body schema。

### 7.3 engine 要做的（最小）

1. widget manifest schema 加可选字段 `trusted_plugins`：纯 schema 扩展
2. 新增 `GET /v1/plugins`：列出已安装 trusted plugin（id + version + host_api + 启停状态）
3. 反代路由 `/api/plugins/:id/*path`：第 6 节已有，复用

### 7.4 webui 要做的（最小）

1. 加载 widget 时查 `GET /v1/plugins`，对照 manifest 的 `trusted_plugins`
2. 缺失 → 降级提示
3. iframe sandbox 允许 same-origin（或通过 postMessage 让父窗口 fetch）

### 7.5 为什么不做更紧的耦合

- 独立升级：widget 升 UI 版本不影响 trusted plugin（widget 有 digest 锁；trusted plugin 升级靠替换目录 + 重启 daemon，**无 digest 锁**，不如 widget 严格——这是显式信任模型的代价）
- 多对多：一个 trusted plugin 可被多个 widget 调，一个 widget 可依赖多个 trusted plugin（HTTP 反代天然支持）
- 降级可行：trusted plugin 没装时 widget 仍可独立工作
- 复用已有模型：semver + digest + manifest，不新增机制
- plugin 间通信：允许通过反代路由互调，engine 不保证语义（见 §6.6）

## 8. 不做的事

- **不做 Dev Mode hook**（本期）：Rust 不能 monkey-patch，阶段替换 hook 等真实需求 + engine 内部稳定后再做
- **不做紧耦合的"绑定插件"机制**：混合架构靠 HTTP 通道，不靠 engine 强制绑定
- **不做 trusted plugin 自动重启**：崩溃 = 用户重启 daemon
- **不做 plugin 间通信保证**：plugin 间可通过反代路由互调（§6.6），但 engine 不保证调用顺序、依赖图、健康检查
- **不做 plugin 调 engine 内部 API**：避免循环依赖
- **不做 trusted plugin digest 锁**：显式信任模型的代价，升级靠用户自己管理（区别于 widget 的 digest-pinned）
- **不做 C-P4.2 直到 C-P4.1 跑通有真实反馈**：YAGNI

## 9. 推进顺序

1. **C-P4.1**（1 个文件改动）：widget_intent 加 read dispatch，让 widget 真的能读 memory/state/worldbook
2. **Trusted Plugin MVP**（6 个文件）：spawn + kill + 反代 + manifest schema + `GET /v1/plugins`
3. **Widget manifest 加 `trusted_plugins` 软依赖字段**：让混合架构可声明
4. C-P4.2、plugin 重启策略等真实需求再决定

## 10. 引用

- AGENTS.md「第三方经验吸收与独立实现（2026-07-11）」
- AGENTS.md「周期性代际重构特例」第 7 条："不得为了'破坏式'而无证据重写"——本设计的反面适用：不得为了"扩展性最大化"而无证据增加接口
- `docs/WIDGET-DEVELOPMENT.md` §5（KNOWN_CAPABILITIES 文档锁）
- `engine/src/extensions/api.rs:516`（C-P4 接入点）
- `engine/src/plugin_tool.rs:723-820`（subprocess 基础设施可复用段）
