# PR #80 独立审计报告

**审计源 LLM**：GLM-5.2
**审计日期**：2026-07-07
**审计范围**：Tauri shell chat.history 接入（commit 561bf0e）
**审计依据**：AGENTS.md 三条守则（独立审计 / 可提出自己的想法 / 质疑历史决策）

## 改动清单

| 文件 | 改动 |
|---|---|
| `ui/src-tauri/src/bus.rs` | +100 行：新增 `chat.history` intent 分支 + `fetch_chat_history` 函数 |
| `ui/src/App.vue` | +5 行：`characters.select` 后触发 `chat.history` |

## 审计方法

1. 完整读两个改动文件
2. 读 engine `/v1/chat/history` 端点实现（`engine/src/daemon/handlers.rs:203-210`）+ `HistoryQuery` 类型（`engine/src/daemon/types.rs:89-95`）
3. 读 `ChatLog` 结构（`engine/src/chat_store.rs:30-51`）+ `ChatMessage` 结构（`engine/src/adapter.rs:80-85`）
4. 读 `ChatWidget.vue` 确认 w-chat scope 期望的 shape
5. 对比现有 `chat_turn_open_envelope` 的 id-keyed shape
6. 跑全量测试（386 engine + 9 Tauri + 97 vitest + 12 security + 24 markdown + 1 神圣不变式 + vue-tsc 0 errors）

## A 项（阻塞）

**无。**

## W 项（非阻塞，可后续）

### W-01：chat.history 用 set 替换 w-chat scope，与并发 chat.send 流式 turn 可能竞态

**位置**：`ui/src-tauri/src/bus.rs:443-448`

**问题**：`chat.history` 成功后用 `emit_state_set` 全量替换 w-chat scope：
```rust
emit_state_set(
    &app_opt,
    format!("state-chat-history-{n}"),
    "w-chat",
    serde_json::json!({ "messages": scope_messages, "order": order }),
);
```

如果用户拉历史时正好有一个 chat.send 流式 turn 在进行中（assistant 文本正在 token-by-token 替换 `/messages/{a{n}}/text`），set 会**全量覆盖** w-chat scope，把正在流式的 assistant 消息清空。

**实际触发条件**：
1. 用户发消息 → chat.send 开始流式
2. 流式过程中用户切换角色（characters.select）→ 触发 chat.history
3. chat.history 的 set 覆盖 w-chat scope，正在流式的 assistant 消息丢失

**严重度评估**：
- 流式 turn 通常几秒内完成，用户在流式过程中切换角色是罕见操作
- 即使发生，engine 端 chat.send 仍在跑，后续 token 会 patch 到 `/messages/{a{n}}/text`，但 `a{n}` 这个 id 已被 set 覆盖掉，patch 会创建一个孤立的 a{n} 条目（因为 patch 是 add/replace 语义，对不存在的 path 会创建）
- 实际效果：流式 assistant 消息会"重新出现"在 order 末尾，但历史消息也被覆盖了

**建议**：在 App.vue 的 characters.select 中，如果正在流式，先取消或等待。或在 bus.rs 的 chat.history 分支中检查是否有进行中的 turn。但这增加复杂度，本 PR 不应扩大范围。**记录为已知限制，后续 PR 处理**。

**严重度**：低（罕见竞态，不破坏数据，仅影响 UX）

### W-02：chat.history 无 emit 形状的单元测试

**位置**：`ui/src-tauri/src/bus.rs:372-461`

**问题**：与 PR #78 W-02 同类问题。chat.history 的 emit 形状（scope = "w-chat", op = Set, state 包含 `messages` map + `order` array）没有专门测试。`dispatch_handles_intent_without_subscriber` 只验证不 panic。

**建议**：补一个测试，验证 chat.history 成功时 emit 的 envelope 形状。但需要 mock HTTP server，工作量较大。可参考 `chat_turn_open_is_one_ordered_patch_envelope` 的模式，但那个测试是纯函数测试不涉及 HTTP。

**严重度**：低（测试覆盖缺口，与现有 characters.list/import/settings 同样缺测试）

### W-03：ChatLog.message_timestamps 未传递到 UI

**位置**：`ui/src-tauri/src/bus.rs:414-442`

**问题**：engine 返回的 ChatLog 包含 `message_timestamps: Vec<Option<String>>`（PR #75 加的消息级时间戳），但 bus.rs 的转换只取 `role` + `content`，丢弃了 `ts`。UI 拿到的历史消息没有时间戳。

**影响**：
- ChatWidget.vue 当前不显示时间戳（只显示 role + text），所以**无直接视觉影响**
- 但 issue #73 方案 B 的目标是"消息级时间戳"，PR #75 已在 engine 端实现，Tauri shell 不传递是能力断层
- 未来 ChatWidget 想显示时间戳时，需要回头补 bus.rs

**建议**：在转换时把 `ts` 也放进 message object：
```rust
scope_messages.insert(
    id.clone(),
    serde_json::json!({
        "id": id,
        "role": role,
        "text": text,
        "ts": msg.get("ts"),  // 或从 message_timestamps[i] 取
    }),
);
```

但要注意 ChatLog 的 JSON 结构：`messages` 是 `Vec<ChatMessage>`（无 ts），`message_timestamps` 是平行 array。bus.rs 当前只读 `messages` array，没读 `message_timestamps`。要传递 ts 需要同时读两个 array 并按 index 合并。

**严重度**：低（当前 UI 不显示时间戳，无视觉影响；未来能力断层）

### W-04：characters.select 触发 chat.history 的递归 dispatch 隐式

**位置**：`ui/src/App.vue:103-112`

```ts
if (name === "characters.select") {
    const id = (params as { character_id?: string } | undefined)?.character_id;
    if (id) {
        selectedCharacterId.value = id;
        // 拉取该角色的 chat history。用 onIntent 走正常 dispatch 路径，不递归
        // （chat.history 不在 characters.select 分支里）。
        onIntent("chat.history", { character_id: id } as Json);
    }
    return;
}
```

**问题**：在 `onIntent` 内部调用 `onIntent` 是一种"递归 dispatch"模式。注释说"不递归"，但实际是递归调用 onIntent 函数本身（只是 name 不同，不会无限递归）。

**独立判断**：这个模式是**可接受的** —— onIntent 对 chat.history 走正常 dispatch 路径，不会回到 characters.select 分支。但注释"不递归"可能误导读者以为没有递归调用。建议注释改为"onIntent 对 chat.history 走正常 dispatch 路径，不会回到 characters.select 分支"。

**严重度**：信息级（注释可读性）

## 信息级

### I-01：chat.history 失败时 w-chat scope 被清空

**位置**：`ui/src-tauri/src/bus.rs:450-458`

chat.history 失败时 emit `{ messages: {}, order: [], error: e.to_string() }`，全量替换 w-chat scope。如果用户之前有对话显示，失败后被清空。

**实际影响**：失败通常发生在 engine 未启动 / 网络问题，此时用户本来就看不到有用内容。**可接受**。

### I-02：fetch_chat_history 用 POST 而非 GET

**位置**：`ui/src-tauri/src/bus.rs:789-801`

`POST /v1/chat/history` 用 POST 传 body `{character_id}`，而不是 GET query param。这是 engine 端的设计（`HistoryQuery` 是 body），bus.rs 跟随。**不是本 PR 的问题**，但记录：REST 语义上 GET 更合适（idempotent read），但 engine 端已定型，本 PR 不改。

## 设计审计

### D-01：历史消息 id 用 `h{i}` 前缀（同意开发决策）

开发文档说"历史消息 id 用 `h{i}` 前缀，与 chat.send 的 `u{n}`/`a{n}` 不冲突"。

**独立验证**：
- chat.send 的 turn open 用 `u{n}` / `a{n}`（n 是 relay seq，单调递增）
- chat.history 用 `h{i}`（i 是 messages array index，从 0 开始）
- `h` 前缀与 `u`/`a` 不同，无字符冲突
- 若用户拉历史后发新消息：历史 `h0, h1, ...` + 新 turn `u{n}, a{n}`，order 数组按 emit 顺序排列（history set 先 emit，turn open patch 后 emit），新消息自然出现在历史之后

**但有一个边界情况**：如果 n 很大（比如 n=100），`u100` 与 `h100` 不同前缀，无冲突。但如果用户多次拉历史（切换角色再切回），每次 history set 会全量替换 w-chat scope，旧历史被新历史覆盖 —— 这是预期行为。

✅ 设计正确。

### D-02：只做 legacy 单 session（同意开发决策）

开发文档说"只做 legacy 单 session 路径，多 session 切换留下个 PR"。

**独立判断**：同意。engine 当前 `POST /v1/chat/history` 的 `HistoryQuery` 只有 `character_id`，不支持 `session_id`。要支持多 session 需要：
1. 扩展 `HistoryQuery` 加 `session_id: Option<SessionId>`
2. handler 改用 `load_or_create_for_session`
3. bus.rs 加 `sessions.list/create/select` intent
4. UI 加 SessionPicker widget

这是 4 个改动点，跨 engine + ui 两层，按 AGENTS.md "incremental推进" 原则不应塞进一个 PR。本 PR 只做 legacy 单 session 是合理的最小步。

✅ 设计合理。

### D-03：转换逻辑在 bus.rs 而非 UI 侧（同意开发决策）

bus.rs 把 `ChatLog.messages`（`Vec<{role, content}>`）转换为 w-chat scope 的 id-keyed shape。这把转换逻辑放在 Rust 侧而非 TS 侧。

**独立判断**：同意。理由：
- bus.rs 是 State Protocol 的 gateway，职责就是把 engine 响应转换为 State Protocol envelope
- 转换逻辑在 Rust 侧可被单元测试覆盖（虽然当前缺测试，W-02）
- UI 侧拿到的就是 w-chat scope shape，无需再转换

✅ 设计合理。

## 历史决策质疑（守则 3）

### H-01：chat.history 用 POST 而非 GET

engine 端 `POST /v1/chat/history` 用 POST 传 body。这是历史决策（PR #38 之前就存在）。

**质疑**：REST 语义上 GET 更合适（idempotent read）。但 POST body 可以传复杂 query（虽然这里只有 character_id），且 engine 端已定型。改 GET 会破坏 webui 老 console（`webui/app.js` 也用 POST）。**不在本 PR 范围**。

### H-02：ChatLog.messages 是 Vec<ChatMessage> 而非 id-keyed map

engine 端 ChatLog.messages 是 `Vec<ChatMessage>`（array），bus.rs 转换为 id-keyed map。为什么不直接用 id-keyed？

**质疑**：array 顺序明确但查找 O(n)；map 查找 O(1) 但顺序需 separate order array。engine 端用 array 是因为 ChatLog 是 append-only 日志，顺序是核心语义。UI 侧用 id-keyed map + order array 是因为虚拟滚动需要 O(1) 查找。**两端 shape 不同是合理的**，bus.rs 做转换是对的。

## 测试结果

| Suite | 结果 |
|---|---|
| engine cargo test (workspace) | 386 pass, 1 ignored |
| Tauri ui cargo test | 9 pass |
| 神圣不变式 `subagent_context_has_no_orchestrator_noise` | 1/1 pass |
| webui security tests | 12/12 pass |
| markdown tests | 24/24 pass |
| UI vitest | 97/97 pass (13 files) |
| vue-tsc --noEmit | 0 errors |

## 结论

**推荐合并。**

- A 项（阻塞）：无
- W 项（非阻塞）：4 项，均为可后续改进
  - W-01：流式竞态（罕见，不破坏数据）
  - W-02：测试覆盖缺口（与现有惯例一致）
  - W-03：message_timestamps 未传递（当前 UI 不显示，无视觉影响）
  - W-04：注释可读性（信息级）
- 设计合理，与 PLAN.md §0 "可双击运行并简单使用" 目标直接对齐
- 解决 P0 第 1 项硬伤：重启不丢历史

## 后续 issue 建议（按 AGENTS.md 时序，PR 合并后提交）

- W-01 + W-03：合并为一条 "Tauri chat.history 竞态保护 + message_timestamps 传递" issue
- W-02：可合并到 issue #79（PR #78 审计遗留）的"bus.rs settings/chat intent 测试覆盖"项
- W-04：信息级，不必单独 issue
