# Trusted Plugins（可信插件）

> #498 §6「Trusted Plugin 计划」的实现文档。Trusted plugin 是**用户显式安装**的
> 长跑子进程（HTTP 服务），与 widget 的 digest-pinned 静态包是两条独立的扩展
> 轨道：widget = 沙箱 iframe + capability grant（零信任）；trusted plugin =
> 本机子进程 + 目录限定（显式信任）。混合架构见 `docs/plans/2026-08-07-widget-trusted-plugin-design.md` §7。

## 1. 安装布局

```
data/plugins/
├── manifests/
│   └── com.example.tts.json   # 声明文件（唯一注册面）
└── com.example.tts/           # 插件目录（command 必须在此目录下）
    └── tts-server             # 可执行文件
```

- 安装 = 放置 manifest + 插件目录；卸载 = 删除 manifest + 重启 daemon。
- **无 digest 锁**：trusted plugin 升级靠用户自己管理（显式信任模型的代价，
  区别于 widget 的 digest-pinned）。
- daemon 启动时扫描 `data/plugins/manifests/*.json`；单个文件坏 JSON /
  校验失败 / id 重复仅 warn 跳过，不阻塞其余插件。

## 2. Manifest 字段

```json
{
  "id": "com.example.tts",
  "version": "1.0.0",
  "command": "./tts-server",
  "args": ["--port", "${AIRP_PLUGIN_PORT}"],
  "port": 8765,
  "host_api": "1"
}
```

| 字段 | 必填 | 说明 |
|---|---|---|
| `id` | ✅ | `ns.name` 形态；同时是 `data/plugins/<id>/` 目录名（拒绝路径分隔符，防 `../` 越界） |
| `version` | ✅ | 仅展示用（`GET /v1/plugins` 下发） |
| `command` | ✅ | 相对 `data/plugins/<id>/` 的可执行路径；canonical 校验越出插件目录一律拒绝 |
| `args` | ❌ | 启动参数；`${AIRP_PLUGIN_PORT}` 占位符替换为 `port` |
| `port` | ✅ | 插件**自己监听**的 loopback 端口，engine 不分配 |
| `host_api` | ✅ | 纯 semver，major 钉死（与 widget 同一校验；engine 当前 major = 1） |

## 3. 生命周期

- **启动**：daemon 启动时逐个 spawn。env 注入 `AIRP_PLUGIN_PORT` /
  `AIRP_DATA_ROOT` / `AIRP_PLUGIN_ID`；**不 env_clear**（trusted plugin
  允许读自己的环境，区别于 plugin_tool 的零信任脚本）。stdout/stderr
  继承 daemon 终端（日志直接可见）。**子进程 cwd = 插件目录**，args 里的
  相对路径（如 `server.js`）以插件目录为基准。
- **端口冲突**：两个插件声明同一 `port` → 第二个 spawn 时子进程启动失败 →
  daemon 记 warn 日志。engine 不做事前端口注册表（冲突在 spawn 时暴露）。
- **崩溃**：子进程退出 → daemon 记录日志，**不自动重启**。
- **退出**：Ctrl+C / SIGTERM → daemon 先终止全部插件（Unix：SIGTERM →
  等 5s → SIGKILL；Windows 直接强杀）再退出。

## 4. HTTP 面

### 4.1 反代：`GET/POST /api/plugins/:id/*path`

转发到 `127.0.0.1:<port>/*path`（method / path / query / body / Content-Type
透传；超时 30s）。

- **挂在鉴权层外**：widget iframe 沙箱没有 daemon token；且不限制 caller——
  loopback 上任何进程都能调（trusted plugin 之间也能互调）。
- **不透传** daemon 的 `Authorization` / `Cookie` / `Origin` 头（daemon 凭据
  不泄漏给插件）。
- 错误映射：未知 id → `404 plugin_not_found`；请求体读取失败 → `400
  plugin_bad_request`；插件未起/已崩/超时 → `502 plugin_unreachable`。
- 请求体受 axum 默认 2MB 上限约束。

### 4.2 列表：`GET /v1/plugins`（鉴权层内，webui 查询用）

返回已安装插件：

```json
{ "plugins": [{ "id": "com.example.tts", "version": "1.0.0", "host_api": "1", "status": "running" }] }
```

- `status` 仅反映 spawn 结果（`running` / `stopped`），**不探活**。
- webui 加载 widget 时对照 widget manifest 的 `trusted_plugins` 软依赖做降级提示。

## 5. 安全边界

- **目录限定**：`command` 必须解析到 `data/plugins/<id>/` 内（canonical
  校验 + 必须是文件）。
- **loopback 拓扑**：插件端口与 daemon 同机；engine 只转发到
  `127.0.0.1:<port>`。
- **信任模型**：engine **不**替 trusted plugin 做安全策略——插件应自行校验
  请求来源（Origin）、要求 auth token、校验 body schema。
- **不做**（见 #498 §8）：自动重启、plugin 间通信保证（顺序/依赖图/健康检查）、
  digest 锁、插件调 engine 内部 API。

## 6. 与 widget 的关系

- widget → trusted plugin 走**普通 HTTP fetch**（`/api/plugins/...`），
  **不走 widget_intent**（widget_intent = widget → engine 的 capability 通道）。
- 混合架构包是两个独立安装体，engine 不做绑定。
