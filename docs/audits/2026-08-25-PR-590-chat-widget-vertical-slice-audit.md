# PR #590 独立审计报告

> **审计模型**：两名独立 GPT-5.6 Terra 审计 agent
> **审计时间**：2026-08-25
> **审计原则**：AGENTS.md 审计守则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：PR #590（`codex/564-chat-vertical-slice`，首个 `core.chat` 写纵切）
> **结论**：**本 PR 的有界代码范围 PASS；#589 不关闭，未完成验收门禁继续留在原 issue**

## 0. 独立验证证据

| 验证项 | 结果 |
|---|---|
| Engine Surface / intent 定向测试 | 15/15 通过；session 定向测试 8/8 通过 |
| Rust 全仓 | `cargo test --workspace --locked` 通过（Engine lib 1475 passed、5 ignored，其余 workspace suites 全绿） |
| Rust 静态门禁 | `cargo fmt --all -- --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings` 通过 |
| Vue | 20 个测试文件、171 项测试通过；typecheck 与 production bundle 通过 |
| 桌面运行时 | shell smoke 5 profiles 通过；5000-row runtime fixture 通过（1.00 ms p95，8 个 virtual rows） |
| 真实 Engine 浏览器链路 | 使用本次源码编译的 `D:\AIRP-Dev\target\debug\airp-core.exe` 运行 `smoke:http-bus` 通过 |

浏览器烟测使用本地确定性 OpenAI-compatible 服务，只证明真实 Engine、HTTP/SSE、浏览器与取消传播链路；它不是“真实 configured provider 人工验收”，后者仍为 #589 的开放门禁。

## 1. 已修复的审计发现

### A1. Surface revision 不能无条件清除临时流状态

初版在任意较新 revision 到达时清除 `awaiting_surface`，可能被无关 Widget patch 误触发。现改为校验当前 Chat 权威投影确实包含本次 send/continue/regen/swipe 结果，并在终态主动刷新 canonical Surface。

### A2. 部分提交取消不能直接丢弃临时内容

`partially_committed` 取消现在进入 `awaiting_surface`，主动读取 canonical snapshot 后再清除临时状态；`not_committed` 才立即清除。刷新失败会保留待同步状态，仅报告刷新错误，不把已成功生成误报为生成失败。

### A3. stop smoke 不能只检查按钮隐藏

真实 Engine browser smoke 现同时断言上游 provider 连接被关闭，并在限流窗口恢复后读取 durable history，确认被取消的半截 continue 没有写入 canonical Chat 历史。

### A4. user scope 的 character discovery 不完整

`/v1/characters`、session list/create 与 Surface/intent 均使用相同 user effective root，并新增跨用户隔离测试。

### A5. 生成期间仍可切换 swipe

Swipe 函数和按钮都受统一 busy 门禁约束，避免与 send/regen/continue/stop 并发。

### A6. Widget manifest intent 名称漂移

`core.chat` manifest 已改为与 executor 一致的 send/regen/continue/stop/swipe/loadMore 封闭集合。

## 2. 首轮 audit bot 处理

- **B1（已修）**：基线日期与 2026-08-25 审计证据对齐。
- **B2（已修）**：session DELETE 接受同族 `user_id` query，在对应 effective root 删除；新增 wrong-scope 404 与 scoped delete 测试。
- **B3（不采纳）**：bot 建议把 `user_id` 绑定到 authenticated principal，但当前 daemon access key 和 desktop session token 都是进程级授权，不携带用户 claim；既有 Chat、Conversation、Persona 等 API 也由进程级授权调用方显式选择 `user_id`，基线明确产品并非多租户。仅在本 PR 三个端点伪造 principal 会形成错误且不兼容的安全模型。真正的多租户身份绑定应作为独立架构变更，而不是本纵切的局部补丁。
- **B4（已修）**：browser smoke 在 provider server 不存在时立即完成清理；存在时先 `close` 再 `closeAllConnections`，避免活动 SSE 令 finally 挂起。
- **B5（已修）**：停止失败时把 session select DOM 值恢复为当前权威 session。
- **B6（已修）**：intent SSE 在成功、typed error、解析错误与大小限制等所有退出路径取消 reader 并释放 lock；测试断言 error 路径实际触发 cancel。
- **N1（已修）**：Vitest 全局 stub 改由 `afterEach` 无条件恢复。

## 3. 权威与恢复边界

- 浏览器只提交 Surface id、instance id、intent 名称与参数；Engine 从已接受的 Surface registry 反查 effective root、character、session、user 与 Widget 类型。
- 未注册、歧义、错实例、错 Widget、过期会话和未知 intent 均 fail closed。
- send/regen/continue 复用既有 Chat pipeline 与 SSE 合同；stop 通过当前 Coordinator generation id 协作取消。
- Vue 的流文本、思考文本、pending/error 和旧页历史仅为临时视图 overlay，不成为第二份 durable truth store。

## 4. #589 保留的未完成门禁

以下项目不属于本 PR 可宣称完成的证据，因此 PR 描述不使用 `Closes #589`：

1. Persona、Scene、Worldbook 的稳定上下文选择与 chip 投影；当前仅有 Character 与 Session。
2. 真实 Engine 下 5000 条历史分页、流式 patch 不重建无关 Widget 的端到端证据；当前分别有分页实现与 5000-row runtime fixture。
3. 携带 in-flight Chat 操作时的 401、断流、Engine 重启端到端恢复证据；当前已有 Bus 级 renew/replay/resync 测试。
4. 真实 configured provider 的人工验收。

这些是原 issue 的剩余完成条件，不另建重复 issue；后续 PR 继续在 #589 下收口。

## 5. 裁决

| 类别 | 数量 | 处理 |
|---|---:|---|
| 本 PR 代码阻塞项 | 0 | 可以进入仓库首轮 audit bot 门禁 |
| 已修复审计项 | 12 | A1–A6、B1–B2、B4–B6、N1 已验证 |
| 不采纳意见 | 1 | B3 与当前进程级授权模型不相容 |
| 原 issue 剩余门禁 | 4 | 保留 #589 开放，不虚报完成 |

**最终建议：PR #590 可以在仓库 audit bot 首轮通过且人工 review 后合并；合并不得关闭 #589。**

---

**审计独立性声明**：两名审计 agent 分别复查 Engine authority boundary、Vue revision/cancellation 状态机与 issue 完成条件；开发侧仅提供代码与可重复命令，没有要求审计沿用实现结论。
