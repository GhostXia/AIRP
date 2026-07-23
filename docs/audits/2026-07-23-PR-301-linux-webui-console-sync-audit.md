# PR #301 深度审计报告

**PR**: feat: Linux WebUI CI + issue #304 样板/webui 同步（合并审计）
**审计日期**: 2026-07-23
**审计范围**: 24 文件，+823 / −40
**权威样板**: `airp-engine-console/`
**派生实现**: `webui/`

---

## 裁决：通过（全部阻塞项已在同 PR 修复）

---

## 一、变更概览

本 PR 包含两个逻辑上独立的变更集：

| 变更集 | 文件 | 目的 |
|---|---|---|
| Linux WebUI CI | `.github/workflows/linux-webui-build.yml`, `deploy/linux-webui/*`, `.gitattributes` | 新增 musl 静态二进制 portable 构建 |
| issue #304 样板/webui 同步 | `airp-engine-console/*`, `webui/*` | NL 区 / JSON 折叠 / 操作列 / model picker / 33 屏 / 05 重命名 |

建议后续拆分为独立 PR 以降低审计与回退粒度，但本次不阻塞。

---

## 二、阻塞项（HIGH — 已修复 ✅）

### H-1 · webui/STYLEGUIDE.md 身份描述错误 ✅ 已修复

**位置**: `webui/STYLEGUIDE.md` L1-5

**问题**: webui 的 STYLEGUIDE 仍自称「本目录是 AIRP 控制台前端的**权威视觉样板**（golden sample）」，但权威样板是 `airp-engine-console/`。PR 修改了此文件（屏数 32→33、组件清单、边界约定）却未修正身份。

**影响**: 后续开发者可能将 webui 视为视觉仲裁源，与样板优先级冲突。

**修复**: 将 webui/STYLEGUIDE.md 首段改为派生实现定位，例如：
> 本目录是 AIRP 控制台前端的**派生 WebUI 实现**，视觉以 `airp-engine-console/` 权威样板为准。

### H-2 · renderSettings model picker 显示虚假「已拉取」状态 ✅ 已修复

**位置**: `webui/assets/console-runtime.js` L369

```javascript
const modelPill = node('span', 'status-pill ok');
modelPill.append(node('i', 'dot'), '已拉取');
```

**问题**: `renderSettings()` 从未调用 `/v1/models`，但 model pill 始终显示 `status-pill ok` + "已拉取"。用户会误以为模型列表已可用，实际上 picker 只是静态文本。

**影响**: 误导用户认为 model picker 功能完整可用；与样板 08 屏的「已拉取 27 个模型」语义不同（样板是静态展示，但 webui 是运行时页面，用户会当真）。

**修复建议**:
- 方案 A：pill 改为 `status-pill neutral` + "保存后自动拉取"，保存设置后再异步拉取并更新 pill
- 方案 B：移除 pill，仅保留 model picker 静态显示 + 降级手输行

---

## 三、中等项（MEDIUM — 已修复 ✅）

### M-1 · worldbook 操作列与样板视觉不一致 + 空操作三元 ✅ 已修复

**位置**: `webui/assets/console-runtime.js` L227

```javascript
node('span', 'op-link' + (entry.enabled === false ? '' : ''), ...)
```

**问题**:
1. 样板 `04-world-book.html` 使用 `.switch` / `.switch.on` 组件做启用/停用切换；webui 用文本链接"启用/停用"替代，视觉不对齐。
2. 三元表达式 `(entry.enabled === false ? '' : '')` 两个分支均返回空字符串，是死代码。

**修复**: 使用 `.switch` 组件对齐样板；移除无效三元。

### M-2 · renderWorkbench「保存修改」按钮语义模糊 ✅ 已修复

**位置**: `webui/assets/console-runtime.js` L177-178

**问题**: 「保存修改」按钮引用了在其后定义的 `editor`（JSON 编辑器），功能与「保存 JSON（整体替换）」完全重复。用户看到「保存修改」会以为保存的是结构化表单，实际上保存的是折叠区内的 JSON。

**影响**: 不会导致运行时错误（闭包在事件循环后执行），但 UX 误导。

**修复建议**: 将按钮文案改为「保存 JSON」或移至 JSON 折叠区内；或保留「保存修改」但实现结构化表单保存逻辑。

### M-3 · 屏数标注不一致 ✅ 已修复

**位置**: 两处 STYLEGUIDE + `screens.js`

**问题**: STYLEGUIDE 声称「33 屏」，但实际 HTML 文件 32 个（01-31 + 33，缺 32），`screens.js` 注册 32 条。「33 屏」是设计稿编号上限，不是实际屏数。

**修复**: 改为「32 屏（编号至 33，缺 32）」或补全 32 屏。

### M-4 · Linux workflow 未显式声明 `toolchain: stable` ✅ 已修复

**位置**: `.github/workflows/linux-webui-build.yml` L27

**问题**: Windows workflow 显式 `toolchain: stable`，Linux 依赖 `dtolnay/rust-toolchain` 默认值。虽然默认即 stable，但显式声明防止 action 默认值变更导致构建漂移。

---

## 四、低优先级项（LOW — 已修复 ✅）

| # | 位置 | 问题 |
|---|---|---|
| L-1 | `linux-webui-build.yml` L27 | `dtolnay/rust-toolchain` SHA 缺版本注释（`# stable`），与 Windows workflow 注释风格不一致 |
| L-2 | `start-airp.sh` | 缺少 `AIRP_LAUNCHER_SMOKE` 对等支持（Windows Start-AIRP.cmd 有），后续补 HTTP smoke 时需要 |
| L-3 | `deploy/linux-webui/README.txt` L18 | 提及 "projects such as SillyTavern"——用户面向文档中引用第三方竞品名称，建议改为泛化描述 |
| L-4 | `console-runtime.js` L191, L239 | NL 区「生成改写」按钮 handler 为 `null`，视觉上可点击但无响应；建议加 `disabled` 属性 |
| L-5 | `linux-webui-build.yml` L32 | `actions/cache@0057852...` SHA 为本仓库首次使用，建议补充 `# v4.x.x` 精确版本注释 |
| L-6 | `console-runtime.js` L367-368 | model picker 为纯静态 `<div>`，无交互能力（不可展开下拉）；与样板的 picker 语义一致（样板也是静态），但 webui 作为运行时页面用户会尝试点击 |

---

## 五、安全与供应链审计

| 检查项 | 结果 |
|---|---|
| GitHub Actions 全部 pin 到 commit SHA | ✅ checkout / rust-toolchain / cache / upload-artifact 均为 SHA |
| `persist-credentials: false` | ✅ |
| `permissions: contents: read` 最小权限 | ✅ |
| 仅 `workflow_dispatch` 触发，不在 PR/push 上跑 | ✅ |
| 启动器仅绑定 `127.0.0.1` | ✅ |
| 不传 `--open-browser`（engine 非 Windows fail-fast） | ✅ 与 `main.rs` L173-178 一致 |
| `set -euo pipefail` + 路径安全检查 | ✅ build.sh 有 `case` 防护 |
| 静态链接验证（ldd） | ✅ build.sh + workflow 双重检查 |
| `--locked` 防止 Cargo.lock 漂移 | ✅ |
| 密钥不回显 | ✅ README 明确说明 secrets.json 为明文 + 安全边界 |
| 无 secrets/tokens 硬编码 | ✅ |
| `.gitattributes` LF 规则 | ✅ 与 `deploy/production/*.sh` 对齐 |

---

## 六、样板 ↔ WebUI 一致性审计

| 文件 | 一致性 | 备注 |
|---|---|---|
| `components.css` | ✅ SHA256 完全一致 | 21-24 四组新组件同步 |
| `STYLEGUIDE.md` 组件清单 | ✅ | 新增 NL/diff/JSON/op-col/combobox |
| `STYLEGUIDE.md` 身份 | ❌ **H-1** | webui 仍自称权威样板 |
| 03 工作台 NL 区 + JSON 折叠 | ✅ 结构对齐 | webui 用 JS DOM 构建，样板用静态 HTML |
| 04 世界书操作列 | ⚠️ **M-1** | 样板用 `.switch`，webui 用文本链接 |
| 05 重命名 | ✅ | `05-presets-models` → `05-presets`，引用全部更新 |
| 08 设置 model picker | ⚠️ **H-2** | 样板静态展示合理，webui 虚假"已拉取"不合理 |
| 14 消息来源标注 | ✅ | 预设 → Provider |
| 33 向导·模型选择 | ✅ | 样板完整屏，webui 重定向到 16-onboarding |
| screens.js 注册 | ✅ | 33 注册 + flows 12→04 / 16→33 |
| onboarding.js 预设链接 | ✅ | `05-presets-models.html` → `05-presets.html` |
| 测试屏数 | ✅ | 31 → 32（当前分支 32 个 HTML 文件） |

---

## 七、CI / 部署脚本审计

### linux-webui-build.yml

- 结构对等 `webui-windows-build.yml`，独立文件不矩阵化 ✅
- 无 `pull_request` / `push` 触发器 ✅
- musl-tools 安装 → rust-toolchain → cache → build → verify → upload 流程完整 ✅
- 验证步骤（ldd / --help / bash -n / 资源齐全 / LICENSE）覆盖充分 ✅
- 缺 `publish-release` job（Windows 有）——合理，Linux 仅手动触发 ✅

### build.sh

- 对等 `build.ps1`：路径安全、`--locked`、ldd 静态检查、tar.gz 打包 ✅
- `chmod 0555` 设置二进制只读可执行 ✅
- `CC_x86_64_unknown_linux_musl` 环境变量正确设置 ✅
- 未打包 `config.json`——与 Windows 一致，engine 会自动创建 ✅

### start-airp.sh

- `export AIRP_DATA_DIR`（commit 2 修复）✅
- `unset` 生产环境变量（对等 Start-AIRP.cmd）✅
- `exec` 替换进程，信号正确传递 ✅
- 缺 `AIRP_LAUNCHER_SMOKE` 支持（L-2）

---

## 八、测试验证

```text
webui/tests/runtime-pages.test.mjs: 11/11 pass
```

- CSP 兼容性（无 inline style / handler）✅
- 屏数断言 32 ✅
- 控制台页面加载共享运行时 ✅
- 05 重命名后引用更新 ✅

---

## 九、审计遗留项（PR 合并后写入 issue）

| # | 来源 | 模块 | 描述 | 严重度 | 建议时机 |
|---|---|---|---|---|---|
| 1 | M-1 | webui | worldbook 操作列对齐样板 `.switch` 组件 | medium | 下个 webui PR |
| 2 | M-2 | webui | 「保存修改」按钮语义澄清或重构 | medium | 下个 webui PR |
| 3 | M-3 | docs | 屏数标注统一（32 屏 vs 33 屏） | low | 随下次样板同步 |
| 4 | L-2 | deploy | Linux launcher 补 smoke 模式 + HTTP smoke 脚本 | low | Linux CI 首次成功运行后 |
| 5 | L-3 | deploy | README.txt 移除第三方竞品名称 | low | 随下次 deploy 变更 |
| 6 | L-4 | webui | NL 区「生成改写」按钮加 disabled | low | 契约交付时 |
| 7 | L-6 | webui | model picker 交互化（下拉展开） | low | 后端 /v1/models 集成后 |

---

## 十、独立审计声明

本审计基于 PR diff 全文、样板与 webui 源文件逐行对比、engine `main.rs` 交叉验证、
webui 测试实际运行结果独立得出。未将开发 agent 的 PR 描述或 CodeRabbit 摘要
作为不可质疑前提。
