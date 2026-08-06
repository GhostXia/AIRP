# PR #496 独立复审：webui 便携包体内置桌面旗舰 UI（airp-ui.exe）

> 审计日期：2026-08-06
> 审计对象：PR #496 `feat: webui 便携包体内置桌面旗舰 UI（airp-ui.exe，共用目录与数据）`
> 审计分支：`ci/desktop-in-webui-package`（HEAD `4c932ef1bc71873ee2cfae1cbdb5fe154fc41684`）
> 审计基线：`main@1e16033b677f94b6eaf752fca09220cbe661b765`
> 审计依据：AGENTS.md「审计 Agent 守则」三原则——独立审计、可提己见、可质疑历史并查证
> 审计模型：**GLM-5.2**（纯文本 LLM）
> 复审起因：用户指示结合其他审计 bot（CodeRabbit）提出的问题再复审一次
> 视觉审查：**未执行**（本 PR 不改动 WebUI 视觉资产 `webui/**`，仅改桌面壳数据根解析、打包脚本、CI 工作流、README 文档与新增 smoke 脚本；桌面壳承载的 WebUI 视觉面未变。若维护者认为包体内 `airp-ui.exe` 首屏呈现需多模态补审，可由 KIMI K3 或同级多模态 agent 补审）

## 0. 审计纪律执行说明

1. 不附和 CodeRabbit 的结论。对其 2 条 actionable comment 逐条回到 HEAD 源码与实测复核，确认其 P1 已被 commit `4c932ef` 以**不同方案**解决（stale），但其 P2 的"修复"**实测仍不工作**——本审计独立复现并升级为阻塞。
2. 复审覆盖 5 个变更文件 + 2 个未改动关联文件（`lifecycle.rs`、`Start-AIRP.cmd`、`engine/src/data_dir/paths.rs`、`engine/src/daemon/mod.rs`）以验证跨模块一致性。
3. 关键结论（B-01）经 PowerShell 实测复现，非手工推断。

## 1. 审计范围

| 文件 | 变更类型 | 复审要点 |
|---|---|---|
| `ui/src-tauri/src/main.rs` | 改 | `resolve_data_root` 便携模式、`data_root` 入 state、`RunEvent::Exit` 复用、新增测试 |
| `deploy/windows-webui/build.ps1` | 改 | `-IncludeDesktop` 开关：sidecar 源文件 → `cargo build -p airp-ui` → 复制进包体 |
| `deploy/windows-webui/smoke-desktop-ui.ps1` | 新增 | 五段断言：就绪/同源承载/共享数据/优雅退出/锁清理 + 端口预检 + finally 清理 |
| `deploy/windows-webui/README.txt` | 改 | 桌面入口说明 |
| `.github/workflows/webui-windows-build.yml` | 改 | build 加 `-IncludeDesktop`、新增桌面 smoke 步骤、PR 触发路径 `ui/local-webui-browser-smoke.mjs` → `ui/**` |

## 2. CodeRabbit 意见逐条复核

### CR-1（build.ps1 L44-68 / smoke-desktop-ui.ps1 L24-27）：在包体内预建 `data\` —— **已解决（stale），非阻塞**

CodeRabbit 原意见：`portable_data_dir()` 原只认 `data\` 目录存在，`build.ps1 -IncludeDesktop` 不预建 `data\`，全新解压包首次桌面启动会回落 `%APPDATA%`，与共享包内数据目标冲突；建议在 build 时预建 `data\` 并在 smoke 中改为断言包体已含 `data\`。

**HEAD 复核**：commit `4c932ef`（PR 第 3 个 commit）以**不同方案**解决了该问题：

1. `main.rs:240-253` `portable_data_dir_from` 判定改为**包体标记**（`airp-core.exe` + `webui/index.html` 齐备即便携），不再认 `data\` 副作用目录。
2. `main.rs:69` setup 的 `std::fs::create_dir_all(&data_root)` 在首次启动时补齐 `data\`。
3. `main.rs:854-869` 测试 `portable_data_dir_requires_package_markers` 三段断言（无标记/仅 data\/标记齐备）覆盖该逻辑。

效果：全新解压包首次桌面启动即命中便携模式，`data\` 由 setup 补齐，与后续 `Start-AIRP.cmd` 启动共享。CodeRabbit 的具体建议（预建 `data\`）是另一种有效修法，但开发 agent 的方案（改判定标准 + setup 补齐）同样有效且更干净（不在归档里放空目录）。**意见 stale，不阻塞。**

### CR-2（smoke-desktop-ui.ps1 L31-39）：端口预检 `throw` 被 `catch` 吞 —— **修复无效，升级为阻塞 B-01**

CodeRabbit 原意见：`throw` 在 `try` 内，`catch` 吞掉，导致脚本继续对抗已占用的端口，后续报误导性失败。建议改用 `$existingEngine` 标志位 + `try` 外 `throw`。

commit `4c932ef` 声称"P2/P3 顺手修：端口探测改为任何 HTTP 响应即视为占用"。**本审计实测：修复不工作。**

HEAD 代码（`smoke-desktop-ui.ps1:32-42`）：

```powershell
try {
    $null = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
    throw "Port $Port is already serving an engine; clean up the leftover process before desktop smoke."
}
catch {
    if ($_.Exception.Response) {
        throw "Port $Port is already occupied; clean up the leftover process before desktop smoke."
    }
    # 连接失败且无 HTTP 响应 = 端口空闲，继续
}
```

**实测复现**（本审计独立运行 PowerShell 5.1，两个场景）：

| 场景 | IWR 行为 | 脚本 throw | catch 中 `$_.Exception` | `.Response` | 实际分支 | 期望 |
|---|---|---|---|---|---|---|
| 端口空闲（连接拒绝） | 抛 WebException | 不执行 | WebException | null | continue | continue ✓ |
| 端口服务 200 | 成功返回 | 抛 RuntimeException | **RuntimeException** | **null** | **continue** | **throw ✗** |
| 端口服务非 2xx | 抛 HttpResponseException | 不执行 | HttpResponseException | 非 null | throw | throw ✓ |

关键：当端口正在服务 200（**正是残留 engine 的典型情形**），IWR 成功返回无异常，脚本自己的 `throw` 抛出 `System.Management.Automation.RuntimeException`，该异常类型**没有 `.Response` 属性**，`if ($_.Exception.Response)` 为 false，脚本落到"端口空闲，继续"注释——**完全无法检测残留 engine**。

实测输出（场景 2）：
```
=== Simulate: IWR succeeds (200) -> script throw RuntimeException ===
Caught exception type: System.Management.Automation.RuntimeException
Exception.Response is null? True
BRANCH: free (no Response) -> continue  <-- BUG: engine is serving but script continues!
```

**影响**：smoke 无法检测残留 engine，后续"engine sidecar remained alive after UI exit"断言会以误导性信息失败（实际原因是上一轮残留，非本轮退出清理失效），CI 难以定位根因。`finally` 块按锁文件 `engine_pid` 清理，但残留 engine 可能不属于本轮锁，清理失效，残留传到下一轮 smoke，形成 flaky。

**严重度**：BLOCKING——PR 描述明言"本 PR 即触发 webui-windows-build.yml 全链路（含桌面编译与 smoke），作为 CI 级验证"，端口预检是该 CI 的前置防线，防线失效直接 undermines PR 的验证主张。

**修复状态**：**已修复（本审计实测验证）**。开发 agent 采纳 CodeRabbit 原方案——`$portOccupied` 标志位 + `try` 外 `throw`：

```powershell
$portOccupied = $false
try {
    $null = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
    $portOccupied = $true
}
catch {
    if ($_.Exception.Response) {
        $portOccupied = $true
    }
}
if ($portOccupied) {
    throw "Port $Port is already serving an engine; clean up the leftover process before desktop smoke."
}
```

本审计用独立进程 mock HTTP server（返回 200）实测三场景：
- 端口服务 200（残留 engine 典型情形）：旧逻辑 `continue`（BUG），新逻辑 `throw`（正确）
- 端口空闲：新逻辑 `continue`（无回归）
- 非 2xx 响应：`Exception.Response` 非 null，新逻辑 `throw`（正确）

**原修复建议（保留供未来参考）**：

```powershell
$existingEngine = $false
try {
    $probe = Invoke-WebRequest -UseBasicParsing -Uri "http://127.0.0.1:$Port/version" -TimeoutSec 1
    $existingEngine = $true   # 任何成功响应即占用
} catch {
    if ($_.Exception.Response) {
        $existingEngine = $true   # 非 2xx 响应也视为占用
    }
    # 无 Response = 连接失败 = 端口空闲
}
if ($existingEngine) {
    throw "Port $Port is already serving an engine; clean up the leftover process before desktop smoke."
}
```

## 3. 独立发现（非 CodeRabbit 提出）

### N-01：跨入口双开未技术防护（已修复）

`lifecycle.rs` 的防双开基于锁文件 `shell_pid` 身份探测（映像名须为 `airp-ui`），仅覆盖 `airp-ui.exe` 双开。但 `Start-AIRP.cmd` 直接运行 `airp-core.exe`，不写锁文件：

- **cmd → 桌面**（先 `Start-AIRP.cmd` 后 `airp-ui.exe`）：壳探到端口占用且承载 webui → `ReuseExternalHosting` 分支优雅复用。**安全。**
- **桌面 → cmd**（先 `airp-ui.exe` 后 `Start-AIRP.cmd`）：cmd 不读锁、不探端口，直接 `airp-core.exe --port 8765`。**两个 engine 同时跑在不同端口，共用同一 `data\`。**

README.txt:13 已警告"Do not run both at the same time"，但无技术防护。

**修复**：`Start-AIRP.cmd` 新增 `netstat + findstr` 端口预检（纯 batch，不依赖外部脚本运行器——打包 smoke 断言 Start-AIRP.cmd 不得调用它们）。端口 8765 已有 LISTENING 进程时直接退出并提示用户清理。实测三场景（空闲→放行 / engine 运行→拦截 / engine 停止→放行）全过。

### N-02：`Start-AIRP.cmd` 端口（8765）与壳默认端口（8000）不一致（已修复）

`Start-AIRP.cmd:32` 硬编码 `--port 8765`；`main.rs:32` `DEFAULT_ENGINE_PORT = 8000`；`engine/src/data_dir/paths.rs:78` `settings.json` 默认 `daemon_port: 8000`；`engine/src/config.rs:138` `AppConfig::default().daemon_port = 8000`。

壳读 `settings.json` 得 8000，cmd 强制 8765——用户先跑 cmd 后跑桌面端会换端口。

**修复**：将三处内部默认值从 8000 → 8765，与 `Start-AIRP.cmd` 硬编码端口及 README 文档一致。8765 是面向用户的文档端口（webui 测试 `widget-runtime.test.mjs` 也用 8765），8000 是内部 outlier。改动：
- `ui/src-tauri/src/main.rs:36`：`DEFAULT_ENGINE_PORT: u16 = 8765`
- `engine/src/config.rs:138`：`daemon_port: 8765`
- `engine/src/data_dir/paths.rs:78`：`"daemon_port": 8765`

测试验证：`cargo test -p airp-ui` 17 passed；`cargo test -p airp-core --lib config::tests` 22 passed（测试用 `AppConfig::default().daemon_port` 对比，不硬编码值，改默认值不破坏断言）。

### N-03：smoke `finally` 清理依赖锁文件存在（已修复）

`smoke-desktop-ui.ps1` 的 `finally` 块按锁文件 `engine_pid` 清理残留 engine。若壳异常回落 `%APPDATA%`（前置预检应已拦截），锁不在包内 `$lock` 路径，锁级清理跳过。

**修复**：`finally` 块新增端口级兜底——`Get-NetTCPConnection -LocalPort $Port -State Listen` 查找 LISTENING 进程并清理，不依赖锁文件路径。防御冗余，确保任何情况下 engine 不残留。

## 4. 其他复核项（通过）

| 项 | 复核 | 结论 |
|---|---|---|
| `resolve_data_root` 三层优先级 | `main.rs:222-233`：env → 便携 → `%APPDATA%`；与 `engine/src/data_dir/paths.rs:21-28` 引擎侧 `AIRP_DATA_DIR` 读取一致 | ✓ |
| `data_root` 入 state 供退出复用 | `main.rs:47,72-75,150-161`：setup 写入，`RunEvent::Exit` 读取同一路径删锁 | ✓ 便携模式下路径不可重新推导，此设计正确 |
| 包体标记判定纯函数 | `main.rs:245-253` 只认 `airp-core.exe` + `webui/index.html`，不认 `data\`；测试三段覆盖 | ✓ |
| `build.ps1 -IncludeDesktop` 链路 | L49 调 `build-engine-sidecar.ps1` 产 externalBin → L55 `cargo build -p airp-ui --release --locked` → L66 复制 exe | ✓ |
| CI 触发路径 `ui/**` | 任何 `ui/` 改动触发全链路 Windows 包体构建 + 两个 smoke | ✓ 合理（桌面壳在 `ui/src-tauri/`） |
| smoke 五段断言 | 就绪（60×250ms）/ 承载（runtime-config.js `mode: 'desktop'`）/ 资产（root + 01-role-list.html）/ 共享数据（锁在包内 `data\`）/ 退出清理（engine 停 + 锁删） | ✓ 覆盖完整，与 `engine/src/daemon/mod.rs:839-849` desktop router 注入的 `mode: 'desktop'` 一致 |
| `finally` 清理顺序 | 先按锁 `engine_pid` 杀残留 engine，再强杀壳，再清 env | ✓ 顺序正确（先 engine 后壳，避免壳死锁文件还在但 engine 没人管） |
| README 桌面入口说明 | `README.txt:10-14` 准确描述两种入口共用 `airp-core.exe`/`webui\`/`data\`，勿同时运行 | ✓ 与实现一致 |
| `lifecycle.rs` 身份探测 | `airp-ui` 前缀匹配壳，`airp-core` 前缀匹配 engine；CSV 解析容忍 `.exe` 与 triple 后缀 | ✓ 未被本 PR 改动，复审确认无回归 |
| CI check runs（审计时） | Rust lint / Rust doc / UI and WebUI 已 success；Rust test / Production topology / Portable Windows WebUI in_progress | 待全绿后合并 |

## 5. 阻塞裁决

| 编号 | 严重度 | 来源 | 状态 | 说明 |
|---|---|---|---|---|
| **B-01** | **BLOCKING** | CodeRabbit CR-2 + 本审计实测复现 | **已修复（实测验证）** | smoke 端口预检改 `$portOccupied` 标志位 + `try` 外 `throw`；mock server 实测三场景全过 |
| CR-1 | 非阻塞（stale） | CodeRabbit CR-1 | 已解决 | commit `4c932ef` 以包体标记判定方案解决，CodeRabbit 建议的预建 `data\` 方案非唯一解 |
| N-01 | 非阻塞 | 本审计独立发现 | **已修复** | `Start-AIRP.cmd` 新增 netstat+findstr 端口预检；实测三场景全过 |
| N-02 | 非阻塞 | 本审计独立发现 | **已修复** | 三处内部默认端口 8000→8765（main.rs + config.rs + paths.rs）；17+22 测试通过 |
| N-03 | 非阻塞 | 本审计独立发现 | **已修复** | smoke finally 新增 `Get-NetTCPConnection` 端口级兜底清理 |

**B-01 + N-01/N-02/N-03 全部已修复且实测验证通过，本审计无阻塞意见，无遗留 issue 需合并后跟踪。** CR-1 已 stale 无需处理。

## 6. 修复确认（B-01）

开发 agent 已采纳 CodeRabbit CR-2 原方案（`$portOccupied` 标志位 + `try` 外 `throw`），代码见 [deploy/windows-webui/smoke-desktop-ui.ps1:29-48](file:///d:/AIRP-Dev/deploy/windows-webui/smoke-desktop-ui.ps1)。

本审计用独立进程 mock HTTP server（返回 200）实测三场景，对比旧/新逻辑：

| 场景 | 旧逻辑（commit `4c932ef`） | 新逻辑（修复后） | 期望 |
|---|---|---|---|
| 端口服务 200（残留 engine） | continue（BUG） | **throw** | throw ✓ |
| 端口空闲 | continue | continue | continue ✓ |
| 非 2xx 响应 | throw | throw | throw ✓ |

修复有效，B-01 关闭。

---

> 本复审由 GLM-5.2 纯文本 LLM 执行，未执行多模态视觉审查（本 PR 不改动 WebUI 视觉资产）。关键阻塞项 B-01 经 PowerShell 5.1 实测复现并实测验证修复有效，非手工推断。
