# 对话体验升级：消息操作与流式生成

> **计划建立时间**：2026-07-19
>
> **审计/规划模型**：Qwen3.8-Max-Preview
>
> **参考来源**：SillyTavern (github.com/SillyTavern/SillyTavern) 公开文档与发布记录（仅作为 RP 用户真实需求的佐证，不作为 AIRP 产品目标）
>
> **基线**：AIRP v0.0.1 (`main@4ac03e3`)

## 背景与需求论证

v0.0.1 已发布，首聊黄金路径已验证。根据 CURRENT-BASELINE 第 3 步和审计 #242，当前最高优先级是提升对话体验。

AIRP 用户在完成首聊后，日常 RP 工作流需要以下消息交互能力：
- 对不满意的 AI 回复重新生成（无需手动重发用户消息）
- 被 token limit 截断的回复能继续生成
- 编辑写错的用户消息
- 删除不想要的单条消息

这些是 RP 用户完成日常对话的基本操作，参考 ST 公开行为确认属于真实高频需求。AIRP 使用自己的 domain model、稳定 ID、session 合同和 SSE 流式架构独立实现。

**RP 用户日常消息交互需求（参考 ST 公开行为确认频率）：**
1. Swipes — 多候选切换（箭头按钮循环 AI 候选回复）
2. Continue — 继续生成（扩展截断回复；空消息=继续）
3. Message Edit — 编辑消息（"保存"和"保存并重新生成"是分开操作）
4. Message Delete — 删除单条消息（不是 rollback 删除后续所有）
5. Smooth Streaming — 可配置 FPS 逐字显示（默认 30 t/s）
6. Auto-Continue — 响应达到 token limit 时自动继续
7. Auto-load Last Chat — 启动时自动加载最近对话
8. Chat Export — JSONL/TXT 导出
9. Checkpoints/Branches — 从任意消息创建分支

## 本次 PR 范围（当前真实需要）

### Engine 改动

#### 1. 扩展 PrepareMode，支持 Regen 和 Continue

文件：`engine/src/chat_pipeline.rs`

- 在 `PrepareMode` enum 增加 `Regen` 和 `Continue` 变体
- `Regen` 模式：
  - 不追加用户消息到 message list（history 已包含）
  - 不持久化用户消息（步骤 13 跳过）
  - 不推进 timeline/checkpoint
  - 仍正常持久化 assistant 响应（finalizer 不变）
- `Continue` 模式：
  - 不追加用户消息
  - 不持久化用户消息
  - finalizer 需要特殊处理：将新文本**追加**到已有 assistant 消息，而非新建

#### 2. 修改 `/v1/chat/regen` 为 SSE 流式端点

文件：`engine/src/daemon/handlers/chat.rs`、`engine/src/daemon/types.rs`

- `RegenRequest` 仅增加 `user_id: Option<String>`（用于 per-user 数据隔离）。
  provider/model 等配置直接读 daemon MutableConfig，不在请求体重复暴露
  （AIRP 取舍：单用户本地场景，regen 用当前活跃配置即可，无需请求级覆盖）。
- handler 改为：
  1. 调用 `ChatService::regen()` 删除最后一条 assistant 消息
  2. 构造内部 `ChatCompletionRequest`（message 为空字符串，`messages_history` 由 pipeline 自动加载）
  3. 调用 `prepare_pipeline_with_mode(payload, state, PrepareMode::Regen)`
  4. 返回 `Sse<...>`（与 chat_completion 相同的流式响应）
- 路由签名从 `Json<ChatLog>` 改为 `Sse<...>`

#### 3. 新增 `/v1/chat/continue` 端点

文件：`engine/src/daemon/handlers/chat.rs`、`engine/src/daemon/mod.rs`、`engine/src/daemon/types.rs`

- 新增 `ContinueRequest`：`character_id`、`session_id`、`user_id`（最小集）
- handler 流程：
  1. 加载 history，确认最后一条是 assistant 消息
  2. 用 `PrepareMode::Continue` 构建 pipeline（history 作为 messages，不追加 user message）
  3. 流式生成
  4. finalizer 将新文本追加到已有 assistant 消息（修改 `run_finalize` 或新增 `run_finalize_continue`）
- 注册路由 `POST /v1/chat/continue`

#### 4. ChatService 增加 `append_to_last` 和 `delete_message` 方法

文件：`engine/src/domain.rs`、`engine/src/chat_store.rs`

- `ChatService::append_to_last(character_id, session_id, text)` — 将文本追加到 ChatLog 最后一条 assistant 消息的 content 末尾，并持久化。用于 continue 的 finalizer。
- `ChatService::delete_message(character_id, session_id, message_id)` — 删除指定单条消息，保留其余消息顺序不变。（AIRP 取舍：删除是移除单条，不是 rollback 删除后续所有，保护用户已有对话上下文。）

#### 5. 测试

- 在 `engine/src/daemon/tests/` 增加 regen SSE 和 continue 的合同测试
- 验证 regen 删除旧消息 + 流式返回新消息
- 验证 continue 追加到已有消息
- 验证 delete_message 只移除目标消息
- 现有 756 lib tests + 25 integration tests 保持全绿

### WebUI 改动

#### 6. Per-message 操作按钮（三点菜单）

文件：`webui/app.js`、`webui/style.css`

- 修改 `appendMsg()`：为每条消息添加 hover 操作栏（`.msg-actions`）
  - user 消息：编辑、删除
  - assistant 消息：重新生成（仅最后一条）、继续（仅最后一条）、删除
- 操作栏默认隐藏，hover/focus 时显示
- CSS：`.msg-actions { opacity:0; } .msg:hover .msg-actions { opacity:1; }`

#### 7. Auto-Regen 交互

文件：`webui/app.js`

- 修改 `btnRegen` 点击处理：调用新 `/v1/chat/regen`（SSE），复用现有 `streamSse()` 渲染流式响应
- 移除旧的"删除后手动重发"逻辑和 confirm 弹窗（AIRP 取舍：rollback-by-ID 已覆盖误操作风险，确认弹窗只增加摩擦）

#### 8. Continue 交互

文件：`webui/app.js`

- 最后一条 assistant 消息显示"继续"按钮
- 点击后调用 `/v1/chat/continue`（SSE），流式追加到当前消息 DOM
- 空消息发送 = 继续（AIRP 取舍：显式继续按钮已存在，空发送是低摩擦快捷路径，误触发可由 rollback 恢复）

#### 9. 消息编辑（保存 ≠ 重新生成）

文件：`webui/app.js`

- 点击 user 消息的“编辑”按钮 → 消息文本变为 textarea
- **保存**：当前为 local-only（仅更新 DOM，刷新后丢失）。持久化需 engine 后续提供消息更新端点，作为 follow-up 工作
- **保存并重新生成**（可选按钮）：保存后调用 rollback 到该消息之前 + regen SSE
- 取消编辑 → 恢复原文
- （AIRP 取舍：编辑保存与重新生成是分离操作，编辑不自动触发 regen，避免意外覆盖后续对话）

#### 10. 消息删除（单条）

文件：`webui/app.js`

- 调用 `DELETE /v1/chat/message`（或 `POST /v1/chat/delete`）删除单条消息
- 删除后 DOM 移除该消息节点，其余消息保持顺序
- 可选确认弹窗（ST 有 "Confirm message deletion" 开关）
- （AIRP 取舍：删除是移除单条消息，不是 rollback 删除后续所有，保护用户已有上下文）

#### 11. WebUI 测试

文件：`webui/tests/`

- 更新 `smoke.mjs`：覆盖 regen SSE、continue、delete_message 端点
- 现有 97 tests 保持全绿

## 执行顺序

1. Engine: PrepareMode 扩展 + regen SSE + continue 端点 + append_to_last + delete_message（步骤 1-4）
2. Engine: 测试验证（步骤 5）
3. WebUI: per-message actions + auto-regen + continue + 空消息继续（步骤 6-8）
4. WebUI: 编辑 + 删除（步骤 9-10）
5. WebUI: 测试（步骤 11）
6. 全量验证矩阵：cargo fmt/clippy/test + node --test + 神圣不变式

## 新增 Issue（后续 RP 用户真实需求，当前 PR 不含但需跟踪）

以下功能经参考 ST 公开行为确认为 RP 用户真实日常需求，但范围超出本次 PR，应创建 GitHub issue 跟踪：

### Issue A: Swipe（多候选响应）— 高优先

ST 最标志性功能。每条 assistant 消息存储多个候选回复，用户通过左右箭头循环切换。
- Engine: 消息存储需支持 candidates 数组（当前 ChatLog 每条消息只有单一 content）
- WebUI: 箭头按钮 + 候选计数器（如 "2/5"）
- 这是 AIRP 相对 ST 最明显的体验缺口，应在本次 PR 之后立即推进

### Issue B: Smooth Streaming（平滑流式输出）— 高优先

ST 默认 30 FPS 逐字显示，当前 AIRP 是 chunk 级更新（每收到一个 SSE chunk 立即渲染）。
- WebUI: 用 requestAnimationFrame + 字符队列实现可配置 FPS 的平滑输出
- 纯前端改动，无需 engine 变更
- 对感知质量影响极大

### Issue C: Auto-Continue（自动继续）— 中优先

当响应因 max_tokens 截断时自动触发 continue。
- Engine: SSE done 事件需携带 stop_reason（`end_turn` vs `max_tokens`）
- WebUI: 检测 stop_reason=max_tokens 时自动调用 /v1/chat/continue
- ST 有 "Target length (tokens)" 配置

### Issue D: Auto-load Last Chat（启动自动加载最近对话）— 中优先

ST 默认启动时加载最近对话。当前 AIRP 每次刷新需手动选择角色和会话。
- WebUI: localStorage 记住上次选中的 character_id + session_id，启动时自动恢复
- 纯前端改动，trivial 但 UX 影响大

### Issue E: Chat Export（JSONL/TXT 导出）— 中优先

ST 支持 JSONL（含 metadata 可重导入）和 TXT（纯文本不可重导入）导出。
- Engine: `GET /v1/chat/export?format=jsonl|txt` 端点
- WebUI: 下载按钮
- 数据可移植性是用户信任基础

### Issue F: Impersonate（角色代入）— 低优先

ST 的 "Quick Impersonate" 按钮让用户写一条角色视角的消息（不让 AI 生成）。
- 本质是以 assistant role 手动插入一条用户写的消息
- Engine: `POST /v1/chat/impersonate`（接受文本，以 assistant role 持久化）
- 对 RP 写作型用户有价值，但非高频

### Issue G: Checkpoints/Branches（对话分支）— 低优先

ST 支持从任意消息创建分支（clone 到该消息为止的 history）。
- Engine: `POST /v1/chat/branch`（从指定 message_id 创建新 session，复制 history）
- 高级功能，当前 rollback 已覆盖基本需求

## 不做的事（P1 取舍）

- 不新增 asset revision 类型
- 不修改 Agent Run
- 不触碰 production topology
- 不扩展 dep-governance 工具链
- 不做 Visual Novel Mode、TTS、图片生成等非文本核心功能
