# PR #65 审计报告

> **审计源 LLM**：GLM-5.2（智谱 AI, 2026）
> **审计日期**：2026-07-06
> **PR**：https://github.com/GhostXia/AIRP/pull/65
> **标题**：M3 import safety closeout: HTTP-level regression + multipart 不实施声明
> **改动**：3 commits；代码改动 2 文件 +141 / -0；含审计报告 1 文件
> **裁决**：✅ 通过，建议合并（v3 复审采纳 Gemini 建议做小重构后通过）
>
> **流程备注**：审计报告 `docs/audits/PR-65-audit.md` 已被提交到本 PR 分支。严格意义上审计产物不应由被审计 PR 自己携带；但仓库惯例为 `docs/audits/` 随 PR 归档，且不影响代码正确性。本次审计仍按独立判断执行。

---

## 1. 审计范围与方法

PR 范围极小（+141 / -0）：
- `engine/src/daemon/handlers.rs` 加 2 个 HTTP-level 回归测试
- `docs/WEBUI-BACKEND-VALIDATION.md` 追加 M3 收口声明

**独立审计三问**：
1. **测试代码本身**有没有 bug / race / 断言错误？
2. **PR 描述的"不实施 multipart"决策**是合理工程判断还是回避债务？
3. **RR-001 护栏**是否真在 HTTP-level 上被覆盖？

---

## 2. v3 复审（第三次审计，合并前最终检查）

### 2.0 v3 新增审查：Gemini Code Assist 建议处理

Gemini 在 v2 后提了一条 review：

> The reviewer suggested refactoring the tests to extract duplicate `DaemonState` initialization boilerplate into a shared helper function to improve maintainability.

**独立判断**：建议合理。两个 `m3_*` 测试确实有约 13 行 `DaemonState` + `MutableConfig` 初始化代码完全重复。既有的 `make_state_with_key` / `make_state_no_key` 在 `mod.rs::tests` 模块内，跨模块无法直接用。在 `handlers.rs::tests` 内加 `make_state_for_http_test()` helper 是干净的解决方案。

**处理**：✅ 采纳，已抽 `make_state_for_http_test() -> (Arc<DaemonState>, TempDir)`。helper 返回 `_tmp` guard 持有 tempdir 防止目录被早回收。

### 2.1 新增审查：审计文件自身的独立性

| 项目 | 结论 |
|---|---|
| `docs/audits/PR-65-audit.md` 是否随 PR 提交 | ⚠️ 是，本 PR 分支包含审计报告文件 |
| 这是否违反独立审计原则 | ⚠️ 严格意义上是 — 审计产物不应由被审计 PR 自己携带 |
| 仓库是否有此惯例 | ✅ 是，PR #60/#62/#63 审计报告均通过 PR 提交到 `docs/audits/` |
| 是否影响代码正确性 | ✅ 否，审计文件为 Markdown，不影响编译与运行时 |
| 是否影响合并裁决 | ✅ 否，仍建议合并 |

### 2.1 代码审查

| 项目 | 结论 |
|---|---|
| ENV_LOCK 加锁 | ✅ 两个测试都正确加锁，与既有 unit test 串行 |
| `super::super::DaemonState` / `create_router` 路径 | ✅ handlers.rs 位于 `daemon::handlers` module，向上两级为 `crate::daemon` 模块，正确 |
| 拒绝测试用 `/etc/passwd` | ✅ env 门控先于 fs::read 触发，跨平台一致 |
| happy-path `Http_M3_Test` id 合法性 | ✅ `validate_id_segment` 只拒 `\0 / \\ / : * ? " < > \|` 与 `..`，大写+下划线合法 |
| 测试 body `Content-Type: application/json` | ✅ 匹配 `Json<ImportCharacterRequest>` extractor |
| `oneshot` 单次请求与 governor 限流 | ✅ 限流只在并发触发，单请求不会触限 |
| `assert_eq!(v["character_id"], "Http_M3_Test")` | ✅ slugify 不小写化，空格→`_`，断言正确 |

### 2.2 PR body / 文档审查

| 项目 | 结论 |
|---|---|
| 决策「不实施 multipart」4 条理由 | ✅ 前 3 条（目标已达成 / 边际价值低 / harness 定位）足够撑住决策；第 4 条（multipart 引入新攻击面）**略夸张**（axum 内置 Multipart extractor 已处理生命周期与解析），但作为补充论据可接受 |
| 验收证据块 | ✅ 引用 `cargo test` 数字与神圣不变式，可复查 |
| 未来触发 multipart 的条件 | ✅ 3 条均合理，作为独立 PR 评估入口 |
| 实现位置索引 | ✅ 引用行号准确（`handlers.rs:319-330` 门控、`mod.rs:200` 10MB limit） |
| 路径引用 | ✅ 准确（`webui/app.js:800-831` 注释明确「NEVER card_path」） |

### 2.3 测试结果复核

- `cargo test -p airp-core --lib daemon::handlers::tests::m3_` → **2 passed**
- `cargo test -p airp-core` → **358 passed; 0 failed**（339 unit + 3 + 11 + 5 integration）
- 神圣不变式 `subagent_context_has_no_orchestrator_noise` + `subagent_prepared_pipeline_has_no_orchestrator_noise` → **2/2 passed**

### 2.4 CodeRabbit / Gemini 交叉参考

- **CodeRabbit**：v3 复审时 `statusCheckRollup` 显示 `CodeRabbit: SUCCESS`，但实质性 review 仍因限流未输出
- **Gemini Code Assist**：v2 后输出一条 review，建议抽 `DaemonState` 初始化样板（见 §2.0，已采纳）
- 本次审计以 GLM-5.2 独立判断为准，Gemini 建议经独立评估后采纳

---

## 3. 独立判断的"非阻塞"观察（不阻止合并）

| 观察 | 评估 |
|---|---|
| 新测试 panic 时不清理 `AIRP_ALLOW_LOCAL_PATH` env | ⚠️ 当前所有 env 测试都加锁，串行执行，**无实际 race**；建议**未来**为 `ENV_LOCK` 加 `scopeguard` 风格 RAII 清理，可单独立 PR |
| happy-path 测试价值 | ⚠️ 与 `card_path_import_allowed_with_local_path_env` 单测有重叠（happy-path 等价于"用 env 门控外的路径导入"），主要价值是给 HTTP-level 补一笔，**可接受** |
| PR body 第 4 条理由略夸张 | ⚠️ 建议未来若触发 multipart 评估，应正面回应此条（axum Multipart extractor 已处理大部分攻击面） |
| 审计文件随 PR 提交 | ⚠️ 严格独立审计原则下这是流程瑕疵；但仓库惯例如此，且不影响代码正确性 |

以上 4 条都是**未来改进项 / 流程改进项**，不是本 PR 阻塞点。

---

## 4. 关键独立判断：决策合理性

**审计守则要求"不附和开发 agent 的结论"**。本 PR 的核心决策是「不实施 multipart」 — 我**支持**此决策，理由：

1. **客观安全现状**：RR-001 护栏已通过 env 门控在进程启动时定 + 单元测试覆盖 + HTTP-level 覆盖三道防线守住。WebUI 自身不发 `card_path`。
2. **multipart 优化 vs 风险**：即使使用 axum 内置 Multipart extractor，仍需 `tempfile` 清理、字段名验证、part 大小限制、Content-Type 边界解析等。harness 场景下不值得。
3. **路线图一致性**：WEBUI-BACKEND-PLAN §10 明确「不做 WebUI 产品化」，multipart 属于产品级特性，与定位不符。
4. **未来触发条件清晰**：3 条触发条件是工程上可量化（用户开始长期用 / >8MB 频繁 / 第三方 widget 需要）。

**决策非"回避债务"，而是"在当前定位下不做非必要工作"**。债务以"未来触发条件"形式显式记录，治理透明度高。

---

## 5. 结论

| 项目 | 结论 |
|---|---|
| 测试代码质量 | ✅ 高（v3 抽 helper 后可维护性提升） |
| 文档质量 | ✅ 高（决策 + 理由 + 验收证据 + 未来触发条件齐全） |
| 决策合理性 | ✅ 工程上合理，与 harness 定位一致 |
| 测试覆盖 | ✅ 358 passed / 0 failed；神圣不变式 2/2 |
| 是否合并 | ✅ **建议合并** |

本 PR 是 webui/ 后端建设计划 M0-M3 的**收口 PR**，通过它 webui/ 短期路线正式完结。建议合并时保留 squash commit，审计报告作为文档 commit 一同归档。

---

## 6. 审计源

- **LLM 模型**：GLM-5.2（智谱 AI, 2026）
- **独立判断依据**：源码 `engine/src/daemon/handlers.rs:319-330`、`engine/src/daemon/mod.rs:172-201`、`engine/src/types.rs:19-36`、`engine/src/data_dir/security.rs:96-121`、`webui/app.js:800-831`
- **审计方法**：直接读 PR diff + 源码核实 + 跑测试 + 独立判断决策合理性
