# E-P2-1: #342 备份恢复闭环最小实施计划

> 关联 issue：[#342](https://github.com/GhostXia/AIRP/issues/342)（data: 交付备份恢复、可恢复删除与核心资产导出闭环）
> 基线：`main@098ba76`（PR #441 合并后）
> 范围定位：v0.0.3 P2 release gate 的最小闭环切片。**不**做云同步、多租户、无限历史、自动定时备份；**只**交付手动 create / list / verify / restore / delete + 删除前自动备份 + WebUI 入口。

## 1. 现状

| 维度 | 当前事实 |
|---|---|
| Engine HTTP | 无 `/v1/backups*` 端点 |
| WebUI | `screens/22-backup-restore.html` 显式渲染"当前 Engine 没有备份/恢复 HTTP API"（`console-runtime.js::renderUnavailable('backup')`）；契约测试断言**不**调用 backup API |
| 删除 | `delete_character`（`domain/chat.rs`）与 `delete_session`（`data_dir/session.rs`）直接 `fs::remove_dir_all`，不可恢复。`delete_session` 已有 tombstone（`deleted_sessions/{sid}` 标记文件）但目录本身已删 |
| revision 底座 | `revision/manifest.rs` 提供 `RevisionManifest` / `AssetKind` / `ApprovedFile` / `file_sha256_hex`；`revision/tree_hash.rs` 提供 `AIRP-TREE-SHA256-v1` + `validate_approved_path`；`revision/atomic.rs` 提供 `commit_revision` staging→rename 模式 + `sync_dir`。**均为 per-asset**，不直接覆盖多资产备份 |
| 路径安全 | `data_dir/security.rs` 提供 `safe_resolve_under_data_root` / `safe_resolve_for_write` / `validate_id_segment` |
| secret 存储 | `secrets.json`（provider key，via `secret_store.rs`）+ `settings.json`（含 `api_key` / `access_api_key` 字段） |

## 2. 设计决策

### 决策 A：独立 `BackupManifest`，不复用 `RevisionManifest`

- `RevisionManifest` 是 **per-asset**（单一 `asset_id`），backup 覆盖多资产，schema 语义不匹配。
- 新建 `engine/src/backup/manifest.rs::BackupManifest`，**复用** `revision::manifest::ApprovedFile`（path + sha256 + bytes 三元组，已是稳定 pub(crate) 类型）与 `revision::tree_hash::compute_tree_sha256` / `validate_approved_path`。
- `BackupManifest` schema v1：
  ```rust
  pub(crate) struct BackupManifest {
      pub schema: u32,              // = 1
      pub backup_id: String,        // ULID
      pub created_at: String,       // RFC3339 UTC
      pub engine_version: String,   // env!("CARGO_PKG_VERSION")
      pub data_schema_version: u32, // = 1
      pub source: BackupSource,     // Manual | PreDelete { scope } | PreRestoreRollback
      pub scope: BackupScope,       // Full | Character { id } | Session { character_id, session_id }
      pub secrets_excluded: bool,   // v1 恒为 true
      pub files: Vec<ApprovedFile>, // 复用 revision::manifest::ApprovedFile
      pub tree_sha256: String,      // AIRP-TREE-SHA256-v1 over files
  }
  ```
- 理由：复用成熟 hash/path 算法，但 schema 独立避免语义混淆；未来 backup 演进不污染 per-asset revision 合同。

### 决策 B：一致性策略——best-effort + 文档化 + 强制维护窗口建议

- **不**实现跨资源锁串行化所有写路径（需改 30+ handler，超出最小闭环范围）。
- v1 策略：
  1. backup 全程持有**进程内 `backup::BACKUP_LOCK: tokio::sync::Mutex<()>`**，串行化 backup vs backup / backup vs restore。
  2. 文件级快照：walk `data_root`，逐文件 `fs::read` 到 staging，计算 hash，写 manifest，原子 rename staging→final。
  3. **文档化限制**：`BackupManifest` 不记录"snapshot 期间无并发写"证明；`docs/` 与 manifest 注释明确"backup 应在维护窗口或无活跃 session 时执行"。WebUI 创建按钮显示警告文案。
  4. **缓解**：`source: PreDelete` / `PreRestoreRollback` 场景下，调用方（`delete_character` / restore handler）已在 character_lock 写锁内或 backup_lock 内，自然串行化相关资源。
- 不选"全锁所有写路径"：范围爆炸，且 v0.0.3 P2 不要求在线热备。
- follow-up issue：跨资源一致性强备份（acquire all character read locks + session locks during snapshot）。

### 决策 C：Secret 排除——denylist 文件级排除

- v1 恒定排除以下文件（manifest 记录 `secrets_excluded: true`）：
  - `secrets.json`（provider key）
  - `settings.json`（含 `api_key` / `access_api_key`）
- 理由：文件级排除最简单且 fail-closed；redact 字段需解析 JSON 易漏。`providers.json`（routing 配置）无 secret，**保留**备份。
- restore 后用户必须重新配置 provider key 与 access key；WebUI restore 确认对话框明确提示。
- 不选"加密 secret 备份"：v1 不做，issue 明确"若提供 secret 备份，必须单独加密并明确授权"——超出最小闭环。

### 决策 D：存储布局

```
data_root/
  backups/
    {backup_id}/
      manifest.json          # BackupManifest JSON
      files/                 # 镜像 data_root 结构（排除 backups/ 自身 + secret 文件）
        characters/...
        presets/...
        providers.json
        ...
    {backup_id2}/...
  backups.index             # 可选 v1 不做，list 直接 scan 目录
```

- staging 目录：`data_root/backups/.staging-{backup_id}/`，原子 rename 为 `data_root/backups/{backup_id}/`。
- **排除 `backups/` 自身**：snapshot walk 跳过 `data_root/backups/` 子树，防止递归与空间爆炸。

### 决策 E：Restore 策略——staging + 原子 swap + 回滚备份

- restore 流程：
  1. acquire `BACKUP_LOCK` write（与 backup 互斥）。
  2. **校验目标 backup 完整性**（file set + per-file SHA-256 + tree SHA-256）；失败 fail-closed。
  3. **创建回滚备份**（`source: PreRestoreRollback`，scope `Full`）——保护当前 data_root 状态。
  4. staging：`data_root/.restore-staging-{backup_id}/`，从 backup `files/` 逐文件复制（路径经 `safe_resolve_for_write` 校验）。
  5. 移除 `data_root` 下除 `backups/` 与 staging 外的所有顶层条目。
  6. 原子 rename staging 内子项 → `data_root/`。
  7. **post-restore 校验**：重新枚举 `data_root`（排除 `backups/`），与 manifest `files` 对比（允许 secret 文件缺失，因为 restore 不写 secret）。
  8. 失败任一步：保留 staging + 回滚备份，返回 `Internal`，**不**清理现场供人工恢复。
- path sandbox：所有 restore 目标路径必须经 `safe_resolve_for_write(data_root, relative_path)` 校验，拒绝绝对路径 / `..` / 空字节 / symlink。staging 写入前逐文件 `validate_approved_path`。

### 决策 F：可恢复删除——scoped pre-delete backup

- `delete_character` 与 `delete_session` 在 `fs::remove_dir_all` 前调用 `backup::create_scoped_backup`：
  - `delete_character`：`scope = Character { id }`，仅备份 `characters/{id}/` 子树。
  - `delete_session`：`scope = Session { character_id, session_id }`，仅备份 `characters/{character_id}/sessions/{session_id}/` + tombstone。
- manifest `source = PreDelete { scope }`，记录删除原因供 WebUI 列表区分。
- `delete_character` / `delete_session` 新增可选 query `?force=true` 跳过 pre-delete backup（advanced / testing）。
- **不**改 `delete_persona` / `delete_plugin_tool` 等：v1 只覆盖用户最痛的 character/session 删除；persona/plugin 配置可重建，follow-up issue。
- 失败处理：pre-delete backup 失败时 `delete` 操作 fail-closed（返回 `Internal`，不删数据），让用户先手动备份。

### 决策 G：WebUI 入口——替换 unavailable renderer

- `console-runtime.js`：移除 `backup: () => renderUnavailable('backup')`，新增 `renderBackup` 实现：
  - `GET /v1/backups` 列表（id, created_at, source, scope, file_count, total_bytes, verified）
  - "创建备份"按钮 → `POST /v1/backups`（显示 secret 排除警告 + 维护窗口建议）
  - 每行 "校验" → `POST /v1/backups/:id/verify`
  - 每行 "恢复" → `POST /v1/backups/:id/restore`（确认对话框，提示将创建回滚备份 + secret 需重配）
  - 每行 "删除" → `DELETE /v1/backups/:id`（确认对话框）
- 更新 `webui/tests/runtime-pages.test.mjs::backup page explicitly stays unavailable` 测试：改为断言**调用**新 API 契约（`/v1/backups` 路径 + renderer 不再是 `renderUnavailable`）。

## 3. 实施切片

### Slice 1：backup 核心 + create/list/get API

**新增文件**：
- `engine/src/backup/mod.rs` — 公共 API 入口 + `BACKUP_LOCK`
- `engine/src/backup/manifest.rs` — `BackupManifest` / `BackupSource` / `BackupScope` schema + 加载校验
- `engine/src/backup/snapshot.rs` — staging → walk → copy → hash → atomic rename
- `engine/src/daemon/handlers/backups.rs` — HTTP handlers

**修改文件**：
- `engine/src/lib.rs` — `pub(crate) mod backup;`
- `engine/src/daemon/mod.rs` — 注册 routes + `DaemonState` 无需新字段（用全局 `BACKUP_LOCK`）
- `engine/src/daemon/handlers.rs` — re-export backup handlers

**HTTP 端点**：
- `POST /v1/backups` — body `{ "source": "manual", "scope": "full" }`，返回 `{ backup_id, created_at, files, total_bytes, tree_sha256 }`
- `GET /v1/backups` — 返回 `[{ backup_id, created_at, source, scope, file_count, total_bytes, verified: null }]`
- `GET /v1/backups/:backup_id` — 返回完整 manifest

**关键不变量**：
- `BackupManifest.schema == 1`
- `backup_id` 为 ULID（`crate::ulid`）
- `files` 路径相对 `data_root`，`/` 分隔，NFC，无 `..` / 绝对路径 / 反斜杠（复用 `validate_approved_path`）
- `tree_sha256` 覆盖 `files`，与 `AIRP-TREE-SHA256-v1` 一致
- staging 原子 rename，`sync_dir` 持久化（复用 `revision::atomic::sync_dir`）
- secret 文件（`secrets.json` / `settings.json`）绝不进入 `files`

**测试**：
- manifest roundtrip JSON
- create backup 后 manifest.verify_against_disk 通过
- secret 文件被排除
- `backups/` 自身不被备份
- 并发 create backup 串行化（两线程并发，第二个等待）
- path traversal 拒绝
- `BACKUP_LOCK` 串行化语义

### Slice 2：verify + restore + delete API

**修改文件**：
- `engine/src/backup/mod.rs` — 加 `verify_backup` / `restore_backup` / `delete_backup`
- `engine/src/backup/snapshot.rs` — 加 restore staging + swap 逻辑
- `engine/src/daemon/handlers/backups.rs` — 加 verify/restore/delete handlers
- `engine/src/daemon/mod.rs` — 注册新 routes

**HTTP 端点**：
- `POST /v1/backups/:backup_id/verify` — 返回 `{ verified: true, checked_files, tree_sha256 }` 或错误详情
- `POST /v1/backups/:backup_id/restore` — 自动创建回滚备份 → swap → post-verify；返回 `{ restored_from, rollback_backup_id, verified: true }`
- `DELETE /v1/backups/:backup_id` — 删除指定 backup（irreversible，需确认）

**关键不变量**：
- restore 前必须 verify 通过
- restore 必须先创建 `PreRestoreRollback` backup
- restore 失败不清理现场（保留 staging + rollback backup 供人工恢复）
- restore 路径全部经 `safe_resolve_for_write` + `validate_approved_path`
- delete backup 不允许删除 `PreRestoreRollback` 中标记 `protected` 的（v1 不实现 protected 标记，所有 backup 都可删，但 WebUI 二次确认）

**测试**：
- verify 合法 backup 通过
- verify 检测篡改（改文件内容 / 删文件 / 加文件 / 改 manifest）
- restore 后 data_root 内容 == backup 内容（除 secret 文件）
- restore 自动创建 rollback backup
- restore 失败时 data_root 不半删（模拟 staging 写入失败）
- path traversal 拒绝（backup 内含恶意路径）
- 删除 backup 后 list 不再包含

### Slice 3：可恢复删除（pre-delete backup）

**修改文件**：
- `engine/src/domain/chat.rs` — `delete_character` 调用 `backup::create_scoped_backup(Character { id })` 在 `remove_dir_all` 前
- `engine/src/data_dir/session.rs` — `delete_session` 同理
- `engine/src/daemon/handlers/characters.rs` — `delete_character_endpoint` 接受 `?force=true`
- `engine/src/daemon/handlers/sessions.rs` — `delete_session_endpoint` 接受 `?force=true`

**关键不变量**：
- pre-delete backup 失败 → delete 操作 fail-closed（不删数据）
- `?force=true` 跳过 pre-delete backup（advanced / testing）
- pre-delete backup `source = PreDelete { scope }`，scope 记录删除目标
- pre-delete backup 与 manual backup 一起在 `GET /v1/backups` 列出，WebUI 用 `source` 字段区分显示

**测试**：
- `delete_character` 后 `GET /v1/backups` 含 `PreDelete` backup，scope = `Character { id }`
- `delete_character?force=true` 不创建 backup
- pre-delete backup 可 restore 恢复 character
- pre-delete backup 失败时 character 未被删
- `delete_session` 同理

### Slice 4：WebUI backup 管理入口

**修改文件**：
- `webui/assets/console-runtime.js` — 新增 `renderBackup` 替换 `renderUnavailable('backup')`
- `webui/screens/22-backup-restore.html` — 无需改结构（runtime 渲染到 `#view`）
- `webui/tests/runtime-pages.test.mjs` — 更新 backup 测试断言新契约

**UI 行为**：
- 列表显示所有 backup，按 `created_at` 降序
- 每行：backup_id（短）+ created_at + source badge（manual/pre_delete/pre_restore_rollback）+ scope + file_count + total_bytes + verified 状态
- 操作按钮：创建（顶部）、校验、恢复、删除（每行）
- 创建对话框：警告"将排除 secrets.json 与 settings.json，备份期间请暂停写入"
- 恢复对话框：警告"将创建回滚备份，当前 data_root 会被覆盖，secrets 需手动重配"
- 删除对话框：警告"不可恢复"

**测试更新**：
- 旧测试 `backup page explicitly stays unavailable without calling a backup API` 改为 `backup page calls backup API and renders list`
- 断言 `console-runtime.js` 含 `renderBackup` 函数
- 断言含 `/v1/backups` API 调用
- 断言不再含 `renderUnavailable('backup')`

## 4. 安全与数据约束对齐

| issue 要求 | 本计划满足方式 |
|---|---|
| 备份 manifest 版本化，记录文件清单、hash、创建版本和数据 schema 版本 | `BackupManifest { schema, engine_version, data_schema_version, files: Vec<ApprovedFile>, tree_sha256 }` |
| 角色、世界书、会话、记忆、Persona、Preset 恢复前后 hash/语义一致性有测试 | Slice 2 测试：restore 后逐文件 SHA-256 + tree SHA-256 校验 |
| restore 失败不会留下部分覆盖状态 | Slice 2：staging→swap 原子，失败保留 staging + rollback，不清理 |
| 删除角色/会话后可在明确窗口内恢复，墓碑/索引状态一致 | Slice 3：pre-delete backup + restore；`delete_session` tombstone 保留 |
| WebUI 能完成 create → verify → mutate/delete → restore → verify 演练 | Slice 4：完整 UI 流程 |
| Windows 便携包和 production topology 至少各一次可重复恢复 smoke | CI 加 backup restore smoke test（engine 集成测试）；production smoke 在 #130 验收时一起做 |
| 文档说明备份是否包含 secrets、版本兼容范围和人工灾难恢复路径 | `docs/BACKUP-RESTORE.md`（本 PR 创建） |
| 默认不把 provider/access secret 放入普通用户资产导出 | 决策 C：denylist 排除 `secrets.json` + `settings.json` |
| restore 不得接受路径穿越、symlink 逃逸或任意服务器本地路径 | 决策 E：`safe_resolve_for_write` + `validate_approved_path` + 拒绝 symlink |
| 不允许用"接口返回成功"代替恢复后的资产完整性验证 | 决策 E：post-restore 校验 + verify API |

## 5. 不在范围（follow-up issues）

- 跨资源一致性强备份（acquire all locks during snapshot）→ follow-up
- 自动定时备份 / cron → follow-up
- secret 加密备份 → follow-up
- 资产级导出（per-character / per-session 独立导出包）→ follow-up（#346）
- backup 压缩 / 增量 → follow-up
- backup 保留策略 / 自动清理 → follow-up
- `delete_persona` / `delete_plugin_tool` pre-delete backup → follow-up
- 跨进程备份锁（v1 进程内 `tokio::Mutex`，单进程 daemon 够用）→ follow-up

## 6. 验收清单

- [ ] Slice 1：`POST/GET /v1/backups` + `GET /v1/backups/:id` 可用，manifest 完整性校验通过
- [ ] Slice 2：`POST /v1/backups/:id/verify` + `POST /v1/backups/:id/restore` + `DELETE /v1/backups/:id` 可用，restore 原子性 + post-verify 通过
- [ ] Slice 3：`delete_character` / `delete_session` 默认创建 pre-delete backup，`?force=true` 跳过
- [ ] Slice 4：WebUI 列表 + create/verify/restore/delete 流程可用，测试更新
- [ ] `cargo test` 全绿（含 `subagent_context_has_no_orchestrator_noise` 不变式）
- [ ] `cargo clippy --all-targets -- -D warnings` 全绿
- [ ] `cargo fmt --check` 全绿
- [ ] `docs/BACKUP-RESTORE.md` 说明 secret 排除、版本兼容、灾难恢复路径
- [ ] 独立审计报告 `docs/audits/2026-08-03-PR-XXX-backup-restore-audit.md`
- [ ] PR 审计 bot 通过 + 人工 review 合并
- [ ] follow-up issues 创建（跨资源强一致性、自动定时、secret 加密、资产级导出、persona/plugin pre-delete、backup 保留策略）

## 7. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 大 data_root 备份慢 / 占空间 | v1 接受；WebUI 警告；follow-up 做增量 / 压缩 |
| 并发写导致快照不一致 | 决策 B：文档化维护窗口；pre-delete / pre-restore 场景由调用方锁保护 |
| restore 覆盖正在运行的 daemon 状态 | v1 接受"restore 后建议重启 daemon"；文档化；follow-up 做 daemon 协调（pause writes during restore） |
| backup lock 与现有锁序冲突 | `BACKUP_LOCK` 是全局叶锁（同 `COMMIT_LOCK`），不嵌套任何 character/session/state 锁；调用方在 character_lock 内调用 backup 时合法（外→内） |
| WebUI 旧测试阻断 | Slice 4 同步更新测试断言 |
| pre-delete backup 失败导致 delete 卡死 | fail-closed 是正确行为；用户可 `?force=true` 绕过或手动清理 |

