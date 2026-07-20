# AIRP Agent Memory

## Local Build Environment

- The AIRP project does **not** require contributors to install their toolchains on `D:`. Contributors may use the standard locations or any paths appropriate for their own environment.
- The paths below are a maintainer-specific override for this checkout only: the maintainer's `C:` drive is full, so the local Rust, Node.js, MSYS2, cache, and generated-build ecosystem has been moved to `D:`. Agents running on this maintainer machine must preserve that setup and must not treat it as a project-wide convention.
- Confirmed toolchain roots on this maintainer machine:
  - `RUSTUP_HOME=D:\.rustup`
  - `CARGO_HOME=D:\.cargo`
  - Rust shims: `D:\.cargo\bin`
  - MSYS2/GNU linker path: `D:\msys64\mingw64\bin`
  - Node.js: `D:\nodejs`
  - npm global prefix: `D:\npm-global`
  - npm cache must be forced to `D:\npm-global\npm-cache` because the default may point to `C:\Users\<user>\AppData\Local\npm-cache`
- Before Rust builds/tests in this maintainer's PowerShell environment, set:
  ```powershell
  $env:RUSTUP_HOME = "D:\.rustup"
  $env:CARGO_HOME = "D:\.cargo"
  $env:npm_config_prefix = "D:\npm-global"
  $env:npm_config_cache = "D:\npm-global\npm-cache"
  $env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
  ```
- In this checkout, use the repo-local target directory `D:\AIRP-Dev\target` unless a task explicitly requires otherwise. Other checkouts should use their own repo-local target directory.
- On this maintainer machine, if a command tries to populate `C:\Users\<user>\.cargo`, `C:\Users\<user>\.rustup`, or npm cache/global data under `C:`, stop and redirect it to the D-drive locations above. Do not apply this restriction to other contributors' machines.

## Audit Agent Charter（审计 agent 守则，2026-07-03 用户立）

任何在本仓执行审计的 agent —— 无论常驻 bot、一次性 review agent、还是被临时指派做 review 的开发 agent —— **必读并遵守以下三条**：

1. **独立审计**。不附和开发 agent 的结论，不把开发文档/现有代码/现有结构当作不可质疑的前提。以"我会不会这样写"的独立判断为准。
2. **可以提出自己的想法**，不必拘泥于开发文档、现有代码和现有结构。若你认为有更好的设计/实现/取舍，直接说出你的方案及理由，哪怕它与既定路线相悖。
3. **agent 的智能是不断迭代增强的**。因此曾经审计/开发此仓的 agent 的能力和眼界，可能远不如现在你这一代。必要时对历史决策产生质疑并主动查证（读源码、跑测试、查文档），不要因"前人已定"就放过可疑处。

### 项目取向（同源用户指示，2026-07-03）

代码应当：**更开放、更透明、在未来更易修正、且更易迭代更新**。审计与开发决策都向这四点对齐。

### 周期性代际重构特例（2026-07-16 用户立）

为防止项目被历史架构、旧工具链和旧一代 agent 的能力上限长期锁死，用户可以按**每半年或每年**的周期，显式启动一次“代际升级”。代际升级允许对 AIRP 进行破坏式重构与迭代升级，包括替换既有架构、协议、模块边界、工具链、数据模型和已不再合理的历史决策；不得仅因“现有实现已经能跑”而拒绝重新设计。

执行时必须遵守：

1. **旗舰模型强制要求**：启动时必须通过官方一手信息核验当时最新、已正式发布且适合复杂软件工程的旗舰级大模型；主导规划、关键实现与独立审计均必须使用该级别模型。preview/实验模型不得冒充正式旗舰，除非用户明确授权。
2. **由用户显式启动**：半年/年度是允许的评估周期，不是到期自动改仓。每次启动必须记录日期、所选周期、模型及版本、核验来源、当前基线、目标、淘汰项和验收标准。
3. **允许推倒重建**：若独立审计证明渐进修改会继续固化错误边界、累积迁移债务或无法达到新一代目标，可以从空白架构开始重建，不必保留旧内部结构、代码组织或实现兼容；“重用旧代码更省事”不是否决重建的充分理由。
4. **大比例重构必须双线并行**：若升级将替换产品核心、主要数据合同、部署方式或大部分模块，以至于无法通过有界 PR 让 `main` 始终保持可发布，则必须建立独立的长期 `remake/<cycle>` 分支或等价隔离产品线。原项目继续接受必要维护、安全修复与数据导出支持；remake 不得在尚未验证时覆盖、冻结或拖垮原主线。
5. **市场验证后才能替代**：remake 启动前必须定义可复核的市场判据与观察窗口，例如自愿试用/迁移、留存、核心任务成功率、稳定性、用户反馈和继续使用意愿。开发 agent、自测分数、代码更新或单次演示不能代表“市场认为更好”。只有证据持续表明 remake 整体优于原项目，并经用户明确批准，才允许按功能、用户和数据批次逐步替代；旧线在迁移完成、回滚窗口结束和用户资产验证通过前不得彻底下线。
6. **破坏旧结构，不破坏用户资产**：可以主动打破内部 API、目录和实现兼容性，但不得静默损坏用户数据、角色卡、世界书、会话、记忆或可恢复能力。任何不兼容变化必须提供版本化 migration、升级前备份、完整性验证和可演练回滚；若决定放弃兼容，必须先获得用户明确批准并提供可读导出路径。
7. **先独立重审，再决定重写**：不得把旧文档、旧测试或旧架构当作不可质疑的前提，也不得为了“破坏式”而无证据重写。先从产品目标、真实用户工作流、安全边界、当前源码和可重复证据重新审计，明确保留、替换、删除与新增内容及其理由。
8. **仍受全部门禁约束**：本特例只扩大可修改范围，不豁免第三方独立实现、许可证/provenance、安全、测试、神圣提示词不变式、PR 审计和人工 review。大型升级应使用独立 epic 与可验收 PR 序列；只有 migration、回滚、生产拓扑和 release-candidate 证据全部通过后才能宣称完成。
9. **完成后重置事实入口**：升级完成后必须重新校准 `CURRENT-BASELINE.md`、开发指南、架构/安全/风险文档和相关专题合同；被淘汰的结论进入历史归档，不得继续作为当前能力或任务依据。

### 第三方经验吸收与独立实现（2026-07-11 用户立）

对任何用于“学习、参考、借鉴、思维窃取、功能对标”的**第三方项目**，只允许吸收理念、需求洞察、公开行为和互操作性经验；AIRP 的实现必须从自身需求与架构边界出发，**完全独立设计并重构，不复用第三方实现代码**，避免版权与许可证风险。

执行要点：
- 不复制、翻译、改写或移植第三方源码、规则文本、prompt、测试、数据集、HTML/CSS、图标及其他视觉资产；即使其许可证表面允许，也默认按独立实现处理
- 可以研究公开文档、界面行为、协议/格式及功能结果，但产出必须采用 AIRP 自己的 domain model、命名、控制流、安全边界和测试
- 研究后在 `docs/ACKNOWLEDGEMENTS.md` 记录项目、吸收的经验、固定版本/日期和许可证；不得把“理念参考”写成已交付能力
- 若任务确实需要引入普通第三方依赖库，必须单独核验许可证、锁定版本并记录 provenance；这不属于“思维吸收后重构”，不得借本条绕过依赖合规
- AIRP-Core/AIRPCLI、AIRP-MCP-Server、AIRP-Gateway、AIRP-State-Protocol 均为作者自己的第一方前序项目，不适用“第三方代码不得复用”的限制，但仍须按 `docs/SOURCE-PROJECT-DECISIONS.md` 复核后吸收

### 流程现状（2026-07-14 更新）

审计 bot 已恢复，并重新作为 §11.1「PR → 审计 → 合并」的**阻塞门禁**。本地测试全绿（含神圣不变式 `subagent_context_has_no_orchestrator_noise`）后可以开 PR，但不得在审计 bot pending、失败或仍有阻塞意见时合并；必须等待审计完成、修复全部阻塞意见并取得通过状态，再由人工 review 决定是否合并。审计 bot 与任何临时审计 agent 均须遵守本守则的独立审计、可提己见、可质疑历史并查证三原则。

### 审计遗留项处理（2026-07-06 用户立）

审计报告中常出现"未改动但写出来的修改意见"（即非阻塞、留 PR 后续、可后续迭代、未来改进等结论）。这些意见不应随 PR 合并而丢失。**PR 合并后，执行审计的 agent 必须将所有"未改动但写出来的修改意见"整理后写入 GitHub issue**，便于后续追溯与跟进。

**审计文件归档（2026-07-20 立规）**：审计报告（`docs/audits/PR-*.md` 或带日期的 `2026-MM-DD-PR-NN-*.md`）随被审计 PR 同分支提交、合并到 main，是**仓库惯例**，不构成审计独立性问题。理由：审计 agent 在 PR 合并前必须独立完成审计并给出阻塞/通过裁决，被审计方无法在合并前修改审计产物（PR 中审计文件的任何改动都会触发新一轮审计）；审计文件随 PR 归档仅为可追溯性需要（"哪条结论来自哪个 PR 的哪次审计"）。若未来需要更强隔离，可改为审计文件由独立 commit 在 PR 合并后追加到 main，但当前惯例维持不变。

**时序约束（2026-07-06 用户立）**：issue 提交**必须在 PR 合并之后**执行。PR 合并前审计遗留项清单可能因 review 反馈、代码调整而变化，提前提交会污染 issue 列表并产生失同步风险。PR 合并后，审计遗留项的"未修"状态才被锁定为最终事实，此时提交 issue 才是准确的。

执行要点：
- 去重：同一问题在多个审计报告中重复出现时，合并为一条
- 分类：按 engine / webui / docs / process 等模块分 issue
- 标注来源：每条注明来源审计报告与原编号（如 PR #38 A1、W-06）
- 优先级建议：每条标严重度与建议时机
- 不修项也要记录：明确"不修"的项应记录原因，避免未来重复提出
