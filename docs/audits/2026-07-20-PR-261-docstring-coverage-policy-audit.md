# PR #261 独立审计报告 — DEV-GUIDE docstring 覆盖政策（#240）

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-20
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#261 docs(dev-guide): document docstring coverage policy (#240)](https://github.com/GhostXia/AIRP/pull/261)
- **分支**：`codex/docs-240-docstring-policy`
- **base**：`main`（mergeStateStatus: CLEAN，mergeable: MERGEABLE）
- **commits**：单 commit（28 行新增）
- **CI**：Rust workspace SUCCESS / UI and WebUI SUCCESS / Production topology SUCCESS / CodeRabbit SUCCESS

## 1. 范围与背景

#240 是 PR #238（feat(webui): ship portable Windows package）的 CodeRabbit final
review 遗留 issue。PR #238 在 final review 上报 "70% vs 80% docstring coverage"
warning；warning 源于内部可执行入口 helper / tests 而非未文档化的 public API。

#240 验收要求（4 条）：
1. document the intended scope of the docstring coverage metric
2. either add useful comments to in-scope private helpers or configure justified exclusions
3. avoid comments that merely restate function names
4. keep Rustdoc warning-free

本 PR 选择"记录 scope + 配置 exclusions"路线，在 `docs/DEV-GUIDE.md` §6 末尾
新增 `#### Docstring 覆盖政策（#240）` 小节（28 行）。

## 2. 独立证据

### 2.1 PR diff（`docs/DEV-GUIDE.md` L168-L194）

新增小节结构：

| 段落 | 内容 |
|---|---|
| 引言 | 区分第三方工具覆盖率（参考信号）vs `cargo doc -W missing-docs`（维护清单）|
| Scope 对照表 | `cargo doc -W missing-docs` (public item) vs CodeRabbit (含 private/tests) |
| 原则 1（优先级） | public API 合同 > private helper 实现 > tests 注释 |
| 原则 2（注释质量） | 禁止"重述函数名"注释（举例 `fn parse_id()` + `/// Parse the id`） |
| 原则 3（private helpers 决策） | 内部可执行入口 / test helper / 私有 helper 默认不在覆盖范围 |
| 原则 4（门禁不变） | `cargo doc -D warnings` 不变；CodeRabbit 覆盖率警告不阻塞 PR 合并 |
| 原则 5（基线管理） | 若未来纳入门禁，必须先记录基线（区分 public/private/tests） |

### 2.2 #240 验收要求对照

| #240 验收 | PR #261 落实 | 满足？ |
|---|---|---|
| 1. document intended scope | Scope 对照表 + 原则 1 + 原则 3 明确 public vs private/tests | ✅ |
| 2. add useful comments OR configure exclusions | 原则 3 配置 justified exclusions：private helpers / tests / 内部入口默认排除 | ✅ |
| 3. avoid restating function names | 原则 2 明确禁止重述性注释，举例 `fn parse_id()` + `/// Parse the id` | ✅ |
| 4. keep Rustdoc warning-free | 原则 4 明确 CI `cargo doc -D warnings` 不变 | ✅ |

4 条验收全部满足。

### 2.3 与 §6 已有内容的衔接

DEV-GUIDE.md §6 在 L152-L166 已有 "Rustdoc 采用合同正确性优先策略" 与
`cargo doc -W missing-docs` 维护清单说明。新小节插入在 L166 之后，承接上文
"维护清单，不是 CI 门禁"的语义，进一步明确"第三方工具覆盖率 ≠ 维护清单"。
衔接自然，无重复。

### 2.4 与项目规则的一致性

根 `AGENTS.md` "Engineering Conventions" 中关于测试的规则未涉及 docstring
覆盖率。本 PR 的政策与项目规则一致：
- 不引入新 CI 门禁（CI `cargo doc -D warnings` 不变）
- 不要求 private helpers / tests 补注释
- CodeRabbit 警告定位为参考信号

### 2.5 政策可执行性

原则 3 提供 PR review 中的可操作话术："private helper，不在覆盖范围"。开发者
在 PR review 中可以直接引用此话术回应 CodeRabbit 警告，避免逐 PR 重复讨论。

原则 5 明确"若未来纳入门禁"的迁移路径，避免政策被锁死。

## 3. 独立意见（按 §Audit Agent Charter 第 2 条）

### 3.1 关于"配置 exclusions"而非"补注释"路线

PR 描述说明：CodeRabbit 报警源于 private helpers / tests / 内部可执行入口，
不是未文档化的 public API；为这些项批量补注释会违反"禁止重述函数名"原则，
提升覆盖率但不增加合同价值。本审计同意此判断。

但本审计进一步建议：政策原则 3 的"默认排除"不应被解读为"private helpers
永远不需要注释"。如果某个 private helper 实现了非显然的不变式（如安全边界、
并发约束、复杂的算法步骤），仍应补注释。本 PR 的政策原则 2 已涵盖这一情形
（"只在能解释公共 API 合同、错误语义、副作用、并发或安全边界时才补注释"），
但原则 2 的措辞偏向 "public API 合同"。建议未来在 #240 后续 PR 中明确"private
helper 的非显然不变式也建议补注释"。

此为非阻塞改进建议。

### 3.2 关于 CodeRabbit 警告不阻塞合并

原则 4 明确"CodeRabbit 覆盖率警告不阻塞 PR 合并，除非覆盖率下降源于 public
API 缺文档"。本审计同意此门禁策略：
- CodeRabbit 警告是参考信号，非合同；
- `cargo doc -D warnings` 是合同门禁；
- 二者分离符合"合同正确性优先"原则。

但本审计建议：若 PR 引入新的 public API 且未补 docstring，CodeRabbit 警告
应升级为阻塞（因为 public API 缺文档会同时触发 `cargo doc -W missing-docs`
和 CodeRabbit）。这一规则在原则 4 中已隐含（"除非覆盖率下降源于 public API
缺文档"），但建议未来在 #240 后续 PR 中明确"public API 缺文档"的具体判定
（如 `cargo doc -W missing-docs` 是否报警）。

此为非阻塞改进建议。

### 3.3 关于政策原则 5 的"基线管理"

原则 5 明确"若未来纳入门禁，必须先记录基线（区分 public/private/tests）"。
本审计同意此预防性条款：避免未来政策变化时既有缺口阻止无关修复。

但本审计建议：当前 PR 应同时记录 2026-07-20 的覆盖率基线（70% CodeRabbit /
当前 `cargo doc -W missing-docs` 维护清单条目数），作为政策未来的"事实基线"。
否则原则 5 的"基线"概念只是抽象约束，无具体数据支撑。

此为非阻塞改进建议，可在本 PR 合并后单独跟进。

## 4. 风险评估

| 风险 | 评级 | 说明 |
|---|---|---|
| 政策歧义 | 低 | 5 条原则措辞清晰，可操作话术明确 |
| 与既有 §6 内容重复 | 零 | 新小节承接 §6 已有"维护清单"语义，无重复 |
| 政策与项目规则冲突 | 零 | 不引入新 CI 门禁，符合根 AGENTS.md 约束 |
| 政策锁死 | 低 | 原则 5 明确未来基线管理路径 |
| CI flaky | 零 | CI 4/4 SUCCESS |

## 5. 阻塞项

无。所有 CI 通过，#240 验收 4 条全部满足，政策可执行。

## 6. 非阻塞 / 后续可追踪项

| 编号 | 内容 | 建议 |
|---|---|---|
| 261-A1（非阻塞） | 原则 2 措辞偏向 public API；建议明确 private helper 非显然不变式也应补注释 | 跟进 #240 后续 PR |
| 261-A2（非阻塞） | 原则 4 中"public API 缺文档"的具体判定未明 | 跟进 #240 后续 PR，明确以 `cargo doc -W missing-docs` 为判据 |
| 261-A3（非阻塞） | 原则 5 的"基线"无具体数据 | 跟进 #240 后续 PR，记录 2026-07-20 覆盖率基线 |

## 7. 审计结论

**通过（PASS，无阻塞项）**。

PR #261 是 #240 验收的精准落实：4 条验收全部满足，5 条原则可执行，CI 全绿，
scope 严格限定 docstring 覆盖政策文档化。可合并。

## 8. Refs

- Issue #240（来源）
- PR #238（CodeRabbit final review 报警来源）
- 根 `AGENTS.md` §Audit Agent Charter
