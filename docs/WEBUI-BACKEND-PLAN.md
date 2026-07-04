# 后端建设计划书（为 WebUI 验证面做准备）

> **作者**：AtomCode (GLM-5.2)，2026-07-04
> **状态**：待审计（用户指示：先送审计 bot 审，通过后再开工）
> **依据**：DEV-GUIDE §3.3/§3.7 + [WEBUI-BACKEND-VALIDATION.md](WEBUI-BACKEND-VALIDATION.md) + [CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md) §3 + 源码核实（`engine/src/daemon/mod.rs`、`handlers.rs`）
> **与既有约束的关系**：本计划只动 `engine/` 后端 + 新增 webui 子目录；**不**改 Tauri UI 产品线、**不**碰 `ui/src/agent-test.ts` 单一测试入口、**不**放开 RR-001 `card_path` 任意路径读护栏。

---

## 1. 目标与定位（先定性，避免范围爬升）

**WebUI 是临时后端可靠性验证 harness，不是产品 UI。**（DEV-GUIDE §3.3 L130、WEBUI-BACKEND-VALIDATION.md §1）

它回答一个问题：**后端能否稳定跑通最小 RP 闭环，让 UI 不再遮住后端不确定性？**

- 长期产品 UI = Tauri/Vue 桌面端（不变）。
- 短期验证面 = 浏览器 WebUI / HTTP harness。
- 稳定核心 = `engine` 是独立 HTTP/SSE 服务，不嵌 Tauri（现状已如此）。

**退出条件**（WEBUI-BACKEND-VALIDATION.md §7）：不经过 Tauri 也能从浏览器复现后端 chat streaming；data persistence / 鉴权 / 错误 / 并发 stream 可观察；浏览器导入不依赖可信本地 `card_path`。达成后 WebUI 可降为 developer-only 诊断页或删除，**不成为默认产品面**。

---

## 2. 现状基线（源码核实，非文档抄录）

### 2.1 端点矩阵（来自 `engine/src/daemon/mod.rs:191-238` 的 `Router::new().route(...)` 枚举）

共 21 个端点 + `/version`：

| 端点 | Method | 鉴权 | SSE | data 副作用 | 备注 |
|---|---|---|---|---|---|
| `/version` | GET | **否** | 否 | 无 | 在 `v1_routes` 外层，不走 `auth_middleware`（`test_audit_10` 守护） |
| `/v1/chat/completions` | POST | 是 | **是** | chat log append | 核心；`Sse::new(build_sse_stream)` |
| `/v1/agent/run` | POST | 是 | **是** | chat log + agent step | M_AGENT-1 多步 loop |
| `/v1/chat/history` | POST | 是 | 否 | 读 |  |
| `/v1/chat/rollback` | POST | 是 | 否 | chat log truncate | destructive |
| `/v1/chat/regen` | POST | 是 | 否 | chat log rewrite |  |
| `/v1/characters` | GET | 是 | 否 | 读 |  |
| `/v1/characters/import` | POST | 是 | 否 | 写 `data/characters/` | **10MB body limit**；`card_path` 风险见 §4.3 |
| `/v1/characters/:id/reextract` | POST | 是 | 否 | 重写 assets |  |
| `/v1/characters/:id/avatar` | GET | 是 | 否 | 读 |  |
| `/v1/characters/:id/state` | GET | 是 | 否 | 读 |  |
| `/v1/characters/:id/state/history` | GET | 是 | 否 | 读 |  |
| `/v1/characters/:id/state/schema` | GET | 是 | 否 | 读 |  |
| `/v1/scenes` | GET/POST | 是 | 否 | POST 写 |  |
| `/v1/scenes/:id` | GET | 是 | 否 | 读 |  |
| `/v1/scenes/:id/characters` | POST | 是 | 否 | 写 |  |
| `/v1/models` | GET | 是 | 否 | 读 |  |
| `/v1/presets` | GET | 是 | 否 | 读 |  |
| `/v1/presets/:id` | GET | 是 | 否 | 读 |  |
| `/v1/sessions/:character_id` | GET/POST | 是 | 否 | POST 创建 session 目录 |  |
| `/v1/settings` | GET/POST | 是 | 否 | POST 写 `config.json` | 热重载 |

### 2.2 安全姿态（源码核实）

- **鉴权**（`mod.rs:110-135`）：`access_api_key` 为 `None` 或空串 → **全放行**；非空 → 要求 `Authorization: Bearer <key>`，常数时间比较。**默认无鉴权**——本地 sidecar 可接受，对外暴露必须设 `AIRP_ACCESS_KEY`。
- **CORS**（`mod.rs:173-176`）：`allow_methods/headers/origin = Any`。浏览器 WebUI 直连无 CORS 阻碍；但也是 DNS-rebind/CSRF 攻击面（DEV-GUIDE §3.3 L131 记录此坑）。
- **限流**（`mod.rs:178-189`）：governor 覆盖**所有** `/v1/*`（A2-7 已修，非仅 chat）；key = Bearer token（已鉴权）或 peer IP。
- **Body limit**：仅 `/v1/characters/import` 显式 `DefaultBodyLimit::max(10MB)`；其余走 axum 默认。

### 2.3 已具备的后端能力（无需新建，WebUI 直接调）

- 真实酒馆卡 PNG 解析（`png_parser`）、path-first 导入（`card_path`）。
- chat 流式 SSE（`chat_pipeline::build_sse_stream`）、id-keyed chat state（PR #6）。
- sessions/scenes/presets/state CRUD、history/rollback/regen。
- agent 多步 loop（M_AGENT-1 骨架，PR #15 补会话工具，待合并）。
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
   - `/v1/characters` 列表 + fixture 导入
   - `/v1/sessions/:id` 列/建
   - `/v1/chat/completions` 发消息 + SSE 流式 transcript 渲染
   - `/v1/chat/history` / `rollback` / `regen`
   - `/v1/characters/:id/state` + `state/history`
   - `/v1/agent/run` 表单
3. **可靠性检查**（M2）：API key 缺失/错误、model 错误、provider timeout/SSE 断流、bearer 缺失、**两个并发 chat stream**、刷新页面、regen/rollback 后置状态。
4. **数据安全边界**（M3）：浏览器导入走 multipart upload + engine 受控临时目录，**禁用 `card_path`**。
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
│   ├── app.ts            浏览器原生 TS（无框架，harness 不配享受 Vue）
│   ├── style.css
│   └── README.md         启动方式 + 退出条件
└── docs/
    └── WEBUI-BACKEND-VALIDATION.md  ← 已存在，验证证据回填至此
```

**技术选型理由**：无框架、无构建链——harness 越薄越好，避免它自己变成要维护的产品。用浏览器原生 ES modules + `fetch` + `EventSource`（SSE）。审计若认为该用 vite 静态服务器，可改，但默认零构建。

### 4.2 engine 侧需要的最小改动

WebUI 优先复用现有端点。**仅当现有端点不足以验证时**才改 engine，且改动必须不破坏 Tauri 路径。预判需要的改动：

1. **`/health`（或 `/readyz`）端点**：返回 `{ "engine": "ok", "provider_configured": bool, "data_root_writable": bool }`。**新增**，不碰现有 `/version`。理由：WebUI 要区分"engine 起了"与"能跑对话"。
2. **CORS 收紧选项**：当前 `Any` 对本地 dev 够用，但若 WebUI 跨机访问（如手机浏览器访问桌面 engine）需配 `AIRP_CORS_ORIGIN` 环境变量。**可选**，MVP 不做，文档标记。
3. **`/v1/characters/import` multipart 支持**：当前接受 `card_path`/`card_json`/`card_png_base64`。为 WebUI 加 `multipart/form-data` 入参（文件上传直读，不落临时盘或落受控临时目录）。**这是 §4.3 的落地动作。**

**不**改的：`/v1/chat/completions` 契约、SSE 事件格式、state 模型、agent loop——这些 WebUI 适配即可。

### 4.3 数据安全边界（M3，硬约束，对应 RR-001）

`card_path` = 引擎侧任意绝对路径读。当前单本地可信 UI 豁免；**浏览器调用方不可信**，必须禁用 `card_path` 分支。

**方案**：`/v1/characters/import` handler 增加调用方可信度判定——
- **判定信号**：新增 `X-AIRP-Client-Trust: local` 头，仅 Tauri sidecar 启动时引擎自带（或 Tauri BusRelay 转发时注入）。WebUI 不发此头 → 走 multipart 分支。
- **行为**：无该头 + 请求是 `card_path` 模式 → `403 Forbidden`，错误信息引导改用 multipart。
- **multipart 分支**：engine 接收上传文件 → 写入 `data/_import_tmp/` 受控临时目录 → 当作 `card_path` 走原有解析 → 解析成功后删临时文件。
- **不**破坏 Tauri 路径：Tauri BusRelay 转发 import 时加该头即可（一处改动，`ui/src-tauri/src/bus.rs` 的 `import_character_via_path`）。

> **审计点**：此判定信号是"自带头"非密码学鉴权——任何能发请求者都能伪造该头。这是否足够？我的判断：**不够**。`card_path` 任意路径读是高危，自带头可伪造=门控失效。**更稳方案**：`card_path` 分支改为**仅在 `access_api_key` 已配置且请求通过 bearer 鉴权**时开放——即"已鉴权 = 可信"。WebUI 若带 bearer 也可用 path，但这要求 WebUI 用户持有 access key（合理：能配 key 的就是可信操作者）。**本计划书提两个方案供审计裁定**，我倾向后者。

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
| **M1** | `webui/` 单页 harness | 不经 Tauri，浏览器配真实 provider → 发消息 → 看到流式回复 + 持久化 history |
| **M2** | 可靠性用例集 | §3.1.3 每条失败路径有可见错误解释；成功路径留下可预测文件/state；**两并发 stream 不破坏 id-keyed chat state** |
| **M3** | 浏览器导入安全边界 | 浏览器调用方**不能**令 engine 读任意本地路径（`card_path` 被禁）；multipart 导入成功 |
| **M4** | 证据回灌 | 验证记录写回 `WEBUI-BACKEND-VALIDATION.md`；Tauri UI 产品化基于已验证后端合同 |

每个里程碑独立 PR，不堆一个大 PR。

---

## 6. 工程纪律

- **分支**：`webui-m0-matrix`、`webui-m1-harness`、`webui-m2-reliability`、`webui-m3-import-safety`，各自从 main 切，§11.1 分支制。
- **测试**：engine 改动跑 `cargo test -p airp-core` + 神圣不变式；WebUI 自身是 harness 不写单测（它就是测试工具），但 M2 用例集要可复现。
- **提交卫生**：精准 add，不 `git add -A`；`webui/` 产物不含构建产物（零构建则无产物）。
- **不合并**：每个 PR 等审计 + 人工 review，开发 agent 不自合并（2026-07-04 用户立）。

---

## 7. 风险与待决项（请审计重点看）

1. **`card_path` 可信度判定信号**（§4.3）：自带头 vs 已鉴权即可信——**请审计裁定**。我倾向"已鉴权即可信"。
2. **WebUI 是否进 workspace**：默认不进（它是 harness，非 crate）。若审计认为该进 Cargo workspace 统一版本，可改。
3. **零构建 vs vite**：我选零构建（薄 harness）。若审计认为 SSE/TS 类型支持不够，可引 vite。
4. **CORS `Any` 是否在 M1 就收紧**：默认不动（本地 dev 够用），文档标记风险。若审计认为 M1 必须收紧，加 `AIRP_CORS_ORIGIN` 环境变量。
5. **`/health` 端点是否必要**：我主张加（就绪探针）。若审计认为 `/version` 够用，可省。
6. **与 PR #15（M_AGENT-2 会话工具）的关系**：本计划不依赖 PR #15 合并——WebUI 调的是现有 `/v1/sessions/*` HTTP 端点，不调 agent 工具层。两者并行不冲突。

---

## 8. 立即执行顺序（审计通过后）

1. 开 `webui-m0-matrix` 分支，把 §2.1 矩阵补全（request/response shape + 缺口两列）→ PR。
2. 开 `webui-m1-harness` 分支，落 `webui/` 单页 + engine `/health`（若审计同意）→ PR。
3. 开 `webui-m3-import-safety` 分支（提前于 M2，因安全边界优先），落 multipart + `card_path` 门控 → PR。
4. 开 `webui-m2-reliability` 分支，跑用例集，证据回填文档 → PR。
5. M4（Tauri 回灌）不属于本计划，留给后续。

---

## 9. 不做的事（显式列出，防审计误以为遗漏）

- 不做 WebUI 的产品化（主题/响应式/无障碍）。
- 不做 WebUI 的鉴权管理 UI（只显示状态 + 收 bearer token）。
- 不做 WebUI 的 i18n。
- 不做 WebUI 的打包/部署（开发者 `python -m http.server` 或 `npx serve webui/` 起即可）。
- 不动 `protocol/` crate。
- 不动 `ui/` 任何文件（除 §4.3 BusRelay 注入 trust 头的一处，若审计同意该方案）。
- 不开第二个 agent 自测入口（守反冗余门禁）。
