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
