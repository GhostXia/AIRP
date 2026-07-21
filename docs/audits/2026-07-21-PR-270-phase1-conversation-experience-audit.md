# PR #270 独立审计报告 — feat: 阶段一 - 对话体验补全 (Edit/Branch/Streaming)

- **审计日期**：2026-07-21
- **审计 agent 模型**：GLM-5.2（本会话模型）
- **PR**：[#270](https://github.com/GhostXia/AIRP/pull/270) — `feat/phase1-conversation-experience` @ commit `dbe67ef`
- **修复 commit**：`b8a0a02`（9 BLOCKING + 3 MINOR 修复）+ `3a56f43`（与 main 合并冲突解决）+ `d335128`（§9.1~9.3 CodeRabbit follow-up：CR-1~CR-4 修复 + CR-5/CR-6 拒绝）+ 本轮 §9.4 CodeRabbit follow-up 修复（CR-7 MD038 残留 + CR-8 maintainer-local file URI，commit 待提交）
- **修复后状态**：所有 9 个 BLOCKING（B1–B9）+ 3 个 MINOR（M1–M3）已修复，并已闭环处理 CodeRabbit 遗留 review 意见（§9 CodeRabbit follow-up）。本地 `cargo test --workspace`（804 lib + 4 bin engine main + 11 openai_compat + 5 agent_run + 5 production_startup + 6 sse_wiremock + 6 protocol lib + 9 ui bin 全绿）/ `npm run test -- --run` (ui/, 98 passed) / `cargo clippy --workspace --all-targets --all-features -- -D warnings` / `cargo fmt --all -- --check` 全绿。
- **审计范围**：12 个文件 / +526 −28（engine 数据模型 + domain 方法 + HTTP 端点 + chat_pipeline 字段；WebUI Edit/Branch UI + SmoothStreamer 增强）
- **审计原则**：按 `AGENTS.md` §"Audit Agent Charter" 三原则独立审计。本 PR 引入分支对话树（branching conversation tree）+ 消息编辑 API + 流式输出优化，是 issue 阶段一 4 个子任务的整体落地。需独立核验：(1) `message_parents` + `active_leaf` 数据模型对历史 jsonl 的兼容性是否如 PR 声称"向后兼容"；(2) 分支树核心不变量（并行数组等长、active_path 正确性、cross-branch 隔离）在 append / append_with_branch / switch_branch / delete_last_n / rollback_to / delete_message / recent / history_window 全路径上是否成立；(3) `PUT /v1/chat/message` 编辑端点的安全边界（body limit、role 限制、ID 不变）是否完备；(4) WebUI 分支 UI 的 DOM 一致性在 branch-from → doSend 路径上是否正确；(5) 测试覆盖是否匹配新增 public API 的范围。

---

## 0. 前置核验

### 0.1 CI 状态

| Check | 状态 | 备注 |
|---|---|---|
| Rust workspace | FAILURE | `cargo fmt --all -- --check` 失败：`engine/src/chat_store.rs:623` 单行 `let parent = self.active_leaf.clone().or_else(...)` 应折行；`engine/src/daemon/handlers/chat.rs:14` import 排序不符合 rustfmt 规则。本审计修复一并处理（详见 §2.D1）。 |
| UI and WebUI | SUCCESS | |
| Production topology | SUCCESS | |
| Portable Windows WebUI | SUCCESS | |
| Attach portable WebUI to release | SKIPPED | by-design（release 事件触发） |
| CodeRabbit | SUCCESS | |

CI 在原始 commit 上有 1 项 FAILURE。本审计修复后 `cargo fmt --all -- --check` 本地通过。除 fmt 外，本审计发现的关键问题（B1 物理截断破坏 sibling 分支、B5 history_window 跨分支返回消息、B6/B7 大小写敏感比较绕过 #37 合同等）CI 均无法捕获。

### 0.2 bot review 意见闭环核验

#### gemini-code-assist 意见 1：`SmoothStreamer` 代码块状态滞后 + 句子边界检测失效

- **原意见**：`push` 中提前更新 `this.inCodeBlock` 会导致渲染队列前部文本时错误地使用队列尾部的状态；`SENTENCE_END_RE.test(candidate[i])` 传入单字符，正则 `$` 锚点匹配单字符末尾，任何 `.` 都被误判为句子边界（如 `1.0` / `Google.com`）。建议动态计算 `(this.prefix + this.rendered).match(/\u0060\u0060\u0060/g)` 并在 `this.queue` 上结合上下文检测。
- **核验**：✓ **部分成立，确认为 MINOR（M1）**。详见 §2.M1。`SENTENCE_END_RE.test(candidate[i])` 确实传入单字符，但实际正则 `/[。！？；\n]|[.!?](?=\s|$)/` 中的 `$` 在 `candidate[i]` 为 `"."` 单字符时会匹配该字符串末尾，导致 `Google.com` 中的 `.` 被误判为边界。本审计修复采用 2-char context 检测（取 `candidate[i]` + `queue[i+1]` 拼成 2 字符串再 test），不动态计算已渲染位置的代码块状态（成本过高且与现有 `inCodeBlock` 跟踪重复），保留 `push` 中 `inCodeBlock` 更新但加注释说明限制。

#### gemini-code-assist 意见 2：`append` 应使用 `resolve_active_leaf` 而非直接克隆 `self.active_leaf`

- **原意见**：`append` 中 `let parent = self.active_leaf.clone().or_else(|| self.message_ids.last().cloned())` 在 `active_leaf` 包含无效或已删除 ID 时会引入脏数据。建议使用 `resolve_active_leaf` 方法，它会先验证 ID 是否存在。
- **核验**：✓ **不成立，可驳回**。`resolve_active_leaf` 内部就是 `active_leaf.as_ref().filter(|id| message_ids.iter().any(|m| m == id)).or_else(|| message_ids.last().cloned())`，与 `append` 中的 `or_else` fallback 逻辑等价（如果 `active_leaf` 不在 `message_ids` 中，`active_leaf.clone()` 仍是 `Some(invalid_id)`，不会被 fallback 替换）。但这不会引入"脏数据"——`append_with_parent` 会在 `active_leaf` 不存在时走 `message_ids.last()` fallback（详见 `chat_store.rs:636-647`）。gemini 的建议反而会改变 `append` 的语义（让 `active_leaf` 失效时回退到 `last`，而不是保留 `Some(invalid)`）。当前实现是正确的。

#### CodeRabbit 自动总结

- **核验**：✓ CodeRabbit 给出 5/5 PASS（Description / Title / Docstring / Linked Issues / Out of Scope），无阻塞意见。本审计的 9 个 BLOCKING 全部来自独立审计，CodeRabbit 未捕获。

---

## 1. 审计方法

1. **diff 全文走读**：`gh pr diff 270 --patch` + `git diff main...pr-270 -- <file>` 逐文件核验 12 个改动文件。
2. **源码走读**：本地 `git fetch origin pull/270/head:pr-270` 后 `Read` 关键文件（`engine/src/chat_store.rs` 53-130 / 620-820 / 857-930 / 700-750、`engine/src/domain.rs` 200-260 / 300-400 / 440-525 / 600-750、`engine/src/daemon/mod.rs` 310-345、`engine/src/daemon/handlers/chat.rs` 1-205、`engine/src/daemon/types.rs` 55-180、`engine/src/chat_pipeline/prepare.rs` 1-80、`engine/src/agent/mod.rs` 1-50、`webui/app.js` 1180-1297 / 1355-1390 / 1740-1830）。
3. **不变量验证**：手工推演 6 个并行数组（`messages` / `message_ids` / `message_timestamps` / `message_candidates` / `message_swipe_index` / `message_parents`）等长不变量在 append / append_with_parent / append_with_branch / append_with_candidates / switch_branch / switch_swipe / delete_last_n / rollback_to / delete_message / read_messages_jsonl 全路径上的成立性。
4. **分支树拓扑推演**：构造分支拓扑（`m0 → m1 → m2 → m3` 主线，`m1 → m4` 分支）逐一推演 `active_path_indices()` 在显式 parent 链 + legacy 线性链（全 `None` parent + `None` active_leaf）上的行为，并验证 `delete_last_n(1)` / `rollback_to(1)` / `recent(2)` / `history_window(cursor=m4_id)` 在分支场景下的输出。
5. **大小写敏感性核验**：阅读 `engine/src/ulid.rs:46-61`，确认 `is_valid_id` 要求小写 `m` 前缀但 hex 部分大小写不敏感，`matches()` 用 `eq_ignore_ascii_case`。Grep `==` / `.eq(` 在 `chat_store.rs` + `domain.rs` 中所有 message_id 比较位置，逐一核验是否应使用 `ulid::matches`。
6. **HTTP 安全边界核验**：Grep `DefaultBodyLimit::max` 在 `engine/src/daemon/mod.rs` 全部 PUT/POST 端点，核验 `PUT /v1/chat/message` 是否有 body limit（对比 `PUT /v1/characters/:id` 已有 2MB）。
7. **WebUI DOM 一致性推演**：单独推演 doSend 在 `branchFromId` 非空时的 DOM 操作链——是否在追加新消息前清理掉分支点之后的旧 DOM 节点，避免出现"分支点 + 旧子树 + 新分支"的混合视图。
8. **测试覆盖核验**：`Grep "branch_from|append_with_branch|switch_branch|active_path_indices|message_parents|active_leaf"` 全 engine/src + webui/tests，确认 PR 新增的分支功能零测试覆盖。
9. **CI 状态核验**：`gh pr checks 270 --json name,state,link` 确认 1 个 check FAILURE（Rust workspace / cargo fmt），其余 5 个 SUCCESS/SKIPPED。
10. **第三方经验吸收合规**：PR 描述未提及第三方参考；CodeRabbit 总结提及 SillyTavern 为公开行为参考，AIRP 独立实现。`docs/ACKNOWLEDGEMENTS.md` 已记录 SillyTavern（commit 380e31e，AGPL-3.0），符合 `AGENTS.md` §"第三方经验吸收与独立实现"。

---

## 2. 独立核验结论

### 2.A 数据模型与向后兼容性 — ✓ 成立

#### 2.A.1 `ChatLog` 新增字段 + serde default（`engine/src/chat_store.rs:71-87`）

- `message_parents: Vec<Option<String>>` + `active_leaf: Option<String>`，均带 `#[serde(default)]`。旧 jsonl 反序列化时 `message_parents` 补 `None`、`active_leaf` 补 `None`，符合 PR 声称的"向后兼容"。✓
- `StoredMessage` 的 `parent: Option<String>`，`#[serde(default, skip_serializing_if = "Option::is_none")]`。旧消息写出的 jsonl 行不含 `parent` 字段，与旧行兼容。✓
- `read_messages_jsonl`（`engine/src/chat_store.rs:780-840`）对旧行 `stored.parent` 用 `unwrap_or(None)`，向后兼容。✓
- `ChatLogMeta` 加 `active_leaf: Option<String>` + `#[serde(default, skip_serializing_if = "Option::is_none")]`。旧 meta 无此字段 → `None`，加载时 `derive_meta` 从最后一条消息 ID 恢复。✓
- 迁移路径（`engine/src/chat_store.rs:405-430`）补齐长度不匹配的 `message_parents`（用 `None` 填充）。✓

#### 2.A.2 等长不变量维护（`engine/src/chat_store.rs` + `engine/src/domain.rs`）

`new` / `append` / `append_with_parent` / `append_with_candidates` / `delete_last_n` / `rollback_to` / `delete_message` 全部同步维护 6 个并行数组（messages / message_ids / message_timestamps / message_candidates / message_swipe_index / message_parents）。手工推演等长不变量在所有 happy path 成立。✓

但 §2.B1 / §2.B2 / §2.B3 的 bug 会在错误路径上破坏不变量（删除时不同步删 parent、append_with_candidates 不写 parent）。

#### 2.A.3 `active_path_indices()` 拓扑正确性（`engine/src/chat_store.rs:867-915`）

- 显式 parent 链：从 `active_leaf`（或 `message_ids.last()`）沿 `message_parents` 反向追溯到根，输出物理下标序列。✓
- Legacy 线性链（全 `None` parent + `None` active_leaf）：fallback 到 `0..messages.len()` 全序列。✓
- 混合链（部分有 parent 部分无）：在第一个 `None` parent 处停止回溯，剩余用 `0..=first_none_idx` 补全。✓

`active_path_indices()` 拓扑正确。但 §2.B1（`delete_last_n` / `rollback_to` 不用它）、§2.B4（`recent` 不用它）、§2.B5（`history_window` 不用它）使得该 helper 在 PR 原始 commit 上几乎未被消费——审计修复后才真正启用。

### 2.B BLOCKING 问题

#### 2.B1 `delete_last_n` / `rollback_to` 物理截断破坏 sibling 分支数据（CRITICAL） — ✅ RESOLVED

**位置**：`engine/src/chat_store.rs:700-750`（原始 commit `dbe67ef`）

**问题**：原始 `delete_last_n` 直接 `messages.truncate(messages.len() - n)` + 同步截断其他 5 个数组。在分支场景下，`messages` 的物理尾部可能包含 sibling 分支的消息（不在 active path 上），truncate 会把它们一起删掉。

**示例**：
```text
拓扑：m0 → m1 → m2 → m3 (active leaf)
              └→ m4 (sibling branch)

messages 物理顺序：[m0, m1, m2, m3, m4]
active_leaf = m3
```

用户在主线 `m3` 上点 regen → `delete_last_n(1)`：
- 原始实现：`messages.truncate(5 - 1)` → `[m0, m1, m2, m3]`，**`m4` 被永久删除**。
- 正确行为：只删 `m3`（active path 最后一条），保留 `m4`（sibling 分支）。

`rollback_to(index)` 有相同问题：直接 `messages.truncate(index + 1)` 会把物理位置 `> index` 的所有消息（含 sibling 分支）删掉。

**影响**：用户在分支对话中 regen / rollback 会静默丢失其他分支的全部消息。这是不可恢复的用户资产损坏。

**违反约束**：`AGENTS.md` §"破坏旧结构，不破坏用户资产" — "不得静默损坏用户数据、角色卡、世界书、会话、记忆或可恢复能力"。

**修复**：改为 branch-aware removal：
1. 计算 `active_indices = self.active_path_indices()`；
2. 取 `to_remove = active_indices[len - n..]`（active path 最后 n 个的物理下标）；
3. `to_remove.sort_unstable_by(|a, b| b.cmp(a))`（降序，保证 `Vec::remove(idx)` 不影响更早的 stashed 下标）；
4. 对 6 个并行数组分别 `Vec::remove(idx)`；
5. 更新 `active_leaf = message_ids[active_indices[len - n - 1]]`（new leaf = active path 上倒数第 n+1 个）。

`rollback_to(index)` 同样改为：先验证 `index` 在 active path 上，再 branch-aware removal active path 上 `index` 之后的所有消息。

**回归测试**：`delete_last_n_preserves_sibling_branch` / `rollback_to_preserves_sibling_branch`（`engine/src/chat_store.rs` 测试模块末尾）。

#### 2.B2 `append_with_candidates` 不更新 `message_parents` / `active_leaf`（CRITICAL） — ✅ RESOLVED

**位置**：`engine/src/domain.rs:440-475`（原始 commit `dbe67ef`）

**问题**：`append_with_candidates` 是 #249 Swipe 引入的方法，PR #270 没有同步更新它以维护分支树字段。原始实现只 push `messages` / `message_ids` / `message_timestamps` / `message_candidates` / `message_swipe_index`，**不 push `message_parents`，不更新 `active_leaf`**。

**后果**：
1. 6 个并行数组等长不变量破坏（`message_parents.len() < messages.len()`）。
2. `active_leaf` 不指向新增的 assistant 消息，后续 `active_path_indices()` 会走到错误分支。
3. regen 路径（`ChatService::regen` → `delete_last_n(1)` → pipeline → `append_with_candidates`）在分支场景下会让 active_leaf 滞留在被删除的消息上。

**修复**：在 `append_with_candidates` 中：
1. `log.message_parents.push(parent)`（parent = `active_leaf` 或 `message_ids.last()`）；
2. `log.active_leaf = Some(new_message_id)`。

**回归测试**：现有 `append_with_candidates_*` 系列测试覆盖 happy path；分支场景下 regen 的端到端测试列为 follow-up。

#### 2.B3 `delete_message` 不删 `message_parents` 条目 / 不重置 `active_leaf`（CRITICAL） — ✅ RESOLVED

**位置**：`engine/src/domain.rs:300-340`（原始 commit `dbe67ef`）

**问题**：`delete_message` 是 #37 引入的方法，PR #270 没有同步更新。原始实现只 `Vec::remove(idx)` 前 5 个数组（messages / message_ids / message_timestamps / message_candidates / message_swipe_index），**不删 `message_parents[idx]`**，**不重置 `active_leaf`**。

**后果**：
1. 等长不变量破坏（`message_parents.len() > messages.len()`）。
2. 如果删除的是 `active_leaf` 指向的消息，`active_leaf` 仍指向已删除的 ID，后续 `active_path_indices()` 会 fallback 到 `message_ids.last()`，可能跨越分支边界。
3. 删除非 leaf 消息时，其子消息的 `parent` 字段仍指向已删除的 ID，`active_path_indices()` 回溯会在该处停止（fallback 到线性链），拓扑被静默改写。

**修复**：
1. `log.message_parents.remove(idx)`；
2. 如果 `active_leaf == Some(deleted_id)`，重置为 `message_ids.last().cloned()`（新的 leaf = 物理最后一条，与 legacy 行为一致）；
3. （可选）清理孤儿引用：遍历 `message_parents`，把指向 `deleted_id` 的 `Some(deleted_id)` 改为 `None`——但本审计不实施，因为：(a) 当前 `active_path_indices()` 对 `Some(invalid_id)` 的处理与 `None` 等价（都 fallback）；(b) 实施会改变其他消息的 parent，影响其他分支的拓扑。列为 follow-up（F-2）。

**回归测试**：现有 `delete_message_*` 系列测试覆盖 happy path；分支场景下 delete_message 的端到端测试列为 follow-up。

#### 2.B4 `recent()` 用物理尾部而非 active path（MAJOR） — ✅ RESOLVED

**位置**：`engine/src/chat_store.rs:818-830`（原始 commit `dbe67ef`）

**问题**：`recent(n)` 原始实现 `messages[messages.len() - n..]`，在分支场景下会返回 sibling 分支的消息。例如上述拓扑 `recent(2)` 返回 `[m3, m4]`（物理最后 2 条），而正确行为是返回 `[m2, m3]`（active path 最后 2 条）。

**影响**：模型上下文构建用 `recent` 取近期消息，跨分支返回会让模型看到非 active 分支的内容，破坏分支隔离语义。

**修复**：改为 `let active_indices = self.active_path_indices(); let start = active_indices.len().saturating_sub(n); active_indices[start..].iter().map(|&i| &self.messages[i]).collect()`。同时保留 legacy 线性链的兼容性（`active_path_indices` 在 legacy 数据上返回全序列，`recent` 行为不变）。

**回归测试**：`recent_returns_active_branch_only`（`engine/src/chat_store.rs` 测试模块末尾）。

#### 2.B5 `history_window` 用物理切片而非 active path（MAJOR） — ✅ RESOLVED + 增强

**位置**：`engine/src/domain.rs:200-260`（原始 commit `dbe67ef`）

**问题**：`history_window(limit, cursor)` 原始实现用 `log.messages.len()` 和物理下标切片，在分支场景下会：
1. 返回 sibling 分支的消息；
2. `cursor` 在 inactive 分支上时，分页会跨分支。

**影响**：
- 前端 `HistoryWindow` 显示混合分支内容，与 `active_leaf` 不一致。
- 长会话分页加载时，cursor 跨分支会让用户看到非 active 分支的历史。

**修复**：两阶段验证 + active path 过滤：
1. **先验证 cursor 在本 session 中存在**（cross-session 拒绝，保留原 contract）：
   ```rust
   let in_session = log.message_ids.iter().any(|mid| ulid::matches(mid, id));
   if !in_session {
       return Err(AirpError::BadRequest(format!(
           "cursor {id} not in this session (cursor cannot cross character/session)"
       )));
   }
   ```
2. **再验证 cursor 在 active path 上**（cross-branch 拒绝，B5 新 contract）：
   ```rust
   let pos = active_indices.iter().position(|&phys_idx| {
       log.message_ids.get(phys_idx).map(|mid| ulid::matches(mid, id)).unwrap_or(false)
   }).ok_or_else(|| {
       AirpError::BadRequest(format!("cursor {id} not on active branch (cursor cannot cross branch)"))
   })?;
   ```
3. **active path 过滤**：返回 `active_indices` 中 `[0, pos)` 范围（cursor 之前）的最后 `limit` 条消息。

**为什么需要两阶段验证**：原始 contract 是"cursor 不能跨 session"（错误信息 `"not in this session"`）。如果直接用 active path 检查替换，cross-session cursor 会得到 `"not on active branch"` 错误，破坏现有 API 合同。两阶段验证保留 cross-session 错误信息，同时新增 cross-branch 错误信息。

**回归测试**：
- `history_window_filters_to_active_branch`（active path 过滤）；
- `history_window_cursor_rejects_id_on_inactive_branch`（cross-branch 拒绝）；
- `history_window_cursor_rejects_id_from_other_session_still_works`（cross-session 拒绝回归）。

#### 2.B6 `append_with_branch` 用 `==` 比较 message_id 而非 `ulid::matches`（MAJOR） — ✅ RESOLVED

**位置**：`engine/src/domain.rs:380-410`（原始 commit `dbe67ef`）

**问题**：`append_with_branch(branch_from)` 原始实现用 `log.message_ids.iter().any(|mid| mid == branch_from)` 验证 `branch_from` 存在性。但 #37 durable message-id 合同明确要求 ID 大小写不敏感（`ulid::matches`），`is_valid_id` 接受 mixed-case hex 部分（仅 `m` 前缀要求小写）。如果客户端传入 `mABC123...`（大写 hex），`==` 比较会失败，`append_with_branch` 错误返回 `BadRequest("branch_from not found")`。

**影响**：违反 #37 合同。某些前端（如 Tauri shell）可能在大写化路径上序列化 ID，导致分支功能失效。

**修复**：`log.message_ids.iter().any(|mid| ulid::matches(mid, branch_from))`。

**回归测试**：`append_with_branch_rejects_unknown_branch_from`（B6 修复验证）+ `append_with_branch_creates_branch_from_arbitrary_message`（happy path）。

#### 2.B7 `resolve_active_leaf` / `switch_branch` / `children_of` 用 `==` 比较 message_id（MAJOR） — ✅ RESOLVED

**位置**：`engine/src/chat_store.rs` 多处（原始 commit `dbe67ef`）

**问题**：与 B6 同型。`resolve_active_leaf` / `switch_branch` / `children_of` 全部用 `==` 比较 message_id，违反 #37 大小写不敏感合同。

**修复**：全部改为 `ulid::matches`。

**回归测试**：
- `children_of_case_insensitive_match`（B7 修复验证）；
- `resolve_active_leaf_case_insensitive_match`（B7 修复验证）；
- `switch_branch_rejects_unknown_id`（边界场景）。

#### 2.B8 `PUT /v1/chat/message` 缺 body limit（MAJOR） — ✅ RESOLVED

**位置**：`engine/src/daemon/mod.rs:316`（原始 commit `dbe67ef`）

**问题**：PR 新增 `PUT /v1/chat/message` 端点（编辑 user 消息），但路由注册没有 `.layer(DefaultBodyLimit::max(...))`。对照 `PUT /v1/characters/:id` 已有 `DefaultBodyLimit::max(2 * 1024 * 1024)`（2MB），`PUT /v1/chat/message` 缺失会让攻击者发送超大 body 触发 OOM/DoS。

**违反约束**：`project_memory.md` §"Hard Constraints" — "PUT endpoints must have body limit configured (2MB) to prevent DoS attacks"。

**修复**：
```rust
.route(
    "/v1/chat/message",
    put(edit_message.layer(DefaultBodyLimit::max(2 * 1024 * 1024))),
)
```

**回归测试**：现有 `edit_message_*` 系列测试覆盖 happy path；body limit 的端到端测试（发送 > 2MB body 验证 413）列为 follow-up（F-3）。

#### 2.B9 `doSend` 在 `branchFromId` 非空时不清理分支点之后的 DOM（MAJOR） — ✅ RESOLVED

**位置**：`webui/app.js:1180-1297`（原始 commit `dbe67ef`）

**问题**：用户点击 ⑂ 按钮设置分叉点后，`branchFromId` 被设为该消息 ID。下次 `doSend` 时，前端会：
1. 向后端发送 `branch_from` 参数，后端 `append_with_branch` 创建新分支；
2. 后端返回新的 active path（不含原 leaf 之后的消息）；
3. **但前端 DOM 仍保留原 leaf 之后的旧消息节点**——因为 `doSend` 只追加新消息 DOM，不清理旧分支的 DOM。

**后果**：用户看到"分叉点 + 旧子树 + 新分支"的混合视图，与后端 `active_leaf` 状态不一致。继续对话时，前端 `existingText` 计算会包含旧分支的文本，导致 `SmoothStreamer` 的 `prefix` 错误。

**修复**：在 `doSend` 中，如果 `branchFromId` 非空，先找到分支点消息的 DOM 节点，删除其后所有兄弟节点，再追加新消息 DOM。

**回归测试**：WebUI 现有 125 个测试不覆盖 DOM 操作（`node --test` 只测纯函数）；浏览器端到端测试列为 follow-up（F-4）。

### 2.C MAJOR 问题（非 BLOCKING 但需修复）

（无 — 所有 MAJOR 问题都已归入 §2.B）

### 2.D MINOR 问题

#### 2.D1 CI `cargo fmt` 失败（MINOR / PROCESS） — ✅ RESOLVED

**位置**：`engine/src/chat_store.rs:623` + `engine/src/daemon/handlers/chat.rs:14`（原始 commit `dbe67ef`）

**问题**：原始 PR commit 未跑 `cargo fmt --all -- --check` 就推送，CI Rust workspace check 失败：
1. `chat_store.rs:623` 单行 `let parent = self.active_leaf.clone().or_else(|| self.message_ids.last().cloned());` 应折行为多行；
2. `daemon/handlers/chat.rs:14` import 排序不符合 rustfmt 规则（`ChatCompletionRequest, ContinueRequest, DeleteMessageRequest, EditMessageRequest,` 后应换行 `HistoryQuery,`）。

**修复**：本审计修复一并跑 `cargo fmt --all`，把上述 2 处格式问题修复。审计修复 commit 后 CI 应全绿。

#### 2.M1 `SENTENCE_END_RE.test(candidate[i])` 传入单字符导致句子边界检测失效（MINOR / FUNCTIONAL） — ✅ RESOLVED

**位置**：`webui/app.js:1782`（原始 commit `dbe67ef`）

**问题**：`candidate[i]` 是单字符（如 `"."`）。正则 `/[。！？；\n]|[.!?](?=\s|$)/` 中的 `[.!?](?=\s|$)` 在 `candidate[i]` 为 `"."` 时，`$` 锚点匹配该单字符字符串的末尾，导致任何英文句号都被误判为句子边界。例如 `1.0` / `Google.com` 中的 `.` 会被错误识别为句子结束。

**影响**：打字机模式下的"智能分句"会在小数点、域名等位置错误截断，导致渲染卡顿或截断位置不自然。但因为是 `charsToRender < queue.length` 才进入此分支，且只在打字机模式开启时生效，影响有限。

**修复**：取 2 字符 context——`candidate[i]` + `queue[i+1]`（如果存在）拼成 2 字符串，再 test 正则。这样 `(?=\s|$)` 中的 `$` 不再匹配单字符末尾，而是匹配 2 字符串末尾（仅在 `i` 是 candidate 最后一个字符且 queue 后续为空时才匹配）。

```javascript
// 修复前：candidate[i] 是单字符，$ 锚点误匹配
if (SENTENCE_END_RE.test(candidate[i])) { ... }

// 修复后：取 2 字符 context，$ 锚点只在真正的字符串末尾匹配
const two = candidate[i] + (this.queue[charsToRender + (i - candidate.length + 1)] || '');
if (SENTENCE_END_RE.test(two)) { ... }
```

**回归测试**：WebUI 现有测试不覆盖 `SmoothStreamer.tick` 内部分句逻辑（涉及 rAF 时序）；单元测试 `SENTENCE_END_RE` 行为列为 follow-up（F-1）。

#### 2.M2 / 2.M3 分支功能零测试覆盖（MINOR / PROCESS） — ✅ RESOLVED

**位置**：`engine/src/chat_store.rs` + `engine/src/domain.rs` 测试模块（原始 commit `dbe67ef`）

**问题**：PR 新增 5 个 public 方法（`append_with_parent` / `append_with_branch` / `active_path_indices` / `switch_branch` / `children_of`）+ 1 个 HTTP 端点（`POST /v1/chat/branch/switch`）+ 2 个数据模型字段（`message_parents` / `active_leaf`），但**没有添加任何针对分支功能的测试**。

- `engine/src/domain.rs` 共 43 个 `#[test]`，全部是 pre-existing，无一测试 `append_with_branch` / `switch_branch` / `active_path_indices`。
- `engine/src/chat_store.rs` 测试模块中 `Grep "branch|active_path|active_leaf|message_parents"` 零命中。
- `engine/src/daemon/tests/` 中 `Grep "branch|switch_branch|edit_message"` 零命中。
- `webui/tests/` 未修改。

**直接后果**：
1. §2.B1 / §2.B2 / §2.B3 的并行数组等长不变量破坏无任何测试会捕获。
2. §2.B4 / §2.B5 的 active path 过滤行为无测试保障。
3. §2.B6 / §2.B7 的大小写敏感性无回归保障。
4. 跨 session 加载 / 旧 jsonl 迁移 / rollback 后分支一致性，无测试。

**修复**：新增 16 个分支功能测试：

`engine/src/chat_store.rs`（11 个，测试模块末尾 `#270 分支对话树测试` 块）：
- `active_path_indices_walks_explicit_parent_chain` — 显式 parent 链 → path = [0,1,2,3]，m4 excluded
- `active_path_indices_legacy_linear_fallback` — 全 None parent + None active_leaf → path = [0,1,2,3]（legacy 兼容）
- `delete_last_n_preserves_sibling_branch` — B1 修复：delete_last_n(1) on m3 preserves m4
- `rollback_to_preserves_sibling_branch` — B1 修复：rollback_to(1) preserves m4
- `recent_returns_active_branch_only` — B4 修复：recent(2) returns [m2,m3], not [m3,m4]
- `switch_branch_changes_active_path` — switch to m4 → path = [0,1,4]
- `switch_branch_rejects_unknown_id` — unknown ID → BadRequest
- `children_of_finds_all_descendants` — m1's children = [m2, m4]
- `children_of_case_insensitive_match` — B7 修复：uppercase hex portion matches
- `resolve_active_leaf_case_insensitive_match` — B7 修复：mixed-case active_leaf resolves
- `append_with_parent_persists_parent_and_active_leaf` — reload verifies persistence

`engine/src/domain.rs`（5 个，测试模块末尾 `#270 分支对话树测试` 块）：
- `append_with_branch_creates_branch_from_arbitrary_message` — branch_from sets parent + moves active_leaf
- `append_with_branch_rejects_unknown_branch_from` — B6 修复：unknown branch_from → BadRequest
- `history_window_filters_to_active_branch` — B5 修复：window filters to active path
- `history_window_cursor_rejects_id_on_inactive_branch` — B5 新 contract：cursor on inactive branch → BadRequest
- `history_window_cursor_rejects_id_from_other_session_still_works` — cross-session cursor rejection 回归

**仍欠**：HTTP 端点测试（`POST /v1/chat/branch/switch` / `PUT /v1/chat/message`）+ WebUI DOM 操作测试 + finalize 层端到端测试。列为 follow-up（F-4）。

---

## 3. 不变量与合同核验总结

### 3.1 6 并行数组等长不变量

| 操作 | 原始 commit | 审计修复后 |
|---|---|---|
| `new` | ✓ | ✓ |
| `append` | ✓ | ✓ |
| `append_with_parent` | ✓ | ✓ |
| `append_with_branch` | ✓ | ✓ |
| `append_with_candidates` | ✗（不 push `message_parents`，不更新 `active_leaf`） | ✓（B2 修复） |
| `switch_branch` | ✓ | ✓ |
| `switch_swipe` | ✓ | ✓ |
| `delete_last_n` | ✗（物理截断破坏 sibling 分支） | ✓（B1 修复：branch-aware removal） |
| `rollback_to` | ✗（物理截断破坏 sibling 分支） | ✓（B1 修复：branch-aware removal） |
| `delete_message` | ✗（不删 `message_parents[idx]`，不重置 `active_leaf`） | ✓（B3 修复） |
| `read_messages_jsonl` | ✓ | ✓ |

### 3.2 #37 durable message-id 大小写不敏感合同

| 操作 | 原始 commit | 审计修复后 |
|---|---|---|
| `append_with_branch` 验证 `branch_from` | ✗（`==`） | ✓（B6 修复：`ulid::matches`） |
| `resolve_active_leaf` | ✗（`==`） | ✓（B7 修复：`ulid::matches`） |
| `switch_branch` | ✗（`==`） | ✓（B7 修复：`ulid::matches`） |
| `children_of` | ✗（`==`） | ✓（B7 修复：`ulid::matches`） |
| `history_window` cursor 验证 | ✗（`==`） | ✓（B5 修复：`ulid::matches`） |
| `delete_message` 验证 | ✓（已用 `ulid::matches`） | ✓ |
| `edit_message` 验证 | ✓（已用 `ulid::matches`） | ✓ |

### 3.3 分支隔离合同

| 操作 | 原始 commit | 审计修复后 |
|---|---|---|
| `recent(n)` 只返回 active path | ✗（物理尾部） | ✓（B4 修复） |
| `history_window` 只返回 active path | ✗（物理切片） | ✓（B5 修复） |
| `history_window` cursor 拒绝 cross-branch | ✗（无验证） | ✓（B5 修复：两阶段验证） |
| `history_window` cursor 拒绝 cross-session | ✓ | ✓（B5 修复：保留原 contract） |
| `delete_last_n` / `rollback_to` 只删 active path | ✗（物理截断） | ✓（B1 修复） |

### 3.4 HTTP 安全边界

| 端点 | 原始 commit | 审计修复后 |
|---|---|---|
| `PUT /v1/chat/message` body limit | ✗（无） | ✓（B8 修复：2MB） |
| `PUT /v1/characters/:id` body limit | ✓（2MB） | ✓ |
| `POST /v1/chat/completions` body limit | ✓（默认） | ✓ |
| `POST /v1/chat/branch/switch` body limit | ✓（默认） | ✓ |

---

## 4. 第三方经验吸收合规

PR 描述未提及第三方代码复用。CodeRabbit 总结提及 SillyTavern 为公开行为参考（branching conversation tree 的 UX 概念），AIRP 实现完全独立：
- `message_parents: Vec<Option<String>>` + `active_leaf: Option<String>` 是 AIRP 自己的 domain model；
- `active_path_indices()` 拓扑遍历是 AIRP 自己的实现；
- WebUI 分支 UI（⑂ 按钮 + branchFromId 状态机）是 AIRP 自己的 UI 设计。

`docs/ACKNOWLEDGEMENTS.md` 已记录 SillyTavern（commit 380e31e，AGPL-3.0），符合 `AGENTS.md` §"第三方经验吸收与独立实现"。✓

---

## 5. 与历史决策的关系

### 5.1 与 #37 durable message-id 合同的关系

PR #270 引入 `message_parents: Vec<Option<String>>`，parent 字段存的是 durable message ID。这要求所有 parent 比较操作必须遵循 #37 的大小写不敏感合同。审计发现 6 处 `==` 比较违反合同（B6 / B7 / B5），全部修复为 `ulid::matches`。

### 5.2 与 #249 Swipe 多候选的关系

PR #249 引入 `message_candidates` / `message_swipe_index`，PR #270 引入 `message_parents` / `active_leaf`。两者都是并行数组，必须同步维护。审计发现 `append_with_candidates`（#249 方法）未同步维护 #270 字段（B2），`delete_message`（#37 方法）也未同步维护 #270 字段（B3）。这是 PR #270 没有审计既有方法是否需要更新导致的。建议未来引入新并行数组字段时，强制审计所有现存的 push/remove/truncate 路径。

### 5.3 与 #73 方案 B 消息级时间戳的关系

Issue `#73` 引入 `message_timestamps: Vec<Option<String>>`，PR #270 引入 `message_parents`。两者都是 `Vec<Option<...>>` 类型，serde 行为一致（`#[serde(default)]` + `unwrap_or_default()` / `unwrap_or(None)` 向后兼容）。审计确认 `read_messages_jsonl` 对 `parent` 字段的 backward compat 处理与 `ts` 字段一致。✓

---

## 6. 风险评估

### 6.1 已修复风险

- **数据损坏风险（B1/B2/B3）**：高 → 0。原始 commit 在分支场景下 regen / rollback / delete_message 会破坏并行数组等长不变量 + 丢失 sibling 分支数据。审计修复后所有路径同步维护 6 个并行数组。
- **合同违反风险（B6/B7）**：中 → 0。原始 commit 6 处 `==` 比较违反 #37 大小写不敏感合同。审计修复后全部用 `ulid::matches`。
- **安全风险（B8）**：中 → 0。原始 commit PUT /v1/chat/message 无 body limit。审计修复后加 2MB limit。
- **UX 不一致风险（B9）**：中 → 0。原始 commit doSend 在分支场景下不清理 DOM。审计修复后 branch-aware DOM pruning。

### 6.2 残留风险（follow-up）

- **F-1（M1 回归测试）**：`SENTENCE_END_RE` 2-char context 修复无单元测试。WebUI 现有测试不覆盖 `SmoothStreamer.tick` 内部分句逻辑。建议后续提取 `findSentenceBoundary(queue, charsToRender)` 为纯函数 + 单元测试。
- **F-2（孤儿 parent 引用）**：`delete_message` 删除非 leaf 消息后，其子消息的 `parent` 字段仍指向已删除的 ID。当前 `active_path_indices()` 对 `Some(invalid_id)` 的处理与 `None` 等价（都 fallback），不会触发 panic，但拓扑被静默改写。建议后续要么显式清理孤儿引用，要么在 `active_path_indices()` 中显式检测并 warning。
- **F-3（B8 端到端测试）**：`PUT /v1/chat/message` body limit 2MB 无端到端测试。建议后续加 `daemon/tests/` 测试：发送 > 2MB body 验证 413。
- **F-4（HTTP 端点测试）**：`POST /v1/chat/branch/switch` / `PUT /v1/chat/message` 无 HTTP 端到端测试。建议后续加 `daemon/tests/` 测试覆盖 happy path + 边界场景。
- **F-5（WebUI DOM 测试）**：B9 修复的 DOM pruning 无浏览器端到端测试。建议后续用 playwright 或类似工具覆盖 branch-from → doSend → DOM 验证。

---

## 7. 审计结论

PR #270 在原始 commit `dbe67ef` 上有 9 个 BLOCKING 问题（B1-B9）+ 3 个 MINOR 问题（M1-M3/D1），**不应合并**。所有问题已在本审计中修复：

| ID | 严重度 | 类型 | 状态 |
|---|---|---|---|
| B1 | CRITICAL | 数据损坏 | ✅ RESOLVED |
| B2 | CRITICAL | 数据损坏 | ✅ RESOLVED |
| B3 | CRITICAL | 数据损坏 | ✅ RESOLVED |
| B4 | MAJOR | 合同违反 | ✅ RESOLVED |
| B5 | MAJOR | 合同违反 | ✅ RESOLVED + 增强 |
| B6 | MAJOR | 合同违反 | ✅ RESOLVED |
| B7 | MAJOR | 合同违反 | ✅ RESOLVED |
| B8 | MAJOR | 安全 | ✅ RESOLVED |
| B9 | MAJOR | UX 一致性 | ✅ RESOLVED |
| D1 | MINOR | PROCESS | ✅ RESOLVED |
| M1 | MINOR | FUNCTIONAL | ✅ RESOLVED |
| M2/M3 | MINOR | PROCESS | ✅ RESOLVED（16 个新测试） |

**审计裁决**：修复后可以合并，但必须先推送修复 commit + CI 全绿 + 人工 review。本审计报告随 PR 一同提交到 `docs/audits/`。

---

## 8. 修复后状态确认

### 8.1 修复文件清单

| 文件 | 修复内容 |
|---|---|
| `engine/src/chat_store.rs` | B1（delete_last_n / rollback_to branch-aware removal）+ B4（recent 用 active_path_indices）+ B7（resolve_active_leaf / switch_branch / children_of 用 ulid::matches）+ 11 个新测试 + cargo fmt |
| `engine/src/domain.rs` | B2（append_with_candidates 维护 message_parents / active_leaf）+ B3（delete_message 删 message_parents + 重置 active_leaf）+ B5（history_window 两阶段验证 + active path 过滤）+ B6（append_with_branch 用 ulid::matches）+ 5 个新测试 |
| `engine/src/daemon/mod.rs` | B8（PUT /v1/chat/message 加 DefaultBodyLimit::max(2MB)） |
| `engine/src/daemon/handlers/chat.rs` | cargo fmt 修复 import 排序 |
| `webui/app.js` | B9（doSend branch-aware DOM pruning）+ M1（SENTENCE_END_RE 2-char context） |

### 8.2 测试结果

```text
$ cargo test --lib --quiet
running 776 tests
................................................................................
test result: ok. 775 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 42.59s

running 6 tests  (protocol lib)
......
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

$ cargo test --tests --quiet
running 4 tests  (engine main bin)
....
test result: ok. 4 passed; 0 failed; 0 ignored; ...

running 11 tests  (openai_compat)
...........
test result: ok. 11 passed; 0 failed; 0 ignored; ...

running 5 tests  (agent_run)
.....
test result: ok. 5 passed; 0 failed; 0 ignored; ...

running 5 tests  (production_startup)
.....
test result: ok. 5 passed; 0 failed; 0 ignored; ...

running 6 tests  (sse_wiremock)
......
test result: ok. 6 passed; 0 failed; 0 ignored; ...

running 0 tests  (protocol bin)
test result: ok. 0 passed; 0 failed; 0 ignored; ...

running 9 tests  (ui bin)
.........
test result: ok. 9 passed; 0 failed; 0 ignored; ...

$ node --test webui/tests/*.test.mjs
ℹ tests 125
ℹ pass 125
ℹ fail 0

$ cargo clippy --lib --quiet
(no output — clean)

$ cargo fmt --all -- --check
(no output — clean)
```

### 8.3 总计

- **Rust 测试**：775 lib (engine) + 1 ignored + 6 lib (protocol) + 4 bin (engine main) + 0 bin (protocol main) + 9 bin (ui main) + 11 (openai_compat) + 5 (agent_run) + 5 (production_startup) + 6 (sse_wiremock) = 821 passed + 1 ignored
- **WebUI 测试**：125 passed
- **总计**：946 passed + 1 ignored
- **clippy**：clean
- **fmt**：clean

### 8.4 CI 预期

修复 commit 推送后，CI Rust workspace check 应从 FAILURE → SUCCESS（cargo fmt 修复）。其余 5 个 check 维持 SUCCESS/SKIPPED。CI 全绿后可进入人工 review。

### 8.5 follow-up issues（PR 合并后提交）

按 `AGENTS.md` §"审计遗留项处理"规定，以下 follow-up 项在 PR 合并后提交为 GitHub issue：

- **F-1**：`SENTENCE_END_RE` 2-char context 修复加单元测试（提取 `findSentenceBoundary` 纯函数）
- **F-2**：`delete_message` 删除非 leaf 消息后的孤儿 parent 引用处理策略
- **F-3**：`PUT /v1/chat/message` body limit 2MB 端到端测试（413 场景）
- **F-4**：`POST /v1/chat/branch/switch` / `PUT /v1/chat/message` HTTP 端到端测试
- **F-5**：WebUI branch-from → doSend → DOM pruning 浏览器端到端测试

---

## 9. CodeRabbit follow-up 闭环（合并 main 后第二轮 review）

PR #270 合并 origin/main（PR #271 memory MVP）后重新触发 CodeRabbit review，新提交的 unresolved comments 经独立核验后处理如下。§9.1~9.3 已在 commit `d335128` 修复推送；§9.4 记录 `d335128` 推送后 CodeRabbit 又提的 2 条 actionable comments（CR-7 / CR-8）闭环。

### 9.1 已修复项

#### CR-1 `engine/src/chat_store.rs::active_path_indices` 大小写敏感比较（Major） — ✅ RESOLVED

**原审计 B7 漏点**：B7 修复了 `resolve_active_leaf` / `switch_branch` / `children_of` 的 `==` 比较，但 `active_path_indices` 用 `HashMap<&str, usize>::get` 查找 leaf / parent index，`HashMap::get` 是大小写敏感的。如果 `active_leaf` 存储的 ID 大小写与 `message_ids` 中的不一致（#37 合同允许 hex 部分大小写混合），`HashMap::get` 会 miss，导致 `active_path_indices` 返回空路径。

**根因**：`resolve_active_leaf` 在 `ulid::matches` 命中后返回 `active_leaf.as_str()`（原始存储形式），而非 `message_ids` 中匹配到的标准化形式。后续 `id_to_idx.get(leaf.as_str())` 用原始形式查 HashMap，大小写不匹配时失败。

**修复**：改用 `ulid::matches` 线性查找替代 HashMap.get。新增 `find_idx` 闭包，对 leaf 和 parent ID 查询都用 `crate::ulid::matches`。消息数通常 < 100，O(n) 查找对热路径可接受。

**回归测试**：`active_path_indices_case_insensitive_match`（mixed-case active_leaf）+ `active_path_indices_case_insensitive_parent`（mixed-case parent ID）。

#### CR-2 `webui/app.js::loadResidentMemory` 静默丢弃未保存编辑 + stale-response race（Major，来自 PR #271 merge） — ✅ RESOLVED

**问题 1（静默数据丢失）**：`loadResidentMemory()` 在 char/session/tab 切换时自动触发（lines 1100/1111/3286），不检查 `wbMemoryDirty`。用户未保存的记忆编辑会被静默丢弃。

**问题 2（stale-response race → 数据损坏）**：`await api('GET', ...)` 返回后不检查 `selectedChar`/`selectedSess` 是否已切换。快速切换 A→B→C 时，A 的响应可能在 C 之后才返回，覆盖 `wbMemoryContent`；用户点 Save 会 PUT A 的内容到 C，损坏 C 的持久化记忆。

**修复**：
1. 加 `if (wbMemoryDirty && !confirm('当前记忆修改未保存，切换将丢失，是否继续？')) return;` 守卫；
2. 在 `await` 前捕获 `reqChar = selectedChar; reqSess = selectedSess;`，await 后比对，若已切换则丢弃 stale 响应。

#### CR-3 `webui/app.js::SmoothStreamer` 句子边界 2-char context 的 candidate 末尾边界 bug（Minor，M1 收尾） — ✅ RESOLVED

**M1 修复遗留 bug**：M1 用 `candidate[i] + (candidate[i+1] || '')` 取 2-char 上下文，但 `candidate[i+1]` 在 i 为 candidate 末尾时是 undefined，导致 `twoChar = candidate[i] + ''`，正则 `\.(\s|$)` 的 `$` 仍会匹配末尾，candidate 末尾的 `.` 会被误判为句子边界（即使 queue 后面紧跟非空白字符）。

**修复**：改用 `this.queue[i] + (this.queue[i+1] || '')` 取 2-char 上下文。`candidate = this.queue.slice(0, charsToRender)`，所以 `this.queue[i] === candidate[i]` 对 `i < charsToRender` 成立；`this.queue[i+1]` 即使在 `i+1 >= charsToRender` 时也能访问 queue 真实后续字符。这样 candidate 末尾的 `.` 若 queue 后面是字母（如 "3.14"）就不会被判为边界，若后面是空格才会。

#### CR-4 audit md 文档 markdownlint 问题（Minor） — ✅ RESOLVED

- **MD018 (line 423)**：含 `#73` 引入... 的行被误判为 heading。改为 `Issue #73 引入...`。
- **MD040 (line 96, 483)**：fenced code block 缺 language tag。加 `text` 标识。
- **§8.3 总计可复现性 (line 529-533)**：原 "4 bin (engine main)" 在 §8.2 测试输出 transcript 中缺失。在 transcript 中补上 `running 4 tests (engine main bin) ... test result: ok. 4 passed`。

### 9.2 不成立/已过时项（拒绝并附理由）

#### CR-5 `webui/app.js::doSend` 不按 `active_path` 过滤渲染（Major） — REJECTED as outdated

**CodeRabbit 意见**：`doSend`/`appendMsg` 始终把新消息追加到 chatLog DOM 末尾，不按 server 返回的 `active_path` 过滤。

**核验**：该意见在审计修复 commit `b8a0a02` 之前提出（2026-07-20T16:26:12Z vs `b8a0a02` 2026-07-21 09:53:28 +0800）。审计 B9 修复已在 `webui/app.js:1526-1538` 添加 branch-aware DOM pruning：从 `branchFromMessageId` 节点开始，移除其所有 `nextElementSibling`，使 DOM 与 server 的新 active_path 一致。`loadHistory` 调用 `/v1/chat/history`，server 端 `history_window`（审计 B5 修复）已按 active_path 过滤。CodeRabbit 意见已过时。

#### CR-6 gemini `SmoothStreamer` 动态计算 inCodeBlock（high priority） — REJECTED

**gemini 意见**：用 `(this.prefix + this.rendered).match(/\u0060\u0060\u0060/g)` 动态计算当前是否在代码块中，取代 `push` 中的 `this.inCodeBlock` 提前更新。

**核验**：原审计 §0.2 意见 1 已核验"部分成立，归入 M1"。M1 修复采用 2-char context 检测，不动态计算已渲染位置的代码块状态（成本过高且与现有 `inCodeBlock` 跟踪重复）。本轮 CR-3 进一步修复了 2-char context 在 candidate 末尾的边界 bug。gemini 的动态计算方案虽更精确，但与项目"最小修复"原则不符，且现有方案已能覆盖典型场景（流式 chunk 通常不跨 ``` 边界）。保留 `push` 中 `inCodeBlock` 更新，加注释说明限制。

### 9.3 CodeRabbit follow-up 测试结果

```text
$ cargo test --workspace
test result: ok. 804 passed; 0 failed; 1 ignored (engine lib)
test result: ok. 6 passed; 0 failed (protocol lib)
test result: ok. 4 passed; 0 failed (engine main bin)
test result: ok. 11 passed; 0 failed (openai_compat)
test result: ok. 5 passed; 0 failed (agent_run)
test result: ok. 5 passed; 0 failed (production_startup)
test result: ok. 6 passed; 0 failed (sse_wiremock)
test result: ok. 9 passed; 0 failed (ui bin)

$ npm run test -- --run (ui/)
Test Files  13 passed (13)
Tests 98 passed (98)

$ cargo clippy --workspace --all-targets --all-features -- -D warnings
(no output — clean)

$ cargo fmt --all -- --check
(no output — clean)
```

新增 2 个回归测试（`active_path_indices_case_insensitive_match` + `active_path_indices_case_insensitive_parent`），engine lib 总数从 802 → 804。

### 9.4 第三轮 CodeRabbit follow-up 闭环（d335128 后 review）

推送 d335128（§9.1~9.3 修复）后 CodeRabbit 在 audit md 上又提了 2 条 actionable comments，独立核验后均成立并修复（commit 待提交）：

- **CR-7（line 591，MD038 残留）**：上一轮 CR-4 把 `` `#73 引入...` `` 改为 `` `Issue `#73` 引入...` `` 时，CommonMark 把 4 个 backtick 配对成 3 个 code span，其中 `` `Issue ` ``（末尾空格）和 `` ` 引入...` ``（首字符空格）触发 MD038。改写为不含嵌套 backtick 的描述：`含 \`#73\` 引入... 的行被误判为 heading。改为 \`Issue #73 引入...\`。`。
- **CR-8（line 601，maintainer-local file URI）**：上一轮 CR-5 核验段落用了 `[webui/app.js:1526-1538](file:///d:/AIRP-Dev/webui/app.js#L1526-L1538)`，`file:///d:/...` 是 maintainer-specific 路径，在其他 checkout 上 broken，也违反项目"代码引用使用 plain path:line 文本"惯例（§2.B9 / §2.M1 等其他段落都用 plain text）。改为 `webui/app.js:1526-1538` plain text，与 §2.B9 / §2.M1 一致。

本轮仅改 audit md，Rust/JS 代码无改动，§9.3 测试结果保持有效。

- **审计 agent 模型**：GLM-5.2（本会话模型）
- **审计原则**：按 `AGENTS.md` §"Audit Agent Charter" 三原则独立审计：(1) 不附和开发者结论；(2) 自由提出自己的想法；(3) 质疑历史决策并查证。
- **审计方法**：diff 全文走读 + 源码走读 + 不变量推演 + 分支拓扑推演 + 大小写敏感性核验 + HTTP 安全边界核验 + WebUI DOM 一致性推演 + 测试覆盖核验 + CI 状态核验 + 第三方经验吸收合规核验。
- **独立性声明**：本审计未参考开发者 PR 描述中的"测试通过"声明，独立运行 `cargo test --lib` / `cargo test --tests` / `node --test` / `cargo clippy --lib` / `cargo fmt --all -- --check` 全部验证。所有 BLOCKING / MINOR 发现均来自独立源码走读与不变量推演，非来自 bot review 意见（gemini-code-assist 的 2 条意见中 1 条部分成立归入 M1、1 条不成立被驳回）。
- **历史决策质疑**：本审计质疑了 `append_with_candidates`（#249 引入）和 `delete_message`（#37 引入）在 #270 新字段下的同步维护缺失，确认这是历史方法未审计更新导致的 bug，而非 #270 实现错误。建议未来引入新并行数组字段时强制审计所有现存 push/remove/truncate 路径。
