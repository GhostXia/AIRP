# Issue #498 补充 2：engine 契约 vs 官方 webui 实现层的边界

> ⚠️ **Superseded for current status**：本文保留设计澄清历史；当前 engine/webui 分层合同以 [CURRENT-BASELINE.md](../CURRENT-BASELINE.md) 与源码为准。

- **类型**: 设计澄清 / 契约分层
- **日期**: 2026-08-07
- **性质**: 只做加法不做减法——不修改原设计、不修改审计 A1-A5、不修改补充 1
- **背景**: 用户提问"第三方用 JS 不用 Vue 写 UI，之前讨论的功能能满足吗"暴露了之前所有讨论的一个根本性遗漏：**engine 契约和官方 webui 实现层没有分层**

## 1. 核心澄清：契约 vs 实现

之前所有讨论把 widget slot / sandboxed iframe / consent UI / Vue 响应式 都当成"AIRP 的扩展模型"，错了。正确分层：

| 层 | 是什么 | 谁决定 | 是否强制第三方 UI |
|---|---|---|---|
| **engine 契约** | HTTP API v1 + access key 鉴权 + widget_intent endpoint + capability grant 校验 + trusted plugin 反代路由 | engine 强制 | **是** |
| **官方 webui 实现层** | slot 容器（5 个内置 slot）+ sandboxed iframe（`allow-scripts` 无 `allow-same-origin`）+ consent UI（从 engine 拉权威缓存）+ Vue 响应式 + provide/inject | webui 自己选 | **否** |

**engine 不强制 webui 必须用 sandboxed iframe。engine 不强制 webui 必须实现 slot。engine 不强制 webui 必须走 widget_intent。**

如果第三方 UI 不走 widget_intent，直接让它的 UI 组件用 access key 调 `/v1/memory/...`，engine 也响应——**capability grant 在这个 UI 里完全失效**。

## 2. 第三方 UI 的两种实现路径

### 情况 A：实现了 AIRP widget 契约

- 提供 slot 容器，挂载 widget
- 用 sandboxed iframe（或等价沙箱）
- 走 widget_intent → engine 做 capability grant
- Vue 不 Vue 无关，JS / Svelte / 原生 / 任意技术栈都能实现

**结果**：widget / trusted plugin / capability grant / consent 全部生效。之前讨论的所有功能都能跑。

### 情况 B：不实现 widget 契约，走 ST 那套任意 JS 注入

- 第三方代码进 UI 主进程，能改任意 DOM
- 不走 widget_intent，直接用 UI 的 access key 调 daemon HTTP API
- 无沙箱，无 consent

**结果**：
- widget 系统在这个 UI 里**不工作**（已装 widget 失效）
- 但 UI 本身能跑——engine 仍然响应所有 HTTP API
- capability grant 完全失效——这个 UI 的"扩展"就是直接调 API
- 安全风险由这个 UI 自己承担（类似 ST，恶意脚本能读所有数据）

## 3. "保留了 js 扩展"——用词修正

用户原话："我选择了 widget，但依旧保留了 js 扩展。这样对吗？"

**对，但"保留"这个词不准确**。准确表述：

> AIRP 选择了 widget 作为官方 webui 的扩展模型。但 engine 不约束 UI 扩展模型，只约束 HTTP API 契约。第三方 UI 可以实现 widget 契约（情况 A），也可以走 ST 那套任意 JS 注入（情况 B）。**这不是 AIRP 主动"保留" js 扩展，是 engine 物理上无法阻止**——因为 engine 和 UI 完全分离，UI 是 daemon HTTP API 的消费者，消费者怎么用 API 是消费者的事。

这是解耦的必然代价，不是设计选择。类似 Linux 桌面环境：X11/Wayland 不强制桌面环境（GNOME / KDE / i3）必须用同一种扩展模型。

## 4. 修正后的契约分层图

```
┌─ engine 契约（强制，所有 UI 必须遵守）───────────┐
│  - HTTP API v1                                    │
│  - access key 鉴权                                │
│  - widget_intent endpoint + capability grant 校验 │
│  - trusted plugin 反代路由                        │
└──────────────────────────────────────────────────┘
                    ▲
                    │ UI 自己决定怎么用
                    ▼
┌─ 官方 webui 实现层（webui 自己选，不强制第三方 UI）─┐
│  - slot 容器（5 个内置 slot）                      │
│  - sandboxed iframe (allow-scripts, 无 same-origin)│
│  - consent UI（从 engine 拉权威缓存）              │
│  - Vue 响应式 + provide/inject                     │
└──────────────────────────────────────────────────┘
                    ▲
                    │ 第三方 UI 可以实现等价契约，也可以走自己的模型
                    ▼
┌─ 第三方 UI（路径 5）──────────────────────────────┐
│  情况 A：实现 AIRP widget 契约（slot + sandbox +   │
│          widget_intent）→ widget 系统生效          │
│  情况 B：走 ST 那套任意 JS 注入 → widget 系统不工作│
│          但 engine 仍响应所有 HTTP API             │
└──────────────────────────────────────────────────┘
```

**官方 webui 是实现层的参考实现，不是唯一实现**。第三方 UI 可以实现等价契约（情况 A），也可以走自己的模型（情况 B）。

## 5. 对原设计与审计的影响

### 5.1 不修改原设计

issue #498 原设计的 widget 层 + trusted plugin 层仍然有效。它们是 engine 契约的一部分，所有 UI 都受约束（如果 UI 选择走 widget_intent）。

### 5.2 不修改审计 A1-A5

A1-A5 仍然有效。审计针对的是 widget_intent handler 的实现问题，与契约分层无关。

### 5.3 修正补充 1 的措辞

补充 1 把 sandboxed iframe + slot + consent 当成"webui 侧现实基础"。准确说法是"**官方 webui 实现层的现实基础**"——这是官方 webui 的选择，不是 engine 的强制。

补充 1 的路径 4（替换 webui 文件）和路径 5（第三方重写 UI）仍然有效，但应补充本澄清：路径 5 下，第三方 UI 实现的扩展模型由第三方 UI 自己决定，engine 不约束。

## 6. 推进建议（加法，不删减）

1. **daemon HTTP API 契约文档化**——engine 契约必须显式文档化（`docs/API-CONTRACT.md` 或等价），这是路径 4/5 的前提，也是本次澄清的落地形式
2. **官方 webui 实现层文档化**——明确 slot / sandbox / consent / Vue 是官方 webui 的实现选择，不是 engine 强制，避免第三方 UI 误以为必须复刻
3. **engine 契约稳定性**——一旦文档化，破坏性变更需要版本化（v1 → v2），不能 silent break
4. 其他原计划不变（C-P4.1 / Trusted Plugin MVP）

## 7. 一句话结论

> AIRP 选择了 widget 作为官方 webui 的扩展模型。但 engine 只约束 HTTP API 契约，不约束 UI 扩展模型。第三方 UI 可以实现 widget 契约，也可以走自己的模型——这不是 AIRP 主动"保留" js 扩展，是 engine 物理上无法阻止。市场自行选择。

## 8. 未执行视觉审查

本澄清为纯文本 LLM（GLM-5.2）设计澄清，不涉及 WebUI 视觉改动，按 AGENTS.md 规则无需多模态补审。
