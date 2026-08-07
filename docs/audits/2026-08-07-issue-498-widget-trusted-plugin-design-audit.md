# 审计报告：issue #498 Widget 与 Trusted Plugin 设计文档

- **审计来源 LLM**：GLM-5.2
- **审计时间**：2026-08-07
- **审计对象**：[docs/plans/2026-08-07-widget-trusted-plugin-design.md](../docs/plans/2026-08-07-widget-trusted-plugin-design.md) / issue #498
- **审计依据**：AGENTS.md「Audit Agent Charter」三原则（独立审计、可提己见、可质疑历史并查证）
- **审计方法**：对照代码事实（`ui/src-tauri/`、`webui/assets/widgets/`、`engine/src/extensions/`、`engine/src/daemon/`）独立核验设计文档每一条断言

## 审计事实（代码现状 vs issue #498 设计）

| 维度 | issue #498 假设 | 代码事实 |
|---|---|---|
| Tauri 架构 | "桌面 UI"（未细化） | `ui/src-tauri/` 是壳，spawn `airp-core daemon` sidecar；webview 导航到 engine 同源承载的 `webui/` |
| webui 代码位置 | 未明确 | `webui/assets/widgets/` 是活的；`ui/src/` 已归档（main.rs 注释明写） |
| Tauri IPC | 未提及 | **无 IPC**——main.rs 注释："webview 运行在远程 URL，无 Tauri IPC 通道" |
| Widget 沙箱 | "前端 iframe 沙箱" | sandboxed iframe 强制 `sandbox="allow-scripts"` 无 `allow-same-origin`，opaque origin |
| BUG-6 硬门禁 | 未提及 | esm entry 缺 `sandbox:true` 一律拒载（widget-host.js:48） |
| Capability 权威 | "engine 侧 grant" | consent.js 已实现 C-P3：从 `GET /v1/extensions/grants` 拉权威缓存，降级到 localStorage |

## 阻塞项（issue #498 与代码冲突，必须修）

### A1（严重）§7.2 通信路径是死的

**问题**：issue #498 画的通信路径是 "Widget iframe → fetch('/api/plugins/...') → Daemon → Trusted Plugin"。

**事实**：sandboxed iframe 是 opaque origin（`sandbox="allow-scripts"` 无 `allow-same-origin`），**任何 same-origin fetch 都失败**。widget 连 `fetch('/api/plugins/...')` 都发不出。这条线在设计上是死的。

**正解**：widget 通过 `emit("fetch", {url, body})` 走 postMessage → 宿主 `widget-host.js` 代理 fetch → 回 `state` 推回 iframe。复用已有 `webui/assets/widgets/sandbox-bridge.js` 的 postMessage 协议，加一种 message kind。

### A2（严重）§6.3 级联 kill 缺失

**问题**：事实进程树：
```
airp-ui (Tauri 壳)
  └─ airp-core daemon (sidecar, CommandChild 持有)
       └─ trusted plugin 子进程（如果按 #498 设计）
```

main.rs:134 `RunEvent::Exit` 只 kill 直接子（`child.kill()`）。engine sidecar 被 kill 时，**trusted plugin 孙进程会变孤儿**，Windows 下端口仍占，下次启动冲突。

**issue #498 §6.3 写**："daemon 退出 → kill 所有子进程"——**没说谁来保证级联**。

**正解**：engine sidecar 必须自己 trap SIGTERM/被 kill 时主动 kill 自己的子进程。Rust 的 `tokio::process::Child` 默认不会在父进程退出时 kill 子进程（与 Unix `prctl(PR_SET_PDEATHSIG)` 不同）。设计必须写明 engine 侧安装 signal handler + child 进程组 kill（Windows 用 Job Object 保证级联）。

### A3（严重）§6.2 端口冲突自己撞自己

**问题**：`DEFAULT_ENGINE_PORT = 8765`（main.rs:36）。issue #498 manifest 示例 `port: 8765`——**和 engine 自己监听的端口冲突**。

**正解**：设计必须保留区间（如 9000-9999 给 trusted plugin），且 manifest 校验时拒绝保留端口（engine 端口 + 任何已声明端口）。

### A4（严重）§6.3 "不 env_clear" 是提权

**问题**：engine sidecar 进程持有 `AIRP_ACCESS_KEY`（进程级随机 bearer，main.rs:87）。如果 trusted plugin 子进程**不 env_clear**，它会继承 `AIRP_ACCESS_KEY`——直接拿到 engine 的进程级 bearer，能调任何 `/v1/*` 端点。

**issue #498 §6.3 写**："不 env_clear——trusted plugin 是用户显式装的"——这个理由不成立。trusted plugin 显式信任 ≠ 给它 engine 的 access key。

**正解**：必须 env_clear，然后只注入设计里列的三个变量（`AIRP_PLUGIN_PORT` / `AIRP_DATA_ROOT` / `AIRP_PLUGIN_ID`）。

### A5（严重）§7.4 与 BUG-6 安全门禁冲突

**问题**：issue #498 §7.4 写"iframe sandbox 允许 same-origin"。

**事实**：widget-host.js:48 `sandboxEnforced` + sandbox-bridge.js:148 强制 `sandbox="allow-scripts"` 无 `allow-same-origin`。**任何放宽都是回归 BUG-6**。

**正解**：issue #498 这条必须删，且明确："widget → trusted plugin 只能走 postMessage 代理 fetch，不能直接 fetch"。

## 设计漏洞（非阻塞，但应记录）

### B1 §6.6 plugin 间通信的隐私问题

plugin A 通过反代路由能读 plugin B 的所有请求/响应（loopback 无 caller 隔离）。如果 B 处理用户语音、A 是日志插件，A 能读 B 的 transcript。issue #498 没标记。

### B2 §6.4 30s 超时对长任务不够

TTS 合成长文本、图像生成都会超 30s。设计没区分短任务路由和长任务路由，也没说是否支持 SSE 流式。

### B3 §7 没区分 builtin 和 esm

"Widget 层 = 前端 iframe 沙箱"只对 esm 路径成立。builtin module widget 是进程内的，不走 iframe。混合架构声明 `trusted_plugins` 软依赖时，builtin 和 esm 都能用，但 trusted plugin 调用通道不同（builtin 直接 fetch，esm 走 postMessage 代理）。

### B4 §6.6 "plugin 间通信允许"与 §8 "不做 plugin 间通信保证"仍然表述模糊

"允许 + 不保证"这种表述在审计报告里会被 flagged——要么禁止（caller 限制），要么明确允许并写明隐私边界。

## 与现状吻合的部分（这些是对的，保留）

- C1: capability 模型复用 KNOWN_CAPABILITIES ✓
- C2: digest-pinned 静态包 ✓
- C3: host_api semver ✓
- C4: C-P4.1 接入点 `api.rs:516` ✓
- C5: Tauri 壳 sidecar 模型 ✓（issue #498 没写但与设计不冲突）

## 修正方向（不动代码，只改设计文档）

issue #498 必须 v2 修订，至少修 A1-A5：

1. **A1 修**：§7.2 通信路径改为 postMessage 代理 fetch（复用 sandbox-bridge.js），删 direct fetch
2. **A2 修**：§6.3 加 engine 侧 signal handler + Windows Job Object 保证级联 kill
3. **A3 修**：§6.2 加保留端口区间（engine + 已声明），manifest 校验拒绝冲突
4. **A4 修**：§6.3 改为 env_clear + 白名单注入三个变量，删"不 env_clear"
5. **A5 修**：§7.4 删"iframe sandbox 允许 same-origin"，明确"widget → trusted plugin 只能走 postMessage 代理 fetch"

## 未执行视觉审查

本次审计为纯文本 LLM（GLM-5.2）审计，**未执行视觉审查**。本 issue 不涉及 WebUI 视觉改动（设计文档审计），按 AGENTS.md 规则无需多模态补审。

## 结论

issue #498 设计方向正确（三档模型、两层并列、声明式 widget、trusted plugin 子进程），但有 **5 个阻塞项与代码事实冲突**，直接按当前设计实现会产出不可运行 / 不安全的代码。建议先修 A1-A5 再进入实现阶段。B1-B4 作为后续迭代项跟踪。
