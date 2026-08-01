# #284 决策 RFC：deferred commit 过渡到 Session Coordinator

- **Issue**: [#284](https://github.com/GhostXia/AIRP/issues/284)
- **关联**: [#286](https://github.com/GhostXia/AIRP/issues/286)、[#381](https://github.com/GhostXia/AIRP/issues/381)
- **来源审计**: `docs/audits/2026-07-20-PR-251-swipe-smooth-streaming-audit.md` §2.C1
- **决策日期**: 2026-08-01
- **状态**: **方案已敲定；本 PR 仅归档设计，不包含实现**
- **当前实现目标**: 方案 N（deferred commit + session generation contract）
- **终局方向**: 方案 O（Session Coordinator）
- **退化备选**: 方案 L（per-session in-flight mutex），仅用于无法按期交付 N 的紧急止血

> 本文经 PR #374 的多轮独立审计修订。原稿推荐方案 L；审计确认其只能串行化
> delete-first 流程，不能解决崩溃丢失、错误删除 user 消息和跨资源提交问题。
> 最终决定是：以 N 交付 #284，以 O 收敛长期会话所有权；不把 L 固化为终局架构。

## 1. 决策摘要

AIRP 将保持现有 `/v1/chat/*`、SSE 事件和用户数据格式兼容，在内部逐步收敛会话写入所有权：

1. 当前实现先采用方案 N：生成前只拍快照，不物理删除消息；生成完成后以 revision / generation id 做原子提交。
2. 方案 N 的入口、命令和状态必须按方案 O 的 Session Coordinator 合同设计，避免形成第二套临时 API。
3. 后续将 chat、agent turn、同步消息变更和 #286 TurnCommit 分阶段迁入 Session Coordinator。
4. 性能结论必须来自基准测试。O 的首要收益是正确性、透明度和可演进性，不宣称 actor/coordinator 天然比短租约更快。

### 1.1 兼容边界

- **必须保持**：现有 HTTP 路由、SSE 数据形状、角色卡/世界书/会话/候选等用户资产可读写。
- **允许改变**：内部锁、模块边界、命令调度、prompt snapshot 和 commit 实现。
- **必须版本化**：任何持久化格式变化都要有 migration、升级前备份、完整性验证和回滚。
- **不得双写**：迁移期间每类会话 mutation 只能有一个权威 owner。

## 2. 已核实的根因

Issue `#252` 审计 §2.C1 指出 regen 与其它会话操作存在跨 streaming 竞态。真实代码路径是：

```text
regen()
  -> 读取 active-path 最后一条消息及候选
  -> delete_last_n(1) 立即持久化
  -> 释放 session_lock
  -> 构造 prompt 并进行秒级 LLM streaming
  -> finalizer append_with_candidates(...)
```

现有 `session_lock` 只保证单次 `ChatService` 写操作原子，不保证上述多步流程原子。

更严重的是，`regen()` 和 `delete_last_n(1)` 没有验证尾消息必须是 assistant。两个并发 regen 的真实时序是：

| 时序 | regen A | regen B | ChatLog 结果 |
|---|---|---|---|
| t1 | 删除 assistant(a) | — | 尾部成为 user |
| t2 | streaming | 读取尾部 user，并将其内容当成旧候选 | user 被删除 |
| t3 | append candidates `[a, b_A]` | streaming | 一条 assistant |
| t4 | — | append candidates `[user.content, b_B]` | 两条 assistant，user 内容被污染为候选 |

因此根因不是单纯“缺少 mutex”，而是以下组合：

- delete-first 在不可控的 LLM streaming 前破坏持久状态；
- generation 没有稳定 target snapshot 和 CAS commit；
- session 内 generation 与同步 mutation 没有统一冲突合同；
- chat、state、agent tools 和后处理尚未收敛到单一会话所有者。

## 3. 方案裁决

| 方案 | 定位 | 优点 | 不采纳为当前终局的原因 |
|---|---|---|---|
| K：marker + finalizer 校验 | 放弃 | 局部改动 | marker 恢复和覆盖语义复杂，仍保留 delete-first |
| L：跨 pipeline mutex | 紧急退化 | 最少代码即可阻止交错 | streaming 挂起会放大会话级阻塞；崩溃仍丢旧消息；新增双锁和 registry 债务 |
| M：per-session executor | 不单独实施 | 单 owner、命令串行 | 若直接重写 SSE/handler，迁移面过大；其有效部分纳入 O |
| **N：deferred commit + generation contract** | **当前实现目标** | 不预删、崩溃安全、短提交、外部兼容 | 需要重写 regen prompt snapshot 和 commit 语义 |
| **O：Session Coordinator** | **终局方向** | 统一 session 命令、状态机、TurnCommit、agent/chat 冲突和恢复 | 必须分阶段迁移，不能作为一个无界 PR 一次完成 |

### 3.1 为什么先 N、后 O

N 直接闭合 #284 的用户资产风险，同时建立 O 所需的命令和 commit 合同。它不要求立即迁移全部 agent tools、锁表和 #286，因此可作为有界实现 PR。

O 的价值是长期收敛，而不是为了“更先进”重写。只有在 N 的行为、故障注入和基准证据通过后，才逐步扩大 Coordinator 的所有权。

## 4. 方案 N：当前实现合同

### 4.1 RegenSnapshot

`regen` 开始时，在现有单次原子读/写边界内取得不可变快照，至少包含：

- `session_id`；
- target assistant `message_id`；
- session / message revision；
- `content`；
- `message_candidates`；
- `message_swipe_index`；
- generation id。

若 active-path 尾部不是 assistant，必须 fail-closed；不得调用 `delete_last_n(1)`，不得把 user 内容当成候选。

### 4.2 Prompt snapshot

当前 `prepare_regen_pipeline()` 依赖“旧 assistant 已被删除”的 history。实施 N 时必须显式重定义：

- prompt history 排除 target assistant；
- target 的候选、content 和 swipe index 来自 `RegenSnapshot`；
- streaming 期间磁盘中的旧消息保持可见、完整；
- prompt 结果应与无并发时旧流程的有效语义一致。

这是一项明确的 prompt 组装层变更，不得伪装成只改 finalizer。

### 4.3 Generation contract

`chat_completion`、`regen`、`continue` 进入同一 per-session generation contract。一个 session 同时只允许一个 active generation。

当前实现默认采用 fail-fast 冲突：

- 并发 generation 返回稳定的 `409 Conflict` + `session_busy`；
- generation 期间的 `swipe`、`edit`、`delete`、`rollback` 同样返回 `409 session_busy`；
- generation 完成后，基于同一 durable message id 的操作可重试；
- 真正不存在或不属于 session 的 message id 保留原有 `BadRequest`，不得与 busy 混用；
- `replace`/取消旧 generation 的产品语义不在本实现范围，后续需独立合同。

WebUI 应在 generation 期间禁用冲突操作，并在收到 `session_busy` 后保留可恢复提示。

### 4.4 原子 commit

LLM streaming 产出只形成内存中的 proposal，不直接修改 ChatLog。完成后提交：

```text
commit_regen(generation_id, snapshot_revision, target_message_id, proposal)
  -> 校验 active generation
  -> 校验 session/message revision 与 target id
  -> 合并旧候选与新候选并应用 SWIPE_CANDIDATES_CAP
  -> 将 message_swipe_index 指向最新候选
  -> 原子替换 target assistant 的候选状态
  -> 清除 active generation
```

CAS 失败必须返回明确冲突且保留旧消息。任何 error、cancel、panic unwind 或 timeout 路径都必须清除/过期 active generation，不得留下永久 busy session。

### 4.5 临界区与后处理

- LLM streaming 不持有 `std::sync::Mutex` 或跨 pipeline 的 async mutex guard。
- ChatLog 和必要 live-state commit 使用短临界区。
- seal、maintenance、memory extraction、user-model extraction 在 generation commit 后执行，不占用 generation 临界区。
- 上游 streaming 及可阻塞 LLM 后处理必须具有显式 timeout/watchdog；具体数值由实现 PR 基于现有 provider timeout 与基准确定。

### 4.6 Registry 生命周期

若 N 使用 per-session lease/coordinator registry：

- 条目必须懒创建并可回收；
- 可使用 `Weak` registry 或只在 registry 为唯一强引用时清理；
- 活跃、排队或持有 owner token 的条目不得被移除并由同 key 新建第二个实例；
- 进程内 cleanup 只管理内存生命周期，不能代替 generation id/revision 的持久一致性校验。

## 5. 方案 O：Session Coordinator 终局合同

### 5.1 所有权

每个 session 有且只有一个逻辑 Coordinator，负责：

- generation 命令和生命周期；
- swipe/edit/delete/rollback 等同步 mutation；
- agent turn 与 chat generation 的冲突决策；
- commit、取消、恢复和可观测状态；
- 最终接入 #286 的 TurnCommit。

“一个逻辑 Coordinator”不等于每个历史 session 永久保留一个常驻 task。实现必须支持懒创建、idle 回收和按需恢复。

### 5.2 状态机

最小状态集合：

```text
Idle
  -> Generating(snapshot, generation_id)
  -> Committing(generation_id)
  -> Idle

任一阶段发现不完整提交
  -> Recovering(turn/generation marker)
  -> Idle 或 fail-closed
```

Coordinator 在等待 LLM 时必须继续拥有 generation 状态，但不能持有阻塞 Tokio worker 的同步锁。其它命令由稳定冲突策略处理，而不是直接观察半完成 ChatLog。

### 5.3 迁移顺序

1. N 建立 Coordinator-compatible command、snapshot、proposal、commit 接口。
2. 将 legacy `/v1/chat/*` mutation 统一通过 Coordinator façade；外部合同不变。
3. 接入 agent run 和 tool mutation，消除同 session 的第二写入口。
4. 接入 #286 TurnCommit/recovery，覆盖 message、state、volume 的跨资源一致性。
5. 移除被替代的 delete-first、B1 回灌、重复锁表和绕过 service 的直接写路径。

每一步都必须保持单 owner；不允许旧路径与 Coordinator 对同一 mutation 双写。

## 6. 锁序和死锁边界

当前已核实的相关顺序是：

```text
coordinator / generation owner
  -> character_lock.read()
    -> session_lock 或 state_lock
```

约束：

- owner/coordinator 必须在调用 `with_session` 或 `StateService` 前取得；
- 持有 `session_lock`、`state_lock` 或 agent tool 内部锁时，不得反向获取 coordinator owner；
- `session_lock` 与 `state_lock` 不得在未审计路径中互相嵌套；
- 同一 task 不得重入同一 session coordinator；
- 同步磁盘 I/O 必须逐步移出 async worker 和长持锁区。

这里的结论仅限上述已记录路径：**在该锁序和禁止反向获取规则下，未发现新增死锁环**。它不是全仓“绝无死锁”的声明。`npc`、`plot`、`world_event` 等 agent tool 路径在纳入 O 前必须单独复核。

## 7. 验收标准

### 7.1 正确性

- 并发 `regen × regen` 不删除 user 消息，不产生两条错误 assistant。
- 成功 regen 保持 durable message id，候选按合同合并，`message_swipe_index` 指向最新候选。
- 候选达到 `SWIPE_CANDIDATES_CAP` 时按现有 cap 语义处理。
- `regen × continue/completion/swipe/edit/delete/rollback` 均符合明确的 busy/stale 合同。
- stream error、取消、timeout、进程终止前故障注入后，旧消息仍完整。
- commit revision 不匹配时 fail-closed，不覆盖并发新状态。

### 7.2 可用性与恢复

- 上游流式挂起不会永久锁死 session；超时后可再次发起 generation。
- seal/maintenance/memory extraction 挂起不阻塞下一次 generation commit。
- registry 回收后，同 key 不会出现两个活跃 owner。
- #286 接入前明确记录跨资源 best-effort 残差，不把 N 宣称为 TurnCommit 已完成。

### 7.3 性能

实现 PR 必须建立并保存基线，至少覆盖：

- 单 session 无竞争 completion/regen 延迟；
- 单 session 冲突请求处理；
- 多 session 并发 generation，确认没有全局串行化；
- 大量 idle session 后 registry 内存可回收；
- 慢 stream 和慢后处理下 Tokio worker 仍可服务其它 session。

性能阈值应根据同机同 provider 基线在实现 PR 中确定。没有测量前，不宣称 O 比 N/L 更快。

### 7.4 兼容与用户资产

- 现有 `/v1/chat/*` 和 SSE consumer 无需因内部迁移重写。
- 现有 ChatLog、候选和 swipe 数据可无损读取。
- 若 schema 变化，必须提供版本化 migration、升级前备份、完整性校验和可演练回滚。
- rollback 测试必须证明可回到实施前版本并继续读取原会话。

## 8. PR 与 issue 边界

本 PR 只归档设计裁决，不关闭 #284，也不声称已修复数据完整性风险。

合并本 PR 后创建实现 issue，执行范围为：

1. 交付方案 N 的 snapshot、prompt、generation contract 和 CAS commit；
2. 建立 O-compatible Coordinator façade，但不在单一 PR 内迁移全部 agent/#286；
3. 添加故障注入、并发、恢复、registry 和性能基准；
4. 在后续独立 epic 中完成 O 的 agent/TurnCommit/domain 写路径迁移。

若 N 经源码 spike 证明无法在当前发布预算内有界交付，才允许采用修订后的 L 作为临时止血；该决定必须记录未解决的 delete-first/崩溃风险、移除时机和回退条件，不能静默降级。

## 9. 审计意见处置

本修订对应 PR #374 的现有阻塞意见：

- 修正 Markdown lint 和代码块语言标识；
- 删除永久强引用 `SESSION_INFLIGHT_LOCKS` 设计，改为可回收 registry 不变式；
- 不再使用与真实 SSE 架构错位的 wrapper-task guard 伪代码；
- 将长 guard 改为 deferred proposal + 短 commit；
- 将“无死锁”收敛为有条件的锁序结论，并纳入 character/state/agent 路径；
- 明确 swipe/continue/completion 的冲突语义，不再把 400/404 当 busy；
- 修正 regen×regen 的真实 user-message corruption；
- 将 L 降为退化备选，N/O 分别作为当前交付与终局方向。

Tokio Mutex 的 FIFO 文档事实不再作为反对 L 的理由；L 被降级的理由是 delete-first、崩溃安全、长持有范围和长期所有权分裂。
