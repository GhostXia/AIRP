# Issue #498 补充：webui 侧扩展路径与 UI 可替换性

- **类型**: 设计补充 / 澄清
- **日期**: 2026-08-07
- **性质**: 只做加法不做减法——不修改原设计，不修改审计报告 A1-A5，只补充 webui 侧设计与新增扩展路径
- **背景**: issue #498 原设计 + 审计报告聚焦 engine/daemon 侧，漏了 webui 侧设计。本补充澄清 webui 侧现实基础，并新增两条扩展路径。

## 1. webui 侧现实基础（issue #498 原设计漏了）

### 1.1 slot 系统是 webui 侧的现实基础

issue #498 通篇没提 slot 系统，但这是 widget 在 webui 侧的挂载点，已经实现。

**engine 侧**（[extensions/mod.rs:626-632](../../engine/src/extensions/mod.rs#L626-L632)）：
```
DEFAULT_SLOT_IDS = [
    "chat.sidebar",
    "chat.panel-right",
    "settings.context",
    "diagnostics.context",
    "workbench.grid",
]
```

engine 强制校验：声明未知 slot → `invalid_slot`（fail-closed，不默默编入任意位置）。

**webui 侧**：
- `webui/assets/widgets/slots.js` + `slots.json` + `boot.js` 已实现挂载
- `widget-host.js` 已实现 vue / module / sandboxed esm / consent gate / error / missing 分流
- `sandbox-bridge.js` 已实现 postMessage 协议（opaque origin）
- `consent.js` 已实现 C-P3：从 `GET /v1/extensions/grants` 拉权威缓存

**结论**：widget 在 webui 侧的骨架已完整，issue #498 应基于这个事实修正，不是重新设计。

### 1.2 Vue 选型澄清

用户选 Vue 的初衷是想支持"第三方修改 UI"。但 Vue 的设计哲学与 ST 那套"任意 JS 注入"是**相反的**：

| 维度 | ST 模型 | Vue 模型 |
|---|---|---|
| 组件树 | 命令式 DOM 操作 | 声明式 render + vdom diff |
| 状态 | 全局可变 | 单向数据流 + reactive |
| 挂载点 | `document.body.appendChild` 任意 | `app.mount('#app')` 受控 |
| 修改别人的组件 | 直接改 DOM | 组件闭包私有，外部访问不到 |

**Vue 的价值是堵住 ST 那套任意注入（声明式 + 组件封闭 + 响应式），不是实现它。** 这是安全价值，不是缺陷。

Vue 化升级路径：主 app `provide` 受控 API（如 `registerSlotItem`）+ widget 是独立 Vue app mount 到 slot 容器。这是 AIRP 版本的"Vue 响应式 + 安全沙箱"。

### 1.3 sandboxed iframe 硬约束（BUG-6）

- `sandbox="allow-scripts"` 无 `allow-same-origin`，opaque origin
- widget 不能直接 `fetch`（审计 A1）
- widget → engine/trusted plugin 只能走 postMessage 代理 fetch
- 任何放宽（`allow-same-origin`）都是回归 BUG-6 安全门禁（审计 A5）

## 2. 第三方"修改 UI"的判别

| 诉求 | 当前 | 未来 | 永远做不到？ |
|---|---|---|---|
| 第三方增加 UI 内容 | ✅ slot 系统已支持 | 加更多 slot | 不是 |
| 第三方修改部分 UI（受控替换） | ❌ 当前 slot 是 add-only | slot 升级 replaceable（`mode: replace`） | 不是 |
| 第三方任意修改 UI 任意位置 | ❌ sandbox 堵死 | dev mode 阶段替换 | **是，这是有意的** |

第三行是 Vue + sandbox 的有意代价——ST 那套"任意注入 JS 改任意 DOM"在 AIRP 架构下永远做不到，且不应该做到。

判别标准仍然用那条：
> "禁用这个扩展，RP 基础工作流会崩吗？会 → 必须在核心；不会 → 可以做胶水层接口。"

- 替换角色卡编辑器 → 禁用后用户仍能用内置编辑器 → 不崩 → replaceable slot
- 替换整个 chat 界面 → 禁用后用户无法聊天 → 崩 → dev mode 或核心

## 3. 新增扩展路径（加法，不删减已有）

issue #498 原设计：widget + trusted plugin + dev mode（未实现）
本补充新增两条**分发级替换**路径，与运行时扩展并列：

### 路径 4：替换 webui 文件（主题优化级别）

- **不是** widget，**不是** dev mode，是**分发级别替换**
- 变更范围：主题 / 视觉体系 / 整体 DX 重设计（>70% UI）
- 信任模型：显式信任（替换用户看到的全部代码）
- AIRP 解耦支持它：webui 是静态文件目录，daemon 同源承载；webui 只通过 HTTP API 调 daemon，无 IPC、无共享内存；Tauri webview 导航到 daemon URL，不绑定特定 webui 版本

**约束**：
1. **替换 webui = 替换 widget 系统本身**：如果替换的 webui 不实现 slot 容器 / sandbox-bridge / consent UI，所有已安装 widget 失效。主题作者要么完整实现 AIRP 的 widget 契约，要么声明"本主题不支持 widget"
2. **API 契约稳定性是前提**：daemon 的 HTTP API 必须升级为一等公民契约（文档化 + 版本化），当前 v1 是事实标准但无显式文档化
3. **升级冲突**：AIRP 升级会带新 webui。直接覆盖 → 用户替换丢失；不覆盖 → 用户停留在旧 webui 错过安全修复。最小处理：升级时检测 webui 是否被替换，提示用户选择
4. **安全边界**：替换的 webui 能调 daemon 所有用户级端点（包括写操作、备份）。恶意 webui 能偷所有角色卡 / 世界书、删数据、上传备份到外部。文档必须明示这个风险

### 路径 5：第三方完全重写 UI（解耦架构的产物）

- AIRP 解耦架构的**直接产物**
- engine 和 webui 完全分离，webui 只是 daemon HTTP API 的**一个消费者**
- 任何遵守 API 契约的 UI 都能接入（React / Svelte / 原生 / 任意技术栈）
- 约束同路径 4
- **意义**：AIRP 的 engine 不绑定任何特定 UI，UI 是可替换的消费者。这是比"替换 webui 文件"更强的声明——不是改 AIRP 的 UI，是 AIRP 的 engine 从一开始就设计为 UI 无关

**这条路径不需要 engine 任何改动**——解耦已经支持。只需要：
1. daemon HTTP API 契约文档化（路径 4/5 的共同前提）
2. 文档明示信任模型与安全边界
3. 升级提示逻辑（可选，非阻塞）

## 4. 修正后的扩展路径全景

```
运行时扩展（daemon 在跑时加载）：
├─ Widget 层（零信任，slot，sandboxed iframe）
├─ Trusted Plugin 层（显式信任，子进程，HTTP 反代）
└─ Dev Mode（最高，阶段替换 hook，未实现）

分发级替换（安装时选择，不是运行时）：
├─ 路径 4：替换 webui 文件（主题优化，>70% UI 变更）
└─ 路径 5：第三方完全重写 UI（接入同一 daemon API，任意技术栈）
```

## 5. 判别标准（按变更范围）

| 变更范围 | 路径 | 信任模型 |
|---|---|---|
| < 30% UI（加按钮/面板/侧边栏） | Widget slot | 零信任（沙箱） |
| 30%-70%（替换某个核心组件，保留其他） | Dev mode 阶段替换（未实现） | 显式信任 + 常驻提示 |
| > 70% UI（主题/视觉体系/DX 重设计） | 路径 4：替换 webui 文件 | 显式信任（分发级） |
| 完全重写 UI | 路径 5：第三方重写 UI 接入 API | 显式信任（分发级） |

## 6. 与审计报告 A1-A5 的关系

本补充不修改审计报告 A1-A5 的任何阻塞结论。A1-A5 仍然有效，issue #498 原设计必须先修 A1-A5 再进入实现。

本补充新增的内容：
- §1 澄清 webui 侧现实基础（slot / Vue / sandbox），这是 issue #498 原设计的事实遗漏
- §3 新增路径 4/5，这是原设计未考虑的扩展路径
- §4 修正扩展路径全景，加法不删减

## 7. 推进建议（加法，不删减原计划）

1. **先做 C-P4.1**（widget executor read 三件套）——原计划
2. **Trusted Plugin MVP**——原计划
3. **daemon HTTP API 契约文档化**——路径 4/5 的前提，新加
4. **替换 webui / 第三方重写 UI**——不需要 engine 改动（解耦已支持），只需文档化 API 契约 + 升级提示逻辑（可选）
5. **Dev mode / replaceable slot**——等真实需求信号，YAGNI

## 8. 未执行视觉审查

本补充为纯文本 LLM（GLM-5.2）设计澄清，不涉及 WebUI 视觉改动，按 AGENTS.md 规则无需多模态补审。
