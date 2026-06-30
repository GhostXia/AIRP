# AIRP-Gateway

**AIRP-Gateway** = AI Roleplay Gateway。通用、高性能的**纯协议桥**：把前端的 HTTP/SSE 请求，鉴权 + 限流后翻译成 MCP（JSON-RPC）调用，转发给上游 MCP 服务。

> ## 🧭 核心理念：乐高积木，不是套件
>
> **本项目对接「任意」MCP 服务、服务「任意」前端，可完全独立使用。跑起来不需要任何其他 AIRP 项目。**
>
> AIRP-Gateway 诞生于 AIRP 生态，但**不属于、不依赖、不绑定**其中任何单一项目——包括 AIRP-Core / AIRP-MCP-Server / AIRP-State-Protocol。它们是互相独立的乐高块：你完全可以只用「本项目 + 你自己的 MCP server + 你自己的前端」，一个 AIRP 项目都不碰。与任何 AIRP 项目的「联动」都是**可选适配**，绝非前提。
>
> 不信？见下方 [30 秒跑通（零 AIRP 依赖）](#30-秒跑通零-airp-依赖)——用本仓库自带的 mock server 即可，CI 全程不依赖任何外部项目。

[![License](https://img.shields.io/badge/license-Apache--2.0%20OR%20MIT-blue)](#许可)

```text
任意前端 (HTTP/SSE)  ──►  AIRP-Gateway  ──►  任意 MCP 服务  ──►  Agent / 后端
   (你的 UI)              (本项目·纯协议桥)   (你的 / 第三方 / AIRP 皆可)
```

> 相关 AIRP 生态（**全部可选**，仅在你需要时拼上）：[AIRP-Core](https://github.com/GhostXia/AIRP-Core) · [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server)（一种上游数据底座）· [AIRP-State-Protocol](https://github.com/GhostXia/AIRP-State-Protocol)（一种前端契约）。详见 [生态联动（可选）](#生态联动可选)。

---

## 30 秒跑通（零 AIRP 依赖）

证明本项目是独立乐高块：下面**不涉及任何其他 AIRP 项目**，只用本仓库自带的最小 mock MCP server（`examples/mock_mcp_stdio.rs`，仅演示用）。

```powershell
# 1. 编译自带 mock（真实场景换成你自己的任意 MCP server）
cargo build --example mock_mcp_stdio
```

`config.toml`（上游 `command` 指向任意 MCP server，这里用刚编译的 mock）：

```toml
bind = "127.0.0.1:8080"

[[upstreams]]
name = "demo"
transport = "stdio"
command = "target/debug/examples/mock_mcp_stdio"   # 换成你的 MCP server 即可
args = []

[[routes]]
path = "/v1/echo"
method = "POST"
upstream = "demo"
target = { kind = "tool", name = "echo" }
```

```powershell
# 2. 起 Gateway（见下「快速开始」嵌入代码），然后：
curl -X POST localhost:8080/v1/echo -d '{\"hi\":1}'
# 命中 mock 的 echo 工具，原样回显。
```

完整链路（前端 HTTP → bridge → 真子进程 MCP → 结果）由 CI e2e 真实验证，全程零外部项目依赖。换上你自己的 MCP server（stdio 或 HTTP）即可投入真实使用。

---

## 设计戒律（Iron Laws）

桥之所以通用，因为它**不懂业务**。任何改动不得破坏：

1. **纯协议桥** — 不做推理、不拼 prompt、不懂「角色 / 预设 / 世界书」。领域语义只以字符串出现在配置里。
2. **库优先** — 核心是 crate，无独立 exe。宿主自管进程启动，调 `Gateway::build(cfg).run()`。
3. **传输无关** — 上游 stdio / HTTP 对 bridge 透明，只经 `McpTransport` trait。新传输零成本接入。
4. **轻依赖、可移植** — 手写 JSON-RPC 契约，分层薄。未来换语言可照搬契约。

业务逻辑归上游 MCP 服务（任何讲 MCP 的 server，AIRP-MCP-Server 只是其中一例）。Gateway 只负责鉴权、限流、转发。

---

## 快速开始（作为库嵌入）

`Cargo.toml`：

```toml
[dependencies]
airp-gateway = { git = "https://github.com/GhostXia/AIRP-Gateway" }
```

宿主代码：

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    airp_gateway::telemetry::init();

    // 分层加载：default -> TOML -> env(AIRP_GW_*)
    let cfg = airp_gateway::GatewayConfig::load(Some("config.toml".as_ref()))?;

    // 连接上游、装配 bridge、起服务
    airp_gateway::Gateway::build(cfg).await?.run().await?;
    Ok(())
}
```

`config.toml`：

```toml
bind = "127.0.0.1:8080"
# access_key = "前端访问令牌（不设则开放）"

[rate_limit]
enabled = true
per_second = 10
burst = 20

[cors]
allow_any = true

# 上游 MCP 服务（stdio）：换成你的任意 MCP server
[[upstreams]]
name = "backend"
transport = "stdio"
command = "your-mcp-server"
args = []

# 上游 MCP 服务（HTTP）：同样支持
# [[upstreams]]
# name = "backend-http"
# transport = "http"
# url = "http://127.0.0.1:3000/mcp/v1"
# auth_token = "可选 bearer"

# 声明式路由：前端路径 -> 上游 MCP 工具/资源
[[routes]]
path = "/v1/do-thing"
method = "POST"
upstream = "backend"
target = { kind = "tool", name = "your_tool_name" }
```

---

## 配置

| 键 | 说明 |
|----|------|
| `bind` | 前端面服务绑定地址，默认 `127.0.0.1:8080` |
| `access_key` | 前端 bearer 令牌（常数时间校验）；不设 = 开放 |
| `rate_limit` | per-IP 令牌桶：`enabled` / `per_second` / `burst` |
| `cors` | `allow_any`，或 `allow_origins = [...]` |
| `upstreams[]` | 上游 MCP 服务：`name` + `transport`(`stdio`\|`http`) |
| `routes[]` | `path` + `method` + `upstream` + `target`(`tool`\|`resource`) |
| `allowed_commands` | stdio 命令白名单（**空=放行**；非空时 stdio `command` 须按全串/文件名匹配，否则启动失败） |
| `max_request_bytes` | 前端请求体大小上限（字节），默认 1 MiB |
| `max_response_bytes` | 上游响应体大小上限（字节），默认 10 MiB |
| `upstream_timeout_secs` | 上游请求超时（秒），默认 30；设 0 禁用 |
| `block_private_upstream_urls` | SSRF 防护：默认 true，拒绝 HTTP 上游指向私网/link-local/保留段。**loopback(127.x/::1/localhost)默认放行**(本地 MCP 常见场景) |
| `allow_arbitrary_args` | 当 `allowed_commands` 非空时，允许 stdio 上游带 `args`，默认 false |

环境变量覆盖（最高优先）：`AIRP_GW_BIND`、`AIRP_GW_ACCESS_KEY`、`AIRP_GW_MAX_REQUEST_BYTES`、`AIRP_GW_MAX_RESPONSE_BYTES`、`AIRP_GW_UPSTREAM_TIMEOUT_SECS`。

中间件顺序：**CORS → 限流 → 鉴权 → 分发**。

### 安全
- **默认仅本机**：`bind` 默认 `127.0.0.1`。改 `0.0.0.0` 即显式对外暴露。
- **暴露告警**：绑定非 loopback 且未设 `access_key` 时，启动打印响亮告警（无鉴权暴露到网络）。
- **stdio 命令白名单**：`allowed_commands` 非空时，拒绝启动不在白名单内的上游命令（防 config 拉起任意程序）。默认放行以保持通用。
- **SSRF**：默认拦私网/link-local/保留段(含云元数据 169.254.169.254)，但**放行 loopback**——本地 MCP 上游开箱即用，不必为常见场景翻 flag。
- 威胁模型：config 为 host 静态加载，上游/agent 无运行时改 config 的入口。详见 [`docs/DESIGN.md`](docs/DESIGN.md) ADR-008。

---

## 架构

| 模块 | 职责 |
|------|------|
| `config` | 分层配置 default → TOML → env |
| `server` | axum 路由、常数时间 bearer 鉴权、CORS、governor 限流 |
| `bridge` | 请求 → MCP 操作 → 响应（领域无关） |
| `mcp::types` | JSON-RPC 2.0 / MCP 线类型（手写） |
| `mcp::client` | 单上游：`initialize` 握手、`call_tool`、`read_resource` |
| `mcp::pool` | name → client 注册表 |
| `mcp::transport` | `McpTransport` trait + `stdio` / `http` 两实现 |

数据流：

```text
前端请求 → CORS → 限流 → 鉴权 → dispatch 兜底
  → 匹配 RouteRule → 取上游 client
  → McpClient（首次惰性 initialize）→ McpTransport
  → 上游 MCP 服务 → result → 回前端
```

可定制点一站式清单（配置 / feature / 代码扩展点）见 [`docs/CUSTOMIZATION.md`](docs/CUSTOMIZATION.md)。
完整设计与开发追踪见 [`docs/DESIGN.md`](docs/DESIGN.md)；前瞻计划书（当前状态 + 路线图 + 版本规划）见 [`docs/ROADMAP.md`](docs/ROADMAP.md)。

---

## 传输支持（上游）

Gateway 作为 MCP 客户端，对接**任意** MCP server：

| 传输 | 状态 |
|------|------|
| **stdio** | ✅ 可用。拉起子进程，行分隔 JSON-RPC，完成 `initialize` 握手。CI 用自带 mock 跑真实跨进程 e2e 验证（零外部依赖） |
| **HTTP (streamable)** | ✅ 可用。`Mcp-Session-Id` 捕获/回传、`MCP-Protocol-Version`（协商值）头、`application/json` 与 `text/event-stream`(SSE) 响应。CI 用 in-process mock（真实 TCP，强制校验头）e2e 验证 |

新增传输 = `impl McpTransport`，bridge 无感。

---

## 生态联动（可选）

> 重申：以下全部是**可选适配**。不接入任何一个，Gateway 也能独立服务任意前端与任意 MCP 服务。

### AIRP-State-Protocol —— 前端契约
[AIRP-State-Protocol](https://github.com/GhostXia/AIRP-State-Protocol) 定义了「Agent 产出声明式数据、UI 只负责渲染」的前端↔网关契约：`Envelope` / `Blueprint` / `State + Patch`(RFC 6902) / 可扩展 Widget 注册表，以及进程级的 **`AgentBus` trait**——并显式把 **AIRP-Gateway 列为该 trait 的实现方之一**。它本身**传输无关**（Tauri IPC / HTTP / SSE / WebSocket 皆可）。

完整链路设想：

```text
AIRP-State-Protocol UI  ──AgentBus(SSE/WS/HTTP)──►  AIRP-Gateway  ──MCP──►  AIRP-MCP-Server  ──►  Agent
        (前端·声明式渲染)                              (本项目·桥)             (数据底座)
```

联动方式（已实现，**feature 默认关**）：`agentbus` feature 提供一个可选 `AgentBus` 适配层(`/airp/dispatch` + `/airp/stream` SSE),把前端 State-Protocol 消息映射到既有 `RouteRule → MCP` 分发,结果按 `Blueprint`/`Patch` 回传。核心桥保持纯净——适配层不进 `bridge`,**纯桥使用者默认不编译这部分项目专属代码**。

启用:
```toml
airp-gateway = { git = "...", features = ["agentbus"] }
```
示例见 `examples/agentbus_sse.rs`，细节见 [`docs/AGENTBUS-ADAPTER.md`](docs/AGENTBUS-ADAPTER.md)。

> 它是「前端那一侧」的契约，与上游 MCP 互不冲突：State-Protocol 管 `前端↔Gateway`，MCP 管 `Gateway↔后端`。

### AIRP-MCP-Server —— 一种（可选）上游
若你正好想用 AIRP 的数据底座 [AIRP-MCP-Server](https://github.com/GhostXia/AIRP-MCP-Server)，把它配成一个普通上游即可——对 Gateway 而言它和任何别的 MCP server 没有区别：

```toml
[[upstreams]]
name = "airp"
transport = "stdio"
command = "airp-mcp"
args = ["mcp", "--data-dir", "./data"]

[[routes]]
path = "/v1/characters"
method = "GET"
upstream = "airp"
target = { kind = "tool", name = "list_characters" }
```

### 接入任意第三方
不必是 AIRP 项目。任何能发 HTTP/SSE 的前端 + 任何讲 MCP 的服务，配一份 `config.toml` 即可对接。

**核心前端无关**——内置 axum HTTP 只是「其中一个前端」。想要别的协议（WebSocket / gRPC / State-Protocol 的 AgentBus / 自定义），直接复用共享核心、自建前端，不必碰本项目核心：

```rust
// 拿到共享状态：内含 bridge（请求→MCP 分发）、upstream pool、config
let state = airp_gateway::GatewayState::build(cfg).await?;

// 你的前端（任意协议）收到请求后，走同一个 bridge 分发到 MCP
let outcome = state.bridge.dispatch(rule, body_json).await?;
```

已 re-export 的构件：`GatewayState` / `Bridge` / `DispatchOutcome` / `McpClient` / `UpstreamPool` / `McpTransport` / `RouteRule` 等。
扩展点：新上游传输 = `impl McpTransport`；新前端协议 = 复用 `bridge.dispatch()`。

---

## 开发

构建工具链固定为 GNU（规避 MSVC `link.exe` 依赖，与 AIRP-Core 一致）：

```powershell
# 本机 D: 盘构建脚本执行受系统策略限制，需将 target 目录重定向到 C:
$env:CARGO_TARGET_DIR = "C:\Users\<you>\airp-gw-target"
cargo +stable-x86_64-pc-windows-gnu check --manifest-path .\Cargo.toml
```

`.cargo/config.toml` 已锁定 `x86_64-pc-windows-gnu` target。

---

## 许可

双许可 **Apache-2.0 OR MIT**，任选其一（"at your option"）。
向本项目贡献即视为以相同双许可授权。
