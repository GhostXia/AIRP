# PR #257 独立审计报告 — baseline bucketing rule + SBOM meta.repo_root 文档

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-20
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#257 docs: tighten baseline bucketing rule and document SBOM meta.repo_root](https://github.com/GhostXia/AIRP/pull/257)
- **分支**：`codex/docs-230-baseline-bucketing`
- **commits**：
  - `5a8dc65 docs: tighten baseline bucketing rule and document SBOM meta.repo_root`（PR 主改动）
  - `e733185 docs(dep-governance): clarify meta.repo_root is hardcoded "." not --repo-root value`（CodeRabbit actionable 跟进）

## 1. 范围与背景

PR #257 关闭 #230 中第 2 项与第 5 项遗留（PR #229 audit leftover）：

- **#230 第 2 项**：`docs/sbom/` 的 `metadata.repoRoot` 跨环境漂移（CI `/workspace`、本机 `D:\AIRP-Dev`），但仓库无文档说明这非 bug
- **#230 第 5 项**：PR #229 A1 暴露的 baseline 数字按 test binary 分桶规则未在 `DEV-GUIDE.md` 固化

PR 改动只有 1 个 commit + 1 个 CodeRabbit 跟进 commit，共 11 行文档：

1. `docs/DEV-GUIDE.md` §8 加 1 条 bullet：baseline 测试计数分桶规则
2. `tools/dep-governance/README.md` §"airp.inventory.json" 加 10 行段落：`meta.repo_root` 字段语义说明（`e733185` 修订为更准确描述）

## 2. 独立证据

### 2.1 DEV-GUIDE.md §8 baseline 分桶规则 — 静态读

新增 bullet：

> - baseline 测试计数按 test binary 分桶（engine lib / engine integration / 其他 workspace member lib+main / WebUI `node --test` / `ui/` Vitest / dep-governance `node --test`），禁止把跨 crate test binary 计数简单相加后贴 "integration" 标签；新增 binary 时同步更新分桶表；

**与 #230 第 5 项比对**：

#230 第 5 项原建议：
> baseline 数字必须按"engine lib / engine integration / 其他 workspace member lib+main"分桶，禁止把跨 crate test binary 计数简单相加后贴 "integration" 标签。

PR 的版本：
- 保留 "engine lib / engine integration / 其他 workspace member lib+main" 三个核心桶
- 扩展 "WebUI `node --test` / `ui/` Vitest / dep-governance `node --test`" 三个额外桶
- 加 "新增 binary 时同步更新分桶表" 操作约束

PR 实际**扩展**了 #230 的建议，覆盖更全。这是审计 charter §2 "可提己见"的合理运用——#230 的建议只覆盖 Rust workspace 三个桶，但 AIRP 实际还有 WebUI / Vitest / dep-governance 三类 Node.js test binary，分桶规则应覆盖全部。

**与实际 test binary 比对**：

| 桶 | 实际 binary | 对应 |
|----|------------|------|
| engine lib | `cargo test --package airp-core --lib` | ✓ |
| engine integration | `cargo test --package airp-core --test '*'` | ✓ |
| 其他 workspace member lib+main | `cargo test --package airp-ui`、`cargo test --package airp-mcp-server` 等 | ✓ |
| WebUI `node --test` | `node --test webui/tests/*.test.mjs` | ✓ |
| `ui/` Vitest | `npx vitest run` (in `ui/`) | ✓ |
| dep-governance `node --test` | `node --test tools/dep-governance/*.test.mjs` | ✓ |

6 桶覆盖当前 AIRP 全部 test binary，无遗漏。

### 2.2 tools/dep-governance/README.md `meta.repo_root` — 静态读 + 实跑验证

新增段落（`e733185` 修订后版本）：

> The document also carries a `meta` block with a `repo_root` field.
> `discover-deps.mjs` always writes the literal `"."` as an intentional
> environment-independent marker, regardless of the value supplied through
> `--repo-root`. It is NOT the relative working directory passed via
> `--repo-root`, nor an absolute path. The field is a diagnostic to identify
> which invocation produced the inventory; it is NOT part of any component's
> dependency identity. Earlier versions wrote `process.cwd()` which caused
> CI vs maintainer-machine drift (`/workspace` vs `D:\AIRP-Dev`); the hardcoded
> `"."` removes that drift without losing provenance.

**与实际实现比对**：

`tools/dep-governance/discover-deps.mjs:481-483`：
```js
const meta = {
  generated_at: new Date().toISOString(),
  repo_root: ".",
```

确认：`repo_root` 字段写死 `"."`，与 `--repo-root` 参数无关，也不是 `process.cwd()`。
README.md 描述（`e733185` 修订后）与实现完全一致。

**与 #230 第 2 项比对**：

#230 第 2 项建议：
> `metadata.repoRoot` 反映调用时的 `process.cwd()`，由调用环境决定（CI 容器 `/workspace`、维护者本机 `D:\AIRP-Dev` 等）...

#230 建议的措辞基于"metadata.repoRoot 反映 process.cwd()"的假设。但实际实现
（`discover-deps.mjs:483`）已经把 `repo_root` 写死为 `"."`，所以 #230 的"假设"
不成立。PR 的描述（修订后）准确反映了实际行为：hardcoded `"."` 而非 `process.cwd()`。

这是审计 charter §1 "独立审计、不附和"的合理运用——#230 是基于旧版本（写 `process.cwd()`）
的描述，PR 修订后基于实际实现（hardcoded `"."`）描述，比 #230 建议更准确。

**与 inventory.json 实际产物比对**：

`docs/sbom/inventory.json` 中的 `metadata.repoRoot` 字段（如果存在）应与 README.md
描述一致。审计未实跑 `discover-deps.mjs` 重新生成 SBOM 验证，但静态读 L483 已确认
实现写死 `"."`。

### 2.3 文档行号变化检查

`docs/DEV-GUIDE.md` 加 1 行，原 L228 "稳定合同写进..." 不变，仅 L202 后多 1 行。
对其他文档引用无影响（grep `DEV-GUIDE.md#L` 在仓库内无精确行号引用）。

`tools/dep-governance/README.md` 加 10 行，原 §"airp.spdx.json" 标题位置下移。
对其他文档引用无影响。

## 3. 阻塞意见

无。

## 4. 非阻塞 / 可后续

| # | 项 | 严重度 | 建议时机 |
|---|----|--------|---------|
| N-1 | README.md 段落说"Earlier versions wrote `process.cwd()`"，但仓库 git log 没有 `process.cwd()` 写入 `repo_root` 的历史 commit 可直接验证。这个历史叙述依赖 #230 的 issue 描述，未来读者若想 git blame 验证会找不到。建议补一句"see #230"作为溯源，或不补——这是文档惯例问题，不阻塞 | 极低 | 不跟进 |
| N-2 | DEV-GUIDE.md §8 bullet 把"`ui/` Vitest"列为独立桶，但 `ui/` 当前用 `node --test` 而非 Vitest（除非 PR #251 之外另有变更）。审计未实跑 `ui/` 测试 binary 确认；如果 `ui/` 实际仍用 `node --test`，bullet 应改为"`ui/` Playwright/node test"或合并到"WebUI `node --test`"桶 | 低 | 下次 baseline 校准或 `ui/` 测试 runner 变更时 |
| N-3 | #230 第 1/3/4 项不在本 PR 范围，#230 关闭时应明确"仅第 2/5 项实现，第 1/3/4 项维持 open" | 低 | PR 合并后跟进 #230 |

## 5. 神圣不变式

- 本 PR 是 docs-only，不触 engine、http 边界、subagent context、normalizer 或神圣提示词不变式。✓
- 不修改任何代码、测试、配置、SBOM 产物本身，仅文档。✓

## 6. 结论

**通过**。

- DEV-GUIDE.md §8 baseline 分桶规则：覆盖 AIRP 当前 6 类 test binary，扩展 #230 第 5 项建议。
- README.md `meta.repo_root` 描述（`e733185` 修订后）：与 `discover-deps.mjs:483` 实际实现一致，比 #230 第 2 项建议更准确（实际是 hardcoded `"."` 而非 `process.cwd()`）。
- 11 行 diff 全部为文档，无行为变化；Rust workspace + dep-governance `node --test` CI 全绿。
- 无阻塞意见。N-1 不跟进、N-2/N-3 留 PR 合并后跟进。
