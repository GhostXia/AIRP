# AIRP 新功能开发路线图 — 进度记录

**记录时间**：2026-07-25
**分支**：`feat/roadmap-phase1-quick-wins`
**PR**：#313（持续追加，未另起 PR）
**基线**：`main@d53acd1`（2026-07-24，Engine 27 工具 + WebUI 33 屏）
**路线图来源**：`AIRP_新功能开发路线图_task-58d.md`（仓库根目录）

---

## 总体进度

| 阶段 | 范围 | 状态 |
|---|---|---|
| Phase 1 | 快速见效（6 项） | ✅ 完成（commit `6e5fc0d`） |
| Phase 2 | Agent 智能化（6 项） | ✅ 完成（commits `17b4744` → `d293fc3`） |
| Phase 3 | RP 沉浸体验（6 项） | ✅ 完成（commits `a49928b`、`23e9716`、`8ce8881`） |
| Phase 4 | 创作工具（6 项） | ✅ 完成（commits `8ce8881` → `08871d2`） |
| Phase 5 | 平台化（7 项） | 🚧 进行中（5.1 / 5.2 / 5.3 完成；5.4 进行中；5.5–5.7 待办） |

**累计提交**（本 PR 上）：
- Phase 1 → Phase 5.3 共 12 个 feat commit 已推送至远端
- Phase 5.4 工作目录未提交（`engine/src/mcp_integration.rs` 新增 + `Cargo.toml` / `Cargo.lock` / `engine/src/lib.rs` 改动）

---

## 各 Phase 明细

### Phase 1: 快速见效 — ✅ 完成

提交：`6e5fc0d feat(webui): Phase 1 快速见效 — 6 项功能补全`

| # | 功能 | 状态 |
|---|---|---|
| 1.1 | 对话导出（Markdown/JSON） | ✅ |
| 1.2 | Drift 回滚按钮 | ✅ |
| 1.3 | 场景添加角色 UI | ✅ |
| 1.4 | 角色关系图谱 | ✅ |
| 1.5 | 角色情感状态 HUD | ✅ |
| 1.6 | Decompose/Analysis 入口 | ✅ |

### Phase 2: Agent 智能化 — ✅ 完成

| # | 功能 | 状态 | 关键提交 |
|---|---|---|---|
| 2.1 | 导演 Agent 编排 | ✅ | `e155223` |
| 2.2 | NPC 自主行动轮（UI） | ✅ | `4b8fb39` |
| 2.3 | 世界时钟与定时事件 | ✅ | `17b4744` |
| 2.4 | 剧情弧编辑器（Engine + UI） | ✅ | `ebdd3af` / `d342636` |
| 2.5 | 长期记忆遗忘曲线 | ✅ | `251093e` |
| 2.6 | 多 Agent 辩论/会议模式 | ✅ | `d293fc3` |

### Phase 3: RP 沉浸体验 — ✅ 完成

| # | 功能 | 状态 | 关键提交 |
|---|---|---|---|
| 3.1 | 多角色群聊 UI | ✅ | `23e9716` |
| 3.2 | TTS 朗读 | ✅ | `a49928b` |
| 3.3 | 场景插图生成 | ✅ | `8ce8881` |
| 3.4 | 氛围 BGM 建议 | ✅ | `a49928b` |
| 3.5 | 打字机 + 表情动画 | ✅ | `a49928b` |
| 3.6 | 对话片段分享卡片 | ✅ | `a49928b` |

### Phase 4: 创作工具 — ✅ 完成

| # | 功能 | 状态 | 关键提交 |
|---|---|---|---|
| 4.1 | 角色卡模板库 | ✅ | `8ce8881` |
| 4.2 | 风格迁移（style learn） | ✅ | `dc8383e` |
| 4.3 | 对话示例生成器 | ✅ | `48f1000` |
| 4.4 | 世界书知识图谱 | ✅ | `9475a63` |
| 4.5 | 剧情时间线导出 | ✅ | `b0623b1` |
| 4.6 | 角色卡版本对比 | ✅ | `08871d2` |

### Phase 5: 平台化与技术扩展 — 🚧 进行中

| # | 功能 | 状态 | 关键提交 |
|---|---|---|---|
| 5.1 | 多 Provider 路由 | ✅ | `8010abb` |
| 5.2 | 本地模型支持（Ollama） | ✅ | `60ad7cc` |
| 5.3 | 插件/自定义工具 | ✅ | `3f72360` |
| 5.4 | MCP 服务器集成 | 🚧 进行中（未提交） | — |
| 5.5 | 多语言 UI（i18n） | ⏳ 待办 | — |
| 5.6 | 自动备份/恢复 | ⏳ 待办 | — |
| 5.7 | 跨设备同步（WebDAV/S3） | ⏳ 待办 | — |

---

## Phase 5.4 MCP 服务器集成 — 当前状态

### 已完成

1. **依赖与特性配置**（`Cargo.toml`）
   - rmcp 升级到 1.7+（实际拉到 1.8.0）
   - 启用 features：`server`、`client`、`transport-io`、`transport-child-process`、`transport-streamable-http-client`、`transport-streamable-http-client-reqwest`、`macros`、`transport-streamable-http-server`、`transport-streamable-http-server-session`

2. **模块注册**（`engine/src/lib.rs`）
   - 已添加 `pub mod mcp_integration;`

3. **核心模块**（`engine/src/mcp_integration.rs`，约 37 KB，~1000 行）
   - **配置模型**
     - `McpTransportConfig`（`Stdio` / `Http` 两个变体，带 `timeout_secs`）
     - `McpServerConfig`（name + description + enabled + transport + env）
     - `validate_server_name`：`^[a-z0-9_]{1,64}$`，首字符不能为数字
     - `validate_command_path`：绝对路径 + canonicalize + 拒绝 PATH 查找/null byte/相对路径
     - `validate_http_url`：https 任意 host 或 http loopback；拒绝 userinfo
   - **持久化**
     - `load_mcp_servers` / `save_mcp_servers`
     - 配置与 env vars 分离存储：`mcp_servers.json` + `mcp_server_env.json`
     - env vars 用 `skip_serializing`，避免分享配置时泄露密钥
   - **运行时**
     - `McpServerRuntime`：持有 `Mutex<Option<RunningService<RoleClient, ()>>>` + `cached_tools: Mutex<Vec<CachedToolMeta>>`
     - `connect` / `disconnect` / `call_tool` / `cached_tools` / `is_connected`
     - `call_tool` 自动重连一次（连接断开时）
   - **工具包装**
     - `McpToolWrapper` 实现 `Tool` trait
     - 注册名格式：`mcp_<server>_<tool>`（避免不同 server 同名工具冲突）
     - `Box::leak` 把 String 转 `&'static str`（与 PluginTool 一致）
   - **测试**：18+ 个单元测试，覆盖校验、持久化 roundtrip、命名空间、wrapper meta 等

### 当前阻塞

**编译错误（未解决）**：在 `spawn_connection` 函数中调用 `().serve(child).await` 报：

```
error[E0599]: no method named `serve` found for unit type `()` in the current scope
```

### 已确认的 API 路径

通过查阅 `rmcp-1.8.0/src/service/client.rs` 源码：

- `serve_client<S, T, E, A>(service: S, transport: T) -> Result<RunningService<RoleClient, S>, ClientInitializeError>`
- `serve_client_with_ct<S, T, E, A>(service, transport, ct)`
- `ServiceExt<RoleClient>::serve_with_ct(self, transport, ct)` — trait 方法

`ClientInfo` 实际是 `pub type ClientInfo = InitializeRequestParams`（`model.rs:900`），其字段结构：
```rust
ClientInfo {
    meta: None,
    protocol_version: ProtocolVersion::default(),
    capabilities: ClientCapabilities::default(),
    client_info: Implementation,  // 注意：不是 server_info
}
```

**注意**：当前代码中用了 `ImplementationData` 和 `server_info` 字段名 — 这是错误的，需要改为 `Implementation` 和 `client_info`。

### 待修复项

1. **修复 `spawn_connection`**
   - 将 `().serve(child)` 替换为 `serve_client((), child)` 或 `().serve_with_ct(child, ct)`
   - `()` 实现了 `Service<RoleClient>`（`ClientHandler for ClientInfo` + blanket impl），用 `serve_client` 即可
   - 自定义 `client_info` 的版本：需要构造 `ClientInfo` 并调用 `serve_client(client_info, transport)`

2. **修复 `ImplementationData` → `Implementation`**
   - 当前 `use rmcp::model::{CallToolRequestParam, ClientInfo, Implementation};` 已正确
   - 但 `spawn_connection` 内仍使用 `ImplementationData { ... server_info: ... }` — 需改为 `Implementation { name, version }` 并放在 `client_info` 字段下

3. **修复 stdio transport 的 serve 调用**
   - `().serve(child).await` → `serve_client((), child).await` 或 `(client_info).serve_with_ct(child, Default::default()).await`

4. **修复 http transport 的 serve 调用**
   - `client_info.serve(transport).await` → `serve_client(client_info, transport).await`

### 后续工作（Phase 5.4 内部）

- [ ] 修复上述编译错误
- [ ] 集成到 `daemon`：
  - 启动时 `load_mcp_servers` → 后台 task 并发 `connect` 所有 enabled server
  - `build_registry` 时遍历 `McpServerRuntime`，为每个 cached tool 注册 `McpToolWrapper`
- [ ] HTTP 端点（`engine/src/daemon/handlers/mcp_servers.rs`）：
  - `GET /v1/mcp-servers` — 列出所有 server 配置 + 连接状态
  - `POST /v1/mcp-servers` — 新增/更新 server 配置
  - `DELETE /v1/mcp-servers/:name` — 删除 server
  - `POST /v1/mcp-servers/:name/test` — 触发重连 + list_all_tools
- [ ] WebUI 页面（`webui/screens/45-mcp-servers.html` + 配套 JS/CSS）：
  - server 列表展示（name / transport / enabled / connected / cached tools 数）
  - 新增/编辑/删除 server
  - 测试连接按钮
  - 工具列表查看
- [ ] 更新 console 导航菜单数组（9 处）+ `runtime-pages.test.mjs` 屏幕数 44 → 45
- [ ] 测试：
  - `cargo test --lib`（确保 lib 测试数 ≥ 940 + 新增 ~20）
  - `cargo clippy` 全绿
  - webui runtime-pages 测试
  - agent-exploration classifier 测试
- [ ] 提交并推送至 PR #313

---

## 后续阶段（5.5–5.7）

### Phase 5.5 多语言 UI（i18n）— ⏳ 待办

- i18n 字典 + 语言切换
- 优先英/日（参考 #308）

### Phase 5.6 自动备份/恢复 — ⏳ 待办

- 定时 tar.gz data_root → 保留 N 份
- WebUI 22 屏激活
- `POST /v1/backup/create` + `POST /v1/backup/restore`

### Phase 5.7 跨设备同步（WebDAV/S3）— ⏳ 待办

- 增量同步角色/会话/记忆
- 冲突解决策略
- P2/P3 优先级

---

## 测试基线（截至 Phase 5.3 提交 `3f72360`）

| 测试套件 | 数量 | 状态 |
|---|---|---|
| `cargo test --lib`（engine） | 940 passed / 1 ignored | ✅ |
| `cargo clippy` | 全绿（warnings denied） | ✅ |
| webui runtime-pages | 46 passed | ✅ |
| agent-exploration classifier | 9 passed | ✅ |

Phase 5.4 完成后预计：
- engine lib 测试数 ≈ 960+（新增 ~20 个 mcp_integration 测试）
- webui runtime-pages 测试屏数 45

---

## 当前未提交改动（git status）

```
modified:   Cargo.lock           # rmcp 1.7 → 1.8.0 + 新依赖
modified:   Cargo.toml           # rmcp features 扩展
modified:   engine/src/lib.rs    # 新增 pub mod mcp_integration;

Untracked:
  AIRP_新功能开发路线图_task-58d.md   # 路线图原文（待决定是否提交）
  engine/src/mcp_integration.rs      # Phase 5.4 核心模块
```

**下一步行动**：修复 `spawn_connection` 编译错误（详见上文"待修复项"），然后继续完成 daemon 集成、HTTP 端点、WebUI 页面。
