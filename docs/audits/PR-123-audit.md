# PR #123 独立审计

> 审计日期：2026-07-12  
> 范围：`pr-b/mock-provider-browser-smoke` 相对 `main`  
> 原则：源码、依赖 API 语义与真实运行结果优先，不沿用 PR 原结论。

## 结论

修复后可合并。原提交的 mock provider 会在 POST 请求体结束时误停 SSE，且 engine 把 `tower_governor::per_second(10)` 误读成 10 req/s；实际语义是每 10 秒补一个令牌。两者共同造成 assistant 不 finalize 与 burst 用尽后的大面积 429。修复后自动 harness 为 56/56，真实浏览器完成连接、恢复与流式消息渲染。

## 阻塞发现与处置

| 编号 | 发现 | 处置 |
|---|---|---|
| A1 | `req.close` 在请求体完成后触发，timer 在 finish chunk / `[DONE]` 前被清除。 | 改为监听 `res.close`；assistant 可 finalize 并落盘。 |
| A2 | `per_second(10)` 是 10 秒/token，不是 10 token/秒。 | 改为显式 `period(Duration::from_millis(100))`，burst 20，并加配置回归测试。 |
| A3 | SSE error event 被拼进成功正文，调用方无法区分失败。 | 独立返回 `errorText`，并防御无 response body。 |
| A4 | `start.bat` 不保证全新 data root，内层 `%errorlevel%` 也会被父 shell 提前展开。 | 启动前清理固定验收目录；内层 cmd 启用 delayed expansion。 |
| A5 | OpenAI-compatible `created` 使用毫秒时间戳。 | 改为 Unix 秒。 |
| A6 | PR 把直接 HTTP harness 称为 browser smoke。 | 更名为 engine 闭环 smoke；文档明确浏览器交互证据与 engine 真相断言是两层验证。 |

## 验证证据

- `node webui/smoke.mjs`：`checks=56 failures=0`；真实 history 为三轮 user/assistant 六条，覆盖 session 隔离、rollback、regen、delete 和 typed errors。
- Codex in-app browser 访问 `http://127.0.0.1:9001/`：连接显示 `已连接 0.1.0`；恢复 `smoke-lyra`、会话和 preset；从页面发送“浏览器验收：请带我看看旧街。”后渲染 user/assistant，事件日志为 `done/24chunks`。
- `cargo test -p airp-core rate_limit_matches_ten_requests_per_second_with_twenty_burst`：1 passed。
- `cargo fmt --all -- --check`：通过。
- 完整 workspace/clippy/UI/神圣不变式：以本分支最终 GitHub PR gate 为准。

## 非阻塞遗留

本轮没有新增未修审计意见。CodeRabbit docstring coverage 是通用建议，零依赖本地 harness 不以批量 docstring 扩张作为合并条件。
