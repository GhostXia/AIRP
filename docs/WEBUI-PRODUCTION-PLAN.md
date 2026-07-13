# WebUI 正式上线计划

> 状态：当前近期执行主入口
>
> 基线日期：2026-07-13
>
> 产品目标：把现有“基本可用的开发/验证 WebUI”推进为普通用户可持续日用、可部署、可升级、可恢复的正式 Web 产品。

P0 的已接受实现合同见 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)。该文档锁定了首方 OCI/Compose + Caddy 同源入口、生产配置、鉴权与远端导入边界；它是设计事实，不表示部署产物或 production smoke 已经交付。

## 1. 首发边界

首个正式版本采用**单实例、自托管、单用户**拓扑：

```text
Browser
  -> same-origin HTTPS reverse proxy
       -> static WebUI
       -> private AIRP engine (/v1/*, /health, /version)
            -> persistent data root
            -> configured model provider
```

- 浏览器只访问一个 HTTPS origin；不要求用户手工拼 engine URL、CORS origin 或 bearer。
- engine 保持 loopback/容器私网监听，不直接暴露公网；反向代理是唯一入口。
- 生产部署必须启用强随机 `AIRP_ACCESS_KEY`，由可信部署层注入；provider secret 只进 engine runtime，不进 WebUI 持久存储、URL、日志或诊断摘要。
- 首发不是多租户 SaaS：不承诺注册、团队空间、管理员后台、计费或跨用户隔离。若未来扩为多用户，须先另立身份、授权、配额与数据隔离设计。
- Tauri/Vue 仍保留为长期桌面客户端，但不再是 WebUI 正式上线的前置 release gate。

## 2. “正式上线”唯一判据

以下门槛全部满足前，只能称开发版或预览版。

### 2.1 部署与安全

- 有版本化、可重复的首方部署产物；不把 `webui/start.bat`、`cargo run` 或裸 `serve.js` 当生产部署。
- 同源 HTTPS 入口可启动、停止、重启和升级；engine 端口默认不对公网开放。
- 生产模式拒绝无 bearer 启动或给出硬失败；CORS、可信代理、请求体上限、超时和速率限制有明确默认值。
- 远端 WebUI 只能上传 JSON/PNG 内容，不能触发 engine 读取服务器任意 `card_path`。
- WebUI 配置 CSP、`frame-ancestors`、MIME sniffing、referrer 和缓存策略；用户内容、Markdown 与错误文本不能形成脚本注入。
- 发布包与镜像不含 API key、真实聊天数据、测试日志或本机路径。

### 2.2 正式 RP 使用闭环

- 首次启动向导完成部署健康检查、provider 配置、模型验证、角色导入、Persona/Preset 选择与首轮对话。
- Persona/Preset 具备可理解的管理、选择和有效配置摘要；角色/会话切换不会静默改变或丢失绑定。
- 角色卡、世界书、Preset、Persona、会话和聊天历史的关键 CRUD 不依赖开发工作台或手写 JSON。
- 连续流式聊天、停止、重试、regen、rollback、历史分页和刷新恢复可稳定日用；branch/swipe/edit 若首发不交付，UI 必须诚实隐藏或标明不可用。
- provider、认证、超时、断流、revision conflict、数据损坏和资源不存在均返回可行动错误。

### 2.3 数据可靠性与运维

- data root 明确持久化；升级前可备份，升级后可验证，失败可恢复到上一版本与上一份数据。
- schema/data migration 版本化、幂等、可测试；不得靠启动时静默覆盖损坏文件“修复”。
- 删除角色、Persona、Preset 和会话采用确认与可恢复策略，或明确记录不可逆边界。
- `/health` 区分 live/readiness；日志结构化、脱敏并有上限，能定位启动、provider、SSE、持久化和迁移错误。
- 提供管理员可执行的备份、恢复、诊断和版本信息流程，不要求阅读源码。

### 2.4 发布质量

- 支持当前稳定 Chrome/Edge；移动端可完成核心聊天，不要求首发拥有完整桌面工作台布局。
- 自动化覆盖 engine contract、WebUI DOM、真实浏览器主路径、认证/安全负向路径、升级/恢复和生产部署 smoke。
- 候选版本在生产拓扑下完成全新安装、旧数据升级、备份恢复和长会话 soak；证据写入版本化文档。
- CI 对正式部署产物执行构建、secret scan、依赖/许可证清单和 smoke；只有全部 release gates 通过才打正式 tag。

## 3. 当前事实与差距

### 已有地基

- 基础 WebUI RP 闭环、provider 设置、角色导入、默认 Persona、Preset 导入/选择、命名会话、SSE 聊天、Agent Run、诊断和错误恢复已存在。
- durable message ID、cursor history、50 条窗口、增量 DOM、rollback-by-ID 已交付。
- engine 已有 loopback 默认、精确 CORS、可选 bearer、限流、统一 outbound redirect policy、typed error 和 `/health`/`/version`。
- `deploy/production/` 已有 digest-pinned OCI build、Compose/Caddy 同源 HTTPS 拓扑、私有 engine 网络、secret bootstrap 和 production WebUI runtime config；CI 会 build 镜像、展开 Compose 并验证 Caddy 配置。
- PR #127 已建立 schema v2 多 Persona 存储、legacy/default 协调、revision、绑定与路径校验；WebUI/API 仍未形成完整多 Persona 生命周期。

### 尚缺的上线能力

- 首方部署 artifact 已落地，但尚缺真实 topology smoke、正式升级/回滚流程、SBOM/notices 与发布签名，因此仍是 P0 preview。
- `webui/serve.js` 与 `start.bat` 是开发工具；production runtime config 已改为同源且隐藏 engine URL/bearer，开发模式仍保留手填 harness。
- 认证是“可选 bearer”，不是面向公网的完整登录系统；首发必须由部署层收口为单用户安全入口。
- Persona/Preset/Worldbook 管理与有效配置合同未闭合；#114/#115/#126 仍是 RP 首发主链。
- 缺备份/恢复、数据迁移发布纪律、soft-delete/回收站、生产日志和运行手册。
- smoke 以 engine truth 为主；真实浏览器、生产拓扑、升级恢复与安全负向门禁不足。

## 4. 推进顺序

### Phase P0：生产地基与威胁边界

1. 按 [P0 架构与威胁边界](WEBUI-PRODUCTION-ARCHITECTURE.md) 实现同源反代拓扑、生产配置合同和首方部署形式；
2. 禁止公网直连 engine，增加 `AIRP_DEPLOYMENT_MODE=production` 的启动前 fail-closed 校验并强制 access key；
3. 生产模式关闭远端 `card_path`，只保留内容上传；
4. 增加安全 headers、body/cache policy、secret/logging 约束；
5. 建立 production smoke：HTTPS 入口 → perimeter auth → private engine 负向验证 → health → provider → 三轮聊天 → 刷新/重启恢复。

退出条件：全新环境按文档一次部署成功，浏览器不需要知道 engine 私有地址或 bearer。

### Phase P1：RP 正式使用面

1. 完成 #114 的 Persona/Preset 管理、选择、绑定和有效配置摘要；
2. 完成 #115 中首发需要的 import report、dry-run/revision 与 PromptAssemblyTrace 摘要；
3. 完成 #126 的 constant worldbook/shared normalization，补齐普通用户可操作的世界书管理；
4. 清理开发诊断控件与日用操作的混杂，把高级工具放入明确的 developer mode；
5. 对 #37 的 branch/swipe/edit 做首发取舍并形成显式合同。

退出条件：用户不编辑磁盘 JSON、不打开开发工作台也能完成角色与 RP 配置的日常生命周期。

### Phase P2：数据可靠性与恢复

1. 版本化 migration registry 与启动前 dry-run/备份；
2. 备份、恢复、导出和可恢复删除；
3. 并发写、磁盘满、损坏 JSON、升级中断和旧版本回退测试；
4. readiness、结构化脱敏日志、诊断包和运维 runbook。

退出条件：旧数据升级失败时不丢数据，管理员能用文档化命令恢复服务。

### Phase P3：发布候选与上线

1. 浏览器兼容、响应式、键盘与基础可访问性收口；
2. 真实 provider 脱敏 smoke、长会话与断网/重连 soak；
3. 构建 SBOM/许可证清单、版本信息、发布说明和校验值；
4. RC 全新安装、升级、备份恢复、回滚四类演练；
5. tag 正式版并保留已知限制与回滚路径。

退出条件：§2 全部有自动化或可复核证据，不以“本机能跑”代替正式发布证明。

## 5. 非首发阻塞项

- #117 ChangeInbox、#87 Agent-first 工作台、#116 Style Review；它们在核心 WebUI 正式版之后继续推进。
- MCP upstream、skills/plugin marketplace、可配置多 Agent 编排。
- Tauri 安装包、sidecar 生命周期与 100k 桌面虚拟列表验收。
- 多租户账户、云同步、团队协作、计费与公共 SaaS 运维。

这些能力不得消失，但不能再抢占 WebUI 正式上线主链。

## 6. 下一批可执行工作

1. WebUI production umbrella issue 为 [#130](https://github.com/GhostXia/AIRP/issues/130)；P0-P3 在其中按独立验收切片追踪；
2. P0 架构/威胁模型已由 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md) 锁定；engine production-mode fail-closed 与 `deploy/production/` OCI/Compose/Caddy artifact 已进入实现，但不等于部署已上线；
3. 下一项是在该 artifact 上完成真实 production topology smoke：HTTPS/perimeter auth、私有 engine 负向、headers/CSP、content upload、三轮 SSE、刷新/重启恢复与 secret scan；
4. 然后按 #114/#115/#126 完成 RP 使用面，不再先做 #117/#87/#116；
5. 每个 PR 更新 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)，区分“已交付”与“下一步”。
