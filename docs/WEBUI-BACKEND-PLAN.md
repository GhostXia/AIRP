# 后端建设计划书（为 WebUI 验证面做准备）

> **作者**：AtomCode (GLM-5.2)，2026-07-04
> **状态**：审计修订版（2026-07-04）：按本文件的收口结论开工；不再把关键安全边界留作实现时临场裁定。
> **依据**：DEV-GUIDE §3.3/§3.7 + [WEBUI-BACKEND-VALIDATION.md](WEBUI-BACKEND-VALIDATION.md) + [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md) §3 + 源码核实（`engine/src/daemon/mod.rs`、`handlers.rs`）
> **与既有约束的关系**：本计划只动 `engine/` 后端 + 新增 webui 子目录；**不**改 Tauri UI 产品线、**不**碰 `ui/src/agent-test.ts` 单一测试入口、**不**放开 RR-001 `card_path` 任意路径读护栏。浏览器/WebUI 永远不走 `card_path`，即使持有 bearer token。

---

## 1. 目标与定位（先定性，避免范围爬升）

**WebUI 是临时后端可靠性验证 harness，不是产品 UI。**（DEV-GUIDE §3.3 L130、WEBUI-BACKEND-VALIDATION.md §1）

它回答两个问题：

1. **后端能否稳定跑通最小 RP 闭环，让 UI 不再遮住后端不确定性？**
2. **用户能否先通过简陋 WebUI 体验完整 agent 能力，让本地 UI 获得充足时间产品化？**

- 长期产品 UI = Tauri/Vue 桌面端（不变）。
- 短期验证面 = 浏览器 WebUI / HTTP harness。
- 稳定核心 = `engine` 是独立 HTTP/SSE 服务，不嵌 Tauri（现状已如此）。

**产品裁定**：早期用户真正需要的是“后端完整 agent 能力”，可以忍受 UI 简陋。因此 WebUI 的优先级不是视觉质量，而是把 chat、agent loop、工具事件、角色/会话、history/regen/rollback、state/history、settings/provider 错误面完整暴露出来；Tauri UI 则继续按正式产品体验打磨。

**退出条件**（WEBUI-BACKEND-VALIDATION.md §7）：不经过 Tauri 也能从浏览器复现后端 chat streaming 和 agent run；data persistence / 鉴权 / 错误 / 并发 stream 可观察；浏览器导入不依赖可信本地 `card_path`；早期用户能在 WebUI 中触达完整后端能力。达成后 WebUI 可降为 developer-only 诊断页或删除，**不成为默认产品面**。

---

## 2. 现状基线（源码核实，非文档抄录）

### 2.1 端点矩阵（来自 `engine/src/daemon/mod.rs:191-238` 的 `Router::new().route(...)` 枚举）

共 21 个端点 + `/version`：

| 端点 | Method | 鉴权 | SSE | Request shape | Response shape | data 副作用 / WebUI 缺口 |
|---|---|---|---|---|---|---|
| `/version` | GET | **否** | 否 | none | `{name, version}` | 无副作用；M1 health ping 可先用它 |
| `/v1/chat/completions` | POST | 是 | **是** | `ChatCompletionRequest`：`message` + `user_profile` 必填；`character_id/session_id/model/endpoint/api_key/...` 可选 | SSE `data: {text}`，结束帧由 stream 完成 | append chat log；M1 核心路径；WebUI 自编号 SSE event order |
| `/v1/agent/run` | POST | 是 | **是** | `AgentRunRequest` = `ChatCompletionRequest` + `max_steps/token_budget/wall_clock_secs` | SSE `AgentEvent`：`plan/tool_call/tool_result/delta/done` | append chat log + agent step；M1 只做表单和事件日志，不做产品化渲染 |
| `/v1/chat/history` | POST | 是 | 否 | `{character_id}` | `ChatLog` | 读；M1 用于刷新持久化 history |
| `/v1/chat/rollback` | POST | 是 | 否 | `{character_id, message_index}` | `ChatLog` | truncate chat log；destructive，WebUI 要二次确认或明显标注 |
| `/v1/chat/regen` | POST | 是 | 否 | `{character_id}` | `ChatLog` | rewrite/delete last assistant message；WebUI 标注会修改历史 |
| `/v1/characters` | GET | 是 | 否 | none | `string[]` | 读；M1 只列角色，不导入 |
| `/v1/characters/import` | POST | 是 | 否 | 当前 JSON：`{character_id?, card_path?, card_json?, card_png_base64?}`；M3 新增 multipart | `{character_id, card_format}` | 写 `data/characters/`；10MB body limit；M1 禁用，M3 前 Web/browser 永不发 `card_path` |
| `/v1/characters/:id/reextract` | POST | 是 | 否 | path `character_id` | JSON asset summary | 重写 card assets；非 M1 范围，后续诊断页可加 |
| `/v1/characters/:id/avatar` | GET | 是 | 否 | path `character_id` | `image/png` bytes | 读；M1 可选预览，不阻塞 |
| `/v1/characters/:id/state` | GET | 是 | 否 | path `character_id` | raw `live.json` | 读；不存在返回 404，WebUI 要显示 empty/missing 区别 |
| `/v1/characters/:id/state/history` | GET | 是 | 否 | path `character_id` + query `limit?` | JSON array parsed from history JSONL | 读；`limit` clamp 1..1000 |
| `/v1/characters/:id/state/schema` | GET | 是 | 否 | path `character_id` | raw `schema.json` | 读；M1 可选，不阻塞 |
| `/v1/scenes` | GET/POST | 是 | 否 | GET none；POST `SceneConfig` | GET `string[]`；POST `{scene_id, path}` | POST 写 scene；非 M1 核心 |
| `/v1/scenes/:id` | GET | 是 | 否 | path `scene_id` | raw `scene.json` | 读；非 M1 核心 |
| `/v1/scenes/:id/characters` | POST | 是 | 否 | `{character_id, role?, intro?}` | `{scene_id, character_count}` | 写 scene；非 M1 核心 |
| `/v1/models` | GET | 是 | 否 | none | upstream `/models` JSON passthrough | 代理上游；依赖 `endpoint/api_key`；M1 用于 provider smoke |
| `/v1/presets` | GET | 是 | 否 | none | `string[]` | 读；M1 可列，不做编辑 |
| `/v1/presets/:id` | GET | 是 | 否 | path `preset_id` | `TavernPrompt[]` | 读；无文件返回 404 |
| `/v1/sessions/:character_id` | GET/POST | 是 | 否 | path `character_id` | GET `SessionId[]`；POST `SessionId` | POST 创建 session 目录；M1 要支持 list/create/select |
| `/v1/settings` | GET/POST | 是 | 否 | GET none；POST `PartialAppConfig` | `SettingsView`，`api_key` 脱敏 | POST 写 `data/settings.json` 并热重载；WebUI 不做 access key 管理 UI |

### 2.2 安全姿态（源码核实）

- **鉴权**（`mod.rs:110-135`）：`access_api_key` 为 `None` 或空串 → **全放行**；非空 → 要求 `Authorization: Bearer <key>`，常数时间比较。**默认无鉴权**——本地 sidecar 可接受，对外暴露必须设 `AIRP_ACCESS_KEY`。
- **CORS**（`mod.rs:173-176`）：`allow_methods/headers/origin = Any`。浏览器 WebUI 直连无 CORS 阻碍；但也是 DNS-rebind/CSRF 攻击面（DEV-GUIDE §3.3 L131 记录此坑）。
- **限流**（`mod.rs:178-189`）：governor 覆盖**所有** `/v1/*`（A2-7 已修，非仅 chat）；key = Bearer token（已鉴权）或 peer IP。
- **Body limit**：仅 `/v1/characters/import` 显式 `DefaultBodyLimit::max(10MB)`；其余走 axum 默认。

### 2.3 已具备的后端能力（无需新建，WebUI 直接调）

- 真实酒馆卡 PNG 解析（`png_parser`）、path-first 导入（`card_path`）。
- chat 流式 SSE（`chat_pipeline::build_sse_stream`）、id-keyed chat state（PR #6）。
- sessions/scenes/presets/state CRUD、history/rollback/regen。
- agent 多步 loop（M_AGENT-1 骨架；PR #15 会话工具已合并）。
- Tauri 打包带 sidecar（PR #13，已验证双击自起）。

### 2.4 缺口（WebUI 要暴露但后端当前不直接的）

- **无 `/health` 或 `/readyz`**：`/version` 可当 health ping，但无就绪探针（engine 起来但 provider 未配 ≠ ready）。
- **无 SSE 事件序号 / 可观察性**：流式 token 无 per-event id，WebUI 想做"event log"要自己编号。
- **`card_path` 任意路径读**（RR-001）：浏览器调用方**不可**用此参数——这是 §4.3 的硬约束。

---

## 3. 范围（做与不做）

### 3.1 做（WEBUI-BACKEND-VALIDATION.md §3）

1. **后端端点矩阵文档**（M0，本计划书已起头 §2.1，需补 request/response shape 与已知缺口两列）。
2. **最小 WebUI/HTTP harness**（M1）：单页浏览器面，调：
   - `/version` + `/v1/settings` 读写（API key 脱敏显示）
   - `/v1/characters` 列表（**不含 import**；浏览器导入必须等 M3 multipart 完成）
   - `/v1/sessions/:id` 列/建
   - `/v1/chat/completions` 发消息 + SSE 流式 transcript 渲染
   - `/v1/chat/history` / `rollback` / `regen`
   - `/v1/characters/:id/state` + `state/history`
   - `/v1/agent/run` 表单 + SSE agent event log（`plan/tool_call/tool_result/delta/done/error`）
   - provider/model/settings 错误展示，作为用户可用性的底线，不做静默失败
3. **可靠性检查**（M2）：API key 缺失/错误、model 错误、provider timeout/SSE 断流、bearer 缺失、**两个并发 chat stream**、刷新页面、regen/rollback 后置状态。
4. **数据安全边界**（M3）：浏览器导入走 multipart upload + engine 受控临时目录，**Web/browser 永久禁用 `card_path`**。
5. **证据记录**（§5）：每次验证记 engine 启动命令、URL、status code、耗时、SSE 事件序、数据目录、失败截图/日志。

### 3.2 不做（硬约束，防范围爬升 + 守既有门禁）

- **不**做桌面 UI 产品打磨（布局/主题/动效/widget 体验）。
- **不**与 `ui/src/agent-test.ts` 竞争第二套 agent 前端控制接口（反冗余门禁，DEV-GUIDE §0 L28、L156）。WebUI 是**人**用的诊断面，不是 agent 自测面。
- **不**让浏览器读任意本地路径（RR-001 护栏）。
- **不**让临时 WebUI 反向决定 Tauri UI 架构（DEV-GUIDE §3.3 L130、Phase 3+ L291）。
- **不**做产品插件/runtime 决策。
- **不**改 `engine` 现有 `/v1/*` 契约为 WebUI 专开方便之门（契约稳定优先，WebUI 适配引擎，不反之）。

---

## 4. 技术方案

### 4.1 WebUI 放在哪

**新建 `webui/` 顶层目录**（与 `engine/` `protocol/` `ui/` 平级），**不**进 `ui/`（`ui/` 是 Tauri 桌面产品，WebUI 是临时 harness，混放会污染产品线 + 反冗余门禁风险）。

```
D:\AIRP-Dev/
├── engine/       ← 后端（本计划可能小幅改，见 §4.2/4.3）
├── protocol/     ← 不动
├── ui/           ← Tauri 桌面产品，本计划不动
├── webui/        ← 新增：临时后端验证 harness
│   ├── index.html        单页
│   ├── app.js            浏览器原生 ES module（无框架、无构建）
│   ├── style.css
│   └── README.md         启动方式 + 退出条件
└── docs/
    └── WEBUI-BACKEND-VALIDATION.md  ← 已存在，验证证据回填至此
```

**技术选型理由**：无框架、无构建链——harness 越薄越好，避免它自己变成要维护的产品。浏览器不能直接执行 TypeScript，所以默认写 `app.js`（可配 JSDoc），用原生 ES modules + `fetch` + `EventSource`（SSE）。只有当手写 JS 明显阻碍验证时，才升级为 Vite/TypeScript。

### 4.1.1 界面风格裁定

**主方向：仿 Claude Code / Codex 的 agent console 风格，不仿 Open WebUI 的平台型产品架构。**

理由：WebUI 的核心任务是让完整 agent 能力可见，而不是提供通用 AI 平台。它要优先展示 agent 如何工作：请求、流式回复、计划、工具调用、工具结果、错误、耗时、状态码、SSE 顺序、state/history 变化。Claude Code / Codex 式的“工作流 + 事件流 + 可折叠执行细节”更贴近这个任务。

M1 默认信息架构：

- 左侧：角色、session、run history。
- 中间：chat transcript，支持 streaming delta 和 markdown。
- 右侧或下方：agent event log + diagnostics。
- 每个 `tool_call` / `tool_result` 可折叠，保留 raw JSON 查看入口。
- settings/model/provider 放在轻量 drawer 或顶部控制条，不做完整设置中心。
- 错误、鉴权状态、HTTP status、耗时、SSE event order 必须直接可见，不静默吞错。

Open WebUI 只可借鉴少量通用聊天习惯：

- session 侧栏。
- model/provider selector。
- markdown 渲染。
- 简单 settings drawer。

明确不借鉴：

- RBAC / user groups / enterprise auth。
- RAG、知识库、文件库、语音、PWA、插件市场、多模型平台。
- Open WebUI 的后端、数据库、部署、权限和产品 IA。
- 把 WebUI 做成独立平台或默认产品面。

### 4.2 engine 侧需要的最小改动

WebUI 优先复用现有端点。**仅当现有端点不足以验证时**才改 engine，且改动必须不破坏 Tauri 路径。预判需要的改动：

1. **`/health`（或 `/readyz`）端点**：返回 `{ "engine": "ok", "provider_configured": bool, "data_root_writable": bool }`。**新增**，不碰现有 `/version`。理由：WebUI 要区分"engine 起了"与"能跑对话"。
2. **CORS 收紧选项**：当前 `Any` 对本地 dev 够用，但若 WebUI 跨机访问（如手机浏览器访问桌面 engine）需配 `AIRP_CORS_ORIGIN` 环境变量。**可选**，MVP 不做，文档标记。
3. **`/v1/characters/import` multipart 支持**：当前接受 `card_path`/`card_json`/`card_png_base64`。为 WebUI 加 `multipart/form-data` 入参（文件上传直读，或只落到 engine 管理的受控临时目录）。**这是 §4.3 的落地动作，必须独立 PR。**

**不**改的：`/v1/chat/completions` 契约、SSE 事件格式、state 模型、agent loop——这些 WebUI 适配即可。

### 4.3 数据安全边界（M3，硬约束，对应 RR-001）

`card_path` = 引擎侧任意绝对路径读。当前单本地可信 UI 豁免；**浏览器调用方不可信**，必须禁用 `card_path` 分支。

**收口方案**：

- **Web/browser 永不走 `card_path`**：即使 WebUI 持有 bearer token，也只能用 multipart/streaming upload 或 fixture id。Bearer token 证明调用者有 API 权限，不证明用户通过本机文件选择器授权了某条绝对路径。
- **`card_path` 只保留给本机桌面路径**：当前 Tauri BusRelay 已经能带 bearer，但后续若要真正加固，应使用更强的本机可信通道（例如 sidecar 启动时生成的非持久 local trust secret、文件选择 token、或 engine 管理的 allowlisted import temp 目录），而不是可伪造的普通 header。
- **M1 不实现 import**：最小 WebUI 先只验证 list/session/chat/history/state；所有浏览器导入推迟到 M3。
- **M3 multipart 分支**：engine 接收上传文件，解析并落库；如需临时文件，只能写入 engine 管理的 `data/_import_tmp/`，解析成功或失败后清理。
- **错误语义**：Web/browser 若提交 `card_path`，返回 `403 Forbidden` 或 `400 Bad Request`，错误 body 明确提示改用 multipart。

### 4.4 鉴权姿态文档化

WebUI 启动时显示当前 engine 的鉴权状态：
- `access_api_key` 未配 → 醒目警告"engine 无鉴权，仅本地 dev 安全"。
- 已配 → 提示输入 bearer token，存 sessionStorage（非 localStorage，关 tab 即清，降 XSS 持久风险）。

**不**在 WebUI 里做 access key 的设置/修改——那扩大了 harness 权限。设置走 engine 环境变量/`/v1/settings`（且 `/v1/settings` 写入需鉴权，鸡生蛋问题用环境变量解）。

---

## 5. 里程碑与验收（可被审计的硬判据）

| 里程碑 | 产出 | 验收判据 |
|---|---|---|
| **M0** | 端点矩阵文档（§2.1 扩全两列：request/response shape + 已知缺口） | 矩阵每行来自源码 `route()` 核实，审计可复查 |
| **M1** | `webui/` 单页 harness（不含 import） | 不经 Tauri，浏览器配真实 provider → 发消息 → 看到流式回复 + 持久化 history；能发起 `/v1/agent/run` 并看到 agent event log |
| **M2** | 可靠性用例集 | §3.1.3 每条失败路径有可见错误解释；成功路径留下可预测文件/state；**两并发 stream 不破坏 id-keyed chat state** |
| **M3** | 浏览器导入安全边界 | 浏览器调用方**不能**令 engine 读任意本地路径（`card_path` 永禁）；multipart 导入成功 |
| **M4** | 证据回灌 | 验证记录写回 `WEBUI-BACKEND-VALIDATION.md`；Tauri UI 产品化基于已验证后端合同 |

每个里程碑独立 PR，不堆一个大 PR。

---

## 6. 工程纪律

- **分支**：`webui-m0-matrix`、`webui-m1-harness`、`webui-m3-import-safety`、`webui-m2-reliability`，各自从 main 切，§11.1 分支制。
- **测试**：engine 改动跑 `cargo test -p airp-core` + 神圣不变式；WebUI 自身是 harness 不写单测（它就是测试工具），但 M2 用例集要可复现。
- **提交卫生**：精准 add，不 `git add -A`；`webui/` 产物不含构建产物（零构建则无产物）。
- **不合并**：每个 PR 等审计 + 人工 review，开发 agent 不自合并（2026-07-04 用户立）。

---

## 7. 剩余风险与默认取舍

1. **`card_path` 本机可信通道仍需后续加固**：本计划先禁止 Web/browser 使用 `card_path`，并把浏览器导入改为 multipart；Tauri 本机 path-first 路径暂按现有本地 sidecar 模型保留。若 engine 未来对外暴露或第三方 widget 能触发 import，必须先做文件选择 token / local trust secret / allowlisted temp dir 之一。
2. **WebUI 是否进 workspace**：默认不进（它是 harness，非 crate）。除非后续引入构建链，否则不进 Cargo/npm workspace。
3. **零构建 vs Vite**：默认 `app.js` 零构建。只有原生 JS 阻碍验证时才引 Vite/TypeScript。
4. **CORS `Any` 是否在 M1 就收紧**：默认不动（本地 dev 够用），文档标记风险。若 M1 需要跨设备访问或对外演示，加 `AIRP_CORS_ORIGIN` 环境变量。
5. **`/health` 端点是否必要**：可做，但不阻塞 M1；`/version` + `/v1/settings` + 一次真实 chat 请求足够启动第一轮验证。
6. **与 PR #15（M_AGENT-2 会话工具）的关系**：PR #15 已合并。WebUI 仍优先调现有 `/v1/sessions/*` HTTP 端点，不依赖 agent 工具层。

---

## 8. 立即执行顺序

1. 开 `webui-m0-matrix` 分支，把 §2.1 矩阵补全（request/response shape + 缺口两列）→ PR。
2. 开 `webui-m1-harness` 分支，落 `webui/` 单页（`index.html` + `app.js` + `style.css`），只做 version/settings/characters list/sessions/chat/history/state/agent-run event log；**不做 import**。`/health` 可同 PR 加，但不是 M1 阻塞项。
3. 开 `webui-m3-import-safety` 分支，落 multipart upload，并保证 Web/browser 无法用 `card_path` 令 engine 读任意本地路径 → PR。
4. 开 `webui-m2-reliability` 分支，跑用例集，证据回填文档 → PR。
5. M4（Tauri 回灌）不属于本计划，留给后续。

---

## 9. 不做的事（显式列出，防审计误以为遗漏）

- 不做 WebUI 的产品化（主题/响应式/无障碍）。
- 不做 WebUI 的鉴权管理 UI（只显示状态 + 收 bearer token；不在浏览器里修改 access key）。
- 不做 WebUI 的 i18n。
- 不做 WebUI 的打包/部署（开发者 `python -m http.server` 或 `npx serve webui/` 起即可）。
- 不动 `protocol/` crate。
- M1 不动 `ui/` 任何文件；M3 也优先不动 `ui/`，除非为了保留 Tauri 本机 path-first 路径必须补本机可信通道。
- 不开第二个 agent 自测入口（守反冗余门禁）。
