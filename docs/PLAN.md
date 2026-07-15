# AIRP 产品与架构计划

> 状态：长期产品原则与目标架构
>
> 最后校准：2026-07-15
>
> 当前事实与近期顺序分别以 [CURRENT-BASELINE.md](CURRENT-BASELINE.md) 和 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md) 为准。本文不证明任何功能已经交付。

## 1. 产品北极星

AIRP 是一个专精 Role Play 的完整 AI Agent 客户端：无头 RP/Agent engine 提供数据、推理、工具、记忆与扩展原语；WebUI、桌面 UI 或未来客户端通过稳定协议使用同一内核。

权威始终是 AIRP 自身用户需求。第一方前序仓库和第三方项目只能提供资产、理念、公开行为与互操作性经验，不能替 AIRP 决定产品边界。来源吸收规则见 [SOURCE-PROJECT-DECISIONS.md](SOURCE-PROJECT-DECISIONS.md)，第三方独立实现与 provenance 见 [ACKNOWLEDGEMENTS.md](ACKNOWLEDGEMENTS.md)。

代码取向：更开放、更透明、未来更易修正、更易迭代更新。表现为清楚的接口与扩展点、可观察的状态/决策/错误、低耦合可替换边界，以及版本化、小步迁移的数据和协议。

## 2. 不可破坏的不变式

1. **干净角色平面**：角色卡、世界书、Preset、Persona、state、记忆和历史进入 RP prompt；工具定义、调用、结果与编排脚手架走模型原生控制平面，不写入角色自然语言。`subagent_context_has_no_orchestrator_noise` 是阻塞门禁。
2. **有界 Agent**：每次运行都有 step、token、成本、墙钟和取消边界；每步可观察、可审计。
3. **能力在 engine 强制**：allowlist、capability、破坏性确认、幂等和单写者规则不能只靠 UI 声明。
4. **数据单一真相**：业务数据由 engine shared service 管理；HTTP、Agent tool、UI 和未来 MCP adapter 复用同一合同，不各自落盘。
5. **大数据不驻留窄管**：大文件优先 path token、multipart 或流式传输；不得把 base64/blob 长期塞入模型上下文、reactive store、Blueprint 或日志。服务端路径只允许可信本地调用。
6. **性能有界**：历史在 engine 完整保留，UI 只取窗口；稳定 ID、增量 patch、流式追加、离屏清理与可测内存上界是产品合同。
7. **扩展受控开放**：工具、事件、宏、技能和声明式 Widget 可以开放，但必须经过 capability、沙箱、用户同意和审计；不执行 Agent 或第三方生成的任意代码。

## 3. 目标架构

```text
WebUI / Tauri / future clients
        │  stable HTTP/SSE + versioned UI protocol
        ▼
AIRP engine
├── Agent kernel
│   ├── provider adapters + bounded loop
│   ├── Tool / Memory / Skill / Hook / Macro / Subagent primitives
│   └── capability, budget, cancel, trace and arbitration
├── RP domain
│   ├── Character / Persona / Preset / Worldbook / Scene
│   ├── Session / History / State / Memory / Revision
│   └── pristine prompt assembly and import diagnostics
└── service adapters
    ├── HTTP/SSE
    ├── built-in Agent tools
    └── optional MCP and extension adapters
```

当前只交付 RP 产品。通用原语必须保持可复用，但不能为了“平台化”提前牺牲 RP 闭环。

## 4. 产品能力支柱

### 4.1 角色、Persona 与 Preset

- 角色卡支持受控导入、稳定 ID、原始 sidecar、canonical view 与明确 provenance。
- Persona 是用户 RP 身份，不是客户端任意拼入的字符串；目标支持 default、角色绑定、session 绑定、revision、导入导出和可观察的有效配置。
- Preset 是可迁移的建议与结构化 prompt 资产；不能把特定模型的机械参数顺序当成跨模型真理。
- 当前实现和剩余产品边界见 [PERSONA-HTTP-API-PLAN.md](PERSONA-HTTP-API-PLAN.md) 与 #114/#115。

### 4.2 Worldbook 与资产规格

- AIRP 使用自己的版本化 canonical model，并保留第三方字段和来源；“成功导入”必须区分 converted、preserved、unsupported、invalid 与 needs-review。
- 已接受的运行时语义由 [WORLDBOOK-SEMANTICS.md](WORLDBOOK-SEMANTICS.md) 定义；高级 SillyTavern 字段只有经过确定性合同和 prompt-placement 测试后才能进入 runtime。
- 长期资产规格策略见 [ASSET-SPEC.md](ASSET-SPEC.md)，对标清单见 [TAVERN-PARITY.md](TAVERN-PARITY.md)。二者都不是当前兼容性声明。

### 4.3 Session、历史与记忆

- 一个 `session_id` 是一个独立开局/存档槽位，显示标题可变而目录身份稳定。
- session 最终必须自包含 history、memory、state、角色卡与世界书工作副本、provenance 和不可变内容 revisions；恢复不能依赖外部可变素材库。
- durable history 合同见 [LONG-HISTORY-CONTRACT.md](LONG-HISTORY-CONTRACT.md)，目标存档/revision 合同见 [SESSION-DATA-DESIGN.md](SESSION-DATA-DESIGN.md)。
- 自进化记忆、Soul 与跨会话学习仍是候选方向，见 [HERMES-MEMORY.md](HERMES-MEMORY.md)，不能写成当前能力。

### 4.4 Agent 与扩展

- Agent loop 是产品脊柱，不是可选附属；内建工具和未来扩展都复用 engine capability 与 trace。
- MCP upstream、skills/plugin runtime、ChangeInbox 与可配置多 Agent 编排按需求逐步实现；不为匹配某个源仓库数量而复制能力。
- 编排原则见 [AGENT-ORCHESTRATION.md](AGENT-ORCHESTRATION.md)，受控扩展产品决策由 #163 跟踪。

### 4.5 UI 与发布

- WebUI 是当前正式产品交付主面；每项能力应贯通 shared service → HTTP/SSE → WebUI → production tests。
- Tauri/Vue 资产保留，继续共享 engine 和协议，但恢复桌面排期前必须重新校准 artifact、sidecar 与性能基线。
- 首发拓扑是单实例、自托管、单用户、同源 HTTPS、私有 engine；P0 合同见 [WEBUI-PRODUCTION-ARCHITECTURE.md](WEBUI-PRODUCTION-ARCHITECTURE.md)，P1–P3 门禁见 [WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)。

## 5. 路线

### P1：正式 RP 使用面

- 解决开发工具链安全告警；
- 闭合 Persona/Preset/Worldbook 管理、选择、绑定、有效配置和诊断；
- 让已有 engine 合同在 WebUI 主路径可见、可操作、可恢复；
- 继续保持 production P0 和干净提示词门禁全绿。

### P2：数据可靠性与运维

- 分阶段落地 session 自包含、统一 revision、完整性验证和恢复导出；
- 版本化 migration、备份/恢复、可恢复删除、readiness、脱敏日志、资源上界与运维 runbook；
- 明确 access log 的用途、字段、输出和保留，或删除不需要的日志复杂度。

### P3：发布候选

- 浏览器兼容与安全负向矩阵；
- 旧数据升级、备份恢复与回滚演练；
- 长会话 soak、资源上界、SBOM/notices、版本与 artifact 门禁；
- 只有全部门禁通过后才能称正式发布。

### 后续方向

ChangeInbox、Agent-first 工作台、Style Review、长期记忆、可配置编排、MCP/skills/plugin 扩展与桌面恢复均在主发布链稳定后推进。

## 6. 仍需显式裁定的产品问题

- 高级 worldbook 字段哪些进入确定性 runtime，哪些只保留为 advisory/retrieval 输入；
- Persona/Preset 的 session snapshot、绑定与历史解释边界；
- extension developer mode、沙箱粒度、分发和兼容承诺；
- 多 Agent profile 的持久化格式、仲裁和人工升级合同；
- 桌面路线恢复时的发布平台、sidecar 生命周期和性能门槛。

这些问题应在对应 issue/ADR 中裁定。已被源码和合同解决的问题不再留在本页作为“开放题”。

## 7. 文档分工

- 当前事实：[CURRENT-BASELINE.md](CURRENT-BASELINE.md)
- 实现交接：[DEV-GUIDE.md](DEV-GUIDE.md)
- 发布门禁：[WEBUI-PRODUCTION-PLAN.md](WEBUI-PRODUCTION-PLAN.md)
- 风险：[RISK-REGISTER.md](RISK-REGISTER.md) / [SECURITY.md](SECURITY.md)
- 文档地图与维护规则：[README.md](README.md)

历史修订、逐 PR 过程与已完成旧计划统一放在 [archive/](archive/)，不再堆入本页。
