# AIRP Agent Memory

## Local Build Environment

- This Windows workspace keeps build tooling on `D:`. Do not install Rust, Cargo, Node, npm globals, MSYS2, caches, or generated build dependencies under `C:`.
- Confirmed local toolchain roots:
  - `RUSTUP_HOME=D:\.rustup`
  - `CARGO_HOME=D:\.cargo`
  - Rust shims: `D:\.cargo\bin`
  - MSYS2/GNU linker path: `D:\msys64\mingw64\bin`
  - Node.js: `D:\nodejs`
  - npm global prefix: `D:\npm-global`
  - npm cache must be forced to `D:\npm-global\npm-cache` because the default may point to `C:\Users\<user>\AppData\Local\npm-cache`
- Before local Rust builds/tests in PowerShell, set:
  ```powershell
  $env:RUSTUP_HOME = "D:\.rustup"
  $env:CARGO_HOME = "D:\.cargo"
  $env:npm_config_prefix = "D:\npm-global"
  $env:npm_config_cache = "D:\npm-global\npm-cache"
  $env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
  ```
- Use the default repo target directory `D:\AIRP-Dev\target` unless a task explicitly requires otherwise.
- If a command tries to populate `C:\Users\<user>\.cargo`, `C:\Users\<user>\.rustup`, or npm cache/global data under `C:`, stop and redirect it to the D-drive locations above.

## Audit Agent Charter（审计 agent 守则，2026-07-03 用户立）

任何在本仓执行审计的 agent —— 无论常驻 bot、一次性 review agent、还是被临时指派做 review 的开发 agent —— **必读并遵守以下三条**：

1. **独立审计**。不附和开发 agent 的结论，不把开发文档/现有代码/现有结构当作不可质疑的前提。以"我会不会这样写"的独立判断为准。
2. **可以提出自己的想法**，不必拘泥于开发文档、现有代码和现有结构。若你认为有更好的设计/实现/取舍，直接说出你的方案及理由，哪怕它与既定路线相悖。
3. **agent 的智能是不断迭代增强的**。因此曾经审计/开发此仓的 agent 的能力和眼界，可能远不如现在你这一代。必要时对历史决策产生质疑并主动查证（读源码、跑测试、查文档），不要因"前人已定"就放过可疑处。

### 项目取向（同源用户指示，2026-07-03）

代码应当：**更开放、更透明、在未来更易修正、且更易迭代更新**。审计与开发决策都向这四点对齐。

### 流程现状（同日更新）

原"审计 bot 复核"环节已不存在（bot 已下线）。§11.1 的"PR → 审计 → 合并"现由**开发者自审 + 人工 review** 承接：本地测试全绿（含神圣不变式 `subagent_context_has_no_orchestrator_noise`）即可开 PR，由人决定合并；不阻塞在"等审计 bot"。未来若重新引入审计 agent，本守则即为其入职文档。

### 审计遗留项处理（2026-07-06 用户立）

审计报告中常出现"未改动但写出来的修改意见"（即非阻塞、留 PR 后续、可后续迭代、未来改进等结论）。这些意见不应随 PR 合并而丢失。**PR 合并后，执行审计的 agent 必须将所有"未改动但写出来的修改意见"整理后写入 GitHub issue**，便于后续追溯与跟进。

**时序约束（2026-07-06 用户立）**：issue 提交**必须在 PR 合并之后**执行。PR 合并前审计遗留项清单可能因 review 反馈、代码调整而变化，提前提交会污染 issue 列表并产生失同步风险。PR 合并后，审计遗留项的"未修"状态才被锁定为最终事实，此时提交 issue 才是准确的。

执行要点：
- 去重：同一问题在多个审计报告中重复出现时，合并为一条
- 分类：按 engine / webui / docs / process 等模块分 issue
- 标注来源：每条注明来源审计报告与原编号（如 PR #38 A1、W-06）
- 优先级建议：每条标严重度与建议时机
- 不修项也要记录：明确"不修"的项应记录原因，避免未来重复提出
