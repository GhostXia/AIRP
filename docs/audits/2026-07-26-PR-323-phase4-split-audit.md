# PR #323 独立审计

> **审计主体**：GLM-5.2 审计代理（本会话独立执行）
> **审计时间**：2026-07-26
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：PR #323（`split-phase4`，head `67f17d2`，5 commits：`27bef9c` / `f1f1844` / `7ca8c22` / `d234ef7` / `67f17d2`）
> **变更性质**：Phase 4 split——4.2 风格迁移 / 4.3 对话示例 / 4.4 知识图谱 / 4.5 时间线导出 / 4.6 角色卡版本对比（37 文件，+6347/-19）
> **结论**：**BLOCK → 经本审计已修复 12 个 CodeRabbit 阻塞项 + 1 个编译错误（`AirpError::Conflict` 缺失变体）+ 2 个 clippy doc-lazy-continuation 警告，待复审确认 PASS。**

---

## 0. 审计来源与独立性声明

- **审计 LLM 模型**：GLM-5.2（本会话驱动模型，纯文本，未执行视觉审查）
- **独立性**：本报告未附和 CodeRabbit 的 review，亦未照搬 PR 描述。所有结论均基于本代理独立阅读 `67f17d2` head 的源码、独立运行测试、独立比对接 Engine 路由 / handler / 类型 / WebUI 行为所得。
- **质疑历史**：本审计对 PR 描述中"Phase 4 split 各子功能可独立审计"的暗示提出**保留**——5 个 commit 共 6328 行新增确实按功能分 commit，但 `dialogue_gen.rs` 的两阶段锁纪律、`mes_example_override` 路径与 `style.rs` 的角色 profile 头部去重，均跨 commit 影响同一文件，须整体审查而非逐 commit 单独放过。

---

## 1. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| 工作区状态 | `git status` + `git log --oneline origin/main..HEAD` | head `67f17d2`，5 commits on `split-phase4` |
| diff 内容 | `git diff --stat origin/main..HEAD` | 37 文件 +6347/-19，与 PR 视图一致 |
| Engine lib 测试（修复前） | `cargo test --lib` | **0 passed / 编译失败**——`AirpError::Conflict` 变体不存在，`dialogue_gen.rs` 无法编译（见 §B0） |
| cargo fmt（修复后） | `cargo fmt --all --check` | ✓ clean |
| cargo clippy（修复后） | `cargo clippy --lib --tests -- -D warnings` | ✓ clean |
| Engine lib 测试（修复后） | `cargo test --lib` | **989 passed / 0 failed / 2 ignored**（ignored 为 mockito 与 bench，与本 PR 无关） |
| WebUI 测试 | `node --test tests/runtime-pages.test.mjs tests/api-client.test.mjs tests/operations.test.mjs tests/agent-harness.test.mjs` | 50/50 pass |
| 锁纪律对齐 | 读 `engine/src/domain.rs` `character_lock` + `engine/src/daemon/handlers/characters.rs::update_character_card` | `dialogue_gen.rs` Phase B 与 `update_character_card` 一致：取 `character.write()` guard → 重读 → 修改 → 写盘 |
| `mes_example_override` 路径 | 读 `engine/src/daemon/handlers/dialogue_gen.rs:99-137` | ✓ 跳过 LLM、走 `count_starts` 校验、走 Phase B 写盘 |
| `<START>` 格式契约校验 | 读 `engine/src/dialogue_gen.rs:194-199` | ✓ `count_starts(&cleaned) == 0` 显式拒绝 |
| `mes_example.bak` 持久化 | 读 `engine/src/daemon/handlers/dialogue_gen.rs:190-203` | ✓ v2 写 `data.mes_example.bak`、v1 写顶层 `mes_example.bak` |
| `timeline_export.rs` 安全索引 | 读 `engine/src/timeline_export.rs:290-323` | ✓ `log.messages.get(*idx).ok_or_else(...)?` 替代 `log.messages[*idx]`，错误传播而非 panic |
| `worldbook_graph.rs` `shared_keys` | 读 `engine/src/worldbook_graph.rs:182-224` | ✓ `for (shared_key, indices) in &key_to_entries` + 重复 key 去重 `if !edge.shared_keys.contains(&k)` |
| `console-runtime.js` 重定向渲染 | 读 `webui/assets/console-runtime.js:695` | ✓ `stylelearn`/`dialoguegen`/`wbgraph`/`timeline`/`carddiff` → `redirectRenderer(...)` 跳到各自 HTML |
| `worldbook-graph.js` async simulation | 读 `webui/assets/worldbook-graph.js:177-252` | ✓ `requestAnimationFrame(runBatch)` + `batchSize=10` + `pinned` 标记 |
| `worldbook-graph.html` a11y | 读 `webui/screens/40-worldbook-graph.html:31-33` | ✓ `role="img" aria-label="..."` + fallback 文本 |

---

## 2. 编译错误（B0，本审计已修复）

### B0 — `AirpError::Conflict` 变体不存在（engine/src/error.rs + dialogue_gen.rs）

**严重度**：阻塞（编译失败，CI 全红）

**位置**：`engine/src/daemon/handlers/dialogue_gen.rs:167`

**问题**：CR1 修复引入 `AirpError::Conflict(...)` 用于 Phase B 重读 stale snapshot 时返回，但 `engine/src/error.rs` 的 `AirpError` 枚举没有 `Conflict` 变体。`cargo build` / `cargo test --lib` 直接编译失败：

```
error[E0599]: no variant or associated item named `Conflict` found for enum `AirpError`
   --> engine/src/daemon/handlers/dialogue_gen.rs:167:32
    |
167 |         return Err(AirpError::Conflict(format!(
    |                    ^^^^^^^^^^^^^^^^ variant or associated item not found
```

PR 描述 / commit message 未提及此问题，说明作者本地未跑 `cargo test --lib`，依赖 CI 触发后才暴露。

**修复**：在 `AirpError` 加入 `Conflict(String)` 变体，HTTP 映射 409 Conflict，code_str = `"conflict"`，与 `BadRequest` / `NotFound` 同级。`public_message` 走 default 分支（`error.to_string()`）——Conflict 的消息本身不含敏感内部路径，可向客户端透出。

---

## 3. CodeRabbit 阻塞项（CR1–CR12，本审计已修复）

### CR1 — `dialogue_gen.rs` read-modify-write race 跨 LLM 生成（Major · Heavy lift）

**位置**：`engine/src/daemon/handlers/dialogue_gen.rs:65-89`（原实现）

**问题**：角色卡在 Line 66-68 读入内存，streaming LLM 生成耗时数秒（Line 88-89），随后整卡对象写回（Line 129-142）。期间任何并发 mutation（另一编辑 / 另一 dialogue-gen / card update）会被静默覆盖。Codebase 已有 `with_session` per-session 锁纪律（`engine/src/domain.rs`），但本 handler 未对齐。

**修复**（两阶段锁纪律）：
- **Phase A（无锁）**：读卡 → LLM 生成（异步，可能数秒）。读到的 `mes_example` 记为 `baseline_mes_example` 作为后续 stale 检测基线。
- **Phase B（`character_lock(cid).write()`）**：重读 card → 比对 `current_mes_example == baseline_mes_example` → 若不等返回 `AirpError::Conflict`（HTTP 409，提示用户重试）→ 等则继续原子写入 `mes_example` + `mes_example.bak`。

放弃"LLM 期间阻塞所有卡写入"的强保证（`std::sync::RwLockWriteGuard` 是 `!Send`，跨 `.await` 持有 future 变 `!Send`，违反 axum `Handler` trait），但保留关键的"原子 read-modify-write"——并发写入被检测到，绝不会丢失。与 `update_character_card` 的锁纪律对齐（其临界区是纯同步 fs 写，无 `.await`，可全程持锁）。

### CR2 — `dialogue_gen.rs` `(300 * payload.turns + 200)` 在 turns 校验前算术可能溢出（Major · Quick win）

**位置**：`engine/src/daemon/handlers/dialogue_gen.rs:85`（原实现）

**问题**：`(300 * payload.turns + 200)` 在 `run_dialogue_gen` 的 `turns ∈ (0, 10]` 校验之前执行。crafted `{"turns": 4000000000}` 在 debug build 触发算术溢出 panic，构成 DoS。同时 `run_dialogue_gen` 内部会无条件覆盖 `temperature`/`max_tokens`，故此计算还是死代码。

**修复**：handler 不再构造 `temperature`/`max_tokens`，传 `None` 给 `GenerationParams`，由 `run_dialogue_gen` 在校验 `turns` 后安全计算。

### CR3 — `dialogue_gen.rs` 无持久化 `mes_example.bak` 备份（Major · Heavy lift）

**位置**：`engine/src/daemon/handlers/dialogue_gen.rs:103-133`（原实现） + `engine/src/dialogue_gen.rs:11-16` + `webui/assets/dialogue-gen.js:126-140`

**问题**：设计文档与 UI 都声明"旧值已备份"，但 handler 仅在 HTTP 响应 `previous_mes_example` 字段返回旧值，从不落盘。若响应丢失（页面刷新 / 网络中断 / 客户端 bug），旧值彻底无法恢复，违反"不破坏用户资产"硬约束。

**修复**：Phase B 在覆盖 `mes_example` 之前，先把旧值写到卡片内的 `mes_example.bak` 字段。v2 嵌套卡写 `data.mes_example.bak`，v1 flat 卡写顶层 `mes_example.bak`。`previous_mes_example` 响应字段保留，便于 UI 即时显示。

### CR4 — `style.rs` 角色 profile 头部重复 + write 错误被静默吞掉（Major · Quick win）

**位置**：`engine/src/daemon/handlers/style.rs:285-299`（原实现）

**问题**：
1. `char_md` 拼接 `# Character Style Profile: {cid}` 头部 + 来源行后，再 `push_str(&profile_md)`——但 `profile_md` 自身已含 `# Style Profile: {profile_id}` 头部 + 来源行，导致 `characters/{cid}/style-profile.md` 出现两层头部和两个时间戳，污染 prompt。
2. `let _ = crate::style::write_profile(...)` 丢弃 `Result`，磁盘满 / 权限不足时 handler 仍返回 `success: true`，欺骗调用方。

**修复**：
1. 用 `profile_md.find("\n- ").map(|i| &profile_md[i+1..]).unwrap_or(profile_md.as_str())` 截取条目部分，丢弃全局头部。
2. `crate::style::write_profile(...)?` 用 `?` 传播错误，与全局 profile 写入纪律对齐。

### CR5 — `dialogue_gen.rs` 缺少 `<START>` 标记格式契约校验（Major · Quick win）

**位置**：`engine/src/dialogue_gen.rs:181-189`（原实现）

**问题**：`clean_dialogue_output` 后只检查 `cleaned.trim().is_empty()`，未检查 `<START>` 标记存在。若 LLM 违反格式契约返回纯解释性文本（无 `<START>`），handler 仍返回 `Ok`，`count_starts` 返回 0，写入 `mes_example` 后 SillyTavern 无法解析，破坏整个功能的目的。

**修复**：在 emptiness 检查后追加 `if count_starts(&cleaned) == 0 { return Err(AirpError::Internal(...)); }`。`mes_example_override` 路径同样校验（`dialogue_gen.rs:111-115`），保证用户持久化路径与 LLM 生成路径走同一格式契约。

### CR6 — `timeline_export.rs` `message_timestamps` 长于 `messages` 时索引越界 panic（Major · Quick win）

**位置**：`engine/src/timeline_export.rs:248-268`（原实现）

**问题**：`for (idx, ts) in &timed` 的 `idx` 来自 `message_timestamps`，但 `log.messages[*idx]` 是直接索引。truncated / desynced 的 on-disk log（或 legacy 文件数组不一致）会让请求线程 panic 而非返回错误。

**修复**：用 `log.messages.get(*idx).ok_or_else(|| AirpError::Internal(...))?` 替代直接索引，untimed 分支同样处理。`build_entries` 签名从 `-> Vec<TimelineEntry>` 改为 `-> Result<Vec<TimelineEntry>, AirpError>`，调用方加 `?`。同步更新 6 个测试加 `.expect("build_entries failed")`。

### CR7 — `worldbook_graph.rs` `shared_keys` 不完整且可能记录错误 key（Major · Quick win）

**位置**：`engine/src/worldbook_graph.rs:182-221`（原实现）

**问题**：`for indices in key_to_entries.values()` 丢弃当前 key，首次创建边时用 `entries[a].keys.iter().find(|k| entries[b].keys.contains(k))` 找"某个"共享 key——后续重复 pair 仅 `weight+1` 不再 `push`。导致 `weight: 3` 的边 `shared_keys` 长度始终为 1，与字段文档"共享的具体 key 列表"矛盾，UI tooltip 显示缺失。

**修复**：改为 `for (shared_key, indices) in &key_to_entries`，首次创建用 `(*shared_key).to_string()`，重复 pair `weight+1` 并 `if !edge.shared_keys.contains(&k) { edge.shared_keys.push(k); }`。删除原 `find` 扫描。

### CR8 — `console-runtime.js` 未注册新屏 38–42 的 renderer（Major · Quick win）

**位置**：`webui/assets/console-runtime.js:40-44`（原实现）

**问题**：nav 暴露 `stylelearn`/`dialoguegen`/`wbgraph`/`timeline`/`carddiff` 五个 screen key，但 renderer dispatch table 无匹配项。从书签或外部链接访问 `console.html?screen=stylelearn` 会落入 `renderDiagnostics` fallback 而非跳到专用页面。

**修复**：在 `renderers` object 末尾加 `stylelearn: redirectRenderer('38-style-learn.html')` 等 5 项。`redirectRenderer = href => () => { location.href = pathWithState(href); }`。注：这五屏本身是独立 HTML 页面（各自加载自己的 `.js`），不经 `console-runtime.js` 渲染，redirect 仅是路由修正。

### CR9 — `dialogue-gen.js` `writeGenerated` 重新生成而非持久化预览（Critical · Heavy lift）

**位置**：`webui/assets/dialogue-gen.js:142-170`（原实现）

**问题**：`writeGenerated` 重新构造与生成阶段相同的 body 再次 POST，触发 temperature 0.7 非确定性 LLM 生成。用户预览 A 后点"写入角色卡"，最终写入的是 B——破坏整个"预览→确认→写入"契约，是本 PR 最严重 bug。

**修复**（前后端协同）：
- **前端**：`writeGenerated` 不再重建 generation body，改用 `{ dry_run: false, append, mes_example_override: lastGenerated }` 把预览内容原样交给 handler。
- **后端**：`DialogueExampleRequest` 新增 `mes_example_override: Option<String>` 字段。Handler 检测到该字段非空时：
  1. 校验非空 + `count_starts > 0`（与 LLM 路径同一格式契约）
  2. 跳过 LLM 调用
  3. 走 Phase B `character_lock` 写盘（与其他路径同一锁纪律 + `mes_example.bak` 备份）

`dry_run=true` 时即使提供 override 也忽略（handler 优先级：override > dry_run）。

### CR10 — `worldbook-graph.js` 同步 O(iterations × n²) 力导向布局锁死 UI（Major · Heavy lift）

**位置**：`webui/assets/worldbook-graph.js:163-223`（原实现）

**问题**：500 节点 × 300 迭代 = ~75M 次力计算，主线程同步执行，多秒冻结无反馈。engine 文档允许最多 500 entries，所以这不是"理论上"而是"实际会触发"。

**修复**：改为 `requestAnimationFrame` 分批：
- 总 300 迭代，每帧 `batchSize=10` 次（60Hz 下约 0.5s 完成）
- `coolFactor = Math.pow(0.05, batchSize/totalIterations)` 每 batch 衰减到 5%（与原 0.95^300 等效）
- `simulationRunning` 防重入
- 每帧 `drawCanvas()` 提供渐进式布局动画
- `pinned` 标记让被拖拽的节点保持位置（详见 CR11）

### CR11 — `worldbook-graph.js` drag-release 后立即 `runSimulation` 把用户布局"弹回"（Major · Quick win）

**位置**：`webui/assets/worldbook-graph.js:368-383`（原实现）

**问题**：`mouseup` 后 `dragNode = null` 再调 `runSimulation()`——刚被释放的节点不再被识别为 `dragNode`，300 次迭代立即把它拖回松弛位置，用户拖到哪都没用。

**修复**：
- `simNodes` 节点对象新增 `pinned: false` 字段
- `mouseup` / `mouseleave` 时 `dragNode.pinned = true` 再 `dragNode = null`，仅 `drawCanvas()` 不调 `runSimulation()`
- 位移循环 `if (a === dragNode || a.pinned) continue;` 跳过 pinned 节点
- `loadGraph` 重置 `simNodes` 时所有节点 `pinned=false`，下次显式刷新会重新布局

### CR12 — `40-worldbook-graph.html` canvas 仅 pointer 交互，无 a11y 名称或 fallback（Major · Heavy lift）

**位置**：`webui/screens/40-worldbook-graph.html:33`（原实现）

**问题**：`#graph-canvas` 无 `aria-label`/fallback 内容，节点详情只能通过 `mousedown`/`click` 触达，键盘和屏幕阅读器用户无法检视任何节点。500 节点的视觉信息对辅助技术完全不可见。

**修复**（最低可行改进）：canvas 加 `role="img" aria-label="世界书知识图谱力导向视图：蓝色边表示条目间共享 key（无向），红色箭头表示某条目的 content 引用了其他条目的 key（有向）；下方冲突警告与节点详情为文本等价物。"`，元素内放 fallback 文本"当前浏览器不支持画布渲染，请参考下方冲突警告与节点详情。"

注：理想方案是相邻的可聚焦节点列表驱动 `showNodeDetail`，本审计仅修最低可行点；完整键盘可达性进入非阻塞项 N1。

---

## 4. 非阻塞项（合并后入 issue）

### N1 — `worldbook-graph.html` 节点详情仍仅 pointer 可达

CR12 修了 canvas 的 a11y 名称 + fallback，但节点详情面板 (`#graph-detail`) 仍只能通过 `mousedown`/`click` 触发。理想方案是相邻的可聚焦节点列表（`<button>` per node），键盘 focus + Enter 触发 `showNodeDetail`。优先级：低（功能可用，只是键盘用户无法浏览节点详情）。

### N2 — `dialogue_gen.rs` Phase A 拿到 stale snapshot 期间的并发写入仍会丢失

CR1 修复后，Phase B 检测到 stale snapshot 会返回 409 Conflict 让用户重试。但若 Phase A 期间发生并发写入，且 Phase A 的 LLM 生成已完成、Phase B 才发现 stale，用户已经浪费了数秒的 LLM 调用 + token。可考虑：
- Phase A 起点先取 `character_lock(cid).read()` 拿到 baseline，LLM 完成后释放，再 Phase B 拿写锁——但这要求 `RwLockReadGuard` 也不能跨 `.await`，同样 `!Send`，不可行。
- 改用 `tokio::sync::RwLock` 让 guard 跨 `.await`——但这会改变 `domain.rs` 全局锁类型，影响其他 handler，本 PR 不宜扩大范围。

优先级：中（用户体验问题，非数据正确性问题）。

### N3 — `mes_example.bak` 无清理策略

CR3 修复后，每次写入 `mes_example` 都会覆盖 `mes_example.bak`。第二次写入时第一次的旧值被丢弃，无法回滚两步。可考虑：
- 保留 `mes_example.bak` + `mes_example.bak.2` + `mes_example.bak.3` 三代
- 或接入 `revision` 系统（与 `card.json` revision 一起 commit）

优先级：低（已有 `revision` 系统作为终极回滚路径，`mes_example.bak` 是即时备份）。

### N4 — `style.rs` 角色 profile 头部去重用 `find("\n- ")` 字符串切分，脆弱

CR4 修复用 `profile_md.find("\n- ").map(|i| &profile_md[i+1..])` 切取条目部分。若 `render_profile_markdown` 未来改格式（如改用 `* ` 列表标记，或在头部加额外段落），此切分会失效。

理想方案：`render_profile_markdown` 拆为 `render_header` + `render_entries` 两个函数，handler 直接调 `render_entries`。优先级：低（与 `style::learn.rs` 内部实现耦合，重构成本小但本 PR 不宜扩大范围）。

### N5 — `worldbook-graph.js` 异步 simulation 未支持 `cancel` / `pause`

CR10 修复后 simulation 是异步的，但用户切换 character 或点"应用并刷新"时，旧 simulation 仍在跑（被 `simulationRunning` 防重入跳过，但已 queued 的 `requestAnimationFrame` 仍会执行最后一次 `drawCanvas`）。视觉上无害，但浪费 CPU。

理想方案：保留 `animationFrameId`，切换时 `cancelAnimationFrame`。优先级：低。

### N6 — `dialogue-gen.js` UI 文案"已备份"在 `mes_example.bak` 持久化前仍可能误导

CR3 修复后 `mes_example.bak` 已落盘，但 UI `renderResult` 仍显示"旧值已备份（{N} 字符）"——这隐含"已持久化到磁盘"，对用户是正确的。但若 `mes_example.bak` 字段在未来被改名或弃用，UI 文案需同步。建议加注释把 UI 与 `engine/src/daemon/handlers/dialogue_gen.rs:190-203` 的字段名绑定。优先级：低。

### N7 — `card-diff.js` `previewHtml` 用 `srcdoc` 注入完整 HTML，未限制 sandbox

`frame.srcdoc = cachedHtml` 把 engine 返回的 HTML diff 直接注入 iframe。engine 端 `card_diff.rs` 的 HTML 渲染是 AIRP 自己实现的（非用户内容），但若未来允许用户自定义 HTML 模板，这会成为 XSS 向量。

建议：`<iframe sandbox="allow-same-origin">` 限制脚本执行（card-diff HTML 不需要脚本）。优先级：低（当前 HTML 来自 engine 可信源，但应纵深防御）。

### N8 — `worldbook_graph.rs` 500 节点上限是 hardcoded

`if entries.len() > 500 { return Err(...); }` 限制 lorebook 条目数。此限制应来自 config（与 engine 的其他 rate limit 一致），而非 hardcoded。优先级：低。

### N9 — `timeline_export.rs` `build_entries` 错误传播后 `load_for_session_if_exists` 仍可能触发 meta 修复写入

`chat_store.rs::load_for_session_if_exists` 注释已说明"当前实现仍委托给 `load_or_create_for_session` 来读取已存在的 JSONL，因此 meta 修复写入在 meta 损坏的边缘场景仍可能发生"。本 PR 的 timeline export 是 read-only 操作，但若 meta 损坏，会触发写入，破坏 read-only 语义。优先级：中（应作为独立重构，见 chat_store.rs:484 注释）。

### N10 — `dialogue_gen.rs` Phase B 失败后 `mes_example.bak` 可能被遗留

若 Phase B 的 `replace_file` 写 `card.json` 成功但写 `raw.json` 失败，`mes_example` 已被覆盖，`mes_example.bak` 已写入，但 `raw.json` 仍是旧值——`card.json` 和 `raw.json` 不一致。可考虑：
- 先写临时文件 + rename 原子化（`replace_file` 已是 atomic，但两个 `replace_file` 之间仍可能中断）
- 或接入 `revision` 系统统一 commit（与 `characters.rs::commit_character_revision` 对齐）

优先级：中（数据一致性问题，但发生概率极低——两次 `replace_file` 之间进程崩溃）。

---

## 5. 结论

PR #323 的 12 个 CodeRabbit 阻塞项 + 1 个编译错误（B0 `AirpError::Conflict`）+ 2 个 clippy 警告（`chat_store.rs::load_for_session_if_exists` doc-lazy-continuation）**已由本审计就地修复**。修复后本地验证通过：

- `cargo fmt --all --check` 干净
- `cargo clippy --lib --tests -- -D warnings` 干净
- `cargo test --lib` = **989 passed / 0 failed / 2 ignored**（ignored 为 mockito feature 与 bench，与本 PR 无关）
- `node --test tests/runtime-pages.test.mjs tests/api-client.test.mjs tests/operations.test.mjs tests/agent-harness.test.mjs`（webui）= **50 passed**
- `cargo build --bin airp-daemon`（隐式覆盖，由 cargo test 触发）通过

修复要点：
- **B0**：补 `AirpError::Conflict` 变体（HTTP 409, code `"conflict"`）
- **CR1**：dialogue_gen.rs 两阶段锁纪律（Phase A 无锁 + Phase B `character_lock` 写锁 + stale snapshot 检测）
- **CR9**：前后端协同 `mes_example_override` 路径，前端 `writeGenerated` 不再重建 generation body
- **CR3**：`mes_example.bak` 落盘到卡片内（v2 `data.mes_example.bak` / v1 顶层）
- **CR10**：`worldbook-graph.js` `requestAnimationFrame` 分批 + `pinned` 标记
- **CR6**：`timeline_export.rs` 安全索引 + `build_entries -> Result`
- **CR7**：`worldbook_graph.rs` `shared_keys` 完整性 + 正确 key

§4 中 10 个非阻塞项（N1–N10）建议合并后入 issue。其中 N2 / N9 / N10 涉及数据一致性但发生概率低，N1 / N5 / N8 是体验问题，N3 / N4 / N6 / N7 是设计债务。

**视觉审查声明**：按 issue #319 补充要求（2026-07-26 用户立），WebUI 改动 PR 必须由 KIMI K3+ 多模态 agent 执行视觉审查。**本审计 agent 为 GLM-5.2 纯文本模型，整轮审计未执行视觉审查**——CR1–CR12 全部修复均基于 HTML 字符串/DOM 契约/源码语义判断，未对 38–42 五屏的实际渲染做截图审查。建议在合并前由多模态 agent 对 PR #323 涉及的视觉改动（特别是 40 屏 canvas 力导向布局异步化后的渐进式动画、42 屏 card-diff HTML 预览 srcdoc 注入、38 屏 style-learn 表单）独立补审。

**建议**：本 commit 推送后，待人工 review、CodeRabbit 复审、`Portable Windows WebUI` CI 复跑通过、以及多模态 agent 视觉审查通过后可合并；合并后由审计 agent 将 §4 中 10 个非阻塞项整理为 GitHub issue。
