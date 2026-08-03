# AIRP 备份与恢复（v0.0.3 P2）

> 适用版本：AIRP 0.0.3（`backup` 模块首版，#342 E-P2-1）
> 实现：`engine/src/backup/`，HTTP 端点见 `engine/src/daemon/handlers/backups.rs`
> 用户入口：WebUI → 设置 → 备份与恢复
> 关联计划：[docs/plans/2026-08-03-e-p2-1-backup-restore.md](plans/2026-08-03-e-p2-1-backup-restore.md)

本文是 AIRP 备份/恢复功能的事实入口，覆盖：备份内容、版本兼容、secret 处理、灾难恢复步骤与 v1 已知限制。**修改 `backup/` 模块或 HTTP 合同时必须同步更新本文。**

## 1. 备份包含什么

| 资产 | 是否备份 | 说明 |
|---|---|---|
| `characters/{id}/`（card / state / sessions / memory / analysis / revisions / ...） | ✅ | 含全部会话历史与 revision snapshot |
| `presets/` | ✅ | preset 配置 |
| `providers.json` | ✅ | provider 路由配置（**不含 secret**） |
| `persona/` | ✅ | persona 定义 |
| `plugins/` | ✅ | plugin 配置 |
| 其他 `data_root` 顶层普通文件（非 secret） | ✅ | |
| `secrets.json` | ❌ | provider API key，**永不备份** |
| `settings.json` | ❌ | 含 `api_key` / `access_api_key` 字段，**永不备份** |
| `backups/` | ❌ | 备份自身不递归 |

manifest `files` 列表记录每个被备份文件的相对路径（`/` 分隔）+ SHA-256 + 字节数；`tree_sha256` 是覆盖 `files` 子树的 `AIRP-TREE-SHA256-v1`，用于完整性校验。

## 2. Secret 处理

**v1 永远不备份 secret**，无论 manual 还是 `PreDelete` / `PreRestoreRollback` 自动备份。manifest 强制 `secrets_excluded: true`，加载时若为 `false` 直接拒绝。

理由：
- 文件级排除最简单且 fail-closed；按字段 redact 需解析 JSON 易漏。
- `secrets.json` / `settings.json` 可能含 provider key、access key 等高价值凭据；普通用户资产导出不应携带。
- v1 不实现加密 secret 备份（issue #342 明确要求"若提供 secret 备份，必须单独加密并明确授权"——超出最小闭环）。

**恢复后必须重新配置**：
1. provider API key（`secrets.json`）
2. access API key（`settings.json` 的 `api_key` / `access_api_key` 字段）

WebUI 恢复对话框会明确提示用户。

## 3. 版本兼容性

manifest 记录两个独立版本字段：

| 字段 | 含义 | 当前值 | 兼容策略 |
|---|---|---|---|
| `schema` | manifest schema 版本 | `1` | 加载时必须 `== 1`，否则拒绝（不降级） |
| `data_schema_version` | backup 内容的 logical schema 版本 | `1` | 加载时若 `> 本引擎支持最大值`，拒绝（向前兼容旧 backup，不向后兼容未来） |
| `engine_version` | 创建 backup 的引擎版本 | `env!("CARGO_PKG_VERSION")` | 仅记录，加载时不强制（data_schema_version 才是兼容判据） |

未来 data 结构大改时递增 `DATA_SCHEMA_VERSION`，老引擎拒绝读新 backup；新引擎读老 backup 时按 `data_schema_version` 决定是否需要 migration。

## 4. 灾难恢复路径

### 4.1 标准恢复（推荐）

适用：data_root 损坏、误删、想回滚到某个时间点。

1. 进入 WebUI → 设置 → 备份与恢复
2. 在备份列表中选择目标 `Full` scope backup（按 `created_at` 排序，最新在前）
3. 点击"校验"确认完整性（`POST /v1/backups/:id/verify`）
4. 点击"恢复"，确认对话框会提示：
   - 将自动创建 `PreRestoreRollback` 备份保护当前状态
   - 当前 `data_root` 会被覆盖（除 `backups/` 与 secret 文件）
   - secret 文件不会被恢复，需要重新配置
5. 恢复完成后建议重启 daemon（让内存中的缓存、character_lock map、session_lock map 重新加载）

API 等价流程：
```
POST /v1/backups/:id/verify
POST /v1/backups/:id/restore   → 返回 { restored_from, rollback_backup_id, verified }
```

### 4.2 回滚恢复

适用：恢复后发现问题，想回到恢复前的状态。

恢复流程会自动创建一个 `source: PreRestoreRollback` 的 `Full` scope backup。在备份列表中按 `source` 字段识别它，对它执行 4.1 流程即可回到恢复前状态。

### 4.3 删除后恢复

适用：误删了 character 或 session。

`delete_character` / `delete_session` 默认会创建 `source: PreDelete` 的 scoped backup（character 或 session 子树）。流程：

1. 在备份列表中找到对应 `PreDelete` backup（按 `source` 字段筛选）
2. **不能直接 restore**：v1 仅支持 `Full` scope restore，scoped restore 会被 fail-closed 拒绝（防止当前 restore 流程删除 data_root 下未备份的不相关数据）
3. 手动从 backup 目录的 `files/` 子树拷贝所需文件回 data_root：
   ```
   data_root/backups/{backup_id}/files/characters/{id}/  →  data_root/characters/{id}/
   ```
4. 重启 daemon

follow-up issue 将实现真正的 scoped restore（仅替换目标子树）。

### 4.4 命令行兜底（backup 不可读时）

若 manifest 损坏但 `files/` 子树完好，可直接从 `data_root/backups/{backup_id}/files/` 拷贝文件回 data_root。此路径绕过完整性校验，**仅在标准恢复失败时使用**，事后建议运行 `POST /v1/backups/:id/verify` 抽查。

## 5. v1 已知限制

| 限制 | 影响 | 缓解 | follow-up |
|---|---|---|---|
| 不串行化所有写路径 | backup 期间并发写可能产生混合快照 | 在维护窗口或无活跃 session 时执行 backup；`PreDelete` / `PreRestoreRollback` 由调用方锁串行化 | 跨资源一致性强备份 |
| 仅支持 `Full` scope restore | `Character` / `Session` scope backup 不能直接 restore | 见 4.3，手动从 `files/` 拷贝 | scoped restore |
| 不加密 secret | provider/access key 不进备份 | 恢复后手动重配 | secret 加密备份 |
| 不自动定时 | 用户必须手动触发或调 API | follow-up 做 cron | 自动定时备份 |
| 不增量 / 不压缩 | 大 data_root 备份慢、占空间 | v1 接受 | 增量 / 压缩 |
| 无保留策略 | backup 累积需手动清理 | WebUI 删除按钮 | 自动清理策略 |
| `delete_persona` / `delete_plugin_tool` 无 pre-delete backup | persona / plugin 误删不可恢复 | v1 只覆盖用户最痛的 character/session；persona/plugin 可重建 | 后续补齐 |
| 进程内 backup lock | 跨进程不安全 | AIRP daemon 单进程前台运行（AGENTS.md） | 跨进程锁（如有需要） |

## 6. 一致性约束（写入合同）

下列不变量同时是合同与测试断言依据，修改 `backup/` 模块时必须保持：

1. **manifest schema v1 不变量**（加载时强制）：
   - `schema == 1`
   - `backup_id` 非空、合法路径段（无 `/` `\` `:` `..`，不以 `.` 开头）
   - `data_schema_version <= DATA_SCHEMA_VERSION`
   - `secrets_excluded == true`
   - `files[].path` 经 `validate_approved_path` 校验（无 `..` / 绝对路径 / 反斜杠 / 空字节）

2. **secret 永不备份**：`SECRET_EXCLUDE_LIST = ["secrets.json", "settings.json"]`（仅 data_root 根目录下同名文件）

3. **`backups/` 自身不递归**：snapshot walk 跳过 `data_root/backups/` 子树

4. **原子性**：create / restore 都用 staging → `sync_dir` → 原子 rename 模式；任一步失败不留下半成品状态

5. **fail-closed**：pre-delete backup 失败 → delete 操作拒绝执行（不删数据）；restore 校验失败 → 拒绝 restore（不动 data_root）；scoped restore → 拒绝（v1）

6. **`BACKUP_LOCK`**（`std::sync::Mutex`，进程内）串行化 backup vs backup / backup vs restore；调用方在 `character_lock` 内调用 backup 合法（外→内序列，LOCK-ORDER 合同）

7. **path sandbox**：所有 restore 目标路径经 `safe_resolve_for_write` + `validate_approved_path` 双重校验；拒绝符号链接与绝对路径

8. **post-restore 校验**：restore 后重新枚举 data_root（排除 `backups/`），与 manifest `files` 对比，并重算每个文件 SHA-256；任一不一致返回 `Internal`，保留 staging + rollback backup 供人工恢复

## 7. HTTP API 速查

| 方法 | 路径 | 用途 | 关键字段 |
|---|---|---|---|
| `POST` | `/v1/backups` | 创建 backup | body `{source: "manual", scope: "full"}` → `{backup_id, created_at, files, total_bytes, tree_sha256}` |
| `GET` | `/v1/backups` | 列出所有 backup | 返回数组，按 `created_at` 降序 |
| `GET` | `/v1/backups/:id` | 取 manifest | 完整 `BackupManifest` |
| `POST` | `/v1/backups/:id/verify` | 校验完整性 | `{verified, checked_files, tree_sha256}` |
| `POST` | `/v1/backups/:id/restore` | 恢复（v1 仅 Full） | `{restored_from, rollback_backup_id, verified}` |
| `DELETE` | `/v1/backups/:id` | 删除 backup | 不可恢复 |

`DELETE /v1/characters/:id` 与 `DELETE /v1/sessions/:id` 接受 `?force=true` 跳过 pre-delete backup（advanced / testing）。

## 8. 关联文档

- 实现计划：[docs/plans/2026-08-03-e-p2-1-backup-restore.md](plans/2026-08-03-e-p2-1-backup-restore.md)
- 路径安全底座：`engine/src/data_dir/security.rs`、`engine/src/revision/tree_hash.rs`
- revision 合同（`ApprovedFile` / `compute_tree_sha256`）：`engine/src/revision/`
- 锁序合同：[docs/LOCK-ORDER-CONTRACT.md](LOCK-ORDER-CONTRACT.md)
- 当前基线：[docs/CURRENT-BASELINE.md](CURRENT-BASELINE.md)
