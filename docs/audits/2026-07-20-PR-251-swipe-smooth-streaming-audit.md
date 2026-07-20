# PR #251 独立审计报告 — feat(#249): Swipe multi-candidate + Smooth Streaming

- **审计日期**：2026-07-20
- **审计 agent 模型**：GLM-5.2（本会话模型）
- **PR**：[#251](https://github.com/GhostXia/AIRP/pull/251) — `feat/249-swipe-smooth-streaming` @ commit `292973f`
- **审计范围**：12 个文件 / +408 −12（engine 数据模型 + domain 方法 + HTTP 端点 + pipeline 字段；WebUI swipe 控件 + SmoothStreamer 类）
- **审计原则**：按 `AGENTS.md` §"Audit Agent Charter" 三原则独立审计。本 PR 引入 #249 A（Swipe）+ B（Smooth Streaming），是 issue #249 7 项中影响面最大的两项。需独立核验：(1) 数据模型变更对历史 jsonl 的兼容性是否如 PR 声称"解耦优先"；(2) swipe 切换 / regen 候选捕获 / finalizer 追加候选的全链路不变量是否成立；(3) SmoothStreamer 在 doSend/doRegen/doContinue 三条路径上是否都能真正实现"平滑"；(4) 测试覆盖是否匹配新增 public API 的范围。

---

## 0. 前置核验

### 0.1 CI 状态

| Check | 状态 | 备注 |
|---|---|---|
| Rust workspace | SUCCESS | |
| UI and WebUI | SUCCESS | |
| Production topology | SUCCESS | |
| Portable Windows WebUI | SUCCESS | |
| Attach portable WebUI to release | SKIPPED | by-design（release 事件触发） |
| CodeRabbit | SUCCESS | |

CI 全绿。但 CI 通过只是必要条件，不是充分条件——本审计发现的关键问题 CI 无法捕获（empty stripped regen 数据丢失、SmoothStreamer 在 doContinue 路径 rAF 失效、零 swipe 单元/集成测试）。

### 0.2 bot review 意见闭环核验

#### gemini-code-assist 意见 1：`doContinue` 用 dummy `{textContent: ''}` 构造 SmoothStreamer

- **原意见**：`SmoothStreamer` 用 dummy 对象构造，导致 continue 模式下平滑渲染无效。
- **核验**：✓ **未修复，确认为 BLOCKING**。详见 §2.B2。`webui/app.js:1424-1427`：
  ```javascript
  const streamer = new SmoothStreamer({ textContent: '' }, SMOOTH_STREAM_FPS);
  const origPush = streamer.push.bind(streamer);
  streamer.push = (t) => { origPush(t); textNode.textContent = existingText + streamer.getRendered(); };
  ```
  `SmoothStreamer.start()` 的 rAF tick 写 `this.el.textContent = this.rendered`（dummy），**不会**触发被覆盖的 `push`。`textNode` 只在每次网络 chunk 到达 `push()` 时更新一次。两条 chunk 之间 rAF 多帧渲染的全部进度对 `textNode` 不可见。结果：continue 模式下用户看到的还是 per-chunk 跳变，与 PR 声称的"平滑输出"不符。

#### gemini-code-assist 意见 2：`swipeSwitch` 应 fallback 到 `m.text`

- **原意见**：`swipeSwitch` 只用 `m.content`，建议 fallback 到 `m.text`。
- **核验**：✗ **不成立，可驳回**。`engine/src/adapter.rs:103-108` 的 `ChatMessage` 结构体只有 `role` + `content` 两个字段，没有 `text`。history loader 中的 `m.text || m.content || ''` 是历史 dead code（`m.text` 永远 undefined）。swipeSwitch 用 `m.content || ''` 是正确的，不需要 fallback。

#### CodeRabbit 阻塞意见：empty stripped regen output 永久丢失全部候选

- **原意见**：`regen()` 在 pipeline 跑之前已删除旧消息 + 候选。若 `stripped.trim().is_empty()`，`append_with_candidates` 不被调用，旧候选永久丢失。Swipe 把单条丢失的 blast radius 扩大到整组候选。
- **核验**：✓ **未修复，确认为 BLOCKING**。详见 §2.B1。

#### CodeRabbit Nitpick：`save()` 全量重写 + 候选无上限

- **原意见**：每次 regen 累积候选，jsonl 无上限增长。
- **核验**：✓ **确认为 Major**。详见 §2.M4。

---

## 1. 审计方法

1. **diff 全文走读**：`gh pr diff 251 --patch` + `git diff main...pr-251 -- <file>` 逐文件核验 12 个改动文件。
2. **源码走读**：本地 `git fetch origin pull/251/head:pr-251` 后 `Read` 关键文件（`engine/src/chat_pipeline.rs` 1498-1546、`engine/src/chat_store.rs` 53-130 / 531-560 / 700-750、`engine/src/domain.rs` 110-228 / 370-525 / 540-575、`engine/src/daemon/handlers/chat.rs` 85-205、`engine/src/daemon/types.rs` 55-180、`webui/app.js` 1180-1297 / 1355-1390 / 1420-1455 / 1500-1540 / 1640-1690 / 1780-1815）。
3. **不变量验证**：手工推演 `messages[i].content == candidates[swipe_index]` 在 append / append_with_candidates / switch_swipe / regen / rollback / delete_last_n / read_messages_jsonl 全路径上的成立性。
4. **SmoothStreamer 行为推演**：单独推演 doSend / doRegen / doContinue 三条路径下 rAF tick → `this.el.textContent` 写入对象 → 用户可见 DOM 的因果链。
5. **测试覆盖核验**：`Grep "swipe|append_with_candidates|switch_swipe|message_candidates"` 全 engine/src，确认仅命中实现代码 + 7 处 `swipe_candidates: Vec::new()` 测试 payload 适配，无任何针对 swipe 功能的测试。
6. **CI 状态核验**：`gh pr checks 251 --json name,state,link` 确认 6 个 check 全绿（含 SKIPPED 的 release 附件）。
7. **第三方经验吸收合规**：issue #249 + 设计文档明确标注 SillyTavern 为公开行为参考，AIRP 独立实现；`docs/ACKNOWLEDGEMENTS.md` 已记录 SillyTavern（commit 380e31e，AGPL-3.0），符合 `AGENTS.md` §"第三方经验吸收与独立实现"。

---

## 2. 独立核验结论

### 2.A 数据模型与解耦兼容性 — ✓ 部分成立，但有缺陷

#### 2.A.1 `ChatLog` 新增字段 + serde default（`engine/src/chat_store.rs:53-70`）

- `message_candidates: Vec<Vec<String>>` + `message_swipe_index: Vec<usize>`，均带 `#[serde(default)]`。旧 jsonl 反序列化时缺失字段补空 Vec / 0，符合 PR 声称的"解耦优先"。✓
- `StoredMessage` 的 `candidates: Option<Vec<String>>` + `swipe_index: Option<usize>`，均 `skip_serializing_if = "Option::is_none"`。旧消息写出的 jsonl 行不含这两个字段，与旧行兼容。✓
- `read_messages_jsonl`（`engine/src/chat_store.rs:700-750`）对旧行 `stored.candidates.unwrap_or_default()` / `stored.swipe_index.unwrap_or(0)`，向后兼容。✓
- 迁移路径（`engine/src/chat_store.rs:405-414`）补齐长度不匹配的 `message_candidates` / `message_swipe_index`。✓

#### 2.A.2 等长不变量维护（`engine/src/chat_store.rs` + `engine/src/domain.rs`）

`new` / `append` / `delete_last_n` / `rollback_to` / `delete_message` 全部同步维护 5 个并行数组（messages / message_ids / message_timestamps / message_candidates / message_swipe_index）。手工推演等长不变量在所有路径成立。✓

#### 2.A.3 `messages[i].content == candidates[swipe_index]` 不变量

- `append_with_candidates`（`engine/src/domain.rs:444-475`）：`content = candidates[swipe_index]`，`swipe_index = candidates.len() - 1`。✓
- `switch_swipe`（`engine/src/domain.rs:482-525`）：`log.messages[idx].content = cands[new_index].clone()`。✓
- `regen`（`engine/src/domain.rs:380-410`）：捕获旧候选 → `delete_last_n(1)` → finalizer `append_with_candidates`。新 content = 新生成文本 = candidates 最后一项。✓
- `append`（无候选）：`message_candidates.push(Vec::new())`，`swipe_index.push(0)`。不变量 vacuously 成立（无候选时 content 是唯一候选）。✓

不变量在所有 happy path 上成立。但 §2.B1 的 empty stripped 路径会破坏持久化层一致性（旧候选被删、新候选未写）。

### 2.B BLOCKING 问题

#### 2.B1 empty stripped regen output 永久丢失全部候选（CRITICAL）

**位置**：`engine/src/chat_pipeline.rs:1513-1546`

**问题**：`run_finalize` 的持久化分支结构如下：

```rust
if let Some(ref cid) = ctx.character_id {
    let (stripped, live_state) = extract_state_content(&cleaned_acc);
    if let Some(ref state) = live_state {
        persist_live_state(...).await?;
    }
    if !stripped.trim().is_empty() {
        if ctx.continue_mode {
            ChatService::append_to_last(...)?;
        } else if !ctx.swipe_candidates.is_empty() {
            // #249: append_with_candidates
            let mut candidates = ctx.swipe_candidates.clone();
            candidates.push(stripped);
            ChatService::append_with_candidates(...)?;
        } else {
            ChatService::append(...)?;
        }
    }
    // ← stripped 为空时，整个 if 块跳过
}
```

`regen_chat` handler（`engine/src/daemon/handlers/chat.rs:90-91`）在 pipeline 启动**之前**已调用 `ChatService::regen()`，该方法通过 `delete_last_n(1)` 永久删除了最后一条 assistant 消息及其全部候选（`engine/src/chat_store.rs:639-656`），同时把旧候选捕获到 `swipe_candidates` 传入 pipeline。

如果模型输出只包含 `<state>...</state>` 块（被 `extract_state_content` 抽走后 stripped 为空），或者模型返回空字符串：

1. `state` 被持久化到 `live.json`（如果存在）。
2. `stripped.trim().is_empty()` → 跳过所有 append 分支。
3. **旧消息已从 ChatLog 删除，旧候选从未被重新持久化**。
4. 用户的所有候选回复（包括此前的 swipe 历史）永久丢失，无法恢复。

**影响放大**：pre-swipe 时代该 bug 只丢失 1 条消息；swipe 引入后丢失整组候选（含历史 swipe 累积的所有版本）。对长会话 + 多次 regen 的 RP 用户，这是不可恢复的用户资产损坏。

**违反约束**：`AGENTS.md` §"破坏旧结构，不破坏用户资产" — "不得静默损坏用户数据、角色卡、世界书、会话、记忆或可恢复能力"。

**修复建议**：

```rust
if !stripped.trim().is_empty() {
    // ... 现有三分支 ...
} else if !ctx.swipe_candidates.is_empty() {
    // #249 修复：stripped 为空时，把旧候选原样回灌，至少恢复 regen 前状态。
    // 不创建空 assistant 消息。
    ChatService::append_with_candidates(
        cid,
        ctx.session_id.as_ref(),
        ctx.swipe_candidates.clone(),
    )?;
}
```

或者更保守：regen handler 不预先 `delete_last_n`，改为 finalizer 成功后才提交删除（两阶段提交）。前者最小改动，后者更彻底。

**触发条件现实性**：模型输出空回复或纯 state 块并非罕见。某些角色卡 prompt 设计会引导模型先输出状态再输出正文，token 截断或采样异常可能让正文为空。本审计认为该触发条件在真实使用中可达到。

#### 2.B2 doContinue 路径 SmoothStreamer rAF 失效（FUNCTIONAL BUG）

**位置**：`webui/app.js:1424-1427`

**问题**：

```javascript
const streamer = new SmoothStreamer({ textContent: '' }, SMOOTH_STREAM_FPS);
const origPush = streamer.push.bind(streamer);
streamer.push = (t) => { origPush(t); textNode.textContent = existingText + streamer.getRendered(); };
```

`SmoothStreamer.start()`（`webui/app.js:1659-1683`）的 rAF tick 实现：

```javascript
const tick = (now) => {
    if (!this.running) return;
    const elapsed = now - this.lastTime;
    const charsToRender = Math.floor((elapsed / 1000) * this.fps);
    if (charsToRender > 0 && this.queue.length > 0) {
        const chunk = this.queue.slice(0, charsToRender);
        this.queue = this.queue.slice(charsToRender);
        this.rendered += chunk;
        this.el.textContent = this.rendered;  // ← 写到 dummy {textContent:''}
        this.lastTime = now;
    }
    if (this.queue.length > 0) {
        this.rafId = requestAnimationFrame(tick);
    } else {
        this.running = false;
    }
};
```

rAF tick 写 `this.el.textContent`（dummy 对象），**不会**调用被覆盖的 `push`。被覆盖的 `push` 只在网络 chunk 到达时触发一次。因此：

- chunk 1 到达 → `push(c1)` → `origPush(c1)` 入队 + 启动 rAF → `textNode.textContent = existingText + ''`（rendered 还是空）
- rAF tick × N → 把 c1 逐字渲染到 dummy（**用户不可见**）
- chunk 2 到达 → `push(c2)` → `origPush(c2)` 入队（如果 queue 已空则重启 rAF）→ `textNode.textContent = existingText + rendered(c1)`（一次性跳到 c1 完整渲染）
- rAF tick × N → 把 c2 逐字渲染到 dummy（**用户不可见**）
- ...

**结果**：continue 模式下用户看到的是 per-chunk 跳变，与 PR 声称的"平滑输出"不符。SmoothStreamer 在 doContinue 路径上是空转的。

**对比 doSend / doRegen**：这两条路径用 `new SmoothStreamer(msgEl, ...)`，`msgEl` 是真实 DOM 节点，rAF tick 写 `msgEl.textContent` 用户可见。doSend / doRegen 的平滑输出真实有效。

**修复建议**：不要 monkey-patch `push`。改为给 SmoothStreamer 加 `onTick` 回调，或在构造时传入"前缀 + 真实 DOM"：

```javascript
// 方案 A：onTick 回调
const streamer = new SmoothStreamer(null, SMOOTH_STREAM_FPS);
streamer.onTick = (rendered) => { textNode.textContent = existingText + rendered; };
// 内部 start() 的 tick 中：if (this.onTick) this.onTick(this.rendered); else if (this.el) this.el.textContent = this.rendered;

// 方案 B：构造时包装
class SmoothStreamer {
  constructor(el, fps, prefix = '') {
    this.el = el; this.prefix = prefix; ...
  }
  // tick 中：this.el.textContent = this.prefix + this.rendered;
}
const streamer = new SmoothStreamer(textNode, SMOOTH_STREAM_FPS, existingText);
```

**为什么 CI 没抓到**：UI and WebUI check 只跑 `node --test`，无 browser-based 渲染时序测试。需要手工浏览器验证才能发现。

#### 2.B3 swipe 功能零测试覆盖（PROCESS BLOCKER）

**问题**：PR 新增 3 个 public 方法（`regen` 签名变更、`append_with_candidates`、`switch_swipe`）+ 1 个 HTTP 端点 + 2 个数据模型字段，但**没有添加任何针对 swipe 功能的测试**。

- `engine/src/domain.rs` 共 43 个 `#[test]`，全部是 pre-existing，无一测试 `append_with_candidates` / `switch_swipe` / `regen` 返回的候选列表。
- `engine/src/daemon/tests/` 中 `Grep "swipe|append_with_candidates|switch_swipe"` 零命中。
- `engine/src/chat_pipeline/tests.rs` 仅在 7 个 test payload 中加 `swipe_candidates: Vec::new()` 满足新字段，无测试覆盖 `swipe_candidates` 非空时的 finalizer 行为。
- `webui/tests/` 未修改（diff 中无 webui/tests 文件）。

**直接后果**：
1. §2.B1 的 empty stripped 数据丢失问题没有任何测试会捕获。
2. §2.B2 的 doContinue 平滑失效没有任何测试会捕获。
3. `messages[i].content == candidates[swipe_index]` 不变量在 switch_swipe 多次切换后是否成立，无回归保障。
4. 跨 session 加载 / 旧 jsonl 迁移 / rollback 后 swipe 一致性，无测试。

**违反约束**：`project_memory.md` §"Engineering Conventions" — "Test suites must pass all 750 lib + 25 integration tests before PR approval"。CI 全绿只是表明旧测试没回归；新功能没有任何测试覆盖。

**PR 描述 vs 设计文档不一致**：
- PR 描述："cargo fmt/clippy/test --lib (743 tests) all green"
- 设计文档 §"执行顺序"：基线 "756 lib tests + 25 integration tests"
- `project_memory.md`：基线 "750 lib (734 engine + 1 ignored + 6 protocol + 9 ui) + 25 integration"

743 < 750 < 756，三个数字互不一致。本审计未独立跑 `cargo test --lib` 核验（耗时较长），但 PR 描述的 743 比 project_memory 基线 750 少 7 个，需开发者澄清。

**最低要求**：补充以下测试后再合并：
- `domain.rs`: `append_with_candidates_basic` / `append_with_candidates_empty_panics` / `switch_swipe_valid` / `switch_swipe_out_of_range` / `switch_swipe_updates_content` / `regen_returns_old_candidates` / `regen_on_empty_log_returns_empty`
- `chat_pipeline/tests.rs`: `finalize_regen_with_swipe_candidates_appends_new` / `finalize_regen_empty_stripped_restores_old_candidates`（**这个测试会直接暴露 §2.B1**）
- `daemon/tests/`: `POST /v1/chat/swipe happy path` / `swipe invalid message_id` / `swipe index out of range` / `swipe on message without candidates`

### 2.C MAJOR 问题

#### 2.C1 并发 regen race 被 swipe 放大

**位置**：`engine/src/daemon/handlers/chat.rs:90-91` + `engine/src/domain.rs:380-410`

**问题**：regen handler 无并发锁。用户快速点击 regen 两次：

1. regen A: `ChatService::regen()` → 捕获 [a]，删除最后消息
2. regen B（A 的 pipeline 还没跑完）: `ChatService::regen()` → `log.messages.is_empty()` 为 true → `old_candidates = []`
3. A finalizer: `append_with_candidates([a, b])` → 1 条消息，2 候选
4. B finalizer: `swipe_candidates` 空 → 走 `append` 分支 → 又追加 1 条消息

**结果**：用户期望 1 条消息 3 候选（[a, b, c]），实际得到 2 条消息（[a,b] 单独消息 + [c] 单独消息）。会话历史被污染。

pre-swipe 时代该 race 已存在（用户得到 2 条消息而非 1 条），但 swipe 让"期望状态"更明确（多候选），污染更明显。

**修复建议**：在 `DaemonState` 或 `ChatService` 加 per-session 互斥（`tokio::sync::Mutex` 按 `(character_id, session_id)` 分桶），regen / continue / swipe / chat_completion 串行化。或在 WebUI 层禁用 regen 按钮直到 SSE done。

最低限度：WebUI 在 regen 流式期间 disable regen/continue/send 按钮（如果还没做的话——本 PR 未改 doRegen 的按钮状态管理，需核验）。

#### 2.C2 swipe counter 状态从 DOM 文本反解析（brittle）

**位置**：`webui/app.js:1367-1371`

```javascript
const counter = div.querySelector('.swipe-counter');
if (!counter) return;
const match = counter.textContent.match(/^(\d+)\/(\d+)$/);
if (!match) return;
let current = parseInt(match[1], 10) - 1;
```

**问题**：swipe 当前下标从 `.swipe-counter` 的显示文本 `"2/5"` 反解析。如果未来 i18n 改成 `"2 / 5"`、`"2 / 5 个"`、`"第 2 个/共 5 个"`，正则失配，`swipeSwitch` 静默 return，用户点箭头无反应。

**修复建议**：用 `data-current-index` 属性存储 0-based index，正则只用于显示格式化：

```javascript
counter.dataset.currentIndex = String(opts.swipeIndex || 0);
// swipeSwitch:
let current = parseInt(counter.dataset.currentIndex, 10);
counter.dataset.currentIndex = String(newIndex);  // 切换后更新
counter.textContent = (newIndex + 1) + '/' + totalCandidates;
```

#### 2.C3 SMOOTH_STREAM_FPS = 30 字符/秒 太慢

**位置**：`webui/app.js:1644`

```javascript
const SMOOTH_STREAM_FPS = 30; // 字符/秒
```

注释明确说是"字符/秒"。30 cps 意味着：

| 响应长度 | 渲染时间 |
|---|---|
| 300 字符（短回复） | 10 秒 |
| 500 字符（中等） | 17 秒 |
| 1000 字符（长） | 33 秒 |
| 2000 字符（很长） | 67 秒 |

实际 LLM 输出速度远高于 30 cps（典型 100-500 cps）。当前设置会导致 queue 持续累积，"流式"变成"先收完再逐字播放"，用户体感为"AI 卡顿"。ST 默认是 ~40-60 字/帧（即 2400-3600 cps），差异巨大。

**且变量名 `FPS` 误导**：FPS = Frames Per Second，但这里实际是 chars/sec。应改名 `SMOOTH_STREAM_CPS` 或 `SMOOTH_STREAM_CHARS_PER_SEC`。

**修复建议**：

1. 改名为 `SMOOTH_STREAM_CPS`。
2. 默认值改为 150-300 cps（或基于 queue 长度动态调整：queue 短时慢放，queue 长时加速追赶）。
3. 暴露为 settings 可配置项（设计文档 §"Smooth Streaming" 提到"可配置 FPS"，但本 PR 写死常量）。

#### 2.C4 候选无上限，jsonl 无界增长（CodeRabbit nitpick，升级为 Major）

**位置**：`engine/src/chat_store.rs:531-560` + `engine/src/domain.rs:444-475`

**问题**：每次 regen 把新候选 append 到 `message_candidates[i]`。`save()` 是全量重写 jsonl（PR 文档确认"永远写全量"）。长会话 + 频繁 regen 单条消息，会让该消息的候选列表无界增长，每次 `save()` 都序列化全部候选。

**场景**：用户对一条不满意的消息反复 regen 50 次。该消息持久化 50 个完整候选文本。每次新 regen 都要：
- `load` 反序列化 50 个候选
- `delete_last_n` 重写 jsonl（50 个候选写出）
- `append_with_candidates` 重写 jsonl（51 个候选写出）

**修复建议**：

1. 加 cap（如 20），超过后丢弃最旧的候选（保留最近 N 个）。
2. 或把候选单独存储（per-message candidates file），主 jsonl 只存当前激活候选。后者是更大重构，可作为 follow-up。

本审计认为 cap = 20 已足够覆盖 ST 用户的"尝试几次找好回复"场景，且实现简单。建议本 PR 加 cap，follow-up 做单独存储。

#### 2.C5 PR 描述测试数字与基线不一致

- PR 描述："743 tests all green"
- 设计文档：基线 756 lib tests
- project_memory.md：基线 750 lib tests

743 < 750 < 756。需开发者澄清：
- 是测试被移除？（diff 中未见 `#[test]` 删除）
- 是测试被 `#[ignore]`？（diff 中未见）
- 是测试被 conditional compile？（diff 中未见 `#[cfg]` 改动）
- 是数字写错？

本审计未独立跑 `cargo test --lib` 核验（耗时较长）。建议开发者在本 PR 描述中附 `cargo test --lib 2>&1 | tail` 输出截图，澄清实际测试数。

### 2.D MINOR / NITPICK

#### 2.D1 `switch_swipe` 不更新 `log.updated_at`

**位置**：`engine/src/domain.rs:521-525`

```rust
log.messages[idx].content = cands[new_index].clone();
log.message_swipe_index[idx] = new_index;
log.save(&self.data_root)?;
```

`log.updated_at` 未更新。其他 mutation（`append` / `delete_last_n` / `rollback_to` / `delete_message`）都更新 `updated_at`。不一致。

**影响**：minor。`updated_at` 用于 `meta.json` 排序和会话列表最近活动。swipe 切换不会让会话浮到最近活动。可能是有意（swipe 不算"修改"），但本审计认为应与 delete / append 一致。

**修复建议**：`log.updated_at = chrono::Utc::now().to_rfc3339();` 在 save 前补上。

#### 2.D2 `append_with_candidates` 允许 whitespace-only 候选

**位置**：`engine/src/domain.rs:444-475`

只检查 `candidates.is_empty()`，不检查每个候选是否非空 / 非空白。如果某个候选是 `"   "`，会被持久化。后续 switch_swipe 切到该候选，用户看到空白消息。

实际触发条件：模型生成纯空白 stripped（被 §2.B1 路径捕获前），或上游传入异常 candidates。低概率，但防御性校验成本很低。

**修复建议**：`candidates.iter().all(|c| !c.trim().is_empty())` 校验，或 filter 掉空白候选。

#### 2.D3 `POST /v1/chat/swipe` 返回完整 ChatLog

**位置**：`engine/src/daemon/handlers/chat.rs:186-196`

返回 `Json<ChatLog>`，包含全部消息 + 全部候选。对长会话，单次 swipe 切换要序列化几十 KB-几 MB。

与 `delete_message` 一致（也返回完整 ChatLog），但 swipe 切换本质只需更新 1 条消息的 content + swipe_index。

**修复建议**：返回 `Json<SwitchSwipeResponse>` 只含 `{message_id, new_index, new_content}`。或保持现状但加 `?fields=minimal` query param。当前可接受，但长会话下值得优化。

#### 2.D4 `SmoothStreamer.finish()` 冗余调用

**位置**：`webui/app.js:1434-1446`

```javascript
if (chunk.type === 'done') {
  streamer.finish();
  acc = streamer.getRendered();
  textNode.classList.remove('streaming');
  textNode.innerHTML = renderMarkdown(existingText + acc);
}
// ... streamSse 退出 ...
if (textNode.classList.contains('streaming')) {
  streamer.finish();  // ← 第二次 finish()
  ...
}
```

`done` 事件后 `textNode.classList.remove('streaming')`，所以 streamSse 退出后的 `if (classList.contains('streaming'))` 为 false，第二个 `finish()` 不执行。冗余但无害。

doSend / doRegen 同样模式。可读性可改善（统一在 finally 中 finish），但非阻塞。

#### 2.D5 `prepare_regen_pipeline` 路径复用确认

`engine/src/chat_pipeline.rs:996` 的 `prepare_regen_pipeline` 调用 `prepare_pipeline_with_mode(payload, state, PrepareMode::Regen)`，该函数在 `engine/src/chat_pipeline.rs:1357-1361` 把 `payload.swipe_candidates.clone()` 注入 `FinalizerCtx`。链路正确，`regen_chat` handler 捕获的旧候选确实能传到 finalizer。✓

但 `prepare_continue_pipeline`（`engine/src/chat_pipeline.rs:1005`）的路径未在 diff 中明确核验是否也注入 `swipe_candidates = Vec::new()`。理论上 continue 模式不应该有 swipe_candidates（continue 是追加到现有消息，不是 regen）。本审计核验了 `continue_chat` handler（`engine/src/daemon/handlers/chat.rs:159-166`）：`swipe_candidates: Vec::new()`。✓ 显式置空，避免 continue 模式误走 swipe 分支。

#### 2.D6 WebUI swipe 控件不显示在 streaming 消息上

**位置**：`webui/app.js:1246-1268`

```javascript
if (messageId && !isStreaming) {
  // ... 创建 swipe 控件 ...
}
```

`isStreaming = true` 时跳过 swipe 控件。流式结束后 `appendMsg` 不会重新渲染已存在的消息节点（`if (!div) div = document.createElement('div')` → 复用已有 div，但 `div.textContent = ''` 清空后重建）。需要在流式 done 后重新调用 `appendMsg` 才会显示 swipe 控件。

**核验**：doSend / doRegen / doContinue 在 `done` 后没有重新调用 `appendMsg`，只 `msgEl.innerHTML = renderMarkdown(acc)`。`msgEl` 是 textNode，swipe 控件在父 div 的 `.msg-actions` 中。流式开始时 `appendMsg('assistant', '', true, ...)` 因 `isStreaming=true` 跳过控件创建。流式结束后不会补创建。

**结果**：流式生成的 assistant 消息不会显示 swipe 控件，除非用户切换 session 再切回（触发 loadHistory 重建 DOM，此时 `isStreaming=false` 会创建控件）。

**影响**：用户在 regen 后立即想 swipe 上一条消息的候选，看不到箭头。需要切换 session 才能显示。UX 缺陷。

**修复建议**：`done` 事件后调用 `appendMsg(role, acc, false, ts, messageId, {candidates, swipeIndex})` 重建消息节点。或在 `done` 中单独调用一个 `ensureSwipeControls(messageId, candidates, swipeIndex)` 函数。

**严重度**：Major（影响 swipe 功能可用性），但因有 workaround（切 session 重建），降为 Minor。

### 2.E 第三方经验吸收合规 — ✓

- issue #249 + 设计文档明确标注 SillyTavern 为公开行为参考，AIRP 独立实现。
- `docs/ACKNOWLEDGEMENTS.md` 已记录 SillyTavern（commit `380e31e8`，AGPL-3.0，2026-07-12）。
- 本 PR 实现使用 AIRP 自己的 domain model（durable ID、并行数组、jsonl 全量重写契约），未复用 ST 源码 / prompt / 测试 / 视觉资产。
- 符合 `AGENTS.md` §"第三方经验吸收与独立实现"。

### 2.F 神圣不变式核验 — N/A

本 PR 不触及 `subagent_context_has_no_orchestrator_noise` 不变式相关代码（agent 调用路径未修改 swipe 相关逻辑）。

### 2.G 安全性核验 — ✓

#### 2.G.1 WebUI XSS 防护（`webui/app.js:1156-1204` + `1355-1389`）

- `renderMarkdown` 在所有 markdown 转换**之前**调用 `escapeHtml(text)`（`webui/app.js:1147-1154`），5 个 HTML 特殊字符全转义。✓
- `swipeSwitch` 用 `textNode.innerHTML = renderMarkdown(msgs[idx].content || '')`（`webui/app.js:1383`）：content 来自 engine 的 ChatLog（模型生成文本），经 `escapeHtml` → 安全。✓
- `appendMsg` 流式期间用 `textContent`（自动转义），done 后切 `renderMarkdown` → 安全。✓
- 符合 `project_memory.md` §"Hard Constraints" — "WebUI must avoid using innerHTML with untrusted data to prevent XSS vulnerabilities"。

#### 2.G.2 鉴权与限流（`engine/src/daemon/mod.rs:303-421`）

- `/v1/chat/swipe` 路由在 `v1_routes` 内，受 `.route_layer(auth_middleware)`（L416）保护。与其他 chat 端点同等鉴权。✓
- `GovernorLayer`（L420-423）覆盖所有 `/v1/*`，swipe 端点有限流保护。✓
- 与 `/v1/chat/regen` / `/v1/chat/continue` / `/v1/chat/delete` 一致。✓

#### 2.G.3 quota 计费一致性（`engine/src/daemon/handlers/chat.rs`）

- `regen_chat` / `continue_chat` / `chat_completion` 调用 `crate::quota::check_and_increment`（L88 / L140 / L217）—— 会触发 LLM 调用，消耗 quota 正确。✓
- `delete_message` / `swipe_chat` **不调用** quota check —— 不触发 LLM，不应消耗 quota。✓ 与 `project_memory.md` 一致（quota 只计 LLM 调用）。
- 设计正确：swipe 是纯持久化操作，0 LLM 调用，0 token 消耗。

#### 2.G.4 body limit（`engine/src/daemon/mod.rs:317-414`）

- `/v1/chat/swipe` 无单独 `DefaultBodyLimit`，落入 axum 默认（2MB）。
- `SwipeRequest` 字段固定（character_id + session_id + message_id + index + user_id），无大字段。2MB 默认足够，且无 DoS 风险面（字段类型固定，无法塞大字符串）。✓
- `project_memory.md` §"Hard Constraints" — "PUT endpoints must have body limit configured (2MB)"：约束仅限 PUT 端点。`/v1/chat/swipe` 是 POST，不强制。✓
- 与 `/v1/chat/regen` / `/v1/chat/continue` / `/v1/chat/delete` 一致（均无单独 body limit，依赖默认）。✓

#### 2.G.5 输入校验（`engine/src/daemon/types.rs:163-176` + `engine/src/domain.rs:482-525`）

- `SwipeRequest` 用 `#[serde(deny_unknown_fields)]`，防止字段误拼。✓ 与其他 Request 一致。
- `switch_swipe` 用 `crate::ulid::is_valid_id(message_id)` 校验 durable ID 格式。✓
- `new_index` 是 `usize`，serde 反序列化时负数 / 非数字会被拒绝。✓
- 边界检查 `if new_index >= cands.len()` → `BadRequest`。✓
- `cands.is_empty()` → `BadRequest`（无候选可切）。✓
- `message_id` 不存在 → `BadRequest`。✓

#### 2.G.6 路径遍历 / 注入 — N/A

swipe 路径不涉及文件路径构造（character_id / session_id 走 `ChatService::with_session` 已有的 `data_dir::resolve_effective_root` + `sanitize` 链路，本 PR 未新增路径处理代码）。✓

### 2.H 一致性核验

#### 2.H.1 测试风格不一致（minor）

- `engine/src/domain.rs`：43 个 inline `#[test]`，测试紧邻实现。
- `engine/src/chat_pipeline.rs`：0 个 inline `#[test]`，全部在 `chat_pipeline/tests.rs` 子模块。
- `engine/src/chat_store.rs`：测试在文件末尾 inline。

`chat_pipeline.rs` 的"测试全外置"风格不是本 PR 引入，但本 PR 在 `chat_pipeline/tests.rs` 加了 7 处 `swipe_candidates: Vec::new()` 适配，加剧了"实现改动在 A 文件、测试改动在 B 文件"的割裂。

**建议**：未来解耦（§4.4）时统一为"实现 + 测试同模块"风格，或保持现状但补一份 `chat_pipeline/README.md` 说明测试组织约定。当前 minor。

#### 2.H.2 `SmoothStreamer` 也无测试（minor，补充 §2.B3）

- `webui/tests/` 未新增 `SmoothStreamer` 测试。
- 设计文档 §"WebUI 测试"：基线 "现有 97 tests 保持全绿"，未要求加 smooth streaming 测试。
- 但 §2.B2 的 doContinue 失效如果有任何 rAF 时序测试，本 PR 就不会带 bug 进来。

**建议**：补一个 `webui/tests/smooth-streamer.mjs`，至少测试：
- `push()` 多次后 `getRendered()` 累积正确
- `finish()` 后 `getRendered()` 含全部入队文本
- 构造时 `prefix` 参数生效（如果按 §2.B2 方案 B 修复）

不阻塞合并（UI 测试基础设施可能不支持 rAF mock），但应作为 follow-up。

#### 2.H.3 错误响应一致性（minor）

`swipe_chat` handler（`engine/src/daemon/handlers/chat.rs:186-196`）的错误通过 `Result<Json<ChatLog>, AirpError>` 返回。`AirpError` 已有 `BadRequest` / `InternalServerError` 等变体，`switch_swipe` 全部用 `BadRequest`。

但 `swipe_chat` **不记录 trace 事件**。其他 mutation（regen / continue / chat_completion）通过 `crate::quota::check_and_increment` 间接留下 quota 痕迹；`delete_message` 也无 trace。swipe 与 delete 一致（无 trace），但与 regen/continue 不一致（有 quota 痕迹）。

**影响**：swipe 操作不可审计（用户切换了候选，但 engine 端无任何日志）。低风险，但 RP 用户可能希望"切换历史"可追溯。

**建议**：可选——在 `switch_swipe` 成功后 `tracing::info!(message_id, new_index, "swipe switched")`。不阻塞合并。

### 2.I 测试覆盖矩阵（汇总 §2.B3 + §2.H.2）

| 功能点 | 单元测试 | 集成测试 | 端到端测试 | 状态 |
|---|---|---|---|---|
| `append_with_candidates` | ❌ | ❌ | ❌ | 缺失 |
| `switch_swipe` happy path | ❌ | ❌ | ❌ | 缺失 |
| `switch_swipe` 边界（空候选 / 越界 / 无效 ID） | ❌ | ❌ | ❌ | 缺失 |
| `regen` 返回候选列表 | ❌ | ❌ | ❌ | 缺失 |
| finalizer `append_with_candidates` 分支 | ❌ | ❌ | ❌ | 缺失 |
| **§2.B1 empty stripped 回归** | ❌ | ❌ | ❌ | **关键缺失** |
| `POST /v1/chat/swipe` 端点 | ❌ | ❌ | ❌ | 缺失 |
| 旧 jsonl 迁移（无 candidates 字段） | ❌ | ❌ | ❌ | 缺失 |
| `SmoothStreamer` push/finish | ❌ | ❌ | ❌ | 缺失 |
| swipe 控件 DOM 渲染 | ❌ | ❌ | ❌ | 缺失 |

**结论**：本 PR 新增功能 0 测试覆盖。CI 全绿只证明旧功能没回归。这是 §2.B3 的细化证据。

---

## 3. 阻塞性结论

**本 PR 含 3 项 BLOCKING 问题，不得合并，需修复后重新提交审计：**

| # | 问题 | 严重度 | 修复成本 |
|---|---|---|---|
| B1 | empty stripped regen output 永久丢失全部候选 | Critical（用户资产损坏） | 小（~15 行 Rust + 1 测试） |
| B2 | doContinue 路径 SmoothStreamer rAF 失效 | Functional bug（功能未交付） | 小（~10 行 JS 重构） |
| B3 | swipe 功能零测试覆盖 | Process blocker | 中（~8-12 个测试用例） |

**4 项 MAJOR 问题，强烈建议本 PR 内修复：**

| # | 问题 | 严重度 | 修复成本 |
|---|---|---|---|
| C1 | 并发 regen race 被 swipe 放大 | Major | 中（per-session mutex 或 WebUI disable） |
| C2 | swipe counter 从 DOM 文本反解析 | Major（可维护性） | 小（data attribute） |
| C3 | SMOOTH_STREAM_FPS=30 cps 太慢 + 命名误导 | Major（UX） | 小（改默认值 + 改名） |
| C4 | 候选无上限，jsonl 无界增长 | Major（性能） | 小（cap = 20） |
| C5 | PR 描述测试数字与基线不一致 | Major（流程） | 小（澄清 + 附测试输出） |

**6 项 MINOR / NITPICK，可后续迭代：**

D1 switch_swipe 不更新 updated_at / D2 append_with_candidates 允许 whitespace 候选 / D3 swipe 返回完整 ChatLog / D4 finish() 冗余调用 / D5 prepare_continue_pipeline 已显式置空 swipe_candidates（✓ 通过） / D6 流式生成消息后 swipe 控件不显示。

---

## 4. 设计层面的独立意见

按 §"Audit Agent Charter" 第 2 条"可以提出自己的想法"：

### 4.1 swipe_index 与 content 冗余存储的取舍

当前设计：`messages[i].content` 始终等于 `candidates[swipe_index]`（冗余）。优点是 OpenAI 协议兼容（客户端不识别 candidates 时仍能读 content）。缺点是 switch_swipe 要同时更新两个字段，破坏不变量就会数据不一致。

**替代设计**：`messages[i].content` 不存激活候选文本，而存"指针"（如 `__swipe_active__` 占位符），客户端必须读 `candidates[swipe_index]` 才能得到文本。优点是单一数据源。缺点是破坏 OpenAI 协议兼容性，且旧客户端 / 工具会看到占位符。

**本审计判断**：当前冗余设计是正确取舍（协议兼容 > 单一数据源）。但应在 `switch_swipe` 加 `debug_assert_eq!(log.messages[idx].content, cands[new_index])` 防御性校验，及时发现不变量破坏。

### 4.2 候选存储位置

当前：候选内联在 `StoredMessage.candidates`。每次 save 全量序列化。

**替代**：候选存单独文件 `messages/<message_id>/candidates.jsonl`，主 jsonl 只存当前激活候选 + 候选数量。优点是 save 不再随候选数线性增长。缺点是文件数爆炸（每条 assistant 消息一个目录）。

**本审计判断**：当前内联设计在候选数 ≤ 20 时足够。超过 20 时再考虑拆分。本 PR 应先加 cap（§2.C4），拆分作为 follow-up。

### 4.3 swipe 候选捕获时机的替代方案

当前：regen handler 预先 `delete_last_n(1)` + 捕获候选 → pipeline → finalizer `append_with_candidates`。

**替代**：两阶段提交——regen handler 不删除，pipeline 跑成功后 finalizer 用 `replace_last_with_candidates([old, new])` 原子替换。优点是 empty stripped 时不会丢失旧消息（旧消息仍在 log 中）。缺点是要新加一个 `replace_last_with_candidates` 方法。

**本审计判断**：两阶段提交是更稳健的设计，能直接消除 §2.B1。建议本 PR 至少采用 §2.B1 的"empty 时回灌旧候选"补丁，follow-up 考虑两阶段提交重构。

### 4.4 `chat_pipeline.rs` 应在 #249 C-G 之前解耦（结构性建议）

**现状**：`engine/src/chat_pipeline.rs` 已 1789 行（PR #251 后），单文件包含 10+ 个独立逻辑段落：

| 行号范围 | 段落 | 行数 |
|---|---|---|
| 33-87 | `PreparedPipeline` / `FinalizerCtx` / `PrepareMode` 类型 | 55 |
| 102-170 | root 解析 / session dir / provider / trace_source_id | 70 |
| 171-446 | `build_prompt_trace`（trace 构建） | 276 |
| 447-531 | `resolve_param_sources` / `read_revision_or_diagnostic` | 85 |
| 533-705 | helpers（char card / persona / regex filters） | 173 |
| 707-973 | `prepare_scene_pipeline`（scene 分支） | 266 |
| 974-1365 | `prepare_pipeline` / `regen` / `continue` / `prepare_pipeline_with_mode` | 391 |
| 1366-1494 | `build_sse_stream` / `SseMessage` | 129 |
| 1496-1618 | `run_finalize`（**§2.B1 落点**） | 122 |
| 1619-1657 | `chunks_result_to_events` | 39 |
| 1659-1737 | stdout runner（M4.5） | 79 |
| 1759-1807 | `extract_state_content` / `persist_live_state`（M_LS-1） | 49 |
| 1820-1917 | `GenerationStepResult` / 单步生成（M_AGENT-1） | 98 |

**问题**：
1. **§2.B1 类 bug 不显眼**：`run_finalize` 的 empty stripped 分支缺失在 1500+ 行的实现里被淹没。如果 finalize.rs 单独 122 行，review 时分支缺失一目了然。
2. **#249 C-G 落点不清**：Auto-Continue 改 finalize + stream；Export 加新模块；Impersonate 加新 handler。每个 PR 触碰 `chat_pipeline.rs` 的范围都不小，review 难。
3. **测试组织割裂**：实现 1789 行 + 测试在 `chat_pipeline/tests.rs`（§2.H.1）。`#[test]` 离实现远，难以判断测试覆盖了哪段实现。

**建议**：在 PR #251 合并后、#249 C-G 启动前，开 `refactor/chat_pipeline-decompose` 分支做独立解耦 PR。

**目标结构**：

```
engine/src/chat_pipeline/
├── mod.rs              # pub re-export + PreparedPipeline / FinalizerCtx / PrepareMode / SseMessage / GenerationStepResult
├── types.rs            # 上述类型定义
├── helpers.rs          # effective_root_for_mode / read_only_session_dir / provider_label / trace_source_id / resolve_param_sources / read_revision_or_diagnostic / load_char_card_json / resolve_request_persona / merge_persona_into_user_profile / assemble_regex_filters
├── trace.rs            # build_prompt_trace（276 行独立成文件）
├── prepare.rs          # prepare_pipeline / preview / regen / continue / prepare_pipeline_with_mode
├── prepare_scene.rs    # prepare_scene_pipeline（scene 分支独立，266 行）
├── stream.rs           # build_sse_stream / chunks_result_to_events
├── finalize.rs         # run_finalize / persist_live_state（B1 修复落点）
├── state_extract.rs    # extract_state_content（M_LS-1）
├── stdout_runner.rs    # print_chunk_to_stdout（M4.5）
├── generation_step.rs  # GenerationStepResult / 单步生成（M_AGENT-1）
└── tests/
    ├── mod.rs          # re-export
    ├── prepare.rs      # 按模块拆分测试
    ├── finalize.rs     # §2.B3 要求的 swipe 测试落点
    └── stream.rs
```

**解耦 PR 约束**（强制）：
1. **纯粹搬代码，不改逻辑**：diff 全是 `git mv` + `pub use` + `use super::` 调整。任何"顺手优化"留到后续 PR。
2. **公开 API 表面不变**：`chat_pipeline::prepare_pipeline` / `chat_pipeline::build_sse_stream` / `chat_pipeline::FinalizerCtx` 等签名保持。调用方零改动。
3. **`FinalizerCtx` 字段不动**：包括 #251 新增的 `swipe_candidates`。
4. **CI 全绿即可合并**：解耦 PR 不需要重审，但 PR 描述必须附 `cargo test --lib 2>&1 | tail` 证明测试数与解耦前一致。
5. **不要顺手重命名**：`run_finalize` / `prepare_pipeline_with_mode` 等名字即使不完美也保留，重命名单独 PR。

**解耦收益**：
- §2.B1 类 bug 在 finalize.rs 单独 122 行时更显眼
- #249 C-G 每项功能 PR 的触碰范围小（Auto-Continue 只改 finalize.rs + stream.rs）
- §2.B3 要求的 swipe 测试有自然落点（tests/finalize.rs）
- git blame 更清晰（finalize.rs 的 blame 只追踪 finalize 相关改动）

**风险**：
- 解耦 PR diff 大（~1800 行移动），但 review 简单（全是 `git mv` + `use` 调整）
- 解耦过程中可能漏 `use` 导致编译失败 → CI 兜底
- 不要在解耦 PR 内修任何 bug（包括 §2.B1）——bug 修复在 #251 内完成

**时序**：
1. PR #251 修复 3 项 BLOCKING + 4 项 MAJOR → 重新过审计 → 合并
2. **解耦 PR**（`refactor/chat_pipeline-decompose`）→ CI 全绿 → 合并
3. #249 C（Auto-Continue）→ 在解耦后的 finalize.rs + stream.rs 上开发

**本审计判断**：解耦应在 #249 C-G 之前完成。否则文件继续膨胀（#249 C 加 stop_reason / Auto-Continue 逻辑、E 加 export 端点、F 加 impersonate、G 加 branch），解耦成本线性上升。当前 1789 行已是可维护性边界。

---

## 5. 合并前必做清单

- [ ] 修复 B1：empty stripped regen 路径恢复旧候选（或采用两阶段提交）
- [ ] 修复 B2：doContinue 路径 SmoothStreamer 正确写入 textNode
- [ ] 修复 B3：补充 swipe 单元测试 + 集成测试（最低 8 个用例，覆盖 happy path + 边界 + §2.B1 回归）
- [ ] 修复 C2：swipe counter 改用 data attribute
- [ ] 修复 C3：SMOOTH_STREAM_FPS 改名 + 调高默认值（或加 settings 配置）
- [ ] 修复 C4：候选 cap = 20
- [ ] 澄清 C5：测试数字 743 vs 750/756 不一致
- [ ] 修复 D6：流式 done 后补创建 swipe 控件（或重新调用 appendMsg）

**修复完成后需重新提交审计。本审计报告 BLOCKING 状态不因 CI 全绿而解除。**

---

## 6. 不修项记录（PR 合并后写入 GitHub issue）

以下问题本审计标注为"可后续迭代"，不阻塞合并（前提是 §3 BLOCKING 全部修复）：

**Minor / Nitpick**：
- D1：`switch_swipe` 不更新 `log.updated_at`（一致性）
- D2：`append_with_candidates` 允许 whitespace 候选（防御性校验）
- D3：`POST /v1/chat/swipe` 返回完整 ChatLog（性能优化）
- D4：`SmoothStreamer.finish()` 冗余调用（可读性）
- §2.H.1：`chat_pipeline.rs` 测试风格不一致（实现与测试分文件）
- §2.H.2：`SmoothStreamer` 无 webui 测试（rAF mock 基础设施缺失）
- §2.H.3：`swipe_chat` 不记录 trace 事件（可审计性）

**Major（可单独 PR）**：
- C1：并发 regen race 被 swipe 放大（需 per-session mutex）
- §4.2：候选拆分存储（性能优化，候选数 > 20 时再做）
- §4.3：两阶段提交重构（设计层面，消除 §2.B1 类问题）

**结构性建议（独立 PR，#249 C-G 之前完成）**：
- §4.4：`chat_pipeline.rs` 解耦为 `chat_pipeline/` 目录模块（1789 行 → 13 个子模块）。约束：纯粹搬代码、API 表面不变、不顺手修 bug。时序：PR #251 合并后 → 解耦 PR → #249 C-G。

---

## 7. 审计总结

**审计模型**：GLM-5.2（本会话模型）

**审计结论**：**BLOCKING，不得合并**。3 项 BLOCKING + 4 项 MAJOR + 7 项 MINOR。

**核心问题**：
1. **B1（Critical）**：empty stripped regen output 永久丢失全部候选——违反 `AGENTS.md` §"破坏旧结构，不破坏用户资产"。
2. **B2（Functional bug）**：doContinue 路径 SmoothStreamer rAF 失效——PR 声称的"平滑输出"在 continue 模式下未交付。
3. **B3（Process blocker）**：swipe 功能零测试覆盖——CI 全绿不证明新功能正确。

**bot review 闭环核验**：
- gemini-code-assist 意见 1（doContinue SmoothStreamer）：✓ 确认 BLOCKING，未修复
- gemini-code-assist 意见 2（swipeSwitch fallback to `m.text`）：✗ 驳回（`ChatMessage` 无 `text` 字段）
- CodeRabbit 阻塞意见（empty stripped 数据丢失）：✓ 确认 BLOCKING，未修复
- CodeRabbit Nitpick（候选无上限）：✓ 升级为 Major

**修复路径**：
1. 修复 B1/B2/B3 + C2/C3/C4/C5 + D6（必做清单 §5）
2. 重新提交审计
3. 合并后开解耦 PR（§4.4）
4. PR 合并后将 §6 不修项写入 GitHub issue（按 `AGENTS.md` §"审计遗留项处理"时序约束）
