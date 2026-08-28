//! Backup / restore / 可恢复删除 闭环（#342 E-P2-1）。
//!
//! ## 模块结构
//!
//! - [`manifest`]：`BackupManifest` schema + 加载/校验
//! - [`snapshot`]：创建、校验、列出、删除 backup 的实现
//!
//! ## 设计原则
//!
//! 1. **复用 revision 底座**：`ApprovedFile` 类型、`compute_tree_sha256`、
//!    `validate_approved_path` 直接复用 `crate::revision`，不重新发明。
//! 2. **独立 manifest schema**：`BackupManifest` 与 per-asset `RevisionManifest` 分离，
//!    避免 schema 语义混淆。
//! 3. **进程内串行化**：`BACKUP_LOCK` 串行化 backup vs backup / backup vs restore。
//!    跨进程安全由单进程 daemon 保证（AGENTS.md）。
//! 4. **secret 永不备份**：v1 恒定排除 `secrets.json` + `settings.json`，
//!    manifest 记录 `secrets_excluded: true`。
//! 5. **fail-closed**：backup / restore / delete 任一步失败不留下半成品状态。
//!
//! ## v1 限制（文档化）
//!
//! - **不**实现跨资源锁串行化所有写路径；backup 期间并发写可能产生混合快照。
//!   调用方应在维护窗口或无活跃 session 时执行 backup。
//! - **不**支持加密 secret 备份。
//! - **不**支持增量 / 压缩 / 自动定时。
//! - **不**实现跨进程备份锁（单进程 daemon 够用）。
//!
//! follow-up issues 将跟踪这些限制。

pub(crate) mod manifest;
pub(crate) mod snapshot;

pub(crate) use manifest::{BackupScope, BackupSource};
pub(crate) use snapshot::{
    create_backup, delete_backup, list_backups, read_backup_manifest, restore_backup,
    verify_backup, with_created_verified_backup, with_verified_backup, CreateBackupOptions,
};
