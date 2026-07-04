# [审计] PR #38 `webui-p0-usability` 发现清单（4 bug + 4 设计缺口 + 5 风险记号）

> **审计源 LLM**: `MiniMax-M3`
> **审计时间**: 2026-07-05
> **审计员立场**: 独立审计（不附开发文档与既有代码结论）
> **完整报告**: [docs/audits/PR-38-audit.md](../audits/PR-38-audit.md)

## TL;DR

PR #38 兑现了"`/v1/models` provider smoke 在 WebUI 中可见"的核心承诺，但合并前放过 4 个真 bug（A 类）+ 4 个设计/验证缺口（B 类）+ 5 个 harness 风险记号（C 类）。A 类 4 个都可在零契约/零行为变化下以 < 50 LOC 修复，建议在下一个 WebUI 收尾 PR 一次性补完，再进 P1。

## A 类：真 bug（合并前应修）

| # | 位置 | 简述 | 严重度 |
|---|---|---|---|
| A1 | [engine/src/daemon/handlers.rs:934-941](../audits/PR-38-audit.md#a1-models_url_from_endpoint-对裸-hostendpoint-返回错位-host-中等严重度) | `models_url_from_endpoint` 对无 `/v1/` + 无路径的 endpoint（如 `https://api.openai.com`）返回 `https://models`（host=`models`），不返回 `invalid_endpoint` 502 也不返回 `https://api.openai.com/models`。错配用户看到 5s `upstream_timeout` 误导 | 中 |
| A2 | [webui/app.js:275,486](../audits/PR-38-audit.md#a2-webui-dosend-的-sse-chunk-三元表达式退化为-noop小严重度但属于明显-dead-code--误改) | `chunk.type === 'body_chunk' ? chunk.text : chunk.text` 两个分支返回相同 `chunk.text`——`think_chunk` 被无差别当 body 渲染，RP 场景 think 块糊在角色台词里 | 小 |
| A3 | [webui/app.js:424-430](../audits/PR-38-audit.md#a3-webui-import-失败时不走-formaterror小严重度ux-直接降级) | import 失败显示用 `(r.data \|\| r.text)`；`r.data` 是 JSON 对象 → `[object Object]`；engine 已写好的中文诊断信息（"card_path 任意路径读已禁用"等）整个丢掉 | 小 |
| A4 | [engine/src/daemon/mod.rs:148-152](../audits/PR-38-audit.md#a4-useroripkeyextractor-的-bearer-截断可能-panic极小概率但后果严重) | bearer token 截断 `&token[..32]` 字节切片；若 32 不在 char boundary 上 panic，热路径 5xx | 极小（高破坏） |

## B 类：设计/验证缺口（应单独 PR 修）

| # | 位置 | 简述 |
|---|---|---|
| B1 | [webui/app.js:450-463](../audits/PR-38-audit.md#b1-并发流测试的断言与注释不符) | 注释"应基本交替"但 `Promise.all` 同步追加保证是 `u-A, u-B` 顺序；测试只 `results.every(r => r.ok)`，**对 PR #6 修复的"不串扰"无断言**——本测试无法回归保护 race |
| B2 | [engine/src/daemon/handlers.rs:21](../audits/PR-38-audit.md#b2-models_proxy_timeout-写死-5s不可配置) | `MODELS_PROXY_TIMEOUT = 5s` 写死；冷启 provider 5s 不够 |
| B3 | [webui/app.js:227-236,299](../audits/PR-38-audit.md#b3-webui-的-appendmsg-不区分-role-与-sender) | `user_profile.name` 写死 `'User'`，多用户场景不可区分 |
| B4 | [webui/app.js:101-114](../audits/PR-38-audit.md#b4-formaterror-漏掉-errorrequest_id--errorhint--errorsuggestion-等) | `formatError` 只展开 5 段；未来扩展字段会丢 |

## C 类：harness 风险记号（不在 P0 范围）

| # | 位置 | 风险 |
|---|---|---|
| C1 | [webui/app.js:502](../audits/PR-38-audit.md#3-c-类harness-风险记号不在-p0-范围但合并前应该留-todo-标) | `setTimeout(connect, 300)` 与用户编辑 URL 竞态，发请求到不存在主机污染 event log |
| C2 | [webui/app.js:36-39](../audits/PR-38-audit.md#3-c-类harness-风险记号不在-p0-范围但合并前应该留-todo-标) | bearer token 长期驻闭包；XSS 可读；sessionStorage 计划未实现 |
| C3 | [webui/app.js:411-413](../audits/PR-38-audit.md#3-c-类harness-风险记号不在-p0-范围但合并前应该留-todo-标) | import 客户端无 size gate；50MB 文件 base64 编码后浏览器 OOM；engine 10MB 限载拒载用户体验差 |
| C4 | [webui/app.js:375](../audits/PR-38-audit.md#3-c-类harness-风险记号不在-p0-范围但合并前应该留-todo-标) | agent run `max_steps: 3` 硬编码 |
| C5 | [webui/app.js:288-291](../audits/PR-38-audit.md#3-c-类harness-风险记号不在-p0-范围但合并前应该留-todo-标) | aborted SSE 流与最后一次 `done` 帧的 logEvent 语义冲突（仅诊断用） |

## 验收标准（建议）

A1-A4 在下一个 WebUI 收尾 PR 一起修完；B1-B4 / C1-C5 各自独立 PR，不要堆回 P0 收尾。

每个修复 PR 必跑：
- `cargo test -p airp-core --test openai_compat`（A1 加 2 个 boundary test）
- `cargo test -p airp-core` 全绿
- WebUI 真实 provider smoke（[docs/WEBUI-BACKEND-VALIDATION.md §5](../../docs/WEBUI-BACKEND-VALIDATION.md) 模板）

## 与既有约束的兼容性

- ✅ RR-001 card_path 任意路径读禁：WebUI 走 `card_png_base64`/`card_json`，本 PR 未发 `card_path`
- ✅ 神圣不变式 `subagent_context_has_no_orchestrator_noise`：未触及 orchestrator
- ✅ Governor 覆盖 /v1/*：`/v1/models` 在覆盖范围
- ✅ 5.0a `CharacterId` newtype：未动 import 范围

无冲突。

## 元数据

- 审计源 LLM: `MiniMax-M3`（开发：MiniMax，2026 年初；本文档由其派生实例于 2026-07-05 生成）
- 审计员立场: 独立审计（参 [AGENTS.md](../../AGENTS.md) 守则）
- 完整报告含 A1 复现实验命令: [docs/audits/PR-38-audit.md](../audits/PR-38-audit.md)
