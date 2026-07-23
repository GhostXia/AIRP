# PR #300 独立审计报告（WorkBuddy 补充审计，归档）

> **审计模型**：WorkBuddy 当前会话（独立验证，未采纳分支内既有审计报告结论）
> **审计时间**：2026-07-23
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：`docs/AGENT-BROWSER-EXPLORATION-PLAN.md`（PR #298 基线）+ PR #300 实施 commit
> **结论**：**3 阻塞（B1-B3，必须修复后合并）+ 3 非阻塞（N1/N4/N5）+ 1 新增（S1）+ 纠正 1 项（N2 不实）**
>
> **归档说明（2026-07-23）**：本审计由 WorkBuddy agent 在 PR #300 合并后独立完成，原文件位于 `.workbuddy/audit-pr300-comment.md`（未跟踪目录，随 PR #301 期间的 `.git` 破坏事件丢失风险高）。现归档至 `docs/audits/` 以满足 AGENTS.md「审计文件归档」可追溯性要求。与 `2026-07-23-PR-300-agent-browser-exploration.md`（M3.2 审计）互为补充：本审计纠正了该报告的 N2（不实）并新增 S1（DOM 脱敏隐私泄漏）；B1/B2/B3 与该报告重叠但描述更精确。归档时未修改任何审计结论原文。

## 0. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| webui harness 测试 | `node --test webui/tests/agent-harness.test.mjs` | ✓ 10 pass / 0 fail |
| classifier 测试 | `node --test tools/agent-exploration/classifier.test.mjs` | ✓ 6 pass / 0 fail |
| 7 个 .mjs 语法 | `node --check` | ✓ 全过 |
| 分类器真实行为 | 我跑的 3 个 case（见 §1 B1） | ⚠️ 纯路径过滤，关键字失效 |
| 计划/注释 vs 代码 | 直接读源 | 见各阻塞项 |

## 1. 阻塞项（必须修复后合并）

### B1. 分类器实际是「纯路径过滤」，关键字完全失效，且与计划注释矛盾

**位置**：`tools/agent-exploration/classifier.mjs:2`（注释）vs `:36-37`（逻辑）

**代码**：
```js
// :2  "命中规则按"文件路径模式 + 内容关键字"组合；只看 +/- 行。"
if (pathHit && keywordHit) hits.add(taskName);
else if (pathHit) hits.add(taskName);   // :36-37
```

**实证（我独立运行）**：
- `chat.rs` 仅改一行无关代码（无关键字）→ 触发 `["regen-swipe-refresh","edit-branch-switch-refresh"]` **两个**任务集
- `README.md` 含 `onboarding`/`swipe` 关键字但无命中路径 → `[]`
- `onboarding.js` 仅改 → 触发 `onboarding-firstchat-refresh`

**结论**：`else if (pathHit)` 吞掉了所有 path 命中，前面的 `pathHit && keywordHit` 分支是死代码；关键字既不独立触发也不收窄 → **分类器 = 纯路径过滤，整条 keyword 链路是装饰**。

**影响**：
- 计划承诺的「按内容智能选择任务集」未实现；
- `chat.rs`（engine 最高频改动文件）每次改动都跑 2/4 任务集（每集最长 30 步 LLM + Chrome），阶段 2 CI 成本不可控；
- 单元测试全过是因为每个用例同时含 path+keyword，恰好掩盖矛盾。

**修复方向（需确认）**：
- **A（推荐）**：实现 `path AND keyword`，并补「仅 path 不触发」「仅 keyword 不触发」单测；
- **B**：接受纯路径过滤，但必须改注释/计划显式声明「路径命中即触发、关键字仅文档用途」，并评估 `chat.rs` 触发 2 任务集的成本是否可接受。

### B2. runner 永远 exit 0，失败信号可能完全丢失；且注释自相矛盾

**位置**：`tools/agent-exploration/runner.mjs:87-91`

```js
// :87 "阶段 2: 任何 task Failed 即 exit 1（...）"
if (run.tasks.some(t => t.result === 'Failed')) {
  console.log('[runner] ' + ... + ' task(s) failed; see report');
  // :90 "不 exit 1；MVP workflow 是 non-blocking，只在 PR 评论里提示"
}
```

**问题**：`:87` 注释说「即 exit 1」、`:90` 注释说「不 exit 1」，代码实际**从不调用 `process.exit(1)`** → 永远 exit 0。

**影响**：
- workflow 设 `continue-on-error: true`（job 级）→ 全部任务失败 CI 仍绿；
- 唯一信号是 PR 评论；若评论 step 失败（`report.md` 未生成 / gh 不可用），失败信号**彻底消失**；
- 与 issue #273「输出可复现缺陷候选报告」承诺不符——无人可见即等于不存在。

**修复方向**：
- **A（推荐）**：runner 失败时 `exit 1`；workflow 保留 `continue-on-error`，但加 `if: failure()` step 发「⚠️ 探索失败」占位评论，确保总有信号。

### B3. runner tracing 异常路径资源泄漏，可致后续任务整批跳过

**位置**：`tools/agent-exploration/runner.mjs:97`（`tracing.start`）/ `:144`（成功 stop）/ `:159-163`（catch stop）/ `:164-166`（finally 仅 `context.close()`）

**风险**：若 catch 内 `context.tracing.stop()` 失败（上下文已损坏等），`finally` 的 `context.close()` 因 tracing 仍活跃而抛错，异常冒泡出 `runTask` → 外层 `for` 循环无 catch → **剩余任务全部跳过**，报告缺尾、审计无法区分「跑失败」与「没跑到」。

**同类缺口**：`:105-106` `page.goto` + `waitForReady` 在 per-task `try`（`:125`）**之外**，二者任一抛错同样整批跳过。

**修复**：
```js
} catch (err) {
  result.result = 'Failed';
  ...
  try { await context.tracing.stop({ path: tracePath }); result.evidence.trace = tracePath; }
  catch (e) { result.evidence.traceError = String(e); }
} finally {
  try { await context.tracing.stop(); } catch {}   // 先停 tracing 再关 context
  await context.close();
}
```
并将 `:105-106` 移入 try（或单独 try/catch 记失败而非中断循环）。

## 2. 非阻塞（建议修复，合并后转 issue）

### N1. v1 / v2 harness 同名全局 `__AIRP_AGENT_TEST__`
`ui/src/agent-test.ts:6,55` `version: 1`；webui harness `version: 2`；`harness-client.mjs:10` 强校验 `version === 2`。两 harness 从不同时加载、无运行时冲突，但维护易混。建议桌面侧改名 `__AIRP_AGENT_TEST_LEGACY__` 或 v1 显式标注。

### N4. reporter 截断 console error 无总数
`tools/agent-exploration/reporter.mjs:60` `slice(0,10)` 无「共 N 条」提示，审计员误判问题规模。建议补 `（共 N 条）`。

### N5. bootstrap-topology.sh state_file 路径固定，并发 PR 互相误删
`deploy/production/bootstrap-topology.sh:39` `state_file="$deploy/.bootstrap-topology.state"`（固定）；`:24` `smoke_id=${GITHUB_RUN_ID:-local}-$$` 含 `$$`（每 step 新 shell PID 不同）。并发 PR 各自写同一 state_file，Job A teardown 读到 Job B 的 `compose_project_name` → **误拆 B 的拓扑**。修复：`state_file` 用 `$GITHUB_RUN_ID`（跨 step 稳定、跨 run 唯一），如 `.bootstrap-topology.$GITHUB_RUN_ID.state`，teardown 按同变量读取。

## 3. 对分支内既有审计报告（docs/audits/2026-07-23-PR-300-...md）的纠正

- **N2（「workflow 缺 chat-space.js / memory/**」）：不成立。** 现 workflow `:8-19` 已含 `webui/assets/chat-space.js`（`:14`）与 `engine/src/memory/**`（`:12`），所有 live `DIFF_TASK_MAP` path 均被 workflow 覆盖（唯一死规则 `engine/src/daemon/handlers/onboarding` 因该文件不存在而无影响）。请勿据此固定块。
- 既有报告 B1 描述为「path OR keyword」——实测为「**纯路径，关键字完全无效**」，以本审计 §1 B1 为准。

## 4. 新增发现（既有报告未覆盖）

### S1. DOM 快照脱敏仅看元素自身属性，子节点文本泄漏（隐私）
webui harness `buildDomSnapshot` 输出**扁平数组（无父链）**；`runner.mjs:211-220` `sanitizeDomSnapshot` 仅当元素**自身** id/classes/role 命中 `messageLike` 才 `[REDACTED]`。于是：
```html
<div class="message"><span>用户私密内容</span></div>
```
中 `span` 因无 message 类而**原样发给外部 LLM**。计划承诺「DOM 快照脱敏 / 不读真实用户数据」，此实现在 DOM 结构变化下失效，给虚假安全感。MVP CI 用 mock + 合成数据风险低，但一旦操作者指向真实实例或外部 LLM 即泄漏。建议：harness 建快照时向上传播 sensitive 标记（含祖先命中），runner 按祖先脱敏；或默认对内容叶子节点脱敏。**非阻塞合并，但依赖该隐私声明前必须修。**

## 5. 裁决

| 类 | 数量 | 处理 |
|---|---|---|
| 阻塞 | 3（B1-B3） | 修复后重审 |
| 非阻塞 | 3（N1/N4/N5） | 合并后转 issue，不阻塞 |
| 新增 | 1（S1） | 合并后转 issue，依赖隐私声明前修 |
| 纠正 | 1（N2 不实） | 以本审计为准 |

**最终建议：暂不合并。** B1（分类器语义失真 + 阶段 2 成本不可控）、B2（失败不可见）、B3（异常整批跳过）均为小修复、方案明确，修后可重审。

---

**审计独立性声明**：本审计独立读源码、跑测试、实证分类器行为得出，未采纳分支内既有审计报告结论，并对其中不准确处（N2、B1 描述）做了纠正。
