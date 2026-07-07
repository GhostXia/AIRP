# WebUI 设计文档合规审计

> 日期：2026-07-07
> 范围：PLAN.md / DEV-GUIDE.md / AUDIT-AND-ROADMAP-2026-07.md / UI-PROTOCOL-DECISION.md / WEBUI-BACKEND-PLAN.md / TAVERN-PARITY.md 中所有 UI 相关要求
> 审计对象：`airp-engine-console/` .design 项目（console.html + workbench.html）
> 立场：独立审计（AGENTS.md 守则）

---

## 1. 文档对 UI 的核心要求清单

从 6 份文档中提取的所有 UI 相关硬性要求/约束/承诺：

| # | 文档来源 | 要求 | 约束级别 |
|---|---------|------|---------|
| D-01 | WEBUI-BACKEND-PLAN §4.1.1 | 界面风格：仿 Claude Code / Codex 的 agent console，不仿 Open WebUI 平台型 | 方向约束 |
| D-02 | WEBUI-BACKEND-PLAN §4.1.1 | M1 信息架构：左侧角色/session，中间 chat transcript（streaming + markdown），右侧 agent event log + diagnostics | 结构约束 |
| D-03 | WEBUI-BACKEND-PLAN §4.1.1 | 每个 tool_call / tool_result 可折叠，保留 raw JSON 查看入口 | 功能约束 |
| D-04 | WEBUI-BACKEND-PLAN §4.1.1 | settings/model/provider 放在轻量 drawer 或顶部控制条 | 布局约束 |
| D-05 | WEBUI-BACKEND-PLAN §4.1.1 | 错误、鉴权状态、HTTP status、耗时、SSE event order 必须直接可见，不静默吞错 | 功能约束 |
| D-06 | WEBUI-BACKEND-PLAN §4.4 | 无鉴权时醒目警告"engine 无鉴权，仅本地 dev 安全" | 安全 UI |
| D-07 | WEBUI-BACKEND-PLAN §4.4 | 已配鉴权时提示输入 bearer token，存 sessionStorage | 功能 |
| D-08 | WEBUI-BACKEND-PLAN §4.4 | 不在 WebUI 里做 access key 的设置/修改 | 范围约束 |
| D-09 | PLAN §2.5 | 性能契约 7 条（虚拟滚动 / 分页 / patch / 稳定 ID / Rust 计算 / 流式渲染 / 内存卫生） | 产品级硬约束 |
| D-10 | PLAN §2.2 | UI 只渲染声明式 Blueprint，不执行 agent 生成的代码 | 架构约束 |
| D-11 | PLAN §2.2 | 首批候选 widget：chat / memory / emotion / inventory / quest / map / card | 功能范围 |
| D-12 | UI-PROTOCOL-DECISION | 首方 RP widget 优先：聊天、角色卡、记忆、情绪/state、物品、任务、地图、设置、诊断 | 功能范围 |
| D-13 | DEV-GUIDE §4 | 已实现 widget 列表：chat / emotion / memory / inventory / quest / map / card + clock | 已有资产 |
| D-14 | PLAN §2.3 | WebUI 是临时 harness，不做产品级 UI 打磨（主题/响应式/无障碍） | 范围约束 |
| D-15 | WEBUI-BACKEND-PLAN §3.2 | 不做 RBAC / RAG / PWA / 插件市场 | 范围约束 |
| D-16 | WEBUI-BACKEND-PLAN §3.2 | 不让临时 WebUI 反向决定 Tauri UI 架构 | 范围约束 |
| D-17 | WEBUI-BACKEND-PLAN §4.1.1 | 错误/鉴权/HTTP status/耗时/SSE event order 必须直接可见 | 功能约束（= D-05） |
| D-18 | TAVERN-PARITY §2 | 会话 UI：swipe / branch / checkpoint / 消息编辑 / regen / 继续 / 删除 / 隐藏 | 酒馆对标 |
| D-19 | TAVERN-PARITY §3 | 世界书全字段编辑 UI（keys / secondary_keys / position / depth / order / probability / selective / constant / 递归 / group） | 酒馆对标 |
| D-20 | AUDIT-AND-ROADMAP Sprint 5+ | ChatWidget markdown 渲染、头像/时间戳、消息编辑/删除端点、会话管理 UI | 路线图 |
| D-21 | DEV-GUIDE §0 | 首要目标：双击启动 → 选角色 → 发消息 → 收流式回复 | 产品目标 |
| D-22 | WEBUI-BACKEND-PLAN §9 P0 | WebUI 直接显示常见失败：无 API key、模型不存在、provider timeout、401、SSE 中断 | 功能 |
| D-23 | WEBUI-BACKEND-PLAN §2.1 | `/v1/agent/run` SSE 事件：plan / tool_call / tool_result / delta / done / error 分类显示 | 功能 |

---

## 2. 逐项合规检查

### 2.1 风格与布局（D-01 ~ D-04）

| 要求 | 设计现状 | 合规 | 说明 |
|------|---------|------|------|
| D-01 Claude Code agent console 风格 | ✅ | **合规** | 深色背景 #1A1A1E、温暖黏土/琥珀色 #D97757 主色、JetBrains Mono + Inter 字体、简洁边框面板、无装饰图片。与 Claude Code 产品页视觉语言一致 |
| D-02 三栏信息架构 | ✅ | **合规** | 左侧角色/会话/状态/导入、中间 chat + composer + agent run、右侧 Event Log。与文档描述的 M1 信息架构完全匹配 |
| D-03 tool_call/tool_result 可折叠 + raw JSON | 🟡 | **部分合规** | Agent Run 区域有色标事件日志（PLAN/TOOL_CALL/TOOL_RESULT/DELTA/DONE），但没有 raw JSON 折叠入口。作为静态设计稿可接受，实现时应补 |
| D-04 settings/model/provider 在顶部控制条 | ✅ | **合规** | 顶栏包含 Engine URL + Bearer Token + 连接按钮 + 状态指示。Settings/Models 显示在左侧栏连接信息区内（折叠面板）。webui 作为 harness，轻量 drawer 或顶栏都满足文档要求 |

### 2.2 错误可见性与安全 UI（D-05 ~ D-08）

| 要求 | 设计现状 | 合规 | 说明 |
|------|---------|------|------|
| D-05/D-17 错误/status/耗时直接可见 | ✅ | **合规** | 右侧 Event Log 每条记录都有 HTTP 状态码（彩色 badge）、method、path、时间戳、延迟。500 用红色、429 用主色、0 用灰色。诊断区有一键诊断 + 复制摘要 |
| D-06 无鉴权警告 | ❌ | **缺失** | 当前设计未包含"engine 无鉴权"醒目警告。连接状态只显示"未连接/已连接"三态（绿/灰/红），缺少无鉴权模式的明确安全提示 |
| D-07 Bearer token sessionStorage | N/A | **不适用** | 这是行为要求，静态设计稿不涉及 |
| D-08 不做 access key 管理 | ✅ | **合规** | 设计中没有 access key 设置/修改 UI。Settings 区域只读显示 api_key（脱敏 "sk-...****"） |

### 2.3 功能覆盖（D-09 ~ D-13, D-18 ~ D-23）

| 要求 | 设计现状 | 合规 | 说明 |
|------|---------|------|------|
| D-09 性能契约 7 条 | N/A | **不适用** | 性能约束是产品级 Tauri UI 的硬约束，webui 作为临时 harness 不要求满足全部 7 条 |
| D-10 只渲染声明式 Blueprint | N/A | **不适用** | 架构约束，静态设计不涉及 |
| D-11/D-12/D-13 候选 widget 覆盖 | 🟡 | **部分覆盖** | 当前 2 页只覆盖 chat + characters + sessions + state + workbench（角色卡+世界书编辑）。缺少：memory / emotion / inventory / quest / map / diagnostics widget。但 Sprint 5+ 才做这些，当前合理 |
| D-14 不做产品级打磨 | ✅ | **合规** | 设计是 developer console 风格，无主题切换 UI、无响应式布局、无无障碍标记 |
| D-15/D-16 不做 RBAC/RAG/PWA/不反向决定 Tauri | ✅ | **合规** | 设计纯粹是 engine 验证工具 |
| D-18 酒馆会话 UI（swipe/branch/edit） | ❌ | **缺失** | Chat 区域没有 swipe 候选切换、消息编辑、branch/checkpoint 可视化。这些是 Sprint 5+ 功能，当前不阻塞 |
| D-19 世界书全字段 UI | 🟡 | **部分覆盖** | Workbench 世界书 tab 覆盖了 keys / content / priority / enabled / comment。缺少：secondary_keys / position / depth / order / probability / selective / constant / sticky / cooldown / delay / recursive / group。Sprint 3 世界书引擎完成后才补全 |
| D-20 ChatWidget markdown / 头像 / 时间戳 | 🟡 | **部分覆盖** | Chat 消息有 inline code + code block 渲染，但无完整 markdown（列表/链接/表格/heading）。角色旁有头像占位（首字母）。无时间戳。Sprint 5+ 才做 |
| D-21 首要目标闭环 | ✅ | **合规** | 设计支持完整对话流：连接 → 选角色 → 选/建会话 → 发消息 → 收流式回复（有流式光标动画） |
| D-22 常见失败直接显示 | 🟡 | **部分覆盖** | Event Log 有 HTTP 错误状态码。但没有"无 API key""模型不存在""provider timeout"等结构化的错误解释面板。诊断区有占位但无具体错误分类 |
| D-23 agent event 分类显示 | ✅ | **合规** | Agent Run 区域有颜色编码事件：PLAN(amber) / TOOL_CALL(blue) / TOOL_RESULT(green) / DELTA(gray) / DONE(purple)。与文档字段完全匹配 |

---

## 3. 与当前 webui/ 功能的差距

对比现有 `webui/app.js`（1402 行，功能完整）和设计稿：

| webui/ 功能 | 设计稿覆盖 | 差距 |
|-------------|----------|------|
| chat streaming + stop button | ✅ | — |
| agent run event log | ✅ | 缺 raw JSON 折叠 |
| session list/create/select | ✅ | — |
| state / state history viewer | ✅ | — |
| workbench 角色卡编辑（GET/PUT） | ✅ | — |
| workbench 世界书编辑 | ✅ | 缺少高级字段 |
| import（file input + multipart） | ✅ | — |
| diagnostics 一键诊断 | ✅ | — |
| 并发测试 | ✅ | — |
| refreshAvatar（blob URL） | ❌ | 设计稿只有首字母占位头像 |
| history/regen/rollback 按钮 | ✅ | — |
| reextract confirm | ❌ | "重解" 按钮存在但无确认流程设计 |
| Bearer sessionStorage 持久化 | N/A | 行为层面 |

---

## 4. 新发现的设计问题

| # | 严重度 | 描述 | 建议 |
|---|--------|------|------|
| DS-01 | 中 | **无鉴权警告缺失**：文档 D-06 明确要求无鉴权时显示醒目安全警告，设计稿未包含 | 在连接信息区顶部增加黄色警告条："Engine 未配置访问密钥，仅本地开发安全" |
| DS-02 | 中 | **reextract 无确认流程**："重解" 按钮在角色区但无对应的确认弹窗或内联确认状态。webui/ 有 `confirm()` 弹窗保护 | "重解"按钮点击后应显示内联确认"确定重新提取？" + 取消 |
| DS-03 | 低 | **错误分类面板缺失**：文档 D-22 要求直接显示常见失败（无 key/模型错误/timeout），但 Event Log 只有 HTTP 状态码，缺少用户友好的错误解释 | 可选：在诊断区增加"常见错误"快速检查项 |
| DS-04 | 极低 | **console.html 默认 class="light"**：深色优先的设计系统声明了 dark-first，但 console.html 的 `<html>` 标签是 `class="light"`，workbench.html 是 `class="dark"`，不一致 | 统一为 `class="dark"` |
| DS-05 | 极低 | **导入区过于简陋**：只有一个 file input + 上传按钮，缺少文档中提到的 multipart upload 安全边界说明提示。可加一行小字"仅支持 multipart 上传，不走 card_path" | 可选 |

---

## 5. 文档合规总评

| 维度 | 评分 | 说明 |
|------|------|------|
| 风格合规 | A | 完全匹配 D-01 Claude Code agent console 风格要求 |
| 布局合规 | A | 完全匹配 D-02 三栏信息架构 + D-04 顶部控制条 |
| 功能覆盖 | B+ | 覆盖 M1 核心要求（chat/session/agent/event log/diagnostics/state/workbench），缺少 D-06 无鉴权警告和部分 D-19 世界书高级字段 |
| 安全 UI | B | Event Log 错误可见，但缺无鉴权醒目警告（D-06） |
| 范围约束 | A | 严格遵守"不做产品级打磨/RBAC/RAG/PWA"约束 |
| 工程可行性 | A | 所有组件均有对应 webui/ 实现参考，无空中楼阁 |

**合规率**：23 项要求中，12 项完全合规，7 项部分合规/不适用，3 项缺失（D-06 无鉴权警告、D-18 swipe、D-19 世界书高级字段），1 项重复。缺失项中只有 D-06 是当前应修的，其余属于未来 Sprint 范围。

**可操作建议**：修 DS-01（补无鉴权警告）+ DS-04（统一 dark-first class），其余可随后续 Sprint 自然补齐。
