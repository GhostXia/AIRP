# AIRP 当前开发基线

本次增量校准（2026-08-30，#632 candidate）：Agent control-plane planner 不再序列化 raw `ToolResult.output` 或原始错误正文；provider 只接收工具显式、默认关闭的 planner projection 和 evidence preview，经统一脱敏、Unicode 安全截断、item/byte/token 限额后编码为 `airp.planner-observations.v1`。planner-only facts 与 final generation evidence 分离，选择只接受本次 wire 可见的 opaque evidence ID，tool/call/result identity 可贯通到最终证据 provenance。工具显式声明 planner result mode；未投影的 readonly 工具不向 planner 广告，mutating 工具则明确暴露 outcome-only/projected 合同。`get_character_state → update_character_state` 与 `list_world_events → trigger_world_event` 两条真实链验证 projected read 可驱动后续操作。未显式投影的内建或插件工具结果保持 local-only。

> 基线日期：2026-08-30
> 代码基线：#577 PR 10f-2 已由 PR #624 合并；PR 11a implementation 已由 PR #627 合并，产品切片仍为 candidate；本文记录其当前重复实例预算合同
> 用途：冷启动开发、审计和产品判断的第一事实入口。  
> 真理顺序：当前源码、manifest、测试与可重复运行证据 > 本文 > 专题合同 > 路线图/研究材料 > 历史归档。

本文只记录当前代码树能够支持的结论。GitHub issues 是未完成工作的实时追踪面；PR、审计报告和历史测试数字只证明对应代码树，不自动证明当前 `HEAD`。

本次增量校准（2026-08-12，#564 PR 1）：Owner 已重新启动协议驱动的 Vue Blueprint/Widget 桌面主面开发，目标架构、资产处置、基线与回退边界见 [`plans/2026-08-12-issue-564-desktop-architecture-baseline.md`](plans/2026-08-12-issue-564-desktop-architecture-baseline.md)。**该阶段运行事实尚未改变**：正式产品主面仍是 `webui/`，Tauri 仍默认承载该 WebUI；当时 Blueprint v2、`/desktop/`、Surface API 与 `HttpEngineBus` 均未交付。

本次增量校准（2026-08-13，#564 PR 2）：Blueprint/Surface Protocol v2 的协议层已建立，机器 authority 为 [`protocol/surface-protocol-v2.json`](../protocol/surface-protocol-v2.json)，Rust/TypeScript guard、双向 fixture、显式 v1 migration 与原子 last-known-good/resync store 已有测试。**该阶段运行事实仍未切换**：Engine Surface endpoint、`HttpEngineBus`、renderer、`/desktop/` 和真实 Widget 工作流当时仍属后续 PR，正式产品主面和 Tauri 默认入口仍是 `webui/`。

本次增量校准（2026-08-21，#564 PR 4）：`ui/` 已具备受限但可运行的 Surface v2 客户端运行时，覆盖递归 `split/tabs/stack/widget` 渲染、原子 snapshot/patch 应用、稳定 Widget relocation、生命周期与局部错误隔离、5000 条消息虚拟滚动，以及浏览器 runtime smoke。PR gate 运行 `smoke:runtime` 并保存截图与 `runtime-evidence.json`。**该阶段产品入口仍未切换**：Engine Surface endpoint 当时仍属 PR 5；`HttpEngineBus`、`/desktop/` 和真实 Engine 垂直链路继续属于后续 PR，正式产品主面和 Tauri 默认入口仍是 `webui/`。

本次增量校准（2026-08-21，#564 PR 5）：Engine 已提供 bearer 保护的 session Surface snapshot/SSE，只读投影 Chat、Memory、Character State 与 Activity，使用有界 replay ring、opaque cursor、过期/前一 boot snapshot resync、多用户有效根隔离和 `protocol/surface-sse-events.json` 机器合同。关键 chat/Agent 失败以封闭脱敏回执保留，reload/resync 后仍可见；Activity 不进入角色 prompt。**真实桌面链路仍未接通**：`HttpEngineBus`、`/desktop/`、Tauri 双入口与 Widget intent 写闭环属于 PR 6–9，正式产品主面仍是 `webui/`。

本次增量校准（2026-08-23，#564 PR 6）：Vue 已通过同源 `HttpEngineBus` 消费 bearer 保护的 Surface snapshot/SSE；认证流使用 streaming fetch、动态 bearer、401 rotation、成功应用后才推进 opaque cursor，并在断流时 replay、协议/patch 失败时确定性 snapshot resync。Engine 同时承载旧 WebUI `/` 与 Vue `/desktop/`，Tauri 默认仍走 `/`，仅 `AIRP_DESKTOP_UI=blueprint` 选择 `/desktop/`，bundle 缺失时可见回退。浏览器和 Tauri 使用同一 REST+SSE Bus，未恢复 Tauri 业务 relay。**写闭环仍未交付**：Widget intent executor 与第三方 host parity 属于 PR 7–9，Vue 当前真实 Surface 为只读迁移入口。

本次增量校准（2026-08-24，#564 PR 7 candidate）：Vue WidgetHost 已消费 Engine 权威 catalog/grants/plugins，并对 catalog major、capability 封闭集、manifest、sandbox 与 trusted-plugin 声明 fail-closed；生产第三方 ESM 不再进入宿主进程，只能在 `sandbox="allow-scripts"`、无 `allow-same-origin` 的 opaque iframe 中运行。WebUI 与 Vue bridge 均以 iframe window、随机 bridge session、稳定 instance id 校验消息，销毁后释放监听；真实 Engine Chrome smoke 验证 digest-pinned source、授权 capability 投影以及 iframe 无法读取 bearer/sessionStorage/宿主 DOM。固定 slot plan 仍只服务 WebUI，不进入 Blueprint v2。**写闭环仍未交付**：当前 Engine Surface 保持只读，Chat/Memory/Character State executor 属于 PR 8–9。

本次增量校准（2026-08-24，#564 PR 8，PR #590 已合并）：Engine 新增 `core.chat` 专用 `/v1/ui/intents` 可信执行边界，从已接受 Surface 反查有效数据根、角色、会话、用户作用域与 Widget 类型，拒绝缺失、歧义、错实例和未知 intent；执行复用既有 Chat pipeline/Coordinator。Vue `/desktop/` 支持 session 选择/创建、历史分页、send SSE、stop、regen、continue 与 swipe，流式文本只保留为临时视图并由下一份 canonical Surface revision 收敛。Memory/Character State/Activity 与第三方 Widget 仍未开放写执行器；真实 provider 手工验收仍是发布边界。

本次增量校准（2026-08-25，#589 recovery/evidence，PR #591 已合并）：Vue 将无终态 SSE EOF 与结果未知的 Chat mutation 明确归入 reconciliation，绝不自动重放；仅在稳定 message ID 投影确认提交后清除临时状态，Engine 报 `not_committed` 或连续两次 fresh idle snapshot 均确认权威历史未变时才开放显式重试，部分/不明提交和 `recovering` 均 fail closed。真实 Engine + packaged Vue + Chrome smoke 以运行时生成的 5,000 条 durable JSONL 验证首屏最近 50 条、cursor 再取 50 条、虚拟 DOM 有界、生成时跟随最新，以及 Chat Surface patch 不重建 Memory/Character State/Activity host。该证据不替代真实 provider 人工验收。

本次增量校准（2026-08-25，#589 stable context，PR #592 已合并）：Engine Chat Surface 追加只读 `context` 投影，Character/Session 来自已接受 Surface scope；user scope 下的 Persona 复用与 Chat pipeline 相同的 session binding → character binding → default 解析并对歧义 fail closed；canonical 角色世界书仅以 path-independent `character:<id>` source ID 投影。Vue 用可访问、保留完整 ID、短视口内自行横向滚动的 chips 呈现真实存在项。chips 是最近一次已接受 Surface 投影的**当前观察值**，不是跨 Surface→intent 的冻结快照或版本锁；若绑定或世界书在投影后变化，Chat pipeline 会在执行时重新解析、读取并校验当时的权威数据。当前 Surface executor 固定 `scene_id=None`，仓库也没有 session→Scene 持久绑定，因此不伪造 Scene chip；该数据合同仍是 #589 开放边界。

本次增量校准（2026-08-26，#593 PR 13 candidate）：Memory Surface 现在明确投影 session-scoped、未分类的 `resident.md` 内容、字符数、容量、exact SHA-256 hash 与来源；`memory.replace` 在既有 memory mutation lock 内执行 hash CAS 和容量检查，冲突不自动重放。Character State Surface 投影 character-scoped state、domain revision、更新时间与来源；`characterState.patch` 只接受有界顶层 add/replace/remove，在既有 character/state locks 内执行 revision CAS、schema 校验与提交；同一有效根下同角色的所有会话生成、UI patch 与 Agent whole-state write 由 daemon-local character gate 串行化。非 UI whole-state writer 也执行 CAS：prompt 注入状态的同一次锁内读取同时返回其 revision，模型 `<state>` finalizer 只替换该精确快照；状态标签从完整 provider raw output 提取，可见消息仍来自 FSM 清洗输出，避免隐藏标签在 finalizer 前丢失。Agent 读取返回 `{revision, updated_at, state}`，更新必须提交 `expected_revision`。冲突会使 Agent 以 `finalization_error` 收口，不会伪报 converged；端到端 Agent 测试锁定 durable assistant message、较新 state、pending recovery marker、同 generation activity receipt 与 wire 结果。关系/剧情 writer 是锁内基于最新值的字段级 mutate，不重放陈旧 whole-state。State 多文件写入以不可变 revision + 原子 `current_revision` 为提交点，`live.json` 与 `history.jsonl` 作为可恢复投影；所有公开的 StateService/HTTP/扩展/Agent/prompt 读取与写入都经锁内恢复，确定性修复提交点之后的进程中断阶段，旧式“投影超前于指针”半提交会回滚到已提交 revision。该合同不宣称 Windows 断电后的目录元数据耐久性，因为当前 std-only `sync_dir` 在 Windows 是 no-op。Vue 两个编辑器在保存中只读，并在 conflict/未知结果后刷新权威 Surface且保留 dirty draft；真实 Engine + Chrome smoke 验证两个写闭环与无关 Chat Host identity 不变。桌面壳对精确 owned sidecar 的意外终止执行至多三次自动恢复：壳 PID 与页面保持不变，复用仅在壳内存中的 access key 拉起新 Engine，交换全新的短时 token，并通知 Vue 原位重建 Bus；旧 token 继续因 Engine 内存边界失效。交互式 Windows 便携包 smoke 强杀包内 Engine PID，验证 lock owner/PID 更新、旧/新 token 401/200、未保存 Memory/State draft 保留且不自动重放，以及恢复后按最新 hash/revision 显式保存。GitHub Windows runner 的非交互会话不暴露 WebView2 CDP，其显式 fallback 只验证真实包内进程、instance/lock 更新与清理，不外推浏览器状态证据。该 candidate **不宣称 PR 13 完成**：真实 provider 模型 `<state>` 交叉验收仍开放。

本次增量校准（2026-08-26，#577 PR 10a candidate）：新增 layout-only Workspace v1 机器合同与独立领域资产。持久化面只允许布局节点、受界 split 比例和 Widget `id`/`type`，不接受 Surface props、Chat、Memory、Character State、Activity、token、路径或可执行 UI 内容；验证复用 Surface 的 ID、引用、重复、深度、节点、children 与 Widget 数量上限。固定 `default` workspace 位于已解析 effective root 的 `ui/workspaces/default`，写入执行 expected-revision CAS、不可变 revision 和原子 `current_revision` 提交；历史沿已提交 parent lineage，orphan 编号跳过且不伪装成 committed history，rollback 把旧布局提交为更高 revision。未知 future major 只能原样导出，当前实现不得覆盖；v1 Blueprint migration 仅 dry-run 并丢弃 props/state/capability。该 candidate **尚无 HTTP、`core.workspace` intent、Surface/Vue 消费、import/apply 或独立 workspace backup 对象**，不得宣称用户工作区已进入产品闭环。

本次增量校准（2026-08-26，#577 PR 10b candidate）：Workspace 资产新增强制 bearer 的 HTTP 适配面与 `HttpEngineBus` 客户端：读取、严格 `1..=256` 历史、原始字节导出、forward rollback，以及首个 Engine-authoritative `resize_split` 命令。mutation 只接受十进制字符串 CAS 与封闭命令，不接受整份布局或 JSON Patch；冲突返回稳定 `workspace_revision_conflict` 和字符串 `current_revision`，future major 返回 `workspace_unsupported_major`，只有原始导出继续可用。daemon bearer 仍是进程级凭证，`user_id` 只表示既有 effective-root 命名空间隔离，不构成租户授权。该 candidate **尚无 Vue store/编辑控件、Surface shell actor、其他布局命令、migration apply/import 或独立 backup 对象**，不得宣称 PR 10 或用户工作区产品闭环完成。

本次增量校准（2026-08-26，#577 PR 10c candidate）：Workspace command reducer 与 `HttpEngineBus` 的封闭命令集扩展为 `open_widget`、`close_widget`、`move_widget`、`resize_split`、`activate_tab` 和 `reset_layout`。open 只接受 Workspace v1 首方 Widget allowlist，并由 Engine 确定性派生 placement node；move 的 index 表示移除源 placement 后目标容器中的插入位置；close/move 会维护 tabs active 引用。所有命令先在当前布局副本上执行，再经完整 Workspace 验证与 expected-revision CAS 一次提交，失败不发布 revision。该 candidate **仍无 Vue 编辑器、保存布局到 session Surface 的消费链、migration apply/import 或独立 backup 对象**，不得宣称 PR 10 完成。

本次增量校准（2026-08-28，#577 PR 10d candidate）：Engine 每次刷新 session Surface 时，从与 session 相同的 effective root 读取并验证 `default` Workspace，把其结构降为 Surface v2 Blueprint，再按首方 Widget type 附加当前 Chat、Memory、Character State 与 Activity 投影；Workspace 不复制业务 props。Surface v2 可表达的保存结构变化沿既有 polling/replay 产生新 snapshot，纯 props 变化仍可产生 patch；registry 以 Workspace revision 单调接收并拒绝并发迟到的旧布局，不同 user effective root 的布局互不别名。未知 future major 或无效 Workspace 在替换 registry 当前值前失败，客户端继续保留最后已接受 Surface，但该显示回退不延续写权限：每次 intent 都重新验证当前 Workspace 与已接受 revision。**Surface v2 Split 尚无 ratio 字段，因此 ratio-only 变化不推进 Surface revision、也不发事件；`ratioBasisPoints` 虽被 Workspace 持久化和验证，却尚不能驱动可见 resize 或动态分辨率自适应。Vue 编辑控件、ratio 消费、migration/import/backup 与多 Workspace 仍未交付。**

本次增量校准（2026-08-28，#577 PR 10e candidate）：生产 Vue 在连接 session Surface 前读取同一 user scope 的 accepted-only Workspace；新 Bus 与 user scope 仅在 Workspace、session list 和 Surface connection 全部就绪后原子发布。命令只使用当前十进制字符串 revision 一次，成功后整体接收 Engine 文档，冲突/失败/未知结果只 GET 最新状态而不自动重放；已发布 Bus 上的手动读取与命令共享 operation epoch，初始化读取另由 candidate/attempt identity 约束，且旧 revision 不得覆盖较新 accepted document。Vue 不改写 accepted Surface Blueprint，只把 Surface v2 尚不能表达的 split ratio 映射到渲染轨道；宽屏可按 5% 步进保存比例，横向 split 在 760 CSS px 以下自动纵向堆叠并隐藏比例控件，返回宽屏后继续使用保存值。tab 点击也改走 Workspace command，等待 Engine Surface polling 收敛，并在 pending 时暴露 busy/disabled 语义。真实 Engine + Chrome smoke 验证默认 65/35 → revision-CAS 70/30、实际 CSS 比例、760/761px 边界、精确无重复命令序列、后续 tab 使用新 revision，以及 Memory/State/Chat 纵切不回归。**open/close/move/reset 控件、拖拽 resize、migration/import/backup 和多 Workspace 仍未交付。**

本次增量校准（2026-08-28，#577 PR 10f-1 candidate）：Engine domain 新增 Blueprint-v1 migration apply 事务：重新计算并绑定已审阅 dry-run 的 source hash、candidate hash 与 converter version，先执行 Workspace CAS 与 current asset 校验，再在固定 `ui/workspaces/default` 范围创建 `pre_migration` 专属 backup、完整 verify，最后把 candidate 作为更高 immutable Workspace revision 提交。Workspace backup 不能由公开通用 create scope 任意请求，也被 generic destructive restore 明确拒绝；Full restore 拒绝含 Workspace assets 的目标 manifest，且 swap 在路径级永不删除、rename 或写入 live `ui/workspaces/`，包括 preflight 后并发首次创建。恢复只读取 verified manifest-approved bytes（读取后再次核对长度/hash），并把迁移前布局作为更高 revision forward commit。revision 0 backup 明确表示迁移前没有 committed Workspace，rollback 会 forward-commit 确定性 default layout。锁序固定为 `WORKSPACE_LOCK -> BACKUP_LOCK -> revision COMMIT_LOCK`，同一 `BACKUP_LOCK` 生命周期贯穿 backup 创建、verify 与 forward commit；backup rename 前按最深到最浅同步完整 staging 目录树，rename 后同步 `backups/` 与 data root，覆盖 Unix 嵌套及首次目录项持久性；reviewed identity/CAS 失败不创建 backup 或 revision。apply 与 migration-backup rollback 的 commit 返回错误后都在持锁状态重读 durable authority：匹配 source identity/layout 的已发布 revision 仅在从 revision 自下而上同步至稳定 effective data root、重同步 current 文件并再次验证后视为成功，未变 pointer 返回 definite-failure，无法读取、矛盾状态或 barrier 再失败返回 outcome-unknown 并要求 refresh 后再恢复；两类错误均只结构化暴露 retained backup ID。manifest schema 仍为 v1：新引擎兼容旧 v1，旧引擎不认识 `pre_migration`/`workspace` 时会 fail closed 或漏列，Workspace recovery 因此要求相同或更新版本。**本切片仅为 Engine domain 安全底座，尚无 migration HTTP/import、Vue 恢复 UI、任意 Workspace JSON import 或多 Workspace。**

本次增量校准（2026-08-28，#577 PR 10f-2 candidate）：新增 bearer-protected Blueprint-v1 Workspace migration HTTP 演练面：严格 dry-run、绑定 reviewed normalized typed-source/candidate/converter identity 的 apply，以及按 effective user root 解析 migration backup 的 forward rollback；source hash 不冒充原始上传字节 hash。请求 revision 只接受十进制字符串，migration 整份 request body 上限为 256 KiB；dry-run 和所有 pre-commit rejection 不创建 Workspace revision 或 backup，apply 成功返回 committed Workspace 与 retained recovery backup ID，rollback 不调用 generic destructive restore。未知请求/Blueprint 字段、跨 user backup、篡改 backup、future-major current Workspace 与 stale CAS 均 fail closed；关键响应 `no-store`。**该切片仍不开放任意 Workspace-v1 JSON import、Vue migration/recovery UI、内部 recovery backup 生命周期策略或多 Workspace。**

本次增量校准（2026-08-30，#577 PR 11a implementation 已由 PR #627 合并，产品切片仍为 candidate）：当保存的 Workspace 放置 `core.emotion` 或 `core.inventory` 时，Engine 从当前 effective root、当前角色的 Character State 投影有界只读 props，并携带 state revision、timestamp 与 character-scoped source。Emotion 只接受 `0..=100` 整数及有界 `emotion_label`/`mood`；Inventory 只接受最多 128 个、ID 唯一且字段有界的 `state.inventory` 项。缺失输入明确显示为未配置，非法输入显示为不可用，不再伪装成情绪 0 或真实空物品栏；合法空数组仍表示真实空。同一 accepted Surface 若有多个 `core.inventory`，按 Blueprint `widgets` 数组顺序仅第一个获得完整投影，后续实例以保留 revision/timestamp/source 的 `available:false` 降级，DOM 顺序、active tab、可见性、请求时序以及只改变布局树的 `move_widget` 均不改变选择；只有改变 `widgets` 数组顺序的操作（例如关闭首实例后重新打开，使其追加到末尾）才可能改变首实例。这是展示预算策略，不修改或迁移 Character State。当前 wire 对重复实例与源侧不可用都使用 `reason:"unavailable"`，Vue 因而只显示诚实的通用“不可用”；只有同 Surface 首实例可用且后续同 provenance 实例不可用时才能人工判断为重复实例降级，首实例也不可用时当前 UI 无法可靠区分。专用 reason 不是对封闭 manifest enum 自动兼容的 additive 变化；#628 必须采用 consumer-first 的 versioned manifest/parser/fixture 升级，并在兼容合同被接受前禁止 producer 发出新 reason。Vue 不再提供 item use/drop 操作，manifest 删除无 executor 的 intent 声明，手工构造的相关 intent 继续 fail closed。**该 candidate 不新增 Emotion/Inventory 独立资产或写 executor；PR 11b/11c、结构化 state migration 与 Vue Workspace migration/recovery UI 仍属后续。**

Generic restore 的 Unix 目录持久性同步与上述合同一起校准：restore staging 同样按最深到最浅逐层 sync；Full swap 在 `ui/` 子项完成后同步 `ui`，在顶层 swap/staging 清理后同步 data root；Character/Session scoped rename 后从 canonical destination parent 自下而上同步到 data root。Windows 仍沿用既有 `sync_dir` no-op 限制。

本次校准（2026-08-09，v0.0.5-rc.2 docs-pass）：当前 `main@affa315` 对应 prerelease `v0.0.5-rc.2`。Windows release workflow 负责 exact-tag 校验、包构建和 browser/desktop smoke，当前公开发布交付物只有 `airp-webui-windows-x64.zip`。依赖清单、SBOM、第三方声明和审计 sign-off 信息仍保留在 tagged git tree 的 `docs/sbom/`，供开发用户直接查阅；它们不再由 release CI 生成、上传或作为 sign-off 门禁，既有 rc.2 资产不在本次变更范围内。只凭候选发布证据不能宣称正式 `v0.0.5`：[#130](https://github.com/GhostXia/AIRP/issues/130) 的真实 provider + 真实 browser + production Compose 验收仍未完成；`release` environment API 当前为 `protection_rules=[]`、`can_admins_bypass=true`，required reviewer 配置仍缺失。

本次校准（2026-08-02 v0.0.3 docs-pass）做了三件事：

1. 将代码锚点从旧 `main@4f3f792` 对齐到当前 `main@830426e`；
2. 吸收 #398–#413 的取消、TurnCommit/Recovering、终态 marker recovery、production smoke 与依赖治理事实，**不把后续 issue 写成已交付**；
3. 保持活文档面收敛：已完成计划与桌面画布接力草案仍在 `docs/archive/`，阅读路径见 [README.md](README.md)。

增量校准（2026-08-03，W-06 闭合）：代码锚点从 `main@830426e` 推进到 `main@e931bf7`，吸收 PR #436/#439/#441（R1 锁序收敛 + 运行时强制 + 回归测试，closes #437/#438/#440）与 PR #445（#342 backup/restore 最小闭环）的交付事实。#342 标记为已交付（v1 限制见 §2.2）；R1 锁序合同补全见 [LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md) §1.5/§2.9/R4。本次为 docs-only 增量校准，§6 验证快照的 full-workspace 数字未在本 docs-pass 重跑，仅追加 #342 与 R1 回归测试的 PR 级证据。

增量校准（2026-08-06，v0.0.4 docs-pass / 审计遗留 #485 D1）：代码锚点从 `main@e931bf7` 推进到 `main@e28ea02`，吸收 PR #462–#465（深度审计报告修正、BUG-1/BUG-2 会话修复、SSE 事件合同固化与 v1 端点守卫、STYLEGUIDE 与截图套件机制）与 PR #480–#487/#491/#492（C-P0 桌面壳承载 webui 与 bearer 注入 → C-P1 widget 运行时与沙箱安全边界及 slot 挂载接线 → C-P2 engine 扩展注册面 → C-P3 capability 权威授权与扩展管理 UI → C-P4 扩展合同收口两批 → webui 侧遗留收束）的交付事实。桌面线从「暂停」转为「Tauri 壳承载 webui」（§1/§2.1）；widget 扩展体系与 capability 授权落地为首批闭环（§2.3），**不外推为成熟生态**。本次为 docs-only 增量校准，§6 验证快照的 full-workspace 数字未在本 docs-pass 重跑，仅追加 C-P0~C-P4 的 PR 级证据。

## 1. 产品与仓库边界

AIRP 是面向 Role Play 的 AI Agent 客户端，采用“无头 Engine + 可换 UI”结构。

| 路径 | 当前职责 | 产品状态 |
|---|---|---|
| `engine/` | `airp-core`：RP 数据、prompt 装配、LLM adapter、Agent loop、HTTP/SSE | 唯一业务内核 |
| `webui/` | 无构建、多页面、同源 WebUI（当前 44 屏；`assets/widgets/` 为 widget 运行时与 SDK 资产面） | **正式产品交付主面** |
| `airp-engine-console/` | WebUI 视觉与交互样板 | 设计基线，不是第二套运行时 |
| `protocol/` | `airp-state-protocol`：共享线协议类型 | Rust workspace 成员 |
| `ui/`、`ui/src-tauri/` | 当前运行事实：Tauri 壳同源承载 engine webui 资产 + bearer 注入、token 续期与 owned Engine 有界自动恢复；Surface v2 authority、客户端原子 store、受限 Blueprint/Widget runtime、Engine session Surface snapshot/SSE、`HttpEngineBus`、同源 `/desktop/`、Engine-authoritative Widget host parity、`core.chat` 写纵切、Memory/Character State CAS-write candidate，以及 Workspace HTTP/命令与 Engine-side Surface 结构消费 candidate；目标事实：#564 恢复 Vue Blueprint/Widget 桌面主面 | **#564 开发中**；默认入口仍是 WebUI `/`，`AIRP_DESKTOP_UI=blueprint` 才选择 `/desktop/`。PR 8/9 仍有真实 provider 与 session→Scene 门禁；Workspace 尚无 Vue 编辑闭环，Surface v2 也尚未传递持久 split ratio，见 §2.3、#577、#589、#593 和 #564 决策 |
| `deploy/windows-webui/` | Windows 便携 WebUI 包 | 当前优先 artifact |
| `deploy/linux-webui/` | Linux musl 便携包 | 手动构建 artifact |
| `deploy/production/` | 单实例自托管 HTTPS preview | P0 拓扑，不是正式发布 |
| `data/` | 运行时数据根规范与安全样例 | 不是共享素材库 |
| `tools/` | 依赖治理、SBOM、Agent 浏览器探索 | 工程工具，不进入 RP 角色平面 |

Rust workspace 只有 `engine`、`protocol`、`ui/src-tauri`。AIRP-Core/AIRPCLI、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 是作者的第一方前序项目，不是当前 runtime 依赖；吸收边界见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)。

## 2. 当前能力矩阵

“已实现”按层描述，不把底层模块、HTTP route、UI 页面或一次测试互相冒充。

| 能力域 | Engine / 数据 | HTTP / Agent | WebUI | 当前边界 |
|---|---|---|---|---|
| 角色、Persona、Preset、场景 | CRUD、导入、绑定、revision、装配 | 主要 CRUD/导入/预览 route；相关 Agent tools（Persona **无**对称 Agent tool） | 管理、选择、导入与诊断入口 | backup/restore 最小闭环已交付（#342，PR #445，v1 限制见 §2.2 切片表）；完整导出/migration 未闭合（#346） |
| 会话与聊天（**产品主路径**） | durable JSONL、稳定 message ID、cursor、rollback、branch/swipe、per-session Coordinator façade；TurnCommit 记录 message/state/volume 提交进度 | OpenAI-compatible `/v1/chat/*` SSE、continue/regen/search、命名 session；generation-scoped `session-state`/`cancel` | 命名会话、流式聊天、Engine 协作停止、编辑/删除/分支/Swipe、导出 | 取消只接受当前 `generation_id`；stale/committing 分别 fail-closed，断线不等于取消。终态 marker 可在已持久化最终阶段后恢复清理，Recovering/未知提交仍 fail-closed。**产品 UI 只绑定本路径**；agent/tool 单 owner、自动 replay/repair、跨资源事务仍未交付（#394/#286）；backup/restore 最小闭环已交付（#342，PR #445），但跨资源一致性 backup 与完整灾难恢复未交付 |
| Conversation runtime（**Engine 合同，未绑产品 UI**） | versioned manifest、append-only event journal、message/turn/observability projection、scene round-robin、受控 policy 注入、长历史 checkpoint/summary 预算、legacy copy-only migration | `/v1/conversations*`、capabilities、policies、migration plan/execute/export/rollback；旧 chat/session/scene API 形状不变 | **尚未绑定**；客户端若接入只能经 capability discovery，不能注入 history/代码/调度语义 | 与 legacy Chat **双轨并存**；切流或冻结须战略决策（#381 E-P0-2）。自动 summary policy、内容型停止条件、全仓 migration registry、跨进程策略沙箱仍开放 |
| Worldbook / state / memory | v4 runtime、state history/schema、resident memory、revision | CRUD、图谱、事件、状态与记忆相关接口/工具 | 编辑、图谱、状态 HUD、记忆面板 | 大量 ST 字段仅为 advisory；完整 session 物化与记忆闭环未完成（#274） |
| Agent 与剧情 | 有界 loop、Director、Council、NPC、剧情弧、世界时钟、定时事件、遗忘曲线 | 约 30 个内置工具 + 可动态加载插件工具 | Agent run、剧情弧、群聊、世界事件 | 并发/失败路径有开放审计项（#284/#344/#381）；不是通用多 Agent 平台 |
| 创作工具 | 图片生成、角色模板、风格学习、对话示例、时间线、卡片 diff | 对应 HTTP | 屏 36–42 等已接入 | 功能存在 ≠ 真实 provider/工作流已验收 |
| Provider / 扩展 | 多 Provider 路由、OpenAI-compatible/Anthropic/Ollama、本地脚本/HTTP webhook 插件 | providers/routing/plugin-tools API；Agent registry 动态合并 | 设置与插件管理入口 | 插件非沙箱；HTTPS webhook 注册+请求 fail-closed DNS 与域名 pin 已落地（RR-014 近端修复 / #381 E-P0-3 / #329 N3）；非通用代码沙箱 |
| Widget 扩展与桌面壳（v0.0.4 新增） | engine `extensions/`：digest-pinned 安装、catalog 权威下发、capability 封闭集与 grant/revoke、desktop session token 签发与 rotation | `/v1/extensions*`、`/v1/widget-intents`（拒绝默认）、`/v1/grants` 统一授权查询面、`/v1/desktop-session*`；digest-pinned 静态包服务在鉴权层外投放、服务时复检摘要 | Tauri 壳同源承载 webui（C-P0）；slot 挂载、opaque-origin iframe 沙箱、consent 授权 UI、扩展管理 UI、Widget SDK | capability 授权由 engine 逐调用强制（C-P3）；第三方 esm 只有同源 digest 目录加载路径；compat harness 锁 hostApi semver 与 capability 封闭集；intent 面无真实执行器；GUI 真机验证未完成（§2.3） |
| Blueprint/Surface 桌面恢复（#564 开发中） | Surface v2 guard；确定性 session Blueprint；Chat/Memory/Character State/Activity 投影；Emotion/Inventory 从 Character State 派生的有界只读投影；有界 replay；脱敏失败回执；扩展 catalog/grants/plugins 权威；Character/Session/effective Persona/canonical Worldbook source context；Memory exact-hash CAS；Character State UI patch + 模型/Agent whole-state revision CAS；State revision 提交点与 live/history 重启修复 | bearer 保护的 snapshot/SSE；opaque cursor + `Last-Event-ID`；失效 cursor snapshot resync；`core.chat`、`core.memory`、`core.character-state` 可信 intent executor；Emotion/Inventory 写 intent 明确未实现 | Vue shell、受限 renderer、原子 store、Widget 生命周期、多尺寸 smoke、同源 `/desktop/`、`HttpEngineBus`、opaque iframe 第三方 host parity；Chat 全纵切；Memory/State dirty/conflict 编辑；Emotion/Inventory 只读来源视图；未知提交状态不自动重放；完整 stable-ID context chips；owned Engine 有界重启、新 token 原位注入与 Bus snapshot 恢复；真实 Engine/Chrome 5,000 历史与无关 Widget identity 证据 | 默认仍为 WebUI `/`；Memory/State 当前仍是有界写 candidate，Emotion/Inventory 无独立资产或写 executor，workspace 与第三方 intent executor 未交付；session→Scene、真实 provider 仍开放；WebUI 固定 slot plan 不进入 Blueprint v2 |
| 部署 | production fail-closed 校验、原子配置更新、secret 脱敏 | loopback 默认；首方 gateway 同源代理 | Windows/Linux 便携包与 production preview | 非多租户；P1/P2/P3 发布门未闭合 |

### 2.1 结构性事实（2026-08-02 审查确认）

这些不是新功能承诺，而是避免误读代码树的硬事实：

1. **双轨会话**：正式 WebUI 走 `/v1/chat/*` + `ChatLog`/`ChatService`；Conversation 是并行 Engine 合同与 HTTP 面，**不能**因 route/测试存在就宣称产品已切换。**v0.0.3 决策（E-P0-2/B）**：冻结 Conversation 功能对称扩张，产品验收不切流。  
2. **单资源原子写 ≠ 跨资源事务**：`finalize` 可对 message → state → volume 逐步 fail-closed，崩溃后跨资源一致性仍是 best-effort（RR-004 / #286）。  
3. **Domain 写路径未完全闭合**：shared service 是目标边界；Agent tools 等路径仍可能直接 `replace_file` / `fs` 写（#381 E-P1-3 / #160）。  
4. **锁模型分裂**：character/session/state/persona/conversation/decay/FTS/quota 等多套锁；async 路径上存在 std 锁 + 锁内磁盘 I/O；poison 策略不一致（#284/#220/#381）。  
5. **桌面线暂停** ⚠️ superseded twice（历史结论）：2026-08-06 C-P0 将桌面线改为「Tauri 壳同源承载 webui + bearer 注入通道」并归档 Vue 主面；2026-08-12 Owner 通过 #564 重新启动 Vue Blueprint/Widget 桌面主面。当前运行事实仍是 C-P0 WebUI 壳，目标架构见 #564 PR 1 决策；两者不得混写。

### 2.2 v0.0.3 收敛切片的已交付边界

以下记录的是 v0.0.3 收敛切片在历史 `main@830426e` 上由源码、测试或生产 harness 支持的边界；当前候选新增事实见本节末与 §6，不把 release 计划写成能力：

| 切片 | 已实现 / 已验证 | 明确未包含 |
|---|---|---|
| #398 | Coordinator 提供 generation-scoped `session-state` 与 cooperative `cancel`；仅当前 generation 可取消，stale/committing 请求返回冲突，WebUI 保留 Engine 权威 `commit_state`。 | 浏览器断线不等于 Engine 取消；不改变跨资源恢复能力。 |
| #399/#403 | durable `TurnCommit` marker 覆盖 message、live state 与 current volume 阶段；中断后公开 `Recovering` 并拒绝新 mutation，marker schema/阶段状态 fail-closed。 | 不包含自动 replay/repair、volume sealing recovery、backup/restore。 |
| #409 | 仅在所有 expected 阶段已 durable 时清理 terminal marker；恢复清理与 session owner/admission registry 锁序串行化；non-terminal、unreadable、unsupported 与 all-false marker 保留为 recovery-required。 | 不包含 payload-aware 自动 replay/journal 或完整灾难恢复。 |
| #410/#411 | production mock smoke 覆盖 generation poll、stale/current cancel、严格 typed SSE terminal、取消后 history、临时 session cleanup；harness 对 cleanup、SSE、cancel poll、response body 与 deadline 使用绝对预算并 fail-closed，合法空响应体可结束；备份入口保持明确不可用，renderer 不发起 backup/restore API 调用。 | 这是 mock/CI 证据，不替代真实 provider、真实 browser 和维护者 Compose 验收；不改变 Engine/API 语义。 |
| #413/#527 (plus #554) | lock-only 更新 `brace-expansion` 2.1.1→2.1.4、`postcss` 8.5.16→8.5.25、`nanoid` 3.3.15→3.3.16；`npm audit --json` 与 `--omit=dev` 均为 0。当前 SBOM 生成报告 693 third-party、unknown license 0、blocked 0；inventory 总记录 697（first-party 4、audit-required 17、auto-pass 680）。Windows workflow 提供 `workflow_dispatch` exact-tag validation/publish code gate、#554 隔离的 `contents: write` release-context job，以及包/browser/desktop smoke；当前公开发布只上传 `airp-webui-windows-x64.zip`。`docs/sbom/` 与工具脚本保留完整依赖审计信息，开发用户可从 tagged git tree 查阅，不再作为 release CI 附件或 sign-off 门禁。 | `release` environment API 当前为 `protection_rules=[]`、`can_admins_bypass=true`，required reviewer 配置仍缺失；正式 `v0.0.5` 仍需 #130 真实 provider/browser/Compose 验收。`ui` 依赖用于构建/测试，production gateway 只发布静态 WebUI，不把 `ui/node_modules` 当 runtime。 |
| #436/#439/#441/#470（R1 锁序收敛 + 运行时强制） | `advance_plot` / `trigger_world_event` / `advance_clock` / `npc_action` / `run_seal_flow` 五个 agent-tool 路径补齐外层 `character_lock.read()`（R1）；`StateService::mutate` 拆为 `mutate_locked` + `mutate` 消除 re-entrancy；`lock_order` 模块提供 R1+R2 运行时强制（thread-local 栈 + RAII Guard，默认 debug；release-profile CI 以 `lock-order-runtime` feature 验证）；4 条并发回归测试（各路径与 `delete_character` 经 `Barrier` 并发，30s 超时检测死锁 + TOCTOU）；lock-map cleanup race 修复（`remove_deleted_*_lock` 移到 write guard drop 之后）。 | 正式 release 默认不启用 tracker 以保持零开销；PR gate 有 `--release --features lock-order-runtime` 专项门；`advance_plot` 仍持 std 锁做同步 I/O（A3 debt）；W-01~W-04 follow-up 见 #442/#443/#444。 |
| #342（PR #445，backup/restore 最小闭环） | 手动 backup（Full / Character / Session scope）+ manifest schema v1（per-file SHA-256 + tree SHA-256）+ `verify_against_disk` 完整性校验 + restore（Full + scoped `swap_scoped_subtree`）+ `PreRestoreRollback` 回滚备份 + post-restore 校验 + `PreDelete` 自动备份（`delete_character` / `delete_session`，`force=true` 可跳过）+ secret 排除（`secrets.json` / `settings.json`）+ `BACKUP_LOCK` 串行化 + WebUI backup 管理界面。82 条 backup 测试通过（PR #445）。 | v1 限制：无自动定时备份；restore swap 阶段不持 `character_lock`（W-02，#447，v1 缓解为维护窗口执行）；Windows `sync_dir` no-op（W-03，#448）；跨资源一致性 backup 未交付；完整 migration / 导出未交付（#346）。审计遗留 W-01~W-06 见 #446/#447/#448/#449/#450/#451。 |

### 2.3 v0.0.4 收敛切片：桌面线与扩展合同（历史 `main@e28ea02`；候选发布历史快照 `main@affa315`）

以下记录 v0.0.3 → `main@e28ea02` 桌面线与扩展合同的已交付边界与已知限制，不把合同闭环外推为成熟生态：

| 切片 | 已实现 / 已验证 | 明确未包含 |
|---|---|---|
| C-P0（PR #480，桌面壳） | Tauri 壳经 `AIRP_DESKTOP_WEBUI_DIR` 同源承载 engine webui 资产（与浏览器宿主跑同一份）；壳持 access key 调 `POST /v1/desktop-session` 换短时效 UI token，以 URL fragment（`#airp-token=...`，不进服务端日志/Referer）导航首屏，首屏写入 `sessionStorage.airp_bearer` 后清理；导航成功后壳按 `expires_in/2`（clamp 5s~4h）调度续期循环——故意用 exchange（只增不撤）而非 renew（rotation），避免与 webui 持有的旧 token 互踢，代价是旧 token 在自身 TTL 内仍有效；交换失败后切 60s 短间隔重试（failed_fast）；webui 撞 401 另有 `POST /v1/desktop-session/renew`（rotation）兜底。Vue 主面随 C-P0 归档。 | 壳续期循环无 GUI 真机验证证据（发布级证据仍为 packaged smoke，RR-006）；不改变浏览器 WebUI 拓扑。 |
| C-P1（PR #481/#482，widget 运行时） | `webui/assets/widgets/`：widget-host 与 slot 挂载（5 个内置 slot：`chat.sidebar`/`chat.panel-right`/`settings.context`/`diagnostics.context`/`workbench.grid`）；第三方 esm 运行于 `sandbox="allow-scripts"` 的 opaque-origin iframe（读不到宿主 DOM/存储/cookie，仅经 postMessage 通信）；首批 widget（时钟/状态胶囊/第三方示范）接线 + 契约测试。 | 非通用代码沙箱：无 CPU/网络/资源隔离；首方 builtin widget 不走 iframe。 |
| C-P2（PR #483，engine 扩展注册面） | `POST /v1/extensions/install` digest-pinned 安装：逐文件 SHA-256 校验、包级摘要即内容寻址目录名（`data_root/extensions/<digest>/`）；安装面强制改写 `entry.source` 为 `/extensions/<digest>/index.js` 且 `sandbox=true`（跨源加载路径在安装面消灭，R0 硬门禁）；slot 必须 ∈ 内置封闭集；`GET /v1/extensions/catalog` 权威下发（内置默认计划打底，engine 无安装扩展时不硬失败；webui 静态 `slots.json` 仅作降级）；`/v1/widget-intents` 拒绝默认合同；digest-pinned 静态包服务在鉴权层外投放（内容寻址不可变 + nosniff + ACAO:*），服务时复检摘要，未注册 digest 一律 404；`POST /v1/desktop-session/renew` token rotation（撤旧发新，旧 token 立即失效；access key 不得被续期）。 | C-P3 无 intent 执行器：授权通过即视为 intent 被接受并留痕，不是真实执行。 |
| C-P3（PR #486，capability 权威授权） | engine 签发/撤销 capability grant（`POST /v1/extensions/:id/grants`，子集授权须 ∈ manifest）；`POST /v1/widget-intents` 逐调用强制：按 widget_type 找已启用记录，capability ∈ `granted_capabilities` 否则 403 `intent_denied`；新装/重装一律从无 grant 起步（consent 不跨身份延续）；授权决策全部 tracing 审计留痕；扩展管理与 consent 授权 UI。 | capability 封闭集仅 6 项（`read/write:memory`、`read:worldbook`、`read/write:state`、`call:tool`）；无执行器；MCP/plugin 授权主体未接入统一面。 |
| C-P4 两批（PR #487/#491，扩展合同收口） | catalog fail-closed：未知 slot 不编入下发计划并 warn；hostApi semver：`host_api` 声明 major 必须等于 `HOST_API_MAJOR=1`，缺省视为 `"1"`，跨 major 一律拒绝安装（前向兼容铁律，不静默尝试）；Widget SDK（onError 容纳合同、manifest 深冻结、esm 必须显式 `sandbox:true`）；compat harness（仅测试构型编译，不进产物）：解析/安装矩阵、前向兼容铁律独立测试、host_api serde 往返、`KNOWN_CAPABILITIES` 与 `docs/WIDGET-DEVELOPMENT.md` §5 文档锁；catalog 顶层下发 `host_api_major` 与 capability 封闭集；`GET /v1/grants` 统一授权查询面（每条带 `kind` 判别字段，当前仅 `kind: "widget"`，additive）；typed error 区分 404/500 storage_error；静态包服务 digest 复检移入 `spawn_blocking`。 | HOST_API_MAJOR bump 时需人工补 compat 矩阵项（见 RISK-REGISTER）；无多 major 兼容过渡机制。 |
| webui 侧收束（PR #492） | SDK onError 容纳（onError 自身抛错也吞掉，不炸宿主）、manifest 深冻结、endpoint-guard 递归扫描（`readdir(..., {recursive:true})`）、token-renewal 测试环境还原、`onUnauthorized` undefined 守卫。 | #485 剩余 W4（applySlotPlan 守卫）/W5（intent handler 结果回传）/W6（catalog 拉取超时）/T1（壳续期日志退避与未读参数）未修，去向见 #493（W4~W6 绑入下一轮桌面线工作，T1 搭进下次触碰 `ui/src-tauri` 的 PR）。 |

## 3. 必须保持的不变式

1. **干净提示词**：RP 角色平面只含 RP 数据与规划器明确选中的有界事实证据；工具参数、未选工具结果、规划/调度/审计/遥测留在结构化控制平面。控制平面 planner 同样只能看到工具显式投影并经 Engine 有界脱敏的 `airp.planner-observations.v1`，不得 fallback 到 raw result/error。选中证据必须带 tool/call/result 来源与摘要、先脱敏、按输入 token 预算截断，并经 `airp.selected-evidence.v1` 单一通道注入。`subagent_context_has_no_orchestrator_noise` 神圣不可弱化。
2. **Engine 单一真相**：handler、UI、Agent tool 不复制持久化规则；写路径应收敛到 shared service。  
3. **有界 Agent**：step/token/墙钟/取消/可观察事件；UI consent 不替代 Engine 授权。  
4. **用户资产优先**：不兼容演进必须有 versioned migration、升级前备份、完整性验证、可读导出与回滚。  
5. **安全默认关闭**：production 监听前 fail-closed；密钥不进普通 settings/URL/前端存储/日志；Web/远端不得启用任意本地路径导入。  
6. **第三方独立实现**：只吸收公开理念/需求/行为/互操作经验；不复制第三方代码、prompt、测试、数据或视觉资产。  
7. **审计门禁**：本地全绿只允许开 PR；审计 bot 通过并修完阻塞意见后，仍由人工 review 决定合并。

## 4. 当前不能宣称

- 不能宣称已正式发布、适合公网多租户、通过完整 P1/P2/P3，或已获市场验证。  
- 不能用页面数、工具数或 Phase 合入数量替代黄金路径成功率、恢复能力与继续使用意愿。  
- 不能宣称完整 session 自包含、跨资源 Turn 事务、全仓统一 migration registry、自动定时备份/恢复、浏览器矩阵或长会话 soak 已交付。  
- 不能把 Conversation 的 copy migration / 可观测性能力外推为 legacy Chat 产品路径或其它资产已具备同等恢复力。  
- 不能宣称完整 MCP 生态、任意插件沙箱、跨设备同步、多语言 UI 或正式资产规格已交付。  
- 不能把桌面 Tauri、production preview、Windows/Linux 便携包的测试结果互相外推。  
- 不能把 Worldbook/Preset **advisory** 字段写成已执行语义；以 [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md) 为准。  
- 不能把「已开 GitHub issue / 已写审计 umbrella」写成「风险已关闭」。
- 不能把 #398–#411 的取消、marker、Recovering 或 harness 证据外推为自动 replay/repair、Agent/tool 单 owner、性能 SLO、backup/restore 或真实 provider/browser/Compose 验收。
- 不能把 Tauri 桌面壳的 webui 承载、token 续期循环与包内自动 desktop smoke 外推为 GUI 真机、打包安装器、真实 provider 或正式发布验收已完成；壳续期循环的 GUI 真机确认仍是开放项。
- 不能把 widget 扩展体系的契约测试与 compat harness 外推为成熟扩展生态：无签名、无市场、无吊销、无多 major 兼容；intent 面当前无真实执行器（C-P3 授权通过即接受）；capability 封闭集仅 6 项。
- #346 完整导出/migration、跨资源一致性 backup、自动定时备份/恢复、#286/#394 O3 replay/repair、#394 O2 Agent/tool ownership、#394 O4/#400 性能与兼容性基准，以及 P2/P3 的签名、browser matrix、soak 仍未交付；#527/#554 的 Windows exact-tag package validation、release-context 隔离和候选 proof 已落地，依赖审计快照仍在 tagged git tree 中，但不再作为 release CI sign-off gate；`release` environment 当前 `protection_rules=[]`、`can_admins_bypass=true`，required reviewer 配置仍缺失，且 #130 真实 provider/browser/Compose 验收未完成。不要把候选发布或代码门写成正式 release assurance。#342 backup/restore **最小闭环已交付**（PR #445，v1 限制见 §2.2），但不得外推为完整灾难恢复或自动定时备份。

## 5. 当前优先级

当前主线不是扩大功能面，而是把已合入能力收敛成可验证、可恢复的 **P1 有限试用**（当前候选 `v0.0.5-rc.2`，正式 `v0.0.5` 仍未就绪）：

**v0.0.4（历史，2026-08-06）**：桌面线与扩展合同切片（C-P0~C-P4，PR #480–#487/#491/#492）已全部合入；该轮 docs-pass 对齐到 `main@e28ea02`。剩余 #485 项 W4/W5/W6/T1 随 [#493](https://github.com/GhostXia/AIRP/issues/493) 跟进；壳续期循环的 GUI 真机确认与打包 artifact 证据仍属发布级门禁。

**当前 v0.0.5-rc.2 P1 门状态（2026-08-09）**：代码与 mock/CI/候选发布证据已覆盖上述收敛切片；正式 `v0.0.5` 尚未完成的外部硬阻塞仍是 [#130](https://github.com/GhostXia/AIRP/issues/130) 的维护者验收：真实 provider + 真实 browser + production Compose，以及 `release` environment required reviewer 配置。CI mock、system Chrome、run `31309894372` 或本地单元测试不能替代该验收。

### 5.0 v0.0.3 已拍板决策

**E-P0-2 · Chat vs Conversation = 选项 B 冻结扩面（2026-07-30，当前仍有效）**

- 产品主路径与 v0.0.3 验收 **只绑定** legacy `/v1/chat/*` + `ChatLog` / `ChatService`。
- Conversation runtime（`/v1/conversations*` 及并行合同）在本窗口内 **冻结功能对称扩张**：仅允许安全修复、既有合同 bugfix、文档/测试诚实性维护；**不得**为 WebUI 切流或与 Chat 对等堆新能力。
- 选项 A（产品切流到 Conversation）需要独立战略决策、迁移/恢复证据与用户明确批准，**不**作为 v0.0.3 默认路径。
- 关联：[#381](https://github.com/GhostXia/AIRP/issues/381) E-P0-2、[#371](https://github.com/GhostXia/AIRP/issues/371)、[#344](https://github.com/GhostXia/AIRP/issues/344)、[#242](https://github.com/GhostXia/AIRP/issues/242)。

1. **Engine 一致性收敛**（[#381](https://github.com/GhostXia/AIRP/issues/381)）：  
   - ~~拍板 Chat vs Conversation（切流或冻结，E-P0-2）~~ → **已决策：B 冻结扩面**（见 §5.0）；
   - ~~Plugin DNS fail-closed + 请求时校验（E-P0-3，升权自 #329 N3）~~ → **已落地**（PR #384 / 见 SECURITY.md）；
   - Turn 级跨资源 commit/recovery（E-P0-1 → 执行面 #286，灾难恢复 #342）；  
   - 锁/async I/O/poison 与同 session 互斥（E-P0-4/5 → #284/#220/#160）。  
2. 用真实 provider 与真实浏览器验证：onboarding → 首聊 → 继续对话 → 刷新恢复 → 服务重启恢复（#130 是当前 P1 外部硬门）。
3. 校准 WebUI 运行时契约、视觉一致性、空/错/慢状态与 browser smoke（#311/#345 等）。  
4. ~~备份、恢复、migration 与回滚的最小闭环（#342，正式 P2 release gate，当前未交付）~~ → **#342 最小闭环已交付（PR #445）**：手动 backup/restore（Full/Character/Session scope）+ pre-delete backup + scoped restore + 完整性校验 + WebUI。剩余：完整 migration/导出（#346）、自动定时备份、跨资源一致性 backup、restore swap 持 character_lock（W-02/#447）。
5. 上述证据稳定前，默认不从 #312 启动无用户证据的新子系统扩张；遵守 #242 范围收敛。

### 5.1 实时工作入口（动手前用 `gh issue view` 复核状态）

| 主题 | Issue |
|---|---|
| Engine 审计 umbrella / 排序 | [#381](https://github.com/GhostXia/AIRP/issues/381) |
| Turn 两阶段 / 跨资源提交 | [#286](https://github.com/GhostXia/AIRP/issues/286) |
| per-session 并发串行化 | [#284](https://github.com/GhostXia/AIRP/issues/284) |
| 持久化/lock 遗留 | [#220](https://github.com/GhostXia/AIRP/issues/220) |
| 备份恢复导出 | [#342](https://github.com/GhostXia/AIRP/issues/342) |
| Plugin/engine 非阻塞遗留（含 DNS N3） | [#329](https://github.com/GhostXia/AIRP/issues/329) |
| Conversation migration 解耦 | [#371](https://github.com/GhostXia/AIRP/issues/371) |
| WebUI 能力展现 / 契约门禁 | [#311](https://github.com/GhostXia/AIRP/issues/311)、[#345](https://github.com/GhostXia/AIRP/issues/345) |
| 范围收敛 / 路线图索引 | [#242](https://github.com/GhostXia/AIRP/issues/242)、[#312](https://github.com/GhostXia/AIRP/issues/312) |
| 桌面线 / 扩展合同遗留（#485 W4~W6/T1） | [#493](https://github.com/GhostXia/AIRP/issues/493)、[#485](https://github.com/GhostXia/AIRP/issues/485) |

## 6. 验证快照

### 6.1 当前候选（2026-08-25，main through PR #592 + #593 Memory/State candidate）

| 范围 | 命令 / 说明 | 结果 |
|---|---|---|
| UI | `npm run typecheck`；`npm test -- --run` | typecheck 通过；Vitest **24 files / 189 passed** |
| Rust full workspace | `cargo test --workspace --locked`（先按 CI 流程生成 Tauri sidecar） | **通过**；`airp-core` lib **1,492 passed / 5 ignored**，其余 binary、integration、protocol、Tauri 与 doc tests 全部通过；神圣不变式通过 |
| Blueprint desktop | `npm run smoke:shell`；`npm run smoke:runtime`；`node responsive-browser-smoke.mjs` | 5 个 shell profile 通过；runtime 8 个 virtual rows、0.90 ms p95；360×320 响应式通过 |
| Real-Engine browser | 显式当前 checkout release `AIRP_ENGINE_BINARY` 后运行 `npm run smoke:http-bus` | 通过；含 5,000 历史分页、Chat 写纵切、Memory hash-CAS replace、Character State revision patch、完整 context chips 与无关 Chat Host identity 保持 |
| Memory/State candidate | `cargo test -p airp-core domain::state::tests --locked`；`memory::resident::tests`；`daemon::tests::surfaces` | State **4 passed**；Memory **10 passed**；Surface/intent **24 passed**；覆盖 stale conflict、并发单胜者、容量/schema、user scope 与跨 session active generation 拒绝 |

### 6.2 历史验证快照（仅证明标注的旧代码树）

| 范围 | 命令 / 说明 | 结果 |
|---|---|---|
| WebUI | `node --test webui/tests/*.test.mjs` | **75 passed, 0 failed**（`main@830426e`，2026-08-02；PR #445 WebUI backup 测试含其中） |
| production harness unit | `node --test deploy/production/*.test.mjs` | **22 passed, 0 failed**（`main@830426e`，2026-08-02） |
| UI | `npm run typecheck`；`npm run test -- --run`（`ui/`） | typecheck 通过；Vitest **13 files / 98 passed**（`main@830426e`，2026-08-02） |
| Rust engine + protocol | `cargo test --workspace --exclude airp-ui --locked` | **1,282 passed, 5 ignored, 0 failed**（`main@830426e`，2026-08-02）。增量：PR #436/#439/#441/#445 新增 R1 回归 + backup 测试，`main@e931bf7` 数字 ≥ 此处；本 docs-pass 未重跑 full-workspace，不把旧数字写成 e931bf7 数字。 |
| Rust full workspace | `cargo test --workspace --locked` | 本机验证边界：`airp-ui` build script 需要生成的 `ui/src-tauri/binaries/airp-core-x86_64-pc-windows-gnu.exe`；因此未完成 full-workspace 测试，不把 exclude 结果写成 full workspace。 |
| #342 backup/restore（PR #445） | `cargo test -p airp-core --lib backup::` 等 | **82 passed, 0 failed**（PR #445，`main@e931bf7`）：manifest schema、atomic snapshot、scoped restore、pre-delete backup、secret 排除、path sandbox、BACKUP_LOCK 语义。神圣不变式 `subagent_context_has_no_orchestrator_noise` 通过。 |
| R1 锁序运行时强制（PR #441） | `cargo test -p airp-core --lib` | 4 条并发回归测试（`advance_plot`/`trigger_world_event`/`advance_clock`/`npc_action` 各与 `delete_character` 经 `Barrier` 并发）+ 9 条 R1 单测通过（PR #441）。 |
| npm dependency audit | `npm audit --json`；`npm audit --omit=dev --json`（`ui/`） | 两个命令均 exit 0，vulnerabilities total **0**；#413 后的 lock-only 版本见 §2.2。 |
| dependency governance / SBOM (tagged-tree manual snapshot) | `discover-deps.mjs --fail-on-block`；`generate-sbom.mjs --fail-on-unknown` | 均 exit 0；**693 third-party / unknown 0 / blocked 0**。inventory 总记录 697（first-party 4、audit-required 17、auto-pass 680）；不属于 release CI 生成或上传。 |
| v0.0.5-rc.2 candidate release | GitHub Actions run `31309894372` on `main@affa315` | Candidate exact-tag package validation and smoke evidence; the current workflow publishes only `airp-webui-windows-x64.zip`, while dependency/audit information remains in the tagged git tree. Prerelease candidate only. `release` environment API returned `protection_rules=[]`, `can_admins_bypass=true`; required reviewer configuration is not present. Existing rc.2 assets are historical and unchanged. |
| production topology / 真实 provider·browser | CI mock/system-Chrome 与本地检查 | 不能替代 #130 维护者真实 provider + 真实 browser + Compose 验收；当前不宣称通过。 |

未在本次校准中完成的 maintainer-run production Compose、真实 provider/browser、Windows/Linux artifact（除 run `31309894372` 的 Windows candidate package smoke）、网络故障、进程崩溃、真实 provider 长会话，以及 Tauri 壳续期循环的 GUI 真机确认，不得由本表推断为通过。2026-08-09 docs-pass 未重跑 full-workspace 测试；对应历史行只证明 `main@affa315`，不能外推到当前 `main@7a90d88`。

## 7. 文档职责（校准后）

| 层级 | 文档 | 职责 |
|---|---|---|
| 事实入口 | [CURRENT-BASELINE.md](CURRENT-BASELINE.md)（本文） | 唯一人工维护的全仓能力基线 |
| 开发治理 | [DEV-GUIDE.md](DEV-GUIDE.md) | 地图、命令、不变式、交付流程 |
| 产品方向 | [PLAN.md](PLAN.md) | 稳定目标与阶段门；不复制 issue 队列 |
| 数据/运行时合同 | [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)、[CONVERSATION-CONTRACT.md](CONVERSATION-CONTRACT.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)、[LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md) | 目标与已交付边界必须分开读 |
| 安全/发布 | [SECURITY.md](SECURITY.md)、[RISK-REGISTER.md](RISK-REGISTER.md)、[WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)、[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) | 威胁、风险、拓扑与发布门 |
| 接口/扩展草案 | [UI-PROTOCOL-DECISION.md](UI-PROTOCOL-DECISION.md)、[AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md)、[ASSET-SPEC.md](ASSET-SPEC.md) | 已决策边界；未实现部分不得当 runtime 事实 |
| 来源治理 | [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)、[ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md) | 第一方/第三方吸收与 provenance |
| 文档地图 | [README.md](README.md) | 分层与阅读路径 |
| 审计原始记录 | `docs/audits/` | 按 PR 归档；不压缩成当前能力清单 |
| 历史 | `docs/archive/` | 已完成设计、草案与月度摘要；**不能覆盖本文** |

参考材料（研究用，非当前能力）：[CAPABILITY-ABSORPTION.md](CAPABILITY-ABSORPTION.md)、[MCP-SERVER-ABSORPTION.md](MCP-SERVER-ABSORPTION.md)、[TAVERN-PARITY.md](TAVERN-PARITY.md)、[HERMES-MEMORY.md](HERMES-MEMORY.md)、[LEARN-NEUROBOOK.md](LEARN-NEUROBOOK.md)。

### 7.1 本 docs-pass 的文档整理动作

| 动作 | 路径 | 原因 |
|---|---|---|
| 归档 | `docs/archive/2026-07-persona-http-api-plan.md`（原 `docs/PERSONA-HTTP-API-PLAN.md`） | 实施计划已交付；接口事实以源码+基线为准 |
| 归档 | `docs/archive/2026-07-29-desktop-ui-canvas-relay-plan.md`（原未跟踪活路径草案） | 桌面发布暂停；草案不占活文档位 |
| 保留 | `docs/audits/2026-07-28-desktop-ui-relay-plan-audit.md` | 计划级审计原始记录 |
| 刷新 | 本文、DEV-GUIDE、WEBUI-PRODUCTION-PLAN 及直接相关事实入口 | 对齐 `main@affa315`、v0.0.5-rc.2 run `31309894372`、#130 未解除状态与 release environment API 实况 |
| 刷新（2026-08-06，历史） | 本文、SECURITY、RISK-REGISTER、UI-PROTOCOL-DECISION | 对齐 `main@e28ea02` 的 C-P0~C-P4 交付事实（审计遗留 #485 D1） |

完整阅读路径与维护规则见 [README.md](README.md)。
