# PR #314 独立审计报告

> **审计主体**：WorkBuddy 审计代理（本会话）
> **审计时间**：2026-07-25
> **审计原则**：AGENTS.md §11.1 三原则（独立审计 / 可提己见 / 可质疑历史并查证）
> **审计范围**：PR #314（`split-phase1`，head `0824b0a`，cherry-pick 自原 #313 的 `6e5fc0d`）
> **变更性质**：纯 WebUI（8 文件，+491/-5），Phase 1 六项功能（1.1 对话导出 / 1.2 Drift 回滚 / 1.3 场景加角色 / 1.4 关系图谱 34 屏 / 1.5 情感状态 HUD / 1.6 Decompose/Analysis 入口）
> **结论**：**BLOCK —— 不通过，需修复 3 个阻塞项后重审**

---

## 0. 独立验证证据

| 验证项 | 方法 | 结果 |
|---|---|---|
| diff 内容 | `gh pr diff 314` 全量抓取（649 行） | ✓ 仅 `webui/` 8 文件，无 Rust/Engine 改动 |
| 端点契约 | 逐一比对 `engine/src` 路由、handler、请求/响应结构体 | ✓/✗ 见 §1 |
| screens 计数 | `ls webui/screens/*.html \| wc -l` | ✓ 34（测试 33→34 正确） |
| CSP 测试 | 独立运行 `node --test tests/runtime-pages.test.mjs` | ✓ 12/12 通过，新屏 34 无内联 `style=` |
| 关系数据形状 | 查 `engine/src/agent/tools/npc.rs` 与 `agent_rp_phase3.rs` 测试 | ✓ `live.json.relationships["A->B"]={type,intensity}` 与图谱解析兼容 |
| 角色枚举 | 读 `engine/src/scene.rs:10` | ✗ `CharacterRole` 仅 `primary`/`npc` |
| 导出窗口 | 读 `engine/src/domain.rs:221-280` | ✗ `history_window` clamp(1,200)，无 cursor 取尾部 |

---

## 1. Engine API 契约核查（不轻信 PR「复用已有 Engine API」声明，逐条查证）

| WebUI 调用（文件:行） | Engine 端点 | 形状核对 | 结论 |
|---|---|---|---|
| `GET …/state`（relationship-graph.js:534 / chat-space.js:111） | `GET /v1/characters/:id/state`（daemon/mod.rs:348） | 返回 `live.json`，含 `relationships` 矩阵 | ✓ 兼容 |
| `POST /v1/chat/history {limit:9999}`（chat-space.js:136） | `history_window`（domain.rs:221） | 返回 `HistoryWindow{messages,message_timestamps}`；`limit.clamp(1,200)` | ✗ 截断（**B3**） |
| `POST …/decompose`（console-runtime.js:189） | `DecomposeResponse{files_written:Vec<String>}`（decompose_handlers.rs:30） | `result.files_written` 字段存在 | ✓ |
| `GET …/analysis`（console-runtime.js:226） | `AnalysisFileList{files:Vec<AnalysisFileEntry>}`（decompose_handlers.rs:51） | 元素是**对象** `{filename,size}`，非字符串 | ✗（**B1**） |
| `GET …/analysis/:filename`（console-runtime.js:238） | `AnalysisFileContent{content}`（decompose_handlers.rs:63） | `file.content` 存在 | ✓ |
| `POST …/analysis/:filename {action}`（console-runtime.js:246-252） | `EnhanceApplyRequest{action,enhanced_md?}`（decompose_handlers.rs:84）→ `EnhancePreview{enhanced_md}` | 请求字段匹配；但 engine 当前 `enhanced_md` 为**占位原文**（decompose_handlers.rs:82 注释） | △ 请求正确，enhance 实质为 no-op（**N5**） |
| `POST /v1/scenes/:id/characters {role}`（console-runtime.js:489） | `AddCharacterBody{role:CharacterRole}`（scenes.rs:102） | `CharacterRole` 仅 `primary`/`npc` | ✗（**B2**） |
| `GET /v1/scenes` / `GET /v1/scenes/:id`（console-runtime.js:471-493） | `SceneConfig{characters:[{character_id,role}]}`（scene.rs:33） | `c.character_id` 字段存在 | ✓ |
| `PUT/GET …/drift` + `POST …/drift/rollback {revision}`（console-runtime.js:584-595） | `RollbackDriftRequest{revision:u64}`（style.rs:41） | `revision` 整数字段匹配 | ✓ |

---

## 2. 阻塞项（必须修复后重审）

### B1. Analysis 文件列表把对象当字符串处理 —— 列表与「查看/Enhance」全坏
`console-runtime.js:235`
```js
const files = list.files || [];
files.forEach(filename => {                       // ← filename 实为 {filename,size} 对象
  row.appendChild(node('div', 'runtime-row-title', filename));   // 渲染 "[object Object]"
  ...
  client.request('GET', '…/analysis/' + encodeURIComponent(filename));  // → /analysis/[object%20Object] 404
});
```
Engine 返回 `files: Vec<AnalysisFileEntry>`（每个元素 `{filename:String, size:u64}`，decompose_handlers.rs:57）。WebUI 把每个元素当**裸字符串**使用，导致：① 列表每行显示 `[object Object]`；②「查看」「Enhance」请求 URL 编码为 `[object Object]` → 404。Phase 1.6 的文件浏览功能完全不可用。

**修复**：`files.forEach(entry => { const filename = entry.filename; … })`，后续 `encodeURIComponent(filename)` 与 `node(..., filename)` 均用 `entry.filename`。

### B2. 场景添加角色：WebUI 角色词表与 Engine 枚举不匹配 —— 3/4 选项 400
`console-runtime.js:273` 下拉给 `main`/`supporting`/`npc`/`narrator`；但 Engine `CharacterRole`（scene.rs:10，`#[serde(rename_all="snake_case")]`）**只有 `Primary`→`primary`、`Npc`→`npc` 两个变体**。serde 默认拒绝未知变体 → 选 `main`/`supporting`/`narrator` 时 Engine 返回 400 反序列化错误，仅 `npc` 可用。Phase 1.3 场景加角色在多数选择下失败。

**修复**（PR 声明「无 Rust 改动」，故对齐 WebUI 到 Engine 契约）：下拉改为 `primary`/`npc`（标签「主角 / NPC」）；或若确需 supporting/narrator，应在 Engine 扩展枚举（超出本 PR 范围，需另立 PR 并补测试）。

### B3. 对话导出静默截断 —— 长对话丢消息（数据完整性）
`chat-space.js:136` 发送 `limit: 9999`；但 `history_window`（domain.rs:228）`limit.clamp(1,200)`，且**无 cursor 时返回尾部最近 limit 条**（domain.rs:274-279）。结果：会话消息 >200 条时，导出只保留**最近 200 条**、静默丢弃更旧消息。这对一个标称「对话导出（Markdown/JSON）」的功能是静默数据丢失，违背项目「不破坏用户资产」底线。

**修复**：导出走全量路径——不传 `limit`（get_chat_history 无 limit 时返回完整 `ChatLog`，同样含 `messages`/`message_timestamps`，chat.rs:42-45）；或在 UI 明确提示「仅导出最近 N 条」。推荐前者。

---

## 3. 非阻塞项（记录，建议后续迭代）

### N1. 设计令牌不一致（离线品牌色）
`relationship-graph.js:413` 画布节点硬编码 `#6366f1`（靛蓝），边框/文字硬编码 `#e67e22`/`#1e293b`；而 `relationship-graph.css:338` 图例点用 `var(--primary, #6366f1)`。项目权威主色为 `#C4663B`（暖陶土橙，见 golden sample `airp-engine-console`）。后果：图例点是陶土橙、画布节点是靛蓝，两者不一致且偏离品牌。建议画布颜色改从 CSS 变量/`getComputedStyle` 取，避免硬编码。

### N2. 导航双源 —— 关系图谱自绘 chrome
`relationship-graph.js:359` 重复实现一份 `pages` 导航数组与 `renderChrome()`，与 `console-runtime.js:26` 的导航同源但独立维护。新增页面若在 console-runtime 登记而遗漏 relationship-graph，图谱页导航即过时。建议抽取共享导航模块。

### N3. 拆分计划文档缺失 —— 可追溯性断点
PR 正文引用 `docs/progress/2026-07-25-split-PR313-plan.md`，但该文件在仓库中不存在（tracked/untracked 均无）。拆分理由与合入顺序无法溯源。建议补文档或移除引用。

### N4. 新功能逻辑无行为测试
`runtime-pages.test.mjs` 仅校验「文件数=34」与「无内联 `style=`」。233 行的力导向图（relationship-graph.js：物理模拟/canvas 绘制/拖拽）与 chat-space.js 95 行 HUD/导出逻辑**无任何行为测试**。PR 称「WebUI 测试 46/46 通过」不覆盖本次新增逻辑。建议补 jsdom/canvas 冒烟或契约测试。

### N5. Analysis「Enhance」当前为 no-op（Engine 侧限制）
`enhance_or_apply_character_analysis` 当前 `enhanced_md` 返回占位原文（decompose_handlers.rs:82），「应用 Enhance 结果」写回未变更内容。UI 处理正常（不报错），但 Enhance 实质无效，待 Engine 实现 LLM 增强。

### N6. 动画循环空转
`relationship-graph.js:tick()` 在 `simRunning=false` 后仍每帧 `requestAnimationFrame` 重绘，空闲时也持续占用。可在布局稳定后停止 rAF，交互时再启。

### N7. HUD 轮询错误被静默吞掉
`chat-space.js:113` `pollState` 的 `catch` 仅隐藏 HUD、无日志，调试困难。建议至少 `log` 错误。

---

## 4. 裁决

| 类 | 数量 | 项 | 处理 |
|---|---|---|---|
| 阻塞 | 3 | B1 Analysis 列表对象当字符串 / B2 场景角色枚举不匹配 / B3 导出静默截断 | **必须修复后重审** |
| 非阻塞 | 7 | N1–N7 | 记录，后续迭代 |

**最终建议：不通过（BLOCK）。** 三项阻塞均为真实合约/数据完整性缺陷且易修（均为前端 1–数行改动）。修复并重审通过后，方满足 AGENTS.md §11.1 审计门禁。

---

**审计独立性声明**：本报告未附和开发结论，独立比对了每个 WebUI 端点调用与 Engine 源（`engine/src/daemon/*`、`scene.rs`、`domain.rs`）的路由、请求体与响应结构体；独立运行 WebUI 测试验证 screen 计数与 CSP；独立阅读 `history_window` 实现确认截断行为。所有结论均来自可复现的源码/测试证据。
