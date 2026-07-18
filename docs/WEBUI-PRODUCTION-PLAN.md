# WebUI 正式上线计划

> 状态：当前近期执行主入口
>
> 基线日期：2026-07-18，`main@2a14b7e` / PR #232
>
> 产品目标：继续把现有“基本可用的开发/验证 WebUI”收敛为首聊黄金路径可重复验收的 P1 有限试用版；再推进可升级、可恢复的正式 Web 产品。

P0 的已接受实现合同见 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)。首方 OCI/Compose + Caddy 同源入口、生产配置、鉴权、远端导入边界与 production topology smoke 已实现；P1-P3 仍是正式上线前置，不能把 P0 全绿写成正式发布。

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

> 验证判据来源：[#207 strategic(rp): adopt 3+1 launch-acceptance targets](https://github.com/GhostXia/AIRP/issues/207)。P1/P2/P3 退出条件必须采用可重复证据，不得继续用模糊口号（“更稳”、“好用”）替代；后续 PR 若修改本节 §2.2/§2.3/§2.4 退出条件，必须同步校准 #207。本文将 #207 的 4 个验收目标（首聊黄金路径 / 资产日用可见性 / 恢复升级可测判据 / 干净提示词 + Agent loop 价值验证）物化为以下门禁。

### 2.1 部署与安全

- 有版本化、可重复的首方部署产物；不把 `webui/start.bat`、`cargo run` 或裸 `serve.js` 当生产部署。
- 同源 HTTPS 入口可启动、停止、重启和升级；engine 端口默认不对公网开放。
- 生产模式拒绝无 bearer 启动或给出硬失败；CORS、可信代理、请求体上限、超时和速率限制有明确默认值。
- 远端 WebUI 只能上传 JSON/PNG 内容，不能触发 engine 读取服务器任意 `card_path`。
- WebUI 配置 CSP、`frame-ancestors`、MIME sniffing、referrer 和缓存策略；用户内容、Markdown 与错误文本不能形成脚本注入。
- 发布包与镜像不含 API key、真实聊天数据、测试日志或本机路径。

### 2.2 正式 RP 使用闭环

- 目标用户：自带 provider key、使用 PNG 角色卡 / `character_book` / preset JSON 的 RP 重度玩家，不是首次接触 RP 的新人。首发面向“消费 ST 格式资产的 RP 重度用户”；[SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md) “吸收资产，不继承产品北极星”仍要求 AIRP 使用自己的 domain model、命名、控制流、安全边界与测试。
- **首聊黄金路径**：在不读开发文档、不打开 dev tools / workbench 的前提下，完成部署健康检查 → provider 配置 → 模型验证 → 角色导入 → Persona/Preset 选择 → 首轮对话全闭环。验收必须同时包含：
  - **真实边界**：至少覆盖真实 provider、真实浏览器和 production topology，不以纯 mock 代替端到端边界；
  - **可重复证据**：自动化浏览器测试、CI topology、agent 辅助检查与维护者人工验收可以组合使用，步骤、输入类别、候选版本和结果必须可复核；
  - **失败分类法**：按阶段（部署健康检查 / provider 配置 / 模型验证 / 角色导入 / Persona/Preset 选择 / 首轮对话）× 类型（UX 混淆 / 错误信息不可行动 / 资产格式不兼容 / provider 特定 / 网络 / 崩溃）记录失败并补回归覆盖；
  - **恢复边界**：页面刷新后继续与服务重启后继续分别验收，不能把其中一项通过写成两项都通过。
- **ST 格式资产兼容性**：使用 AIRP 自有或合成的代表性 fixture 覆盖 PNG 角色卡、`character_book` 与 preset JSON 的导入、可见配置和首聊使用；不得复制第三方测试、数据集或资产。
- **当前缺口**：首次启动向导（onboarding wizard）已由 PR #212（[#209](https://github.com/GhostXia/AIRP/issues/209)）落地 Phase 1（6-stage 状态机 + Port 合同 + 动态 import 边界 + fail-open 降级 F1–F4，F5–F6 为向导内重试）；下一步继续补齐真实浏览器自动化、production topology 下的黄金路径、刷新/重启恢复和实际发现的产品缺口。Shadow DOM 隔离与 Port 版本协商跟踪于 [#210](https://github.com/GhostXia/AIRP/issues/210) / [#211](https://github.com/GhostXia/AIRP/issues/211)。
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
- **可复核恢复判据**（#207 目标 3，Phase P2 退出条件必须全部满足，不再使用“更稳”口号）。每条判据须有可执行验收定义，避免不同实现都能宣称满足：
  1. **升级前后根哈希稳定**：升级前对 session 数据根（`characters/` + `sessions/` + `third_party/` + 全局 settings/personas/presets，不含 `target/`、日志、临时文件）计算 `AIRP-TREE-SHA256-v1`（规范化规则：JSON 键排序、CRLF→LF、去除 trailing whitespace、按相对路径排序的文件树叶子哈希聚合）；升级中断后重启，重新计算并比对，根哈希一致 = 数据零丢失。比对时点：升级前快照 vs 升级完成且 engine 重启后。
  2. **损坏行不阻塞**：单条 JSONL 行损坏（截断 / 乱码 / 重复 durable ID / 缺字段）时，session 仍可加载，损坏行跳过并记入 `chat_log_meta.json` 的 `skipped_lines`，剩余 history 完整可读。负向测试矩阵须覆盖：行截断、字段缺失、ID 冲突、编码损坏四类。
  3. **旧版可冷启动**：列出必须支持冷启动的旧版本（当前为 pre-命名-session legacy 角色级数据 + 已命名 session UUID 数据）；冷启动加载时**不得**静默修改 legacy 文件（不重写 `chat_log_meta.json`、不迁移目录、不重生成 ID），只读加载并在 metadata 标注 `legacy=true`；任何修复须显式 migration 命令触发并留 audit log。
  4. **备份恢复 ID 稳定**：备份 → 恢复 → 继续对话后，session 内所有 durable message_id 与备份逐字节匹配（无重新生成、无重映射、无 gap）；稳定性校验 = 恢复后 history 响应的 ID 集合 ⊆ 备份 ID 集合，且新增消息延续原 ID 序列无冲突。
  5. **soft-delete 窗口**：删除角色 / Persona / Preset / 会话后进入可恢复窗口，默认保留 7 天（可配置）；可恢复对象 = 被删资产本身 + 其绑定关系元数据；恢复入口 = WebUI 回收站 UI 或管理员命令；恢复后状态 = 资产回到 active、`deleted_at` 清空、绑定关系复原；窗口过期后物理删除并记 audit log。

### 2.4 发布质量

- 支持当前稳定 Chrome/Edge；移动端可完成核心聊天，不要求首发拥有完整桌面工作台布局。
- 自动化覆盖 engine contract、WebUI DOM、真实浏览器主路径、认证/安全负向路径、升级/恢复和生产部署 smoke。
- 候选版本在生产拓扑下完成全新安装、旧数据升级、备份恢复和长会话 soak；证据写入版本化文档。
- CI 对正式部署产物执行构建、secret scan、依赖/许可证清单和 smoke；只有全部 release gates 通过才打正式 tag。
- **干净提示词 + Agent loop 价值验证**（#207 目标 4，分两层）：
  - **L0 自动化发布门禁**（CI 强制，每次 PR 须产出证据；门禁状态以 CI artifact 为准，不以文档断言为准）：
    1. `subagent_context_has_no_orchestrator_noise` 不变式门禁——Agent 编排脚手架不进入角色 prompt（详见 [PLAN.md §2.1](PLAN.md)）；通过条件 = CI `Rust workspace` job 中该项测试 green；
    2. 本轮 `PromptAssemblyTrace` 可见来源（card / persona / lorebook / state / preset / scene / memory / history / user）——trace 完整性自动回归；通过条件 = CI 中 `prompt_assembly_trace` 相关测试覆盖所有上述 source_kind 且断言非空。
  - **L1 差异化价值证明**（场景证据库，发布候选人工复核，**不强制嵌入首轮对话流程**）。须定义可复现的 pass/fail 标准，避免主观判定：
    1. **证据模板**（每条场景记录须包含）：场景描述 / 触发的 Agent tool 与调用参数 / 用户价值观察（可量化：减少操作步数、启用 otherwise 不可能的能力、或修复否则需手改 JSON 的问题）/ 基线对比（无 Agent loop 时用户须手动完成的等价操作）/ 复核人 / 复核日期 / 可重放脚本或 session 快照路径；
    2. **场景选择规则**：≥3 个真实 RP 场景，且覆盖 ≥2 个不同 Agent tool 类别（如 state 更新类 / preset 调校类 / worldbook 编辑类），避免单工具巧合；
    3. **最低可接受结果**：每条场景须满足“减少操作步数 ≥1”或“启用 otherwise 不可能的能力”之一；基线对比须真实可复现，不得为假设；
    4. **pass/fail 判定**：RC 通过 = 所有场景证据齐全 + 每条满足最低结果 + 复核人签字记录入库；RC 失败 = 证据缺失 / 结果低于阈值 / 干净角色平面不变式缺乏用户价值证明；不强制要求每位用户首轮对话触发 Agent tool——避免 onboarding 扭曲。

## 3. 当前事实与差距

### 已有地基

- 基础 WebUI RP 闭环、provider 设置、角色导入、默认 Persona、Preset 导入/选择、命名会话、SSE 聊天、Agent Run、诊断和错误恢复已存在。
- durable message ID、cursor history、50 条窗口、增量 DOM、rollback-by-ID 已交付。
- engine 已有 loopback 默认、精确 CORS、可选 bearer、限流、统一 outbound redirect policy、typed error 和 `/health`/`/version`。
- `deploy/production/` 已有 digest-pinned OCI build、Compose/Caddy 同源 HTTPS 拓扑、私有 engine 网络、secret bootstrap 和 production WebUI runtime config；CI 会 build 镜像并启动一次性真实拓扑，验证 perimeter auth、私有 engine、CSP/headers、content-only import、三轮增量 SSE、重启持久化、浏览器注入/取消和 secret scan。
- 多 Persona 存储、plural HTTP CRUD、chat pipeline 激活、effective endpoint、WebUI 自动/显式选择和角色/session 绑定/解绑已交付；#114 统一有效配置摘要已交付（Persona 激活来源 + 参数来源 chips）；Persona 高级生命周期、跨资产完整 revision/provenance 合同和完整 Preset 生命周期仍未闭合。
- Worldbook v4 `selective`/`secondary_keys` runtime、v3 presence-aware migration/诊断、普通用户主面板管理、advisory 只读可见性和 PNG/JSON 到最终 prompt 的回归已交付。
- Preset 规范化导入报告、原始输入 sidecar、Agent `get_preset`/`update_preset`（含 dry-run 与确认门控）、不可变版本目录和原子 current 指针已交付；HTTP/UI 受控 dry-run、完整 revision/provenance/collision 合同仍未闭合。
- `PromptAssemblyTrace` 已接入真实 chat pipeline，并交付无写副作用的脱敏 HTTP preview 与 WebUI 本轮配置/有序装配摘要；Phase 2 (#115) 6 类 asset revision（character/persona/preset/lorebook/state/memory）已全部接入统一 `content_revision` 合同（PR #201/#202/#203/#206/#215），新数据 `EffectiveIds` 6 个 `*_revision` 字段填充实际 u64，旧数据推送 `*_revision_unavailable` 诊断并在 WebUI chips 显示 `unavailable`，禁止用 mtime 伪造 revision；#114 统一有效配置摘要已交付（Persona 激活来源 + 参数来源 chips，engine 侧复用 `PersonaService::resolve_effective_persona` 与 HTTP effective 端点同源）；但 base lock / drift / rollback / 受控 dry-run / 完整 provenance 审计仍未交付。
- 命名 session 已统一目录 UUID、history 响应与 metadata 身份；自包含 state/角色卡/worldbook 工作副本、统一 revision、恢复导出仍是分阶段合同。
- PR #232 已收口 P1 代码候选的关键边界：chat finalization 失败关闭、结构化 SSE commit state、ambiguous/partial commit 不盲重发、Persona 删除路径与清理安全、客户端 secret/error 脱敏，以及带哈希/卷校验和 loopback 验证屏障的人工冷备份/回滚逃生路径。

### 尚缺的上线能力

- P0 首方部署 artifact 与真实 topology smoke 已落地，P1 代码候选已形成但黄金路径与可重复验收仍需继续开发；正式升级/自动回滚流程、发布签名及 P1/P2 产品门禁仍缺，因此尚非正式发布。SBOM/notices 已由 PR #218 `tools/dep-governance/` 提供离线生成（SPDX-2.3 / CycloneDX 1.5 / 人类可读声明，当前快照在 `docs/sbom/`），但尚未嵌入 release pipeline 作为强制度量。
- `webui/serve.js` 与 `start.bat` 是开发工具；production runtime config 已改为同源且隐藏 engine URL/bearer，开发模式仍保留手填 harness。
- 认证是“可选 bearer”，不是面向公网的完整登录系统；首发必须由部署层收口为单用户安全入口。
- Persona/Preset/Worldbook 完整资产生命周期与有效配置合同仍有缺口；#115 Phase 2 6 类 revision 合同与 trace 收口已落地，#114 统一有效配置摘要已交付（PR #217），#114 Persona/Preset 高级生命周期（base lock/drift/rollback/dry-run/provenance 审计）后移到 P2；#126 已交付的 v4 runtime、主面板编辑和端到端回归不得重复实现。
- 单资源持久化边界已加固（PR #219/#227/#232：quota race、chat_store 原子性、replace_file fsync/扩展名、character_lock、lorebook 保留、volume 溢出，以及 chat finalization 关键写入失败关闭），但跨资源事务、`AIRP-TREE-SHA256-v1` 完整性校验、版本化 migration registry、自动备份恢复、soft-delete/回收站、完整 production observability contract 和运行手册仍缺；当前 Caddy access log/filter 只是 P0 局部实现，仍需在 P2 决定是否保留及其用途、字段、输出和保留策略；#220 deferred 项（character_lock poison 恢复、record_tokens spawn_blocking、resolve_param_sources 重构、temperature None 语义）后移到 P2。
- engine-truth smoke 与 production system-Chrome smoke 已并行进入 CI；浏览器兼容矩阵、升级恢复与完整发布安全门禁仍不足。

## 4. 推进顺序

### Phase P0：生产地基与威胁边界（已完成）

1. 按 [P0 架构与威胁边界](WEBUI-PRODUCTION-ARCHITECTURE.md) 实现同源反代拓扑、生产配置合同和首方部署形式；
2. 禁止公网直连 engine，增加 `AIRP_DEPLOYMENT_MODE=production` 的启动前 fail-closed 校验并强制 access key；
3. 生产模式关闭远端 `card_path`，只保留内容上传；
4. 增加安全 headers、body/cache policy、secret/logging 约束；
5. 建立 production smoke：HTTPS 入口 → perimeter auth → private engine 负向验证 → health → provider → 三轮聊天 → 刷新/重启恢复。

退出条件已由 PR #132/#133/#135/#136 和 GitHub run `29249333920` 满足：一次性环境可启动真实 HTTPS/Compose 拓扑，浏览器不需要知道 engine 私有地址或 bearer。P0 的局部 access-log filtering 不等于 P2 结构化可观测性已经完成。

### Phase P1：有限试用版（当前唯一近期主线）

P1 不是正式发布，也不以补齐全部资产生命周期为目标。它只验证 AIRP 是否能让少量目标用户安全、连续地完成真实 RP 首聊；“是否愿意再次使用”保留为诊断信息，不作为 P1 退出门槛。

1. 已有地基保持不变：P0 production topology、Vite/Vitest 安全升级、onboarding wizard、Persona effective/绑定、Preset/Worldbook 基础管理、`PromptAssemblyTrace`、有效配置摘要和单资源持久化加固均已交付；
2. 闭合唯一黄金路径：部署健康检查 → provider 配置 → 模型验证 → 角色导入 → Persona/Preset 选择 → 首轮流式对话 → 页面刷新/服务重启后继续当前会话；
3. 修复黄金路径上的认证、provider、网络、超时、SSE、历史恢复和持久化失败，使错误 fail-visible、可行动、可重试，不允许按钮永久 loading 或成功响应掩盖落盘失败；
4. 保持 secret 边界：API key/access key 不进入 URL、WebUI 持久存储、日志、诊断摘要或错误详情；
5. 保持用户资产底线：角色卡、世界书、Persona、Preset、session 和聊天历史不得静默删除或覆盖；关键写入失败不得返回虚假成功；明确 data root，并提供升级前人工备份与恢复旧目录/旧版本的简短操作说明；
6. 冻结横向基础设施：不新增 asset revision 类型、revision chip、provenance 展示、dependency-governance 自动化、桌面功能或新的大型管理面。现有 revision 在 P1 只作为实验性诊断基础，不承诺回滚/恢复能力；
7. 对 branch/swipe/edit、完整 Persona/Preset 生命周期、大对象上传和高级 developer controls 做诚实取舍：未交付能力隐藏、禁用或明确标为非 P1，不为追求表面完整阻塞首聊。

退出条件：

- 用户不编辑磁盘 JSON、不读开发文档、不打开 dev tools/workbench 即可完成黄金路径；
- 刷新页面后和重启服务后均可以继续，并分别记录两项恢复结果；常见网络/认证/provider 失败可重试；
- secret scan 与现有安全负向门禁全绿；
- 针对角色卡、世界书、Persona、Preset、session、聊天历史的 P1 路径，没有已知静默丢失、静默覆盖或虚假成功；
- onboarding → 首聊黄金路径在真实 provider、真实浏览器和 production topology 下可重复通过，每个失败按阶段×类型记录并补回归覆盖；
- P1 候选版明确标注“有限试用”，并提供 data root 与人工备份/恢复说明。

P1 验收记录至少包含候选版本、provider 类别、浏览器/设备、生产拓扑版本、各阶段结果、刷新恢复、重启恢复、失败的阶段×类型和重试结果。自动化、agent 辅助检查与维护者人工验收均可作为证据，但局部 mock 或单元测试不能单独证明端到端通过；任何 secret 泄露、资产损坏、虚假成功或无法恢复的永久 loading 都会阻止 P1 退出。外部反馈若未来自然出现，可用于发现问题和调整优先级，但不构成继续开发或 P1 工程验收的前置条件。

### Phase P2：数据可靠性与恢复

1. 完成 #114 剩余高级生命周期：Persona base lock、drift/history/rollback、头像、导入导出/备份恢复，完整 Preset 生命周期与 HTTP/UI 受控 dry-run；
2. 先选择一种高价值资产完成“工作副本/读取真值 → revision 完整性校验 → 差异比较 → 回滚 → 恢复 → 导出”的端到端 vertical slice；验证后再决定是否横向扩展其余 asset revision，不以新增 chip 代替恢复能力；
3. 版本化 migration registry 与启动前 dry-run/备份；
4. 备份、恢复、导出和可恢复删除；
5. 并发写、磁盘满、损坏 JSON、升级中断和旧版本回退测试；
6. readiness、结构化脱敏日志、诊断包和运维 runbook。

退出条件：§2.3“可复核恢复判据”5 条全部满足（升级前后根哈希稳定 / 损坏行不阻塞 / 旧版可冷启动 / 备份恢复 ID 稳定 / soft-delete 窗口），并以自动化或可复核证据呈现，不再使用“更稳”作为发布口号。Phase 2 (#115) 6 类 `content_revision` 可见性已落地，但 `AIRP-TREE-SHA256-v1` 完整性校验与完整 provenance 链路（source / converted / preserved / unsupported / invalid / needs-review）用户可读仍是本阶段验证项，退出条件对齐 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md) 自包含 session 合同。

### Phase P3：发布候选与上线

1. 浏览器兼容、响应式、键盘与基础可访问性收口；
2. 真实 provider 脱敏 smoke、长会话与断网/重连 soak；
3. 构建 SBOM/许可证清单、版本信息、发布说明和校验值（PR #218 `tools/dep-governance/` 已提供 SPDX-2.3 / CycloneDX 1.5 离线生成器，需嵌入 release pipeline 并补发布签名）；
4. RC 全新安装、升级、备份恢复、回滚四类演练；
5. tag 正式版并保留已知限制与回滚路径。

退出条件：§2 全部有自动化或可复核证据，不以“本机能跑”代替正式发布证明。本阶段额外要求 §2.4“干净提示词 + Agent loop 价值验证”两层全部满足：L0 自动化门禁（`subagent_context_has_no_orchestrator_noise` 不变式 + `PromptAssemblyTrace` source_kind 完整性，含 persona）CI artifact green；L1 场景证据集（≥3 真实 RP 场景、≥2 工具类别、按证据模板记录、复核人签字）按 §2.4 L1 pass/fail 判定通过。

## 5. 非首发阻塞项

- Persona/Preset 高级生命周期、完整 revision/provenance/collision 审计、自动 backup/restore、migration registry 与 soft-delete；它们进入 P2，不阻塞 P1 有限试用。
- 新增 asset revision 类型/chip、dependency-governance 自动升级与 GitHub 写入自动化、SBOM release gate 和发布签名；它们不得抢占 P1 黄金路径。
- #220 中不直接造成 secret 泄露、虚假成功或用户资产损坏的性能、锁 poison 一致性、代码组织与默认值展示改进。
- #117 ChangeInbox、#87 Agent-first 工作台、#116 Style Review；它们在核心 WebUI 正式版之后继续推进。
- MCP upstream、skills/plugin marketplace、可配置多 Agent 编排。
- Tauri 安装包、sidecar 生命周期与 100k 桌面虚拟列表验收。
- 多租户账户、云同步、团队协作、计费与公共 SaaS 运维。

这些能力不得消失，但不能再抢占 WebUI 正式上线主链。

## 6. 下一批可执行工作

1. WebUI production umbrella issue 为 [#130](https://github.com/GhostXia/AIRP/issues/130)；P0-P3 在其中按独立验收切片追踪；
2. P1 代码候选已形成；第一优先级是继续开发首聊黄金路径，并用真实 provider + 真实浏览器 + 生产拓扑建立逐阶段可重复验收，分别记录刷新恢复与服务重启恢复；
3. data root、升级前人工冷备份和恢复旧目录/旧版本的 P1 逃生说明已交付；现在补齐真实浏览器自动化与 production topology 黄金路径，修复首聊阻塞、永久 loading、不可行动错误、secret 泄露、虚假成功和关键资产静默损坏；
4. #114/#115/#190/#192 的关闭状态与原验收项存在偏差，按 [#231](https://github.com/GhostXia/AIRP/issues/231) 核对后重新打开或拆分剩余项；该流程修正不阻塞 P1 代码主线，但必须在宣称对应 umbrella 完成前完成；
5. Persona/Preset 高级生命周期、完整 revision 恢复 vertical slice、#220 非阻塞项、工程治理自动化、release pipeline 强制度量与发布签名后移到 P2/P3；
6. [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 只在能力边界、执行顺序或 release batch 发生实质变化时统一校准；不再要求每个小 PR 单独创建 baseline recalibration。
