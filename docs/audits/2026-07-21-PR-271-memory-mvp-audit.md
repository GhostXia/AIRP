# PR #271 独立审计报告 — 阶段二：记忆系统 MVP（Resident Memory）

- **审计源模型**：GLM-5.2（Trae IDE，按根 `AGENTS.md` §Audit Agent Charter 独立执行）
- **审计日期**：2026-07-21
- **审计 charter**：根 `AGENTS.md` §Audit Agent Charter（独立审计 / 可提己见 / 可质疑历史并查证）
- **PR**：[#271 feat: 阶段二 - 记忆系统 MVP (Resident Memory)](https://github.com/GhostXia/AIRP/pull/271)
- **分支**：`pr-271`
- **base**：`main`（mergeStateStatus: CLEAN，mergeable: MERGEABLE）
- **commits**：单 commit `2fd2eef feat: 阶段二 - 记忆系统 MVP`
- **diff 规模**：15 files changed, 871 insertions(+), 7 deletions(-)
- **审计执行方式**：独立审查 + 修复（用户指示 `独立审查+修复`），审计与修复在同一分支提交

## 0. 审计结论（裁决）

**条件通过（PASS with fixes）**：原 PR 存在 3 项阻塞问题（B1/B2/B3）与 4 项警告（W1/W2/W3/W4），
审计期间全部修复并补齐 19 个新测试。本地全测试套件通过：
- Rust lib：786 passed, 1 ignored（含 19 个审计新增测试）
- Rust integration：29 passed（agent_run 4 + main bin 4 + openai_compat 11 + production_startup 5 + sse_wiremock 5）
- protocol lib：6 passed
- ui main bin：9 passed
- WebUI：125 passed
- clippy --workspace --all-targets -D warnings：0 warnings

**门禁状态**：本地全绿。CI 与 CodeRabbit 仍需在 PR 上独立验证。审计阻塞意见已全部修复，
无遗留阻塞项。本审计报告随 PR 同分支提交、合并到 main，符合仓库惯例（见 AGENTS.md
"审计文件归档"立规）。

## 1. 范围与背景

PR #271 实现 RP 能力增强工程阶段二的 4 个子任务：

| 子任务 | 实现 |
|---|---|
| 2.1 常驻有界记忆 | `engine/src/memory/resident.rs`：每角色/每 session 一份 `resident.md`，默认 2000 字符上限，超限触发 LLM 压缩 |
| 2.2 自动事实抽取 | `engine/src/memory/extract.rs` + `chat_pipeline/finalize.rs`：finalize 后异步触发，控制平面 LLM 调用 |
| 2.3 用户模型学习 | `engine/src/memory/user_model.rs`：每用户一份 `user_model.md`，PR body 称"抽取逻辑预留" |
| 2.4 WebUI 可见性 | `webui/app.js` + `index.html` + `style.css`：工作台"记忆"tab，手动编辑/保存 |

新增 API：`GET/PUT /v1/memory/resident`、`GET/PUT /v1/memory/user-model`。

## 2. 独立证据

### 2.1 PR diff 概览

| 文件 | +/- | 审计关注点 |
|---|---|---|
| `engine/src/memory/resident.rs` | +155 | 路径安全、原子写、容量语义 |
| `engine/src/memory/extract.rs` | +122 | 异步任务治理、错误传播 |
| `engine/src/memory/compress.rs` | +85 | 清理逻辑健壮性 |
| `engine/src/memory/user_model.rs` | +131 | 路径安全、死代码 |
| `engine/src/memory/mod.rs` | +23 | 模块导出 |
| `engine/src/daemon/handlers/memory.rs` | +149 | 输入校验、body limit |
| `engine/src/daemon/mod.rs` | +22 | 路由表、body limit |
| `engine/src/daemon/handlers.rs` | +4 | 模块声明 |
| `engine/src/chat_pipeline/finalize.rs` | +100 | 异步抽取任务 spawn、clippy |
| `engine/src/chat_pipeline/prepare.rs` | +13 | resident memory 注入点 |
| `engine/src/chat_pipeline/prepare_scene.rs` | +13 | resident memory 注入点（场景） |
| `engine/src/lib.rs` | +1 | `pub mod memory;` |
| `webui/app.js` | +56 | dirty 状态、char count 一致性、切换重载 |
| `webui/index.html` | +3 | tab 结构 |
| `webui/style.css` | +1 | tab 样式 |

### 2.2 阻塞问题（B）

#### B1（阻塞）：`user_id` 使用 `String` 而非 `UserId` newtype，路径遍历漏洞

**位置**：`engine/src/daemon/handlers/memory.rs` 原 `UserModelQuery` / `UpdateUserModelRequest`

**原代码**：
```rust
#[derive(Debug, Deserialize)]
pub struct UserModelQuery {
    pub user_id: String,  // ← 无校验
}
#[derive(Debug, Deserialize)]
pub struct UpdateUserModelRequest {
    pub user_id: String,  // ← 无校验
    pub content: String,
}
```

**问题**：`user_id: String` 直接拼入 `data_root.join("users").join(user_id).join("user_model.md")`，
未走 `validate_id_segment`。攻击者发送 `user_id=../../../etc/passwd` 可路径遍历。
PR #271 的 `character_id` 走 `CharacterId::new` 显式校验（handler 内），但 `user_id` 漏网。

**对照项目硬约束**（AGENTS.md）：
> URL handling must include null byte interception and path.resolve + path.relative to prevent path traversal

**修复**：将 `user_id: String` 改为 `user_id: UserId` newtype。`UserId` 在 serde 反序列化
路径强制 `validate_id_segment`，拒绝 `..` / `a/b` / 含空字节等。`read_user_model` /
`write_user_model` 同步改为 `&UserId` 参数。

**验证**：新增 2 个集成测试覆盖 GET（Query 提取器返回 400）与 PUT（Json 提取器返回 422）路径。

#### B2（阻塞）：PUT /v1/memory/* 端点缺少 `DefaultBodyLimit`，DoS 风险

**位置**：`engine/src/daemon/mod.rs` 路由表

**原代码**：未对 `update_resident_memory` / `update_user_model` 配置 body limit。

**问题**：`content: String` 字段无上限，攻击者可发送 GB 级 body 耗尽内存。

**对照项目硬约束**（AGENTS.md）：
> PUT endpoints must have body limit configured (2MB) to prevent DoS attacks

**修复**：在路由表对两个 PUT 端点加 `DefaultBodyLimit::max(2 * 1024 * 1024)`，与
`/v1/characters/:character_id` PUT（2MB）和 lorebook PUT（2MB）一致。

**验证**：新增 2 个集成测试，发送 2MB+64B body，断言返回 413 Payload Too Large。

#### B3（阻塞）：`user_model.rs` 死代码误导——`inject_user_model` / `append_user_model` /
`USER_PREFERENCE_EXTRACTION_PROMPT` 全程无人调用，且 prepare 路径未接入

**位置**：`engine/src/memory/user_model.rs` + `engine/src/memory/mod.rs`

**原代码**：`mod.rs` 导出 `inject_user_model` / `append_user_model` /
`USER_PREFERENCE_EXTRACTION_PROMPT`，但：
- `chat_pipeline/prepare.rs` 与 `prepare_scene.rs` 均未调用 `inject_user_model`
- `finalize.rs` 未调用 `append_user_model`
- 全仓 grep 无任何调用点

**问题**：PR body 称"2.3 用户模型自动学习 - 抽取逻辑预留"，但代码呈现的是"已就绪的注入 API"，
误导后续开发者以为 prepare 路径已接入。死代码 + 误导性导出违反"更开放、更透明、在未来更易修正"
的项目取向（AGENTS.md 项目取向）。

**修复**：删除 `inject_user_model` / `append_user_model` / `USER_PREFERENCE_EXTRACTION_PROMPT`
三个死函数/常量。在 `mod.rs` 模块级 doc 新增 `## PR #271 审计修复（B3）` 小节明确说明：
"MVP 范围内只做手动编辑，相关死代码已删除，待后续 PR 真正接入抽取/注入时再加回。"

### 2.3 警告（W）

#### W1：`write_resident_memory` / `write_user_model` 非原子写，崩溃可留损坏文件

**位置**：`engine/src/memory/resident.rs`、`engine/src/memory/user_model.rs`

**原代码**：`fs::write(&path, content)` 直接覆盖目标文件。若进程在 write 中途崩溃
（断电、OOM、panic），目标文件可能被截断或留半截内容，下轮读取即读到损坏数据。

**修复**：改用 `crate::data_dir::replace_file`（temp + rename + parent fsync），
与项目其他原子写路径（settings、character card、preset 等）一致。

**验证**：新增 2 个测试 `test_write_is_atomic_no_residue` 与
`user_model_write_leaves_no_temp_or_bak_residue`，断言写入后目录只有目标文件，
无 `.tmp` / `.bak` 残留。

#### W2：WebUI 记忆 tab 缺失 dirty 状态、char count 不一致、切换不重载

**位置**：`webui/app.js`

**原代码 3 个子问题**：
1. 切换 character / session 时，记忆 tab 不重载内容，显示的是上一个会话的 resident.md
2. 编辑后无 dirty 标记，用户切换 tab 即丢失未保存内容
3. char count 用 `string.length`（UTF-16 code unit 计数），与 Rust 端 `chars().count()`
   （Unicode scalar value 计数）不一致——对 emoji / 罕用 CJK 扩展字符会偏差

**修复**：
- 新增 `wbMemoryDirty` 状态 + `setMemoryDirty()` 函数，编辑后显示 `●` 标记
- 新增 `memoryTabVisible()` helper
- 新增 `codePointCount()` 使用 `Array.from(s).length`，与 Rust `chars().count()` 对齐
- 在 `charSelect` change 与 `sessSelect` change handler 中，当记忆 tab 可见时调用
  `loadResidentMemory()` 重载

#### W3：`compress.rs` 清理逻辑过激——剥除续行 + 非 bullet 输入返回空字符串

**位置**：`engine/src/memory/compress.rs` 原 `compress_resident_memory` 内联清理

**原代码**：只保留以 `- ` 开头的行，丢弃所有续行（bullet 后缩进的非空行）；
当 LLM 返回纯段落（无 bullet）时，清理结果为空字符串，触发"compression failed"误报。

**修复**：抽出 `cleanup_compression_output` 为 `pub(crate)` 函数：
- 保留 `- ` 开头的 bullet 行
- 保留 bullet 后的空行（段落分隔）
- 保留 bullet 后的缩进续行（多行 bullet 内容）
- 若清理结果为空（无 bullet），回退到 `raw.trim()`，避免误报"compression failed"

**验证**：新增 6 个单测覆盖 bullet / 续行 / fallback / 空输入 / 空行 / 混合。

#### W4：`finalize.rs:144` clippy `clone_on_copy`——`Option<SessionId>` 是 `Copy`

**位置**：`engine/src/chat_pipeline/finalize.rs:144`

**原代码**：`let session_id = ctx.session_id.clone();`，但 `SessionId` 实现 `Copy`，
`Option<SessionId>` 也是 `Copy`，`.clone()` 冗余。

**修复**：改为 `let session_id = ctx.session_id;`。此问题由 PR #271 新增代码引入
（git blame 确认 `2fd2eef4`），不是历史遗留。

### 2.4 Frozen Snapshot 语义验证

PR body 称"Frozen Snapshot 语义：本轮抽取落盘 → 下轮 prepare 阶段才注入 prompt（防模型自反应）"。

独立验证：
- `finalize.rs` 在本轮 assistant 输出清理完成后 spawn 异步任务调用 `extract_facts` →
  `append_resident_memory`，落盘到 `session_dir/resident.md`
- `prepare.rs` 与 `prepare_scene.rs` 在下一轮构造 prompt 时调用 `inject_resident_memory`
  读取同一文件
- 时序正确：抽取与注入不在同一轮，模型本轮看不到自己被抽取的事实，满足"防自反应"语义

### 2.5 异步任务治理验证

`finalize.rs` 使用 `tokio::task::JoinSet` 管理 maintenance / extraction 异步任务：
- `join_set.spawn(...)` 结构化并发，finalize 返回时 JoinSet drop 即取消未完成任务
- 抽取失败 `tracing::error!` 但不影响主对话流（best-effort 语义符合 PR body）
- 维护任务（volume_manager::run_maintenance）同样 best-effort

治理合理，无阻塞问题。

## 3. 独立意见（按 §Audit Agent Charter 第 2 条）

### 3.1 关于"用户模型学习"子任务的范围收窄

PR body 子任务 2.3 原文："每用户一份 `data/users/{uid}/user_model.md`，提供
read/write/inject API（抽取逻辑预留）"。实际交付只有 read/write，inject 是死代码（B3）。

**独立意见**：MVP 收窄到"手动编辑 user_model.md"是合理选择——自动抽取用户偏好需要
长期对话样本积累，且抽取 prompt 设计需要单独验证。但 PR body 的"提供 inject API"措辞
与实际交付不符，应明确为"仅 read/write API，inject 留待后续 PR"。B3 修复已在 `mod.rs`
doc 中明确此点。

### 3.2 关于 `auto_extract` 字段的移除

原 `ResidentMemoryConfig` 有 `auto_extract: bool` 字段，但全程无读取点——抽取触发逻辑
硬编码在 `finalize.rs` 的 `if let Some(ref cid) = ctx.character_id` 分支，不读 config。

**独立意见**：未接入的 config 字段是"未来扩展占位"，但违反"不在当前任务加未来扩展"
的工程惯例（AGENTS.md Engineering Conventions 隐含）。B3 修复中一并删除 `auto_extract`
字段，待真正需要可配置抽取开关时再加回。

### 3.3 关于 `compress.rs` 的 LLM 调用健壮性

`compress_resident_memory` 调用 LLM 合并压缩，但未对 LLM 返回内容做长度校验——理论上
LLM 可能返回比原内容更长的结果（"压缩"反而膨胀）。当前实现直接落盘，下轮读取时若超限
会再次触发压缩，形成"压缩-膨胀-再压缩"循环。

**独立意见（非阻塞，留后续）**：建议后续 PR 在 `compress_resident_memory` 返回前校验
`result.chars().count() <= capacity`，若膨胀则保留原内容并 `tracing::warn!`。
本 PR 范围内不修，避免扩大改动面。

### 3.4 关于抽取 prompt 的版本化

`extract.rs` 的 `EXTRACTION_PROMPT` 是硬编码常量。未来若需迭代抽取 prompt
（例如调整抽取粒度、增加 few-shot），需要修改源码并重新部署。

**独立意见（非阻塞，留后续）**：建议后续 PR 将抽取 prompt 外置为
`data/prompts/extraction.md` 或 settings 字段，与角色卡 prompt 同级管理。本 PR 范围内不修。

## 4. 测试覆盖

### 4.1 PR 原有测试

PR #271 原 commit 声称"全部 769 个测试通过"，但未新增 memory 模块的 route-level 集成测试
（`engine/src/daemon/tests/` 下无 `memory.rs`）。memory 模块只有 `resident.rs` /
`user_model.rs` 内部的少量单测，`compress.rs` 无单测。

### 4.2 审计新增测试（19 个）

| 文件 | 新增测试 | 覆盖 |
|---|---|---|
| `engine/src/memory/resident.rs` | `test_write_is_atomic_no_residue` | W1 原子写无残留 |
| `engine/src/memory/user_model.rs` | path traversal rejection + atomic write no residue | B1 + W1 |
| `engine/src/memory/compress.rs` | 6 个 `cleanup_compression_output` 单测 | W3 清理逻辑 |
| `engine/src/daemon/tests/memory.rs` | 10 个 route-level 集成测试 | B1 / B2 / W1 / roundtrip |

`engine/src/daemon/tests/memory.rs` 是新文件，已在 `engine/src/daemon/tests/mod.rs` 注册
`mod memory;`。

### 4.3 测试结果

```
cargo test --workspace
- airp-core lib: 786 passed, 1 ignored
- airp-core main bin: 4 passed
- agent_run integration: 4 passed
- openai_compat integration: 11 passed
- production_startup integration: 5 passed
- sse_wiremock integration: 5 passed
- airp-state-protocol lib: 6 passed
- airp-ui main bin: 9 passed

cargo clippy --workspace --all-targets -- -D warnings
- 0 warnings

webui node --test
- 125 passed, 0 failed
```

## 5. 修复文件清单

审计修复涉及的文件（均在 `pr-271` 分支，随本审计报告同分支提交）：

| 文件 | 修复项 |
|---|---|
| `engine/src/memory/mod.rs` | B3 删除死代码导出 + 模块 doc 说明 |
| `engine/src/memory/resident.rs` | W1 原子写 + 删除 `auto_extract` 字段 + 新增原子写测试 |
| `engine/src/memory/user_model.rs` | B1 `UserId` newtype + W1 原子写 + B3 删除死代码 + 新增测试 |
| `engine/src/memory/compress.rs` | W3 抽出 `cleanup_compression_output` + 6 个单测 |
| `engine/src/daemon/handlers/memory.rs` | B1 `UserId` newtype |
| `engine/src/daemon/mod.rs` | B2 `DefaultBodyLimit::max(2MB)` |
| `engine/src/daemon/tests/mod.rs` | 注册 `mod memory;` |
| `engine/src/daemon/tests/memory.rs` | 新增 10 个集成测试 |
| `engine/src/chat_pipeline/finalize.rs` | W4 clippy `clone_on_copy` 修复 |
| `webui/app.js` | W2 dirty 状态 + char count 一致性 + 切换重载 |

## 6. 门禁裁决

- **审计阻塞项**：B1 / B2 / B3 全部修复，无遗留
- **审计警告项**：W1 / W2 / W3 / W4 全部修复，无遗留
- **本地测试**：全绿（lib 786 + integration 29 + protocol 6 + ui 9 + webui 125 + clippy 0）
- **CI / CodeRabbit**：需在 PR 上独立验证（审计不替代 CI）

**裁决：条件通过（PASS with fixes）**。审计阻塞意见已全部修复并补齐测试，
本地全测试套件通过。待 CI 与 CodeRabbit 在 PR 上独立验证通过后，可由人工 review
决定是否合并。

## 7. 后续跟进（非阻塞，不要求本 PR 修）

| 编号 | 内容 | 建议时机 |
|---|---|---|
| F-1 | `compress_resident_memory` 增加结果长度校验，防止"压缩膨胀"循环 | 下一个记忆系统迭代 PR |
| F-2 | 抽取 prompt 外置为 `data/prompts/extraction.md` 或 settings 字段 | 下一个记忆系统迭代 PR |
| F-3 | 真正接入用户模型自动抽取/注入时，重新加回 `inject_user_model` / `append_user_model`（B3 删除的死代码） | 阶段三用户模型学习 PR |
| F-4 | 考虑为 `ResidentMemoryConfig` 加 `capacity_chars` 的 settings 热重载支持 | 低优先级，按需 |
