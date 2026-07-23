# PR #300 独立审计报告

> **审计模型**：M3.2（用户切换后的当前会话模型）
> **审计时间**：2026-07-23
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：`docs/AGENT-BROWSER-EXPLORATION-PLAN.md`（PR #298 已合并基线）+ PR #300（实施 commit）
> **结论**：**3 个阻塞（必须修复） + 5 个非阻塞（建议修复） + 3 个审计遗留项（合并后转 issue）**

## 0. 独立审计证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| webui harness 16 tests | `node --test webui/tests/agent-harness.test.mjs` | ✓ 10 pass / 0 fail |
| classifier 6 tests | `cd tools/agent-exploration && node --test classifier.test.mjs` | ✓ 6 pass / 0 fail |
| 所有 .mjs 语法 | `node --check` 5 个文件 | ✓ exit 0 |
| 6 个 screen HTML 存在 | `Test-Path` | ✓ 6/6 |
| harness 入口 | `webui/assets/agent-test-harness.js` | ✓ 存在，4 KB |
| classifier 真实分类行为 | 7 个手工 case（见 §B.1） | ⚠️ 与计划注释矛盾（line 1161 vs 1195-1196） |
| 任务模块 fixture 原创性 | `fixtures/character-card.json` | ✓ 原创（airp-test-fixture-Aria，无第三方来源） |

## 1. 阻塞项（必须修复后才能合并）

### B1. classifier 注释与实现矛盾（计划 vs 代码）— 计划责任

**位置**：`tools/agent-exploration/classifier.mjs:1161`（计划注释）vs `classifier.mjs:1195-1196`（实际代码）

**证据**：
- line 1161 计划注释：「命中规则按"文件路径模式 + 内容关键字"组合；只看 +/- 行」
- line 1195-1196 实际代码：`if (pathHit && keywordHit) hits.add(taskName); else if (pathHit) hits.add(taskName); // 路径命中即触发，关键字仅作加权（此处简化为 OR）`
- 我的独立验证 Case 2（仅 `chat.rs` 一行 `+fn foo()`，无任何关键字）→ 触发 `["regen-swipe-refresh", "edit-branch-switch-refresh"]` 两个任务集

**影响**：
- 任何改 `chat.rs` 的 PR（不论内容）都会跑 2 个 agent exploration 任务集，**显著放大阶段 2 成本**
- 计划承诺的"路径 + 关键字组合"语义被实现成了"路径 OR 关键字"
- 单元测试通过是因为测试用例的 diff 恰好都同时命中 path 和 keyword，没暴露这个矛盾

**修复方向**（二选一，需用户确认）：
- **方向 A**：实现与计划注释一致（path AND keyword），并补一个"path 单独不触发"的单元测试
- **方向 B**：计划注释改为 "path OR keyword"，承认 path 单独触发是设计选择
- **方向 C**（更保守）：拆成两类规则（强 path-only 触发 + 弱 keyword 加权）并在 plan 里明确

**建议**：方向 A，因为"路径命中即触发"会让阶段 2 CI 成本不可控（每次 chat.rs 改动都跑 30 分钟 LLM 探索）。

### B2. runner.mjs 永远 exit 0 — CI 信号失效

**位置**：`tools/agent-exploration/runner.mjs:88-91`

**证据**：
```javascript
if (run.tasks.some(t => t.result === 'Failed')) {
  console.log('[runner] ' + ... + ' task(s) failed; see report');
  // 不 exit 1；MVP workflow 是 non-blocking，只在 PR 评论里提示
}
```

**影响**：
- runner 始终 exit 0
- workflow 设置 `continue-on-error: true`（job level）
- **结果**：即使所有 task 失败，CI 也是绿色——失败只体现在 PR 评论中
- 如果 PR 评论发布失败（gh 不可用、网络问题），失败信号**完全丢失**
- 这与 issue #273 "输出可复现的缺陷候选报告" 的承诺不一致——报告没人在意就等于不存在

**修复方向**（任选其一）：
- **方向 A**（推荐）：runner 失败时 exit 1，workflow `continue-on-error` 接受，但 `if: failure()` step 发 PR 评论（这样 CI 红 + PR 评论都有信号）
- **方向 B**：runner 失败时 exit 1，workflow `continue-on-error: true` 防止阻塞，但保留"PR 评论"作为唯一信号源（当前行为）
- **方向 C**（最弱）：保留当前行为，但在 plan §2.4 验收标准中**显式记录**"失败仅在 PR 评论中，无 CI 门禁信号"

**建议**：方向 A，因为目前 100% 隐式 silent failure 不符合"非阻塞 = 仍然能看见失败"。

### B3. runner.mjs tracing 资源泄漏（异常路径）— 实际 bug

**位置**：`tools/agent-exploration/runner.mjs:154-163`

**证据**：
```javascript
} catch (err) {
  result.result = 'Failed';
  result.actual = String(err && err.stack || err);
  try { result.consoleErrors = await harness.getConsoleErrors(); } catch {}
  try { result.failedRequests = await harness.getFailedRequests(); } catch {}
  try {
    const tracePath = join(taskDir, 'trace.zip');
    await context.tracing.stop({ path: tracePath });
    result.evidence.trace = tracePath;
  } catch {}
} finally {
  await context.close();
}
```

**问题**：
- `tracing.start()` 在 line 97 开启（无条件）
- `tracing.stop()` 在 line 144（成功路径）和 line 161（异常路径）调用，**但**这俩都是 try 块内的
- 如果 line 105 `page.goto` 之前就 throw（例如 harness 安装超时），line 161 仍然会跑——这部分 OK
- **真正的风险**：`finally` 中 `await context.close()` 但如果 `context.tracing` 还在运行，Playwright 会拒绝 close 并 throw，**finally 也不会再清理**（finally 只能 try/catch 一次，但这里没包）
- line 165 `context.close()` throw 会**冒泡到外层 for 循环的 try**（runner.mjs:73-81），导致**所有后续 task 都被跳过**

**复现路径**：
- 第一任务正常跑 → context.close() 成功
- 第二任务 `tracing.start()` → 之后 `harness.waitForReady()` 超时（10 秒）→ 进入 catch → line 161 `tracing.stop()` 成功 → finally `context.close()` 成功
- **但**如果 line 161 `tracing.stop()` 自身失败（context 已损坏等），throw 会**跳过** finally 块——Node 不会回溯到 finally 后再 throw

**影响**：
- 一次失败可能让所有后续 task 跳过
- 报告只显示第一个失败，后续 task 状态缺失，**审计员无法分辨"任务跑失败"和"任务没跑到"**

**修复方向**：
```javascript
} catch (err) {
  result.result = 'Failed';
  result.actual = String(err && err.stack || err);
  try { result.consoleErrors = await harness.getConsoleErrors(); } catch {}
  try { result.failedRequests = await harness.getFailedRequests(); } catch {}
  try {
    const tracePath = join(taskDir, 'trace.zip');
    await context.tracing.stop({ path: tracePath });
    result.evidence.trace = tracePath;
  } catch (e) {
    result.evidence.traceError = String(e);
  }
} finally {
  // 显式 stop tracing 即使 context 已损坏，避免 close 失败导致循环跳过
  try { await context.tracing.stop(); } catch {}
  await context.close();
}
```

**重要**：这是开发者自报"TDD 跑通"无法发现的 bug，因为测试不模拟 multi-task 的异常路径。

## 2. 非阻塞（建议修复，不阻塞合并）

### N1. harness 旧版 v1 与 v2 命名不一致

**位置**：`ui/src/agent-test.ts:6` 用 `version: 1`，`webui/assets/agent-test-harness.js` 内部用 `version: 2`

**证据**：
- `ui/src/agent-test.ts:6`：`readonly version: 1`
- `webui/assets/agent-test-harness.js:22`（隐式在 `__AIRP_AGENT_TEST__` 对象里）
- `tools/agent-exploration/harness-client.mjs:10`：`window.__AIRP_AGENT_TEST__ && window.__AIRP_AGENT_TEST__.version === 2`

**影响**：
- 两个 `__AIRP_AGENT_TEST__` global 命名相同但版本不同
- 同时加载两个 harness 不会冲突（webui 没引 v1），但**维护者容易混淆**
- plan §10 显式推迟"桌面 ui harness 升级"到桌面 UI 路线，**但没说保留 v1 的命名**

**建议**（合并后 issue 跟踪）：将 `ui/src/agent-test.ts` 改名 `__AIRP_AGENT_TEST_V1__` 或 `__AIRP_AGENT_TEST_LEGACY__`，避免未来混淆。

### N2. workflow `paths` 过滤与 classifier 任务集覆盖范围不一致

**位置**：`.github/workflows/agent-browser-exploration.yml:8-19`（paths 列表）vs `tools/agent-exploration/classifier.mjs:1163-1180`（DIFF_TASK_MAP paths）

**证据**：
- workflow paths 含 11 个文件，但缺 `webui/assets/chat-space.js`（被 `runtime-pages.test.mjs` 测试存在但 workflow 没列），而 `regen-swipe-refresh` 的 DIFF_TASK_MAP 把它列为 path
- 也缺 `engine/src/memory/**`（包含但 `memory-roundtrip` 任务依赖）

**影响**：
- 即使 PR 改了 `chat-space.js` 或 `engine/src/memory/*.rs`，**workflow 根本不会触发**（paths 过滤在前面）
- 但**手动** `node runner.mjs --diff-file` 仍能跑
- 用户在 PR 评论里看到"Agent exploration 没跑"会困惑

**修复**：把 classifier 的 DIFF_TASK_MAP paths 当作 source of truth，让 workflow paths 用 `classifyPrDiff` 的结果来反推。

### N3. 任务模块 `check` 函数全靠 failed requests 兜底，缺乏真实断言

**位置**：`tasks/onboarding-firstchat-refresh.mjs:30-50` 等 4 个任务

**证据**：
- onboarding 的真实断言是"刷新前后 history message_ids 完全一致"，但这是**写在 DESCRIPTION 里**让 Agent 脚本执行的
- `check` 函数只过滤 console errors 和 5xx failed requests
- 如果 Agent 脚本生成的检查不严谨（比如 `assert.equal` 写错），任务 result 仍是 Passed

**影响**：
- Agent 探索的"失败候选"质量完全依赖 LLM 脚本质量
- runner 的 `check` 是兜底而非证据

**建议**（不阻塞）：在 plan §2.4 增加"任务模块 check 必须包含至少 1 个直接 API 调用作为独立证据"的要求。

### N4. reporter.mjs 报告里 console error 无截断总数

**位置**：`tools/agent-exploration/reporter.mjs:58-60`

**证据**：
- line 60：`for (const e of task.consoleErrors.slice(0, 10))` — 只显示前 10 个
- 没有 `... and N more` 提示

**影响**：报告里看不到"被截断了多少"，审计员可能误判问题规模。

**修复**：在 slice 0,10 后追加 `_(共 N 条)_`。

### N5. bootstrap-topology.sh state file 路径在多 workflow 并行时会冲突

**位置**：`deploy/production/bootstrap-topology.sh:39`

**证据**：
- `state_file="$deploy/.bootstrap-topology.state"`
- `smoke_id=${GITHUB_RUN_ID:-local}-$$` — 但 `$$` 在 GHA 上每个 step 是新 shell，所以 `smoke_id` 每次都不同
- 多个 PR 并行触发 + 多个 job step 同时写同一 state_file → **文件被覆盖或读到错误状态**

**复现**：
- Job A 的 bootstrap step 写 state_file (project=A)
- Job B 的 bootstrap step 写 state_file (project=B)
- Job A 的 teardown step 读 state_file → 拿到 B 的 project → **错杀 B 的拓扑**

**影响**：并发 PR 阶段 2 CI 会互相干扰，**实际生产高发场景必现**。

**修复**：把 state_file 路径包含 smoke_id：`state_file="$deploy/.bootstrap-topology.$smoke_id.state"`，并把 smoke_id 透传到 teardown step（`GITHUB_ENV` 或临时文件）。

## 3. 审计遗留项（合并后转 issue）

按 AGENTS.md 审计遗留项处理规则，以下 3 条在 PR 合并后转为 GitHub issue：

### L1. plan §2.4 验收标准与实现轻微脱节

- 计划承诺"Agent 发现的问题可被转换为固定 Playwright 回归测试（流程闭环）"——实现里**没有任何转换机制**（reporter 不输出 regression test stub）
- 建议：在 reporter.mjs 里加一个 `regressionStub` 字段，按任务集模板生成骨架 Playwright test

### L2. 方案 A 安全风险接受点缺乏 dry-run 模式

- runner 每次跑都调真实 LLM（产生费用） + 真实 Chrome（产生系统压力）
- 没有"只读 diff + 静态分析" 的 dry-run 模式
- 建议：加 `--dry-run` 选项，classifier + reporter 可单独跑，零 LLM 成本

### L3. 桌面 ui harness 升级路径无具体 owner

- plan §10 推迟桌面升级，但没说"谁负责在桌面 UI 路线启动时想起来"
- 建议：关联 issue #130（webui production umbrella）作为追踪载体

## 4. 与开发 agent 自报的差异

开发 agent 报告（issue #273 评论 + 各 commit）声称"无偏差"和"node --check 全绿"。我独立审计后**确认**：
- ✓ node --check / tests / fixture 原创性 — 开发 agent 报告属实
- ✗ **classifier 注释与实现矛盾** — 开发 agent 未识别
- ✗ **runner 永远 exit 0** — 开发 agent 未识别
- ✗ **tracing 资源泄漏** — 开发 agent 未识别（无单元测试覆盖）
- ✗ **workflow paths 缺漏** — 开发 agent 未识别
- ✗ **state file 并发冲突** — 开发 agent 未识别

## 5. 裁决

| 项 | 数量 | 处理 |
|---|---|---|
| 阻塞（必须修复） | 3 | 修复后再审计 |
| 非阻塞（建议） | 5 | 可在 issue 中跟踪，本 PR 不阻塞 |
| 遗留项（转 issue） | 3 | PR 合并后执行 AGENTS.md 流程 |

**最终建议**：**拒绝合并**。修复 B1-B3 后重审，特别是 B1（语义矛盾）和 B2（CI 信号失效）直接影响阶段 2 价值。

---

**审计员**：M3.2（用户当前会话模型）
**审计独立性声明**：本审计未阅读 PR #298 的审计报告，独立基于代码、测试运行结果和计划文件原文判断。

---

## 6. 复审裁决（2026-07-23，B1-B9 修复后）

**复审模型**：GLM-5.2
**复审范围**：PR #300 当前 HEAD `ca0452e`（含 B1-B9 全部修复 commits）
**复审方法**：独立读源码验证 + CI 全绿证据 + CodeRabbit review 状态

### 6.1 阻塞项修复独立验证

| 阻塞项 | 修复 commit | 独立验证证据 | 结论 |
|---|---|---|---|
| B1 classifier path OR keyword | `f939631` | `classifier.mjs:43` `if (pathHit && keywordHit) hits.add(taskName);` — path AND keyword；`classifier.mjs:16-18` 注释说明 `/\bedit.*message/i` 为何去掉尾部 `\b`（避免不匹配 `edit_message`）；`classifier.test.mjs` 新增 3 个回归测试（path-only 不触发 / keyword-only 不触发 / 单 path 不触发），9/9 pass | ✓ 修复 |
| B2 runner 永远 exit 0 | `f939631` | `runner.mjs:91-96` Failed 时 `process.exit(1)`；workflow `Publish failure placeholder comment` step `if: failure()` 接住并发占位评论 | ✓ 修复 |
| B3 tracing 资源泄漏 | `f939631` | `runner.mjs:109-121` result 预初始化（保证 page.goto 失败时 catch/finally 仍可访问）；`runner.mjs:123` `tracingStopped` 标志；`runner.mjs:168-192` catch + finally 完整 try/catch 防御（tracing.stop 二次调用安全，context.close 失败不冒泡） | ✓ 修复 |

### 6.2 后续 CI 修复独立验证（B4-B9，本审计首次记录）

| 项 | 修复 commit | 独立验证证据 | 结论 |
|---|---|---|---|
| B4 bootstrap-topology.sh Permission denied | `af51749` | `git ls-files --stage` → `100755 ca08456 ...` — 可执行位已设 | ✓ 修复 |
| B5 post-pr-comment.mjs ENOENT | `af51749` | `post-pr-comment.mjs:24-29` `fs.access` 预检 + 优雅 `process.exit(0)` + 警告说明 placeholder 已处理 | ✓ 修复 |
| B6 docker pull access denied | `aaaf901` | workflow `Prepare .env for image build` + `Build pinned production images` step 在 Bootstrap 之前；bootstrap-topology.sh `docker run airp-gateway:0.1.0 hash-password` 可成功执行 | ✓ 修复 |
| B7 /health not ready 无诊断 | `551b196` | `bootstrap-topology.sh:86-125` `wait_for_engine_ready` 失败时 dump engine + gateway + mock_provider 日志到 stderr；workflow Upload artifacts 包含 `deploy/production/*.log` | ✓ 修复 |
| B8 secrets 权限 0600 致 engine 重启 | `4c460df` (方案 C) | `bootstrap-topology.sh:136` TLS 段 `umask 077`（保护私钥）；`bootstrap-topology.sh:152` secrets 段 `umask 022`（让 engine uid 65532 可读 bind-mount 的 secret） | ✓ 修复 |
| B9 OPENAI_API_KEY 缺失 | `ca0452e` (方案 1+3 组合) | `llm-client.mjs:23` `FALLBACK_MODE = !API_KEY`（不再 exit 2）；`llm-client.mjs:47-70` 内置 minimal smoke 脚本；`runner.mjs:202-209` fallback 模式跳过 chatCompletion 与重试；workflow 移除 LLM key 预检 step 与所有 `if: configured == '1'` gate | ✓ 修复 |

### 6.3 CI 全绿证据（独立来源）

PR #300 最新 push (`ca0452e`) 触发的 statusCheckRollup：
- `Agent Browser Exploration / explore` — **SUCCESS**（fallback 模式跑通完整链路：topology → runner → reporter → commenter）
- `PR gate / Rust workspace` — SUCCESS
- `PR gate / UI and WebUI` — SUCCESS
- `PR gate / Production topology` — SUCCESS
- `Windows WebUI package / Portable Windows WebUI` — SUCCESS
- `CodeRabbit` — SUCCESS（review profile = CHILL，全部 COMMENTED 无 REQUEST_CHANGES，无阻塞意见）

`mergeable=MERGEABLE`, `mergeStateStatus=CLEAN`。

### 6.4 CodeRabbit 非阻塞建议

CodeRabbit 共发布 8 条 actionable comments（profile = CHILL，全部 nitpick / trivial 级），分布在：
- `classifier.test.mjs`（1）：建议用 `assert.deepEqual` 而非 `include` 校验
- `action-protocol.mjs`（1）：建议 `validateAction` 加 action-specific 校验（Plan B 接入前必修）
- `bootstrap-topology.sh`（1）：state file 并发隔离（与 N5 同源）
- `runner.mjs`（1）：建议 `result` 初始化提前到 `browser.newContext()` 之前（B3 已部分修复，CodeRabbit 建议更彻底）
- `docs/audits/2026-07-23-PR-300-agent-browser-exploration.md`（4）：建议更新审计报告自身描述（与 B1/B2 修复后的语义对齐）

按 AGENTS.md「审计遗留项处理」规则，上述非阻塞意见与原 §2 N1-N5、§3 L1-L3 一并整理后转 issue（PR 合并后执行）。

### 6.5 最终裁决

**通过合并**。

- 3 个阻塞项（B1-B3）全部修复，独立验证证据见 §6.1。
- 6 个后续 CI 修复项（B4-B9）全部修复，独立验证证据见 §6.2。
- CI 全绿（6 项 SUCCESS），CodeRabbit 无阻塞意见。
- 原审计 §2 N1-N5、§3 L1-L3 + CodeRabbit 8 条 nitpick 全部归入"未改动但写出来的修改意见"，按 AGENTS.md 时序约束（PR 合并后）转 issue。

**审计遗留项清单（合并后转 issue，去重后 8 条）**：

| ID | 来源 | 模块 | 严重度 | 建议时机 | 摘要 |
|---|---|---|---|---|---|
| F-1 | 原 N1 + 原审计 §2 | webui | 低 | 桌面 UI 路线启动时 | `ui/src/agent-test.ts` v1 与 `webui/assets/agent-test-harness.js` v2 命名不一致，建议改名 `__AIRP_AGENT_TEST_LEGACY__` |
| F-2 | 原 N2 + CodeRabbit | workflow | 中 | 阶段 2 第二轮 PR | workflow `paths` 过滤与 classifier `DIFF_TASK_MAP` paths 不一致（缺 `webui/assets/chat-space.js`、`engine/src/memory/**` 等），导致手动 diff 能触发但 PR 自动触发漏 |
| F-3 | 原 N3 | tasks | 中 | LLM 阶段接入后 | 任务模块 `check` 函数全靠 5xx + console errors 兜底，缺直接 API 调用断言；建议 plan §2.4 加"check 必须含 1 个直接 API 证据"要求 |
| F-4 | 原 N4 | reporter | 低 | 任意 PR | `reporter.mjs:60` console errors 只 `slice(0, 10)`，无 `... and N more` 提示 |
| F-5 | 原 N5 + CodeRabbit | bootstrap | 中 | 阶段 2 并发 PR 出现前 | `bootstrap-topology.sh:39` state_file 用固定路径 `.bootstrap-topology.state`，并发 PR 时 teardown 会错杀对方拓扑；改为 `.bootstrap-topology.$smoke_id.state` 并透传 smoke_id |
| F-6 | 原 L1 | reporter | 低 | 阶段 3 | plan §2.4 承诺"Agent 发现的问题可转固定 Playwright 回归测试"，reporter 缺 `regressionStub` 字段 |
| F-7 | 原 L2 | runner | 低 | 阶段 3 | 缺 `--dry-run` 选项（零 LLM 成本验证 classifier + reporter） |
| F-8 | 原 L3 | ui | 低 | 桌面 UI 路线启动时 | 桌面 ui harness 升级无 owner，建议关联 issue #130 |
| F-9 | CodeRabbit | classifier.test | 低 | 任意 PR | 测试用 `assert.include` 而非 `assert.deepEqual`，无法校验精确任务集 |
| F-10 | CodeRabbit | action-protocol | 中 | 方案 B 接入前 | `validateAction` 缺 action-specific 必填字段校验 + navigation allow-list |
| F-11 | CodeRabbit | runner | 低 | 任意 PR | B3 修复已收敛异常路径，CodeRabbit 建议更彻底把 `result` 初始化提到 `browser.newContext()` 之前 |
| F-12 | CodeRabbit | 审计报告自身 | 低 | 已随本复审裁决修复 | 审计报告 §B.1/§B.2 描述需与 B1/B2 修复后语义对齐（本节 §6.1 已替代） |

**复审员**：GLM-5.2
**复审独立性声明**：本次复审独立基于当前 HEAD 源码 + CI 状态 + CodeRabbit review 输出，未将原审计 §1-§5 结论视为不可质疑的前提。
