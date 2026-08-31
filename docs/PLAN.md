# AIRP 产品与架构计划

> 状态：活路线原则
> 校准：2026-08-09，`main@affa315`；当前候选：`v0.0.5-rc.2`（prerelease）
> 当前能力与开放缺口以 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 和 GitHub issues 为准。

> 当前发布事实（2026-08-09）：Windows release workflow 的候选链路完成 exact-tag 校验、包构建和 browser/desktop smoke，公开发布交付物为 Windows 便携包；依赖审计信息仍随 tagged git tree 保存在 `docs/sbom/`，不作为 release 附件或 CI sign-off 门禁。正式 `v0.0.5` 仍未就绪：[#130](https://github.com/GhostXia/AIRP/issues/130) 的真实 provider + 真实 browser + production Compose 验收，以及 `release` environment required reviewer 配置仍是阻塞门。

## 1. 产品目标

AIRP 要成为可本地拥有、可审计、可恢复的 Role Play Agent 客户端：

- 角色、Persona、Preset、Worldbook、会话和记忆由用户持有；
- Engine 是数据、装配、Agent 执行和安全边界的唯一真相；
- WebUI 是当前正式交付主面，桌面端是保留维护线；
- Agent 能力服务 RP 工作流，不以通用编排平台、工具数量或页面数量作为产品目标；
- 内部结构可以迭代甚至重建，但用户资产必须可迁移、验证、导出和回滚。

## 2. 当前阶段：P1 收敛

Phase 1–5.3 的大量功能已在 2026-07-25 至 2026-07-26 合入；此后 Conversation 合同与多批修复继续进入 `main`（当前锚点见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md)）。约束已从“缺功能”转为“功能面超过验证面”。在 P1 证据闭合前，默认冻结无直接用户证据的新子系统扩张。

### 2.1 Engine 收敛门（2026-08-09；原则延续）

在继续 5.4+ 功能扩张前，必须先处理 [issue #381](https://github.com/GhostXia/AIRP/issues/381) 指出的一致性问题：

1. **Chat vs Conversation 双轨**：**v0.0.3 已拍板选项 B**——产品 WebUI / 发布验收只绑定 legacy `/v1/chat/*`；Conversation runtime **冻结功能对称扩张**（仅安全/bugfix/文档诚实性）。切流（选项 A）另需战略决策与用户批准，见 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) §5.0。
2. **Turn 级跨资源提交 / 恢复**：单资源原子写不等于跨资源事务（见 #286/#342）。
3. **插件 DNS 与锁/async 模型**：DNS fail-closed + 请求时 pin 为近端安全门（#329 N3）；锁/并发正确性优先于新工具/新页面。

Agent decision-input 边界按 #178/#632 收口：普通 Generate 保留完整 RP 装配；显式 assignment 与被选择的最小工具证据使用 typed provider blocks；planner 只消费工具显式声明、Engine 有界脱敏的 planner-only projection 与 evidence preview。工具声明 outcome-only/projected result mode，无法返回读取事实的 readonly 工具不进入 planner 广告。raw 参数/结果/错误、planner transcript、调度预算与 telemetry 不进入任何 provider payload。该合同是后续智能 NPC/多 Agent 扩张的前置安全地基，不代表完整 runtime 或 UI 已交付。

详细排序与去重挂接以 #381 与 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) §5 为准。

### 2.2 P1 通过条件

P1 通过需要同时满足：

1. 新用户能完成 onboarding、配置 provider、导入/创建角色并完成真实首聊；
2. 继续生成、编辑、分支、Swipe、刷新页面和重启服务后，历史与活动状态保持一致；
3. 网络中断、provider 错误、取消、超时和关键落盘失败不会产生虚假成功或静默损坏；
4. provider key、bearer、路径和私密正文不会泄漏到 URL、浏览器持久化、日志、诊断或错误响应；
5. Windows 便携包有可重复 smoke；production preview 的结论只用于其自身拓扑；
6. 关键路径有自动化证据，并完成真实浏览器/人工验收。

页面、route、测试数量或单次演示都不能单独满足这些条件。

## 3. 交付顺序

### P1-A：正确性与恢复

- 处理并发、锁序、跨资源提交、failure injection 和误报成功；
- 统一删除、revision、migration、备份与回滚边界；
- 把 screen 34–44 纳入空/错/慢/刷新状态和最小 browser smoke；
- 清理已实现但 issue/文档仍标为未实现的状态漂移。

### P1-B：真实黄金路径

- 真实 provider + 真实浏览器；
- onboarding → 首聊 → 多轮 → 页面刷新 → 服务重启；
- 单角色、scene/群聊、Agent run 各取最小代表路径；
- 记录成功条件、失败分类、恢复动作和用户可理解提示。

### P2：资产生命周期与运维

- versioned migration registry、升级前备份、完整性检查和演练回滚；
- 自动备份/恢复、可恢复删除、支持包与运维 runbook；
- Persona/Preset/Worldbook/session 的完整 revision、drift、collision、export/import；
- 长会话窗口化/虚拟化与资源上界。

### P3：发布候选

- 浏览器与平台矩阵、长会话/故障/恢复 soak；
- SBOM、notices、签名、构建 provenance 和 release rollback；
- 文档、安装、升级、卸载、数据导出和已知限制；
- 明确的用户试用、核心任务成功率、留存和继续使用意愿观察窗口。

正式发布或代际替代必须由用户明确批准，不能由开发完成度自动触发。

## 4. 扩展方向的准入

issue #312 保留了历史功能路线图。5.4 外部 MCP、5.5 多语言、5.6 自动备份/恢复、5.7 跨设备同步等方向，不按编号自动执行。新扩展只有满足以下条件才进入实现：

1. 有具体用户工作流或已复现缺口；
2. 不绕过 P1 的正确性、恢复和安全门；
3. 能沿 `shared service → HTTP/Agent → WebUI → test` 纵向闭环；
4. 明确数据所有权、授权、资源上界、失败/回滚和 secret 边界；
5. 不复制第三方实现，并完成依赖许可证/provenance 核验。

## 5. 稳定架构原则

- **单一 Engine**：UI、handler、Agent tool 复用同一 domain service。
- **结构化控制平面**：工具、调度、validator 和审计不进入 RP 角色 prompt。
- **默认有界**：外部调用、并发、重试、存储和大对象都有上界与取消。
- **版本化数据**：稳定 ID、原子写、revision conflict、migration、完整性与导出优先。
- **能力可换**：provider、UI、模型和工具通过明确接口替换，用户数据不绑定某一实现。
- **证据分层**：单元、route、browser、artifact、production、人工与市场证据各自只证明自身覆盖面。

专题合同见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)、[LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)、[WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md)、[SECURITY.md](SECURITY.md) 和 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)。


