# #284 设计方案：per-session in-flight Mutex 防止并发 regen race 被 swipe 放大

- **Issue**: [#284](https://github.com/GhostXia/AIRP/issues/284)
- **来源审计**: `docs/audits/2026-07-20-PR-251-swipe-smooth-streaming-audit.md` §2.C1
- **关联 PR**: #373（#283 已合并，方案 J baseline 校验）
- **状态**: **设计待审计复核**（本 PR 不含代码改动，仅提交设计方案）
- **主导模型**: GLM-5.2（Trae IDE）

> 本文档仅为方案设计，待审计 agent 独立复核方案选择与死锁分析后，再行实现。按 AGENTS.md「审计 agent 守则」三原则：独立审计、可提己见、可质疑历史并查证。

## 1. 背景

#252 审计 §2.C1 指出：`regen_chat` 与 `swipe` / 另一次 `regen_chat` 之间存在并发竞态，可能放大为用户资产损坏。#284 为该问题的修复 issue。

#283（PR #373，已合并）已用方案 J（baseline 校验 + 写盘段持 `session_lock`）修复 `seal_volume` 的同类问题。本文档评估 #284 是否沿用方案 J，或采用不同机制。

## 2. 问题根因

### 2.1 现有锁结构核验

`engine/src/domain.rs` 中 `session_lock` 是 `std::sync::Mutex`（per `character_id/session_id`），通过 `with_session` 串行化所有 `ChatService` 写操作：

- `regen` / `append` / `append_with_candidates` / `append_to_last` / `rollback` / `delete_message` / `delete_last_n` / `switch_swipe` 全部走 `with_session`，**单次写操作是原子的**。

**但跨 streaming 的多步操作不是原子的**。`regen_chat` handler（`engine/src/daemon/handlers/chat.rs`）的实际执行流：

```
handler 持 session_lock
  └─ ChatService::regen() → delete_last_n(1) + 捕获 old_candidates
释放 session_lock
  └─ prepare_regen_pipeline（只读，构建 prompt）
  └─ tokio::spawn(streaming task)   ← 不持任何锁，秒级 LLM streaming
       └─ finalizer: 持 session_lock
            └─ append_with_candidates(old_candidates + new)
          释放 session_lock
```

两次持锁之间存在秒级 LLM streaming 间隙，允许其他 handler 插入。

### 2.2 race 时序推演

#### regen × regen（#284 原文核心场景）

| 时序 | regen A | regen B | ChatLog 状态 |
|---|---|---|---|
| t1 | 持锁 → delete_last_n(1)，old_candidates=[a] | — | 末尾 user 消息（assistant 已删） |
| t2 | 释放锁，spawn streaming A | — | 同上 |
| t3 | — | 持锁 → delete_last_n(1)（无消息可删）→ old_candidates=[] | 同上 |
| t4 | — | 释放锁，spawn streaming B | 同上 |
| t5 | streaming A 完成，finalizer 持锁 → append_with_candidates([a, b]) | — | 末尾 1 条 assistant 消息 2 候选 |
| t6 | — | streaming B 完成，finalizer 持锁 → swipe_candidates 空 → 走 `append` 分支追加新消息 | **末尾 2 条 assistant 消息** |

**结果**：用户得到 2 条消息而非 1 条 3 候选，会话结构被污染。

#### regen × swipe

- t1: regen A 持锁 → delete_last_n(1) → 释放锁
- t2: swipe 持锁 → 读取消息 → 末尾已是 user 消息（assistant 已删）→ **404 或操作错误对象**

#### regen × continue

- t1: regen A 持锁 → delete_last_n(1) → 释放锁
- t2: continue 持锁 → `append_to_last`（追加到最后消息）→ 最后已是 user 消息 → **BadRequest 或污染 user 消息**

#### chat_completion × regen

- t1: send A 持锁 → append user msg → 释放锁，spawn streaming A
- t2: regen B 持锁 → delete_last_n(1)（删除刚 append 的 user msg？或上一条 assistant？）→ 释放锁
- t3: streaming A 完成，finalizer append assistant → **顺序错乱**

### 2.3 影响面总结

- **regen × regen**：会话污染（2 条消息），用户资产损坏
- **regen × swipe / continue**：404 / BadRequest，用户体验损坏但无数据丢失
- **completion × regen**：顺序错乱，可能数据损坏
- **共同根因**：跨 streaming 的多步操作（delete → stream → append）不是原子的

## 3. 方案对比

### 3.1 方案 K：in-flight marker + finalizer 校验（延续 #283 baseline 模式）

regen handler 在 `with_session` 内做 delete + 写一个 in-flight marker（如 `regen_in_progress` 标记到 ChatLog 元数据或独立文件），finalizer 在 `with_session` 内检查 marker：
- marker 存在且匹配本次 regen 的 session → 正常 append
- marker 不存在或被其他 regen 覆盖 → 说明被并发 regen 插入 → 当前 regen 的结果作为新候选追加（而非覆盖）

**优点**：最小改动，延续 #283 的 baseline 模式，不引入新锁。
**缺点**：
- in-flight marker 需要持久化（崩溃恢复复杂）
- regen race 的"期望状态"不明确——两个 regen 都成功，结果该合并还是该分叉？语义不清
- baseline 校验在 regen 上会变成"检测到变化就 Conflict"，但两个 regen 都成功时该保留哪个？与 seal_volume 不同（seal_volume 是覆盖 current.md，regen 是追加新消息）

### 3.2 方案 L：per-session in-flight `tokio::sync::Mutex`（推荐）

引入 `tokio::sync::Mutex` 作为 per-session in-flight 锁，keyed on `character_id/session_id`（同 `session_lock`）。handler 在 regen/continue/completion 开始时 `lock().await` 获取并持有 guard，guard 传入 spawn 的 streaming task，finalizer 完成后 guard drop。其他 handler 在开始时尝试获取同一锁——**排队等待**而非插队。

**优点**：
- 彻底消除 race，语义清晰（同一 session 的 pipeline 串行执行）
- `tokio::sync::Mutex` 可跨 `.await`，天然适配 streaming 场景
- 是 #252 审计报告的建议方向（"mutex 持有时长：整个 pipeline"）

**缺点**：
- 新锁与现有 `session_lock`（std::sync::Mutex）并存，需明确分工
- 锁持有跨 `.await`，必须 `tokio::sync::Mutex`（不能用 std::sync::Mutex）
- 需处理死锁（见 §4.4）

### 3.3 方案 M：per-session executor channel

每个 session 一个 mpsc channel，所有写操作排队串行执行。handler 把任务投递到 channel，channel 的消费者单线程处理。

**优点**：无锁，天然串行，最彻底。
**缺点**：
- 架构改动大（每 session 常驻 task，资源开销）
- streaming 期间 task 阻塞 channel，影响并发
- 不适合现有 handler 同步返回 SSE stream 的架构

## 4. 推荐方案 L 细节

### 4.1 锁结构

```rust
// engine/src/domain.rs（新增）
use std::collections::HashMap;
use std::sync::Mutex as StdMutex;
use tokio::sync::Mutex as TokioMutex;

lazy_static! {
    static ref SESSION_INFLIGHT_LOCKS: StdMutex<HashMap<String, Arc<TokioMutex<()>>>> =
        StdMutex::new(HashMap::new());
}

pub fn session_inflight_lock(
    character_id: &str,
    session_id: Option<&SessionId>,
) -> Arc<TokioMutex<()>> {
    let key = session_lock_key(character_id, session_id);
    let mut map = SESSION_INFLIGHT_LOCKS.lock().expect("inflight locks map poisoned");
    map.entry(key)
        .or_insert_with(|| Arc::new(TokioMutex::new(())))
        .clone()
}
```

### 4.2 handler 层获取

`regen_chat` / `continue_chat` / `chat_completion` 在 prepare 之前获取 in-flight guard，guard 传入 pipeline，spawn 的 streaming task 持有 guard，finalizer 完成后 drop：

```rust
// engine/src/daemon/handlers/chat.rs（伪代码）
pub async fn regen_chat(...) -> Response {
    let inflight = session_inflight_lock(character_id, session_id.as_ref());
    let inflight_guard = inflight.lock().await;   // 跨整个 pipeline 持有

    // prepare（持 guard，不持 session_lock）
    let pipeline = prepare_regen_pipeline(...).await?;

    // spawn streaming task，guard move 进去
    tokio::spawn(async move {
        let _guard = inflight_guard;   // 持有直到 streaming + finalizer 完成
        let stream = build_sse_stream(pipeline, ...).await;
        // stream 内 finalizer 完成后 _guard drop
    });

    // ... 返回 SSE response
}
```

### 4.3 guard 跨 `.await`

`tokio::sync::MutexGuard` 可跨 `.await`，传给 spawn 的 task。这是 `tokio::sync::Mutex` 相对 `std::sync::Mutex` 的核心优势，也是方案 L 可行的前提。

### 4.4 死锁分析

**两个锁的分工**：
- `session_lock`（std::sync::Mutex）：保护 ChatService 单次写操作的瞬时锁，`with_session` 内获取释放，不跨 `.await`
- `session_inflight_lock`（tokio::sync::Mutex）：保护跨 streaming 多步操作的 in-flight 锁，handler 层获取，finalizer 完成后释放，跨多个 `.await`

**锁获取顺序**（所有路径统一）：
1. handler 顶层：获取 `session_inflight_lock`（持有）
2. handler 内 ChatService 写操作：`with_session` 获取 `session_lock`（瞬时，释放）
3. spawn streaming task：持有 `session_inflight_lock`
4. finalizer 内 ChatService 写操作：`with_session` 获取 `session_lock`（瞬时，释放）
5. streaming + finalizer 完成：`session_inflight_lock` 释放

**死锁判定**：
- `session_inflight_lock` 持有期间，finalizer 内获取 `session_lock` 是瞬时释放，不持有 `session_inflight_lock` 反向获取的路径
- 两个锁的 key 相同（同 character_id/session_id），但是不同锁实例，无环形等待
- 唯一风险：同一 task 持有 `session_inflight_lock` 后再获取 `session_inflight_lock`（递归锁）——但 regen/continue/completion handler 不会嵌套调用，无此风险

**结论**：方案 L 无死锁风险。

### 4.5 与 #283 方案 J 的互补关系

| 维度 | #283 方案 J | #284 方案 L |
|---|---|---|
| 场景 | seal_volume 写盘段（sync，毫秒级） | regen/continue/completion 跨 streaming（async，秒级） |
| 锁类型 | std::sync::Mutex（session_lock 复用） | tokio::sync::Mutex（新增 in-flight 锁） |
| 持有时长 | 写盘三元组（不跨 .await） | 整个 pipeline（跨多个 .await） |
| baseline 校验 | 有（current.md + index.md） | 不需要（串行化即消除 race） |
| 失败模式 | Conflict + 调用方重试 | 排队等待（无失败） |

两者针对不同场景，互补而非替代。

## 5. 不在 in-flight 锁覆盖范围的写操作

以下操作是单次同步写（走 `with_session`，session_lock 已保护原子性），**不需要** in-flight 锁：

- `swipe` / `switch_swipe`
- `delete_message` / `delete_last_n`
- `edit_message`
- `rollback`
- `advance_clock`（已独立审计为 Bug F 修复）

**与 in-flight regen 的交互**：regen 删除消息后 streaming 中，用户 swipe 该消息 → 404（消息已删）。这是合理行为（用户操作了已被 regen 删除的消息），不需 in-flight 锁保护。用户应等待 regen 完成再操作。

**例外考虑**：若审计认为 swipe/continue 在 in-flight regen 期间应排队而非 404，需在 handler 层对这些操作也获取 in-flight 锁。本设计暂不覆盖，待审计复核。

## 6. 测试计划

### 6.1 回归测试（必做）

1. **regen × regen race**：两个 regen 并发触发，验证：
   - 后触发的 regen 排队等待，不插队
   - 最终 ChatLog 末尾为 1 条 assistant 消息，候选数 = regen A 候选数 + regen B 候选数
   - 不产生 2 条消息
2. **regen × swipe race**：regen streaming 中触发 swipe，验证：
   - swipe 等待 regen 完成后操作新的消息（若 in-flight 锁覆盖 swipe）
   - 或 swipe 404（若不覆盖，按本设计）
3. **regen × continue race**：regen streaming 中触发 continue，验证：
   - continue 等待 regen 完成（若覆盖）
   - 或 continue BadRequest（若不覆盖，按本设计）

### 6.2 死锁回归测试

并发触发 100 次 regen × swipe × continue 混合操作，验证无死锁（超时返回即判定死锁）。

### 6.3 性能回归测试

in-flight 锁串行化后，单 session 吞吐下降（regen 必须排队）。验证：
- 单 session 串发 10 次 regen，总时长 vs 无锁基线（应基本持平，锁等待是预期行为）
- 多 session 并发 regen，无相互阻塞（in-flight 锁是 per-session，不应跨 session 串行化）

## 7. 待审计复核的问题

请审计 agent 独立复核以下问题，给出阻塞/通过意见：

1. **方案选择**：方案 L（per-session in-flight tokio::sync::Mutex）是否是 #284 的正确修复方向？是否有更好的方案？
2. **锁粒度**：in-flight 锁是否应覆盖 regen / continue / chat_completion 三条路径？还是仅 regen（race 风险最高）？
3. **死锁分析**：§4.4 的死锁分析是否成立？是否有未考虑的死锁路径？
4. **swipe/continue 在 in-flight regen 期间的行为**：本设计选择"404 / BadRequest"（用户操作了已被 regen 删除的消息）。是否应改为排队等待（in-flight 锁覆盖 swipe/continue）？
5. **与 #283 方案 J 的兼容性**：两者并存是否会产生新的 race 或死锁？
6. **guard 传递机制**：guard 从 handler move 到 spawn 的 task，是否会影响 SSE response 的及时返回（response 不应等待 streaming 完成）？

## 8. 实现计划（待审计通过后）

本 PR 仅提交设计文档。审计复核通过后，将在另一 PR 实现代码改动：

1. `engine/src/domain.rs`：新增 `SESSION_INFLIGHT_LOCKS` + `session_inflight_lock`
2. `engine/src/daemon/handlers/chat.rs`：`regen_chat` / `continue_chat` / `chat_completion` 获取 in-flight guard，传入 spawn task
3. `engine/src/chat_pipeline/stream.rs`：`build_sse_stream` / `FinalizerCtx` 接收 guard，finalizer 完成后 drop
4. `engine/src/chat_pipeline/types.rs`：`FinalizerCtx` 增加 `inflight_guard` 字段（或用其他传递机制）
5. 回归测试 + 死锁测试 + 性能测试
6. fmt / clippy / test / doc 全绿

## 9. 审计模型声明

本设计方案由 **GLM-5.2**（Trae IDE）主导。按 AGENTS.md「审计报告必须声明所用 LLM 模型」要求，本设计文档的方案选择、死锁分析、测试计划均由 GLM-5.2 产出，未经其他模型复核。等待审计 agent 独立复核。
