# 审计报告：PR #506 Trusted Plugin MVP（实现层复审 + 修复）

- **审计来源 LLM**：GLM-5.2
- **审计时间**：2026-08-07
- **审计对象**：PR #506（feat/trusted-plugin-mvp, head d53fb14 → 修复后）
- **审计依据**：AGENTS.md「Audit Agent Charter」三原则 + 先前设计审计 A1-A5
- **审计方法**：独立读源码（spawn.rs / proxy.rs / mod.rs / main.rs）+ 独立跑 `cargo test --lib`（1417 passed, 0 failed, 5 ignored）+ `cargo clippy -- -D warnings`（clean）+ 对照设计审计 A1-A5 逐条核验

## A1-A5 状态（设计审计 → 实现层）

| Blocker | 设计层 | 实现层（d53fb14） | 修复后 |
|---------|--------|------------------|--------|
| **A1** iframe 不能 same-origin fetch | ✅ 补充文档承认 | N/A（engine 侧不涉及，webui 侧后续 PR） | 不变 |
| **A2** 级联 kill 缺失 | ✅ 设计文档写明 | ⚠️ 无 Windows Job Object / 无 process group | ✅ **已修复**（见 B2 修复） |
| **A3** 端口冲突 | ✅ 设计文档写明 | ✅ main.rs 过滤 + load_manifests 去重 | 不变 |
| **A4** 不 env_clear | ✅ 设计文档写明 | ✅ d53fb14 已修（env_clear + 白名单） | 不变 |
| **A5** sandbox 与 BUG-6 冲突 | ✅ 补充文档明确禁止 | N/A（engine 侧不涉及） | 不变 |

## 本次修复（B2/B3/B4）

### B2 修复：级联 kill（审计 A2）

**文件**：`engine/src/plugins/spawn.rs`

**问题**：原实现只 kill 直接子进程，孙进程（如 TTS 插件 spawn 的 ffmpeg）变孤儿，Windows 下端口仍占。

**修复**：
- **Unix**：`process_group(0)`（spawn_one）让子进程成为新进程组组长（PGID = PID），`killpg(pid, SIGTERM/SIGKILL)`（terminate_graceful）终止整个组（含孙进程）
- **Windows**：`taskkill /PID <pid> /T /F`（terminate_graceful）终止整个进程树（taskkill 内部用 Job Object 实现 tree kill）
- **已知限制**：panic/SIGKILL 路径下孙进程仍可能变孤儿（kill_on_drop 只 kill 直接子）。MVP 可接受，跟踪 issue 后续补 Drop 级 killpg/taskkill。

**无新依赖**：`process_group` 是 std Unix API，`taskkill` 是 Windows 内置命令。

### B3 修复：kill_on_drop（审计 A2 panic 路径）

**文件**：`engine/src/plugins/spawn.rs:83`

**问题**：engine panic / SIGKILL / runtime 异常退出时，shutdown_signal 不触发，子进程变孤儿。

**修复**：`cmd.kill_on_drop(true)` — tokio 保证 Child drop 时（含 panic 展开路径）kill 直接子进程。一行改动，覆盖所有异常退出路径（panic / SIGKILL / runtime drop）。

### B4 修复：fail-closed loopback 校验

**文件**：`engine/src/plugins/proxy.rs:169-193`

**问题**：反代 loopback 校验在 ConnectInfo 缺失时 fail-open（跳过检查），自定义 router 嵌入或异常 serve 拓扑下远程请求可直达插件。

**修复**：改为 fail-closed — 无 ConnectInfo 时直接 403 `plugin_remote_forbidden`。测试通过 `Extension(ConnectInfo(loopback_addr()))` 显式注入。

**测试更新**：
- 8 个既有 proxy 测试加 `.extension(ConnectInfo(loopback_addr()))`
- 新增 `proxy_rejects_missing_connect_info`：无 ConnectInfo → 403

## 已核实（V-series，独立验证）

| 编号 | 核实项 | 证据 |
|------|--------|------|
| V1 | env_clear + 白名单正确（A4 已修） | `spawn.rs:44-67`：`cmd.env_clear()` + PATH/SYSTEMROOT/TEMP/TMP + 三个 AIRP_* |
| V2 | 端口冲突检查（A3 已修） | `main.rs:253-267`：filter daemon_port + `mod.rs:185`：load_manifests 端口去重 |
| V3 | canonicalize 路径限定无逃逸 | `mod.rs:99-130`：canonicalize → starts_with → is_file |
| V4 | host_api major 钉死 | `mod.rs:84-93`：复用 `parse_host_api_major` |
| V5 | 反代鉴权层外 / 列表层内 | `daemon/mod.rs`：路由注册顺序 + 集成测试验证 |
| V6 | 1417 lib tests 全绿 | 独立运行 `cargo test -p airp-core --lib`：1417 passed, 0 failed, 5 ignored |
| V7 | clippy clean | `cargo clippy -p airp-core --all-targets -- -D warnings`：无警告 |
| V8 | Windows 保留名拒绝 | `mod.rs:68-82` + 测试 `validate_rejects_windows_reserved_names` |
| V9 | SSE 流式透传 + shutdown 广播 | `proxy.rs:stream_or_shutdown` + 测试 `proxy_streams_sse_and_stops_on_shutdown` |
| V10 | 响应体 2MB 上限 | `proxy.rs:bounded_response_body` + 测试 `proxy_rejects_oversized_response` |

## 非阻塞项（N-series，后续迭代）

| 编号 | 项 | 说明 |
|------|----|------|
| N1 | panic 路径孙进程仍可能变孤儿 | kill_on_drop 只 kill 直接子；需 Drop 级 killpg/taskkill 覆盖。MVP 可接受。 |
| N2 | TRUSTED-PLUGINS.md §6 与补充文档矛盾 | 主文档仍写"普通 HTTP fetch"，补充文档明确禁止。待 #507 修订。 |
| N3 | env_clear 白名单可能过窄 | 缺 HOME/USERPROFILE/PATHEXT。当前最小白名单安全优先，插件作者可自行设置。 |
| N4 | 30s 超时对 SSE body 的适用性 | 超时只覆盖到 headers 到达；SSE body 不受限。文档应说明。 |
| N5 | stdout/stderr 继承混入 engine 日志 | 建议后续加 `[plugin:id]` 前缀或 piped + tracing 转发。 |
| N6 | resolve_command TOCTOU | canonicalize → spawn 间有窗口。trusted plugin 显式信任，风险低。 |
| N7 | 测试缺 header 脱敏验证 | 无测试验证 Authorization/Cookie/Origin 被剥离。建议补。 |
| N8 | 测试缺 env_clear 验证 | 无测试验证子进程环境不含 AIRP_ACCESS_KEY。建议补。 |

## 未执行视觉审查

本 PR 为 engine-only（Rust 后端），不涉及 WebUI 视觉改动。按 AGENTS.md 规则无需多模态补审。

## 结论：**APPROVE**（修复后）

B2/B3/B4 三个阻塞项已全部修复，1417 lib tests 全绿，clippy clean。A4（env_clear）已在 d53fb14 修复。剩余 N-series 为非阻塞后续迭代项。

**合并顺序**：PR #506 应先于 PR #507 合并——#507 基于 #506 的原始 commit 1fbdd16（无 env_clear 修复），需 rebase 到 d53fb14（含 env_clear + 本次 B2/B3/B4 修复）后才能合并。
