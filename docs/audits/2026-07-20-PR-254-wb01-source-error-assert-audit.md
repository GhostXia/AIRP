# PR #254 独立审计报告 — WB-01 source_error assert + smoke retry

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-20
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#254 test(worldbook_normalizer): WB-01 source_error assert](https://github.com/GhostXia/AIRP/pull/254)
- **分支**：`chore/149-wb01-source-error-assert`
- **commits**：
  - `3d04bcf test(worldbook_normalizer): assert source_error on empty source object (WB-01)`（PR 主改动）
  - `86dff1d fix(smoke): retry production-browser-smoke page.goto on transient network errors`（审计期间为消除 CI flaky 追加）

## 1. 范围与背景

PR #254 关闭 #149。#149 是 PR #145 合并后的审计遗留 issue，明确要求在
`test_empty_source_object` 上补 `source_error` / `replacement_error` 断言，保护
"malformed replacement 不得被当作合法空世界书"的回归路径。

PR 主改动只增加 5 行测试断言：
```rust
// `{}` is an unsupported source shape, not a legal empty worldbook.
// Asserting `source_error` here protects against malformed replacement
// requests silently clearing existing data (WB-01, PR #145 audit leftover).
assert!(report.source_error.is_some());
assert!(report.replacement_error().is_some());
```

第二个 commit `86dff1d` 是审计过程中本 agent 为消除 PR #254 自身 CI flaky
(Production topology smoke `page.goto ERR_NETWORK_CHANGED`) 追加的修复；
本审计同时覆盖。

## 2. 独立证据

### 2.1 WB-01 主改动 — Rust 源码静态读

`engine/src/orchestrator/worldbook_normalizer.rs:204-243` `extract_raw_entries`：
对 `{}` 输入路径走：

1. `source.get("entries")` → None
2. `source.as_array()` → None
3. `source.as_object()` → Some(empty map)
   - `looks_like_entry(empty_map)` = false（不含 `content`/`keys`/`key`）
   - 进入 `!map.is_empty() && ...` 分支，`!map.is_empty()` 对 `{}` 为 false
4. Fall through 到 `Err("unsupported worldbook shape; ...")`

回到 `normalize_worldbook:142-155`，`Err(reason)` 路径返回：
```rust
WorldbookImportReport {
    source_error: Some(reason),
    ..Default::default()
}
```

`replacement_error()`（同文件 L103-114）：
```rust
if let Some(reason) = &self.source_error {
    return Some(reason.clone());
}
```

即 `{}` 输入必满足 `source_error.is_some()` 与 `replacement_error().is_some()`。
新断言与实现一致。

### 2.2 实跑证据

```
$ cargo test --package airp-core --lib orchestrator::worldbook_normalizer::tests::test_empty_source_object
   Compiling airp-core v0.1.0 (D:\AIRP-Dev\engine)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 11.41s
     Running unittests src\lib.rs (target\debug\deps\airp_core-6aff6ce215769fea.exe)

running 1 test
test orchestrator::worldbook_normalizer::tests::test_empty_source_object ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 754 filtered out; finished in 0.00s
```

断言通过。`{}` 不再被当作合法空世界书。

### 2.3 邻近测试回归检查

`test_empty_entries_produces_empty_lorebook`（L1013）断言 `{"entries": []}`
是合法的 empty clear，不进入 `source_error` 路径。这是因为
`extract_raw_entries` L207 进入 `entries` 分支后 `as_array()` 命中，返回
空 `arr.iter().collect()`，不走 Err 路径。`replacement_error()` 在
`total_input == 0` 时返回 None（L107 条件 `total_input > 0` 不成立）。

即合法空 clear 与非法 `{}` 在 normalizer 中行为严格区分，WB-01 的语义
断言不会误伤 `{"entries": []}` 的合法 clear 路径。

### 2.4 smoke retry 改动 — 静态读

`ui/production-browser-smoke.mjs` 原 L53：
```js
const response = await page.goto(origin, { waitUntil: 'domcontentloaded' });
```

修改后包装为 3 次重试，匹配瞬时网络错误：
```js
let response;
for (let attempt = 1; attempt <= 3; attempt++) {
  try {
    response = await page.goto(origin, { waitUntil: 'domcontentloaded' });
    break;
  } catch (err) {
    if (attempt < 3 && /ERR_NETWORK_CHANGED|ERR_CONNECTION_REFUSED|ERR_CONNECTION_RESET|ERR_NAME_NOT_RESOLVED/.test(err?.message || '')) {
      console.log(`page.goto transient error (attempt ${attempt}/3): ${err.message}`);
      await page.waitForTimeout(2_000);
      continue;
    }
    throw err;
  }
}
```

与 PR #251 `production-browser-restart-smoke.mjs` 的 transient retry 思路
一致；retry 上限 3、间隔 2s，最大延迟 6s 在 CI smoke timeout 之内。非
transient 错误（assertion / 解析错误 / 真实 4xx-5xx）直接 throw，不被吞掉。

`assert.equal(response?.status(), 200)` 不变，断言语义保留。

## 3. 阻塞意见

无。

## 4. 非阻塞 / 可后续

| # | 项 | 严重度 | 建议时机 |
|---|----|--------|---------|
| N-1 | `86dff1d` 把 smoke retry 塞进 #254 的 chore/test 分支，scope 实际上跨了 WB-01 测试与 smoke 基建两个独立主题。理想是拆两个 PR；本 agent 因合并需求选择保留，但建议未来同类小修直接独立 PR，避免审计/回溯混淆 | 低 | 下次 smoke 改动时 |
| N-2 | retry 的瞬时错误正则 `ERR_NETWORK_CHANGED|ERR_CONNECTION_REFUSED|ERR_CONNECTION_RESET|ERR_NAME_NOT_RESOLVED` 是手抄清单，未来 Chrome 升级新增错误码会漏。考虑改成 negative match（非 `assert.*` / 非 `TypeError`）或允许任何包含 `ERR_` 前缀的字符串 | 低 | Chrome 升级或下次 flaky 出现新错误码时 |
| N-3 | #149 中 WB-NF-01/02/03 的"不修"决定不在本 PR 范围，#149 关闭时应明确"仅 WB-01 实现，WB-NF-01/02/03 维持不修决定" | 低 | PR 合并后关闭 #149 时 |

## 5. 神圣不变式

- `subagent_context_has_no_orchestrator_noise`：本 PR 不触 orchestrator context
  或 subagent 脚手架；测试侧改动不影响该不变式。✓
- normalizer 不变式①（不注入 agent 脚手架）：本 PR 仅补断言，不改实现。✓
- WebUI CSP / 路径穿越 / PUT body limit：本 PR 不触 webui http 边界。✓

## 6. 结论

**通过**。

- WB-01 主改动：5 行测试断言，与 normalizer 实现行为一致，由 `cargo test`
  实跑验证通过。合法 `{"entries": []}` clear 路径不受影响。
- smoke retry：3 次重试 + 2s 间隔，与 PR #251 思路对齐；非 transient 错误
  仍 throw，断言语义保留。修复了 PR #254 自身 CI flaky。
- 无阻塞意见。N-1/N-2/N-3 留 PR 合并后写入 GitHub issue 跟进。
- 推荐合并后关闭 #149 时显式说明 WB-01 实现完毕、WB-NF-01/02/03 维持不修。
