//! Backup snapshot 创建、校验、恢复与删除（#342 E-P2-1）。
//!
//! ## 创建流程
//!
//! 1. acquire `BACKUP_LOCK`（进程内 `std::sync::Mutex`）串行化 backup vs backup / restore
//! 2. 生成 `backup_id`（uuid v4 simple，32 hex）
//! 3. 创建 staging 目录 `data_root/backups/.staging-{backup_id}/files/`
//! 4. walk `data_root`（排除 `backups/` 自身 + secret 文件），逐文件复制到 staging `files/`
//! 5. 计算 per-file SHA-256 + tree SHA-256
//! 6. 构造 manifest，写入 staging `manifest.json`
//! 7. 全量校验 manifest vs staging
//! 8. 自最深子目录向 staging 根逐层同步全部目录项
//! 9. rename staging → hidden `.pending-{backup_id}`, sync stable parents, then
//!    atomically rename pending → `data_root/backups/{backup_id}/`
//! 10. sync `backups/` again to persist the final publication name
//!     - failure at this final barrier returns `BackupPublicationOutcomeUnknown`
//!       with the safe backup ID; callers must refresh and verify
//!
//! ## 一致性限制（v1）
//!
//! v1 **不**实现跨资源锁串行化所有写路径。backup 期间并发写可能产生混合快照。
//! 调用方应在维护窗口或无活跃 session 时执行 backup。
//!
//! ## `PreDelete` / `PreRestoreRollback` 串行化范围（#447 修正）
//!
//! 上述说法曾写作"由调用方持有的 character_lock / backup_lock 自然串行化相关
//! 资源"，但该措辞不准确：
//!
//! - `PreDelete`：`delete_character` / `delete_session` 持 `character_lock` 调
//!   `create_backup`，**backup 创建阶段**确实被 character_lock 串行化。但
//!   `delete_character` 的 `remove_dir_all` 在 `BACKUP_LOCK` 释放后、character
//!   write guard 持有期间执行——若并发 `restore_backup`，restore swap 与 delete
//!   的 remove_dir_all 竞态。
//! - `PreRestoreRollback`：`restore_backup` 内部调 `create_backup_locked`（持
//!   `BACKUP_LOCK`），**rollback backup 创建阶段**确实串行化。但 restore 的 swap
//!   阶段（`swap_full_data_root` / `swap_scoped_subtree`）不持任何 character_lock，
//!   可与并发的 `append_to_current`（持 character.read + session_lock）、
//!   `StateService::mutate`（持 character.read + state_lock）竞态。
//!
//! 结论：仅 backup **创建阶段**被串行化；restore swap 阶段不持 character_lock，
//! 必须确保无活跃写（维护窗口或暂停 daemon）。follow-up issue 将在 swap 阶段
//! acquire character_lock 以实现跨资源强一致性。
//!
//! ## Scoped restore（v1）
//!
//! v1 支持 `BackupScope::Full` / `Character` / `Session` 三种 scope 的 restore：
//! - **Full**：仅当当前 data root 与目标 manifest 均不含 `ui/workspaces/` 时，
//!   替换 `data_root` 下所有顶层条目（除 `backups/`），见 `swap_full_data_root`
//! - **Character / Session**：仅替换 `subtree_prefix` 子树（如 `characters/alice/`），
//!   其他 data_root 内容不受影响，见 `swap_scoped_subtree`
//! - **Workspace**：generic restore 明确拒绝，必须走 forward-only workspace rollback
//!
//! 这是 #342 pre-delete 备份可恢复能力的核心保证：删除 character/session 时创建的
//! scoped backup 能 restore 回原资源而不影响其他 character/session。

use crate::error::AirpError;
use crate::revision::manifest::{file_sha256_hex, ApprovedFile};
use crate::revision::tree_hash::compute_tree_sha256;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use super::manifest::{
    BackupManifest, BackupScope, BackupSource, BACKUP_MANIFEST_SCHEMA, DATA_SCHEMA_VERSION,
    SECRET_EXCLUDE_LIST,
};

/// 全局 backup 串行化锁。
///
/// 串行化 backup vs backup / backup vs restore，防止 staging 冲突与 tree hash 不一致。
/// 跨进程安全由调用方负责（AIRP daemon 单进程前台运行，AGENTS.md）。
///
/// 类型为 `std::sync::Mutex`（非 `tokio::sync::Mutex`）：backup/restore 是同步阻塞
/// 文件 IO，调用方（HTTP handler / agent tool）应通过 `tokio::task::spawn_blocking`
/// 把整个调用搬到 blocking pool，避免占用 tokio worker 线程。
///
/// LOCK-ORDER: 全局叶锁（与 `revision::atomic::COMMIT_LOCK` 同层级）。持此锁时不得
/// 获取任何 per-character / per-session / per-state 资源锁。调用方在 character_lock
/// 内调用 backup 时合法（外→内序列）。
///
/// poison 恢复：guarded value 是 `()`，poison 仅表示持锁线程 panic，无实际状态损坏。
/// 用 `unwrap_or_else(|p| p.into_inner())` 恢复，与 `character_lock` / `session_lock`
/// / `state_lock` 既有模式一致。
pub(crate) static BACKUP_LOCK: Mutex<()> = Mutex::new(());

/// backup 创建选项。
#[derive(Debug, Clone)]
pub(crate) struct CreateBackupOptions {
    pub data_root: PathBuf,
    pub source: BackupSource,
    pub scope: BackupScope,
}

/// backup 创建结果。
///
/// `backup_dir` 当前仅供测试与未来 restore 流程使用（derive 自 `data_root/backups/{backup_id}`）。
#[derive(Debug, Clone)]
pub(crate) struct CreatedBackup {
    pub backup_id: String,
    pub manifest: BackupManifest,
    #[allow(dead_code)]
    pub backup_dir: PathBuf,
}

/// 创建一个 backup。
///
/// 全流程串行化在 `BACKUP_LOCK` 内。返回 manifest 与最终 backup 目录路径。
pub(crate) fn create_backup(opts: &CreateBackupOptions) -> Result<CreatedBackup, AirpError> {
    let _guard = BACKUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    create_backup_locked(opts)
}

/// Create and verify a backup, then run `action` while `BACKUP_LOCK` is still
/// held. The published backup remains on disk if `action` fails, but cannot be
/// deleted/restored concurrently before the protected transaction ends.
/// Lock-free list callers may observe it once its directory is published.
pub(crate) fn with_created_verified_backup<T>(
    opts: &CreateBackupOptions,
    action: impl FnOnce(&CreatedBackup) -> Result<T, AirpError>,
) -> Result<(T, CreatedBackup), AirpError> {
    let _guard = BACKUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let created = create_backup_locked(opts)?;
    created
        .manifest
        .verify_against_disk(&created.backup_dir)
        .map_err(|error| {
            AirpError::Internal(format!(
                "new backup {} failed verification: {error}",
                created.backup_id
            ))
        })?;
    let result = action(&created)?;
    Ok((result, created))
}

/// Verify an existing backup and run `action` while `BACKUP_LOCK` remains
/// held, preventing delete/restore races with consumers of approved bytes.
pub(crate) fn with_verified_backup<T>(
    data_root: &Path,
    backup_id: &str,
    action: impl FnOnce(&BackupManifest, &Path) -> Result<T, AirpError>,
) -> Result<T, AirpError> {
    let _guard = BACKUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let manifest = read_backup_manifest(data_root, backup_id)?;
    let backup_dir = data_root.join("backups").join(backup_id);
    manifest.verify_against_disk(&backup_dir)?;
    action(&manifest, &backup_dir)
}

/// `create_backup` 的内部实现，**调用方必须已持有 `BACKUP_LOCK`**。
///
/// 用于 `restore_backup` 在持锁状态下创建 rollback backup，避免 `std::sync::Mutex`
/// 不可重入导致的死锁。
fn create_backup_locked(opts: &CreateBackupOptions) -> Result<CreatedBackup, AirpError> {
    create_backup_locked_with_sync(opts, &mut sync_dir)
}

fn create_backup_locked_with_sync(
    opts: &CreateBackupOptions,
    sync: &mut impl FnMut(&Path) -> Result<(), AirpError>,
) -> Result<CreatedBackup, AirpError> {
    let backup_id = uuid::Uuid::new_v4().simple().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let engine_version = env!("CARGO_PKG_VERSION").to_string();

    let backups_root = opts.data_root.join("backups");
    let staging_dir = backups_root.join(format!(".staging-{backup_id}"));
    let pending_dir = backups_root.join(format!(".pending-{backup_id}"));
    let backup_dir = backups_root.join(&backup_id);
    let staging_files_dir = staging_dir.join("files");

    // 清理可能残留的 staging
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir)?;
    }
    fs::create_dir_all(&staging_files_dir)?;

    // walk data_root，复制批准文件到 staging/files/
    let mut approved_files: Vec<ApprovedFile> = Vec::new();
    let subtree_prefix = opts.scope.subtree_prefix();

    walk_and_copy(
        &opts.data_root,
        &opts.data_root,
        &staging_files_dir,
        &mut approved_files,
        &subtree_prefix,
    )?;

    // 计算 tree_sha256（覆盖 staging/files/ 下所有批准文件）
    let tree_sha256 = compute_tree_sha256(&staging_files_dir)?;

    // 构造 manifest
    let manifest = BackupManifest {
        schema: BACKUP_MANIFEST_SCHEMA,
        backup_id: backup_id.clone(),
        created_at,
        engine_version,
        data_schema_version: DATA_SCHEMA_VERSION,
        source: opts.source.clone(),
        scope: opts.scope.clone(),
        secrets_excluded: true,
        files: approved_files,
        tree_sha256,
    };

    // 写入 manifest.json
    let manifest_bytes = manifest.to_json_bytes()?;
    let manifest_path = staging_dir.join("manifest.json");
    {
        let mut file = fs::File::create(&manifest_path)?;
        file.write_all(&manifest_bytes)?;
        file.sync_all()?;
    }

    // 全量校验 manifest vs staging
    manifest.verify_against_disk(&staging_dir).map_err(|e| {
        AirpError::Internal(format!(
            "backup staging 全量校验失败（不应发生，请报告 bug）: {e}"
        ))
    })?;

    // Persist every nested directory entry before publishing the staging tree.
    // Directory fsync is not recursive on Unix, so syncing only staging/ would
    // not make files/ui/workspaces/... crash-durable.
    sync_directory_tree_bottom_up_with(&staging_dir, sync)?;

    // Publish through a hidden name first. Readers skip dot-prefixed entries,
    // so post-rename parent-barrier failures cannot expose an incompletely
    // established backup as authoritative.
    fs::rename(&staging_dir, &pending_dir).map_err(|e| {
        AirpError::Internal(format!(
            "rename staging {} -> pending backup {} 失败: {e}",
            staging_dir.display(),
            pending_dir.display()
        ))
    })?;

    // Persist the hidden payload and, for the first backup, the `backups/`
    // entry in data_root before making the final name visible.
    sync(&backups_root)?;
    sync(&opts.data_root)?;

    fs::rename(&pending_dir, &backup_dir).map_err(|e| {
        AirpError::Internal(format!(
            "rename pending backup {} -> final backup {} 失败: {e}",
            pending_dir.display(),
            backup_dir.display()
        ))
    })?;
    // A failure here leaves a complete, pre-synced payload under the final
    // name, but the final name's crash durability is outcome-unknown.
    sync(&backups_root).map_err(|error| {
        tracing::error!(err = %error, %backup_id, "final backup publication barrier failed");
        AirpError::BackupPublicationOutcomeUnknown {
            backup_id: backup_id.clone(),
        }
    })?;

    Ok(CreatedBackup {
        backup_id,
        manifest,
        backup_dir,
    })
}

fn sync_directory_tree_bottom_up(root: &Path) -> Result<(), AirpError> {
    sync_directory_tree_bottom_up_with(root, &mut sync_dir)
}

fn sync_directory_tree_bottom_up_with(
    root: &Path,
    sync: &mut impl FnMut(&Path) -> Result<(), AirpError>,
) -> Result<(), AirpError> {
    for directory in collect_directory_tree_bottom_up(root)? {
        sync(&directory)?;
    }
    Ok(())
}

fn collect_directory_tree_bottom_up(root: &Path) -> Result<Vec<PathBuf>, AirpError> {
    let mut directories = vec![root.to_path_buf()];
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_symlink() {
                return Err(AirpError::Internal(format!(
                    "backup staging contains symlinked directory entry: {}",
                    entry.path().display()
                )));
            }
            if file_type.is_dir() {
                let child = entry.path();
                directories.push(child.clone());
                pending.push(child);
            }
        }
    }
    directories.sort_by(|left, right| {
        right
            .components()
            .count()
            .cmp(&left.components().count())
            .then_with(|| left.as_os_str().cmp(right.as_os_str()))
    });
    Ok(directories)
}

/// 递归 walk `current`（rooted at `data_root`），复制批准文件到 `staging_files_dir`。
///
/// 排除规则：
/// - `data_root/backups/` 子树（防止递归与空间爆炸）
/// - secret 文件（`secrets.json` / `settings.json`，仅根目录下的这些文件名）
/// - `subtree_prefix` 之外的路径（scoped backup 时）
///
/// 路径全部相对 `data_root`，`/` 分隔。
fn walk_and_copy(
    data_root: &Path,
    current: &Path,
    staging_files_dir: &Path,
    approved: &mut Vec<ApprovedFile>,
    subtree_prefix: &str,
) -> Result<(), AirpError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;

        // 拒绝符号链接
        if metadata.file_type().is_symlink() {
            return Err(AirpError::Internal(format!(
                "backup 拒绝符号链接: {}",
                path.display()
            )));
        }

        // 计算相对路径（`/` 分隔）
        let relative = path
            .strip_prefix(data_root)
            .map_err(|_| {
                AirpError::Internal(format!(
                    "strip_prefix 失败: {} vs {}",
                    path.display(),
                    data_root.display()
                ))
            })?
            .to_str()
            .ok_or_else(|| AirpError::Internal(format!("路径含非 UTF-8 字节: {}", path.display())))?
            .replace('\\', "/");

        // 排除 backups/ 顶层目录（仅 data_root/backups/，不误删嵌套的同名子目录）
        if current == data_root && relative == "backups" {
            continue;
        }

        if metadata.is_dir() {
            // scoped backup: 只进入 subtree_prefix 子树或其祖先目录。
            // - "characters/alice" 是 subtree_prefix 时，"characters" 是其祖先，
            //   必须进入才能到达目标；"characters/bob" 不在子树内，跳过。
            // - 判断条件：relative 是 subtree_prefix 的前缀（祖先）或
            //   relative 以 subtree_prefix 开头（在子树内）。
            if !subtree_prefix.is_empty() && !is_ancestor_or_within(&relative, subtree_prefix) {
                continue;
            }
            walk_and_copy(
                data_root,
                &path,
                staging_files_dir,
                approved,
                subtree_prefix,
            )?;
        } else if metadata.is_file() {
            // secret 排除：仅 data_root 根目录下的 secret 文件名
            if current == data_root && SECRET_EXCLUDE_LIST.contains(&relative.as_str()) {
                continue;
            }
            // scoped backup: 只包含 subtree_prefix 子树文件（边界对齐，防
            // "characters/alice" 误匹配 "characters/alicia"）
            if !subtree_prefix.is_empty() && !is_within_subtree(&relative, subtree_prefix) {
                continue;
            }

            // 路径安全校验（双重保险）
            crate::revision::tree_hash::validate_approved_path(&relative).map_err(|e| {
                AirpError::Internal(format!("backup 文件路径非法 {relative:?}: {e}"))
            })?;

            // 复制文件内容到 staging
            let staging_path = staging_files_dir.join(&relative);
            if let Some(parent) = staging_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let bytes = read_file_bytes(&path)?;
            let mut file = fs::File::create(&staging_path)?;
            file.write_all(&bytes)?;
            file.sync_all()?;

            let hash = file_sha256_hex(&bytes);
            approved.push(ApprovedFile {
                path: relative,
                sha256: hash,
                bytes: bytes.len() as u64,
            });
        } else {
            return Err(AirpError::Internal(format!(
                "backup 拒绝非普通文件/目录的特殊入口: {}",
                path.display()
            )));
        }
    }
    Ok(())
}

/// 读取文件完整字节。
fn read_file_bytes(path: &Path) -> Result<Vec<u8>, AirpError> {
    let mut file = fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

/// 判断 `relative` 是否是 `subtree_prefix` 的祖先目录或位于其子树内。
///
/// - `"characters"` 是 `"characters/alice"` 的祖先 → true（需进入才能到达目标）
/// - `"characters/alice"` 是 `"characters/alice"` 自身 → true
/// - `"characters/alice/sessions"` 以 `"characters/alice"` 开头 → true
/// - `"characters/bob"` 不是 `"characters/alice"` 的祖先或子树 → false
///
/// 边界对齐：`"char"` 不应被当作 `"characters"` 的祖先，因此用 `"/"` 分隔符对齐。
fn is_ancestor_or_within(relative: &str, subtree_prefix: &str) -> bool {
    if relative == subtree_prefix {
        return true;
    }
    // relative 是 subtree_prefix 的祖先：subtree_prefix 以 "relative/" 开头
    if subtree_prefix.starts_with(&format!("{relative}/")) {
        return true;
    }
    // relative 在 subtree_prefix 子树内：relative 以 "subtree_prefix/" 开头
    if relative.starts_with(&format!("{subtree_prefix}/")) {
        return true;
    }
    false
}

/// 判断文件路径 `relative` 是否位于 `subtree_prefix` 子树内（含 subtree_prefix 自身）。
///
/// 与 `is_ancestor_or_within` 的区别：文件不会是 subtree_prefix 的祖先，
/// 因此只需检查 `relative == subtree_prefix` 或 `relative` 以 `"subtree_prefix/"` 开头。
fn is_within_subtree(relative: &str, subtree_prefix: &str) -> bool {
    if relative == subtree_prefix {
        return true;
    }
    relative.starts_with(&format!("{subtree_prefix}/"))
}

/// 列出所有 backup 的 manifest。
///
/// 扫描 `data_root/backups/`，跳过所有点号前缀的 staging/pending 目录与非目录入口。
/// 返回的 manifest 按 `created_at` 降序排序（最新在前）。
pub(crate) fn list_backups(data_root: &Path) -> Result<Vec<BackupManifest>, AirpError> {
    let backups_root = data_root.join("backups");
    if !backups_root.exists() {
        return Ok(vec![]);
    }
    let mut manifests: Vec<BackupManifest> = Vec::new();
    for entry in fs::read_dir(&backups_root)? {
        let entry = entry?;
        let ft = match entry.file_type() {
            Ok(ft) => ft,
            Err(_) => continue,
        };
        if !ft.is_dir() {
            continue;
        }
        let file_name = entry.file_name();
        let name = match file_name.to_str() {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        let manifest_path = entry.path().join("manifest.json");
        if !manifest_path.is_file() {
            continue;
        }
        let bytes = fs::read(&manifest_path)?;
        match BackupManifest::from_json_bytes(&bytes) {
            Ok(m) => manifests.push(m),
            Err(e) => {
                tracing::warn!(
                    backup_id = name,
                    error = %e,
                    "跳过无法解析的 backup manifest"
                );
            }
        }
    }
    // 按 created_at 降序排序
    manifests.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(manifests)
}

/// 读取指定 backup_id 的 manifest。
pub(crate) fn read_backup_manifest(
    data_root: &Path,
    backup_id: &str,
) -> Result<BackupManifest, AirpError> {
    let backup_dir = data_root.join("backups").join(backup_id);
    if !backup_dir.is_dir() {
        return Err(AirpError::NotFound(format!("backup {backup_id} not found")));
    }
    let manifest_path = backup_dir.join("manifest.json");
    let bytes = fs::read(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AirpError::NotFound(format!("backup {backup_id} manifest not found"))
        } else {
            AirpError::Io(e)
        }
    })?;
    let manifest = BackupManifest::from_json_bytes(&bytes)?;
    // 校验 backup_id 一致性
    if manifest.backup_id != backup_id {
        return Err(AirpError::Internal(format!(
            "backup_id 不匹配: 路径={backup_id}, manifest={}",
            manifest.backup_id
        )));
    }
    Ok(manifest)
}

/// 校验 backup 完整性。
///
/// 返回 `(checked_files_count, tree_sha256)`。失败返回具体错误。
pub(crate) fn verify_backup(
    data_root: &Path,
    backup_id: &str,
) -> Result<(usize, String), AirpError> {
    let _guard = BACKUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let manifest = read_backup_manifest(data_root, backup_id)?;
    let backup_dir = data_root.join("backups").join(backup_id);
    manifest.verify_against_disk(&backup_dir)?;
    Ok((manifest.files.len(), manifest.tree_sha256))
}

/// 删除指定 backup。
///
/// 不可恢复。删除前会校验 manifest 合法性（防止误删非 backup 目录）。
pub(crate) fn delete_backup(data_root: &Path, backup_id: &str) -> Result<(), AirpError> {
    let _guard = BACKUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // 先校验 manifest 合法性，防止误删
    let manifest = read_backup_manifest(data_root, backup_id)?;
    if manifest.backup_id != backup_id {
        return Err(AirpError::Internal(format!(
            "backup_id 不匹配，拒绝删除: 路径={backup_id}, manifest={}",
            manifest.backup_id
        )));
    }
    let backup_dir = data_root.join("backups").join(backup_id);
    fs::remove_dir_all(&backup_dir)
        .map_err(|e| AirpError::Internal(format!("删除 backup {backup_id} 失败: {e}")))?;
    sync_dir(&data_root.join("backups"))?;
    Ok(())
}

/// 从指定 backup 恢复 data_root。
///
/// ## 流程（决策 E）
///
/// 1. acquire `BACKUP_LOCK`
/// 2. 校验目标 backup 完整性（file set + per-file SHA-256 + tree SHA-256）；fail-closed
///    Workspace scope 在 generic restore 入口被拒绝；Full scope 在当前 data root 或
///    目标 manifest 含 Workspace assets 时也被拒绝，均不进入回滚点创建或 swap
/// 3. 创建回滚 backup（`source: PreRestoreRollback`，`scope: Full`）——保护当前 data_root
/// 4. staging：`data_root/.restore-staging-{backup_id}/`，从 backup `files/` 逐文件复制
///    （路径经 `validate_approved_path` + `safe_resolve_for_write` 双重校验）
/// 5. 根据 `manifest.scope` 选择替换策略：
///    - **Full scope**：移除 `data_root` 下除 `backups/` 与 staging 外的所有顶层条目，
///      再 rename staging 内顶层条目 → `data_root/`
///    - **Character / Session scope**：仅替换 `subtree_prefix` 子树
///      （如 `characters/alice/`），其他 data_root 内容不受影响
/// 6. post-restore 校验：重新枚举恢复范围，与 manifest `files` 对比
///    （允许 secret 文件缺失，因为 restore 不写 secret）
/// 7. 回滚备份创建后的失败会返回结构化 recovery backup ID；进入 swap 前为
///    `BackupRestoreFailed`，进入 swap 后保守归类为 `BackupRestoreOutcomeUnknown`
///    并保留 staging + 回滚备份供恢复
///
/// ## Scoped restore 不变量（v1）
///
/// - 仅替换 `manifest.scope.subtree_prefix()` 子树；其他 character / session /
///   顶层文件保持原样
/// - 替换前移除现有子树（若存在），再 rename staging 子树到目标位置
/// - 子树路径组件经 `validate_approved_path` 校验，拒绝 `..` / 绝对路径 / 反斜杠
///
/// ## 返回
///
/// `(restored_backup_id, rollback_backup_id)` — 调用方可据此向用户报告。
pub(crate) fn restore_backup(
    data_root: &Path,
    backup_id: &str,
) -> Result<(String, String), AirpError> {
    restore_backup_with_post_swap_hook(data_root, backup_id, || Ok(()))
}

fn restore_backup_with_post_swap_hook(
    data_root: &Path,
    backup_id: &str,
    post_swap_hook: impl FnOnce() -> Result<(), AirpError>,
) -> Result<(String, String), AirpError> {
    let _guard = BACKUP_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    // 1. 校验目标 backup 完整性
    let manifest = read_backup_manifest(data_root, backup_id)?;
    if matches!(manifest.scope, BackupScope::Workspace { .. }) {
        return Err(AirpError::BadRequest(
            "generic restore 不支持 Workspace scope；请使用 workspace forward-only rollback"
                .to_string(),
        ));
    }
    let manifest_has_workspace = manifest
        .files
        .iter()
        .any(|file| file.path == "ui/workspaces" || file.path.starts_with("ui/workspaces/"));
    let current_has_workspace = data_root.join("ui").join("workspaces").exists();
    if matches!(manifest.scope, BackupScope::Full)
        && (manifest_has_workspace || current_has_workspace)
    {
        return Err(AirpError::BadRequest(
            "generic full restore cannot replace or remove Workspace assets; use Workspace forward-only recovery"
                .to_string(),
        ));
    }
    let backup_dir = data_root.join("backups").join(backup_id);
    manifest.verify_against_disk(&backup_dir).map_err(|e| {
        AirpError::Internal(format!(
            "restore 拒绝：目标 backup {backup_id} 完整性校验失败: {e}"
        ))
    })?;

    // 2. 创建回滚 backup（Full scope，保护当前 data_root 状态）
    // 调用 create_backup_locked 而非 create_backup，因为本函数已持有 BACKUP_LOCK
    //（std::sync::Mutex 不可重入，调 create_backup 会死锁）。
    let rollback_opts = CreateBackupOptions {
        data_root: data_root.to_path_buf(),
        source: BackupSource::PreRestoreRollback,
        scope: BackupScope::Full,
    };
    let rollback = create_backup_locked(&rollback_opts)?;
    let rollback_id = rollback.backup_id.clone();

    // Failures while preparing the private staging tree are definite: the
    // authoritative data root has not entered the swap yet.
    let staging_dir = prepare_restore_staging(data_root, backup_id, &backup_dir, &manifest)
        .map_err(|error| retained_restore_error(error, &rollback_id, false))?;

    // 5. 根据 scope 选择替换策略
    let subtree_prefix = manifest.scope.subtree_prefix();
    let apply_result = (|| {
        if subtree_prefix.is_empty() {
            // Full scope：替换 data_root 下所有顶层条目（除 backups/ 与 staging）
            swap_full_data_root(data_root, &staging_dir, &rollback_id)?;
        } else {
            // scoped：仅替换 subtree_prefix 子树
            swap_scoped_subtree(data_root, &staging_dir, &subtree_prefix, &rollback_id)?;
        }
        post_swap_hook()?;
        sync_dir(data_root)?;

        // 6. post-restore 校验：重新枚举恢复范围，与 manifest.files 对比
        let expected_files: std::collections::HashSet<String> =
            manifest.files.iter().map(|f| f.path.clone()).collect();
        post_restore_verify(data_root, &manifest, &expected_files)
    })();
    apply_result.map_err(|error| retained_restore_error(error, &rollback_id, true))?;

    Ok((backup_id.to_string(), rollback_id))
}

fn prepare_restore_staging(
    data_root: &Path,
    backup_id: &str,
    backup_dir: &Path,
    manifest: &BackupManifest,
) -> Result<PathBuf, AirpError> {
    let staging_dir = data_root.join(format!(".restore-staging-{backup_id}"));
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(|e| {
            AirpError::Internal(format!(
                "restore: 清理残留 staging {} 失败: {e}",
                staging_dir.display()
            ))
        })?;
    }
    fs::create_dir_all(&staging_dir)?;

    let backup_files_root = backup_dir.join("files");
    for file in &manifest.files {
        crate::revision::tree_hash::validate_approved_path(&file.path).map_err(|e| {
            AirpError::Internal(format!(
                "restore 拒绝：backup 含非法路径 {:?}: {e}",
                file.path
            ))
        })?;
        let dest =
            crate::data_dir::safe_resolve_for_write(&staging_dir, &file.path).map_err(|e| {
                AirpError::Internal(format!(
                    "restore: staging 目标路径 {:?} 解析失败: {e}",
                    file.path
                ))
            })?;
        let src = backup_files_root.join(&file.path);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = read_file_bytes(&src)?;
        let mut file = fs::File::create(&dest)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    sync_directory_tree_bottom_up(&staging_dir)?;
    Ok(staging_dir)
}

fn retained_restore_error(error: AirpError, backup_id: &str, outcome_unknown: bool) -> AirpError {
    tracing::error!(err = %error, recovery_backup_id = %backup_id, outcome_unknown, "backup restore failed after retaining recovery backup");
    if outcome_unknown {
        AirpError::BackupRestoreOutcomeUnknown {
            backup_id: backup_id.to_string(),
        }
    } else {
        AirpError::BackupRestoreFailed {
            backup_id: backup_id.to_string(),
        }
    }
}

/// Full scope restore：移除 data_root 下除 `backups/`、staging 与 `ui/` 外的所有
/// 顶层条目，再 rename staging 内顶层条目 → `data_root/`。`ui/` 单独逐子项替换，
/// 永不删除或 rename `ui/workspaces/`，因此并发 Workspace 首次创建也不会被抹除。
fn swap_full_data_root(
    data_root: &Path,
    staging_dir: &Path,
    rollback_id: &str,
) -> Result<(), AirpError> {
    let staging_name = staging_dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| AirpError::Internal("staging 目录名含非 UTF-8 字节".to_string()))?;
    preflight_full_restore_ui(data_root, staging_dir)?;

    let removed_entries = collect_top_level_entries(data_root);
    for entry in &removed_entries {
        let name = entry.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
            AirpError::Internal("data_root 顶层条目名含非 UTF-8 字节".to_string())
        })?;
        if name == "backups" || name == staging_name || name == "ui" {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry)?;
        if metadata.file_type().is_symlink() {
            // 删 symlink 本身（不跟随）
            std::fs::remove_file(entry)?;
        } else if metadata.is_dir() {
            fs::remove_dir_all(entry)?;
        } else {
            fs::remove_file(entry)?;
        }
    }
    sync_dir(data_root)?;

    // rename staging 内顶层条目 → data_root/
    let staging_entries = collect_top_level_entries(staging_dir);
    for entry in &staging_entries {
        let name = entry
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| AirpError::Internal("staging 顶层条目名含非 UTF-8 字节".to_string()))?;
        if name == "ui" {
            continue;
        }
        let dest = data_root.join(name);
        fs::rename(entry, &dest).map_err(|e| {
            AirpError::Internal(format!(
                "restore: rename staging {} -> data_root/{} 失败: {e}（回滚 backup 仍可用: {rollback_id}）",
                entry.display(),
                name
            ))
        })?;
    }
    swap_full_ui_preserving_workspaces(data_root, staging_dir, rollback_id)?;
    // 清理空 staging 目录
    let _ = fs::remove_dir(staging_dir);
    // Persist top-level renames/removals and staging cleanup after the swap.
    sync_dir(data_root)?;
    Ok(())
}

fn preflight_full_restore_ui(data_root: &Path, staging_dir: &Path) -> Result<(), AirpError> {
    for (label, path) in [
        ("current", data_root.join("ui")),
        ("staging", staging_dir.join("ui")),
    ] {
        if !path.exists() {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(AirpError::Internal(format!(
                "restore: {label} ui path must be a real directory"
            )));
        }
    }
    if staging_dir.join("ui").join("workspaces").exists() {
        return Err(AirpError::BadRequest(
            "generic full restore cannot replace Workspace assets".to_string(),
        ));
    }
    Ok(())
}

/// Replace non-Workspace children of `ui/` without ever removing or renaming
/// the `ui/` directory or its `workspaces/` child.
fn swap_full_ui_preserving_workspaces(
    data_root: &Path,
    staging_dir: &Path,
    rollback_id: &str,
) -> Result<(), AirpError> {
    let current_ui = data_root.join("ui");
    let staging_ui = staging_dir.join("ui");

    if current_ui.exists() {
        for entry in collect_top_level_entries(&current_ui) {
            if entry.file_name().and_then(|name| name.to_str()) == Some("workspaces") {
                continue;
            }
            let metadata = fs::symlink_metadata(&entry)?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(&entry)?;
            } else {
                fs::remove_file(&entry)?;
            }
        }
    }

    if staging_ui.exists() {
        fs::create_dir_all(&current_ui)?;
        for entry in collect_top_level_entries(&staging_ui) {
            let name = entry
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    AirpError::Internal(
                        "restore: staging ui entry contains non-UTF-8 bytes".to_string(),
                    )
                })?;
            if name == "workspaces" {
                return Err(AirpError::BadRequest(
                    "generic full restore cannot replace Workspace assets".to_string(),
                ));
            }
            let destination = current_ui.join(name);
            fs::rename(&entry, &destination).map_err(|error| {
                AirpError::Internal(format!(
                    "restore: rename staging ui entry {} failed: {error} (rollback backup retained: {rollback_id})",
                    entry.display()
                ))
            })?;
        }
        let _ = fs::remove_dir(&staging_ui);
    }
    if current_ui.exists() {
        // Persist non-Workspace child removals and renames. Syncing data_root
        // alone does not persist entries inside ui/ on Unix.
        sync_dir(&current_ui)?;
    }
    Ok(())
}

/// Scoped restore：仅替换 `data_root/{subtree_prefix}/` 子树，
/// 其他 data_root 内容不受影响。
///
/// 流程：
/// 1. 校验 `subtree_prefix` 路径组件合法（拒绝 `..` / 绝对路径 / 反斜杠 / 空字节）
/// 2. 计算目标 `data_root/{subtree_prefix}/` 与源 `staging/{subtree_prefix}/`
/// 3. 若目标存在，rename 到 trash 目录再 remove_dir_all（避免 rename 覆盖失败）
/// 4. 确保 `data_root/{subtree_prefix}` 的父目录存在
/// 5. rename `staging/{subtree_prefix}/` → `data_root/{subtree_prefix}/`
/// 6. 清理 staging 下空的祖先目录
fn swap_scoped_subtree(
    data_root: &Path,
    staging_dir: &Path,
    subtree_prefix: &str,
    rollback_id: &str,
) -> Result<(), AirpError> {
    // 路径安全：subtree_prefix 必须是合法相对路径
    crate::revision::tree_hash::validate_approved_path(subtree_prefix).map_err(|e| {
        AirpError::Internal(format!(
            "scoped restore: subtree_prefix {:?} 非法: {e}",
            subtree_prefix
        ))
    })?;

    let dest_subtree =
        crate::data_dir::safe_resolve_for_write(data_root, subtree_prefix).map_err(|e| {
            AirpError::Internal(format!(
                "scoped restore: 目标子树 {:?} 解析失败: {e}",
                subtree_prefix
            ))
        })?;
    let src_subtree = crate::data_dir::safe_resolve_for_write(staging_dir, subtree_prefix)
        .map_err(|e| {
            AirpError::Internal(format!(
                "scoped restore: 源子树 {:?} 解析失败: {e}",
                subtree_prefix
            ))
        })?;
    if !src_subtree.exists() {
        return Err(AirpError::Internal(format!(
            "scoped restore: staging 中不存在子树 {}（backup 可能是空的，或 manifest 损坏）",
            subtree_prefix
        )));
    }

    // 若目标子树存在，先移到 trash 再删（避免 rename 覆盖现有目录失败）
    if dest_subtree.exists() {
        let trash_dir =
            data_root.join(format!(".restore-trash-{subtree_prefix}").replace('/', "-"));
        if trash_dir.exists() {
            fs::remove_dir_all(&trash_dir).map_err(|e| {
                AirpError::Internal(format!(
                    "scoped restore: 清理残留 trash {} 失败: {e}",
                    trash_dir.display()
                ))
            })?;
        }
        fs::rename(&dest_subtree, &trash_dir).map_err(|e| {
            AirpError::Internal(format!(
                "scoped restore: rename 现有子树 {} -> trash 失败: {e}",
                dest_subtree.display()
            ))
        })?;
        fs::remove_dir_all(&trash_dir).map_err(|e| {
            AirpError::Internal(format!(
                "scoped restore: 删除 trash {} 失败: {e}（回滚 backup 仍可用: {rollback_id}）",
                trash_dir.display(),
            ))
        })?;
    }

    // 确保目标父目录存在
    if let Some(parent) = dest_subtree.parent() {
        fs::create_dir_all(parent)?;
    }

    // rename staging 子树 → data_root 子树
    fs::rename(&src_subtree, &dest_subtree).map_err(|e| {
        AirpError::Internal(format!(
            "scoped restore: rename staging {} -> data_root {} 失败: {e}（回滚 backup 仍可用: {rollback_id}）",
            src_subtree.display(),
            dest_subtree.display()
        ))
    })?;
    let destination_parent = dest_subtree.parent().ok_or_else(|| {
        AirpError::Internal("scoped restore destination has no parent".to_string())
    })?;
    sync_directory_chain_to_root(destination_parent, data_root)?;

    // 清理 staging 下空的祖先目录（如 staging/characters/ 已空则删）
    cleanup_empty_staging_ancestors(staging_dir, subtree_prefix);

    // 清理空 staging 根
    let _ = fs::remove_dir(staging_dir);
    Ok(())
}

fn sync_directory_chain_to_root(start: &Path, root: &Path) -> Result<(), AirpError> {
    for directory in collect_directory_chain_to_root(start, root)? {
        sync_dir(&directory)?;
    }
    Ok(())
}

fn collect_directory_chain_to_root(start: &Path, root: &Path) -> Result<Vec<PathBuf>, AirpError> {
    let canonical_root = fs::canonicalize(root)?;
    let canonical_start = fs::canonicalize(start)?;
    if !canonical_start.starts_with(&canonical_root) {
        return Err(AirpError::Internal(
            "restore sync path is outside data root".to_string(),
        ));
    }
    let mut directory = canonical_start;
    let mut chain = Vec::new();
    loop {
        chain.push(directory.clone());
        if directory == canonical_root {
            return Ok(chain);
        }
        directory = directory.parent().map(Path::to_path_buf).ok_or_else(|| {
            AirpError::Internal("restore sync chain did not reach data root".to_string())
        })?;
    }
}

/// 从 staging 根开始向下清理 `subtree_prefix` 路径中已空的祖先目录。
/// 例如 `subtree_prefix = "characters/alice"`，清理 `staging/characters/`（若空）。
fn cleanup_empty_staging_ancestors(staging_dir: &Path, subtree_prefix: &str) {
    let components: Vec<&str> = subtree_prefix.split('/').collect();
    if components.is_empty() {
        return;
    }
    // 从最深的祖先往上尝试删除空目录
    for depth in (1..components.len()).rev() {
        let ancestor_rel = components[..depth].join("/");
        let ancestor = staging_dir.join(&ancestor_rel);
        if ancestor.is_dir()
            && fs::read_dir(&ancestor)
                .map(|mut d| d.next().is_none())
                .unwrap_or(false)
        {
            let _ = fs::remove_dir(&ancestor);
        }
    }
}

/// 收集目录下所有顶层条目（不递归）。
fn collect_top_level_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    if let Ok(read_dir) = fs::read_dir(dir) {
        for entry in read_dir.flatten() {
            entries.push(entry.path());
        }
    }
    entries
}

/// post-restore 校验：manifest 中的每个文件必须存在且 hash 匹配。额外文件不导致
/// 失败：既有 v1 restore 合同容忍维护窗口外的并发写，本实现还必须保留
/// `ui/workspaces/`。secret 文件本来就不在 manifest 中，也不会由 restore 写入。
fn post_restore_verify(
    data_root: &Path,
    manifest: &BackupManifest,
    expected_files: &std::collections::HashSet<String>,
) -> Result<(), AirpError> {
    let mut actual_files: std::collections::HashSet<String> = std::collections::HashSet::new();
    collect_data_root_files_excluding_backups(data_root, data_root, &mut actual_files)?;

    // 期望集合 = manifest.files（restore 已写入这些文件）
    // 实际集合可能多出 secret 文件（用户在 restore 后手动重建的 secret 文件）——允许
    let mut missing: Vec<String> = expected_files.difference(&actual_files).cloned().collect();
    missing.sort();
    if !missing.is_empty() {
        return Err(AirpError::Internal(format!(
            "post-restore 校验失败：以下 manifest 文件未恢复到 data_root: {missing:?}"
        )));
    }

    // 校验每个恢复文件的 SHA-256 仍与 manifest 一致
    for file in &manifest.files {
        let abs = data_root.join(&file.path);
        let bytes = fs::read(&abs)?;
        let actual_hash = crate::revision::manifest::file_sha256_hex(&bytes);
        if actual_hash != file.sha256 {
            return Err(AirpError::Internal(format!(
                "post-restore 校验失败：文件 {:?} SHA-256 不匹配（manifest={} actual={}）",
                file.path, file.sha256, actual_hash
            )));
        }
    }

    Ok(())
}

/// 递归收集 `data_root` 下所有普通文件（相对路径，`/` 分隔），跳过 `backups/` 子树
/// 与 `.restore-staging-*` / `.staging-*` 临时目录。
fn collect_data_root_files_excluding_backups(
    data_root: &Path,
    current: &Path,
    out: &mut std::collections::HashSet<String>,
) -> Result<(), AirpError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let relative = path
            .strip_prefix(data_root)
            .map_err(|_| {
                AirpError::Internal(format!(
                    "strip_prefix 失败: {} vs {}",
                    path.display(),
                    data_root.display()
                ))
            })?
            .to_str()
            .ok_or_else(|| AirpError::Internal(format!("路径含非 UTF-8 字节: {}", path.display())))?
            .replace('\\', "/");

        if current == data_root {
            // 跳过 backups/ 顶层目录
            if relative == "backups" {
                continue;
            }
            // 跳过 staging 临时目录
            if relative.starts_with(".restore-staging-") || relative.starts_with(".staging-") {
                continue;
            }
        }

        if metadata.is_dir() {
            collect_data_root_files_excluding_backups(data_root, &path, out)?;
        } else if metadata.is_file() {
            out.insert(relative);
        }
    }
    Ok(())
}

/// 同步目录（持久化目录元数据）。
///
/// 与 `revision::atomic::sync_dir` 同语义：
/// - Unix：调用 `sync_data` 并传播错误
/// - Windows：`sync_data` 对目录句柄返回 `ERROR_ACCESS_DENIED`，完全不打开句柄
fn sync_dir(path: &Path) -> Result<(), AirpError> {
    #[cfg(unix)]
    {
        let file = fs::File::open(path)?;
        file.sync_data()
            .map_err(|e| AirpError::Internal(format!("sync_dir {:?} 失败: {e}", path)))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

// 提供 write_all trait 方法所需 import
use std::io::Write;

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_opts(dir: &Path, source: BackupSource, scope: BackupScope) -> CreateBackupOptions {
        CreateBackupOptions {
            data_root: dir.to_path_buf(),
            source,
            scope,
        }
    }

    #[test]
    fn create_backup_empty_data_root_succeeds() {
        let dir = tempdir().unwrap();
        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();
        assert!(created.backup_dir.is_dir());
        assert!(created.backup_dir.join("manifest.json").is_file());
        assert!(created.backup_dir.join("files").is_dir());
        assert_eq!(created.manifest.files.len(), 0);
        assert_eq!(created.manifest.source, BackupSource::Manual);
        assert_eq!(created.manifest.scope, BackupScope::Full);
        assert!(created.manifest.secrets_excluded);
    }

    #[test]
    fn backup_directory_barrier_orders_every_nested_parent_bottom_up() {
        let root = tempdir().unwrap();
        fs::create_dir_all(root.path().join("files/ui/workspaces/default/revisions/1")).unwrap();
        fs::create_dir_all(root.path().join("files/characters/alice")).unwrap();

        let ordered = collect_directory_tree_bottom_up(root.path()).unwrap();
        for directory in &ordered {
            if directory == root.path() {
                continue;
            }
            let parent = directory.parent().unwrap();
            if parent.starts_with(root.path()) {
                let child_index = ordered.iter().position(|item| item == directory).unwrap();
                let parent_index = ordered.iter().position(|item| item == parent).unwrap();
                assert!(
                    child_index < parent_index,
                    "child {} must sync before parent {}",
                    directory.display(),
                    parent.display()
                );
            }
        }
        assert_eq!(ordered.last(), Some(&root.path().to_path_buf()));
        assert_eq!(ordered.len(), 9);
    }

    fn write_nested_backup_source(root: &Path) {
        fs::create_dir_all(root.join("characters/alice")).unwrap();
        fs::write(root.join("characters/alice/card.json"), b"{}").unwrap();
        fs::create_dir_all(root.join("ui/workspaces/default/revisions/1")).unwrap();
        fs::write(
            root.join("ui/workspaces/default/revisions/1/workspace.json"),
            b"{}",
        )
        .unwrap();
    }

    fn backup_sync_trace_label(data_root: &Path, path: &Path) -> String {
        if path == data_root {
            return "$data_root".to_string();
        }
        let relative = path.strip_prefix(data_root).unwrap();
        let mut parts: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts
            .get(1)
            .is_some_and(|part| part.starts_with(".staging-"))
        {
            parts[1] = "$staging".to_string();
        }
        parts.join("/")
    }

    #[test]
    fn backup_publication_traces_real_directory_barriers_in_order() {
        let root = tempdir().unwrap();
        write_nested_backup_source(root.path());
        let opts = make_opts(root.path(), BackupSource::Manual, BackupScope::Full);
        let mut trace = Vec::new();

        let created = create_backup_locked_with_sync(&opts, &mut |path| {
            trace.push(backup_sync_trace_label(root.path(), path));
            Ok(())
        })
        .unwrap();

        assert_eq!(
            trace,
            [
                "backups/$staging/files/ui/workspaces/default/revisions/1",
                "backups/$staging/files/ui/workspaces/default/revisions",
                "backups/$staging/files/ui/workspaces/default",
                "backups/$staging/files/characters/alice",
                "backups/$staging/files/ui/workspaces",
                "backups/$staging/files/characters",
                "backups/$staging/files/ui",
                "backups/$staging/files",
                "backups/$staging",
                "backups",
                "$data_root",
                "backups",
            ]
        );
        created
            .manifest
            .verify_against_disk(&created.backup_dir)
            .unwrap();
    }

    #[test]
    fn every_backup_directory_barrier_failure_avoids_partial_authority() {
        const BARRIER_COUNT: usize = 12;
        for fail_at in 0..BARRIER_COUNT {
            let root = tempdir().unwrap();
            write_nested_backup_source(root.path());
            let opts = make_opts(root.path(), BackupSource::Manual, BackupScope::Full);
            let mut call_index = 0;

            let result = create_backup_locked_with_sync(&opts, &mut |_path| {
                let current = call_index;
                call_index += 1;
                if current == fail_at {
                    Err(AirpError::Internal(format!(
                        "injected backup sync failure at barrier {fail_at}"
                    )))
                } else {
                    Ok(())
                }
            });

            let error = result.expect_err(&format!("barrier {fail_at} unexpectedly succeeded"));
            assert_eq!(call_index, fail_at + 1);
            let listed = list_backups(root.path()).unwrap();
            if fail_at + 1 < BARRIER_COUNT {
                assert!(
                    !matches!(error, AirpError::BackupPublicationOutcomeUnknown { .. }),
                    "barrier {fail_at} was incorrectly classified as final publication"
                );
                assert!(
                    listed.is_empty(),
                    "barrier {fail_at} exposed a backup before final publication"
                );
            } else {
                let backup_id = match error {
                    AirpError::BackupPublicationOutcomeUnknown { backup_id } => backup_id,
                    other => panic!("expected publication outcome unknown, got {other:?}"),
                };
                assert_eq!(listed.len(), 1);
                let manifest = &listed[0];
                assert_eq!(manifest.backup_id, backup_id);
                let backup_dir = root.path().join("backups").join(&manifest.backup_id);
                manifest.verify_against_disk(&backup_dir).unwrap();
            }
        }
    }

    #[test]
    fn scoped_restore_sync_chain_reaches_data_root_bottom_up() {
        let root = tempdir().unwrap();
        let start = root.path().join("characters/alice/sessions");
        fs::create_dir_all(&start).unwrap();
        let canonical_root = fs::canonicalize(root.path()).unwrap();
        let canonical_start = fs::canonicalize(&start).unwrap();

        assert_eq!(
            collect_directory_chain_to_root(&start, root.path()).unwrap(),
            vec![
                canonical_start,
                canonical_root.join("characters/alice"),
                canonical_root.join("characters"),
                canonical_root,
            ]
        );
    }

    #[test]
    fn create_backup_includes_regular_files() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::create_dir_all(dir.path().join("characters").join("alice")).unwrap();
        fs::write(
            dir.path()
                .join("characters")
                .join("alice")
                .join("card.json"),
            "{}",
        )
        .unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        assert_eq!(created.manifest.files.len(), 2);
        let paths: Vec<&str> = created
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(paths.contains(&"a.txt"));
        assert!(paths.contains(&"characters/alice/card.json"));
    }

    #[test]
    fn create_backup_excludes_secrets() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("secrets.json"), r#"{"key":"secret"}"#).unwrap();
        fs::write(dir.path().join("settings.json"), r#"{"api_key":"x"}"#).unwrap();
        fs::write(dir.path().join("providers.json"), "{}").unwrap(); // 非秘密，保留

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        let paths: Vec<&str> = created
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(!paths.contains(&"secrets.json"), "secrets.json 必须被排除");
        assert!(
            !paths.contains(&"settings.json"),
            "settings.json 必须被排除"
        );
        assert!(
            paths.contains(&"providers.json"),
            "providers.json 非秘密，应保留"
        );
    }

    #[test]
    fn create_backup_excludes_backups_dir_itself() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();

        // 先创建一个 backup
        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let first = create_backup(&opts).unwrap();

        // 再创建第二个 backup，应不包含第一个 backup 的文件
        let second = create_backup(&opts).unwrap();

        let paths: Vec<&str> = second
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(paths.contains(&"a.txt"));
        assert!(
            !paths.iter().any(|p| p.starts_with("backups/")),
            "backup 不应包含 backups/ 子树: {:?}",
            paths
        );
        // 第一个 backup 仍存在
        assert!(first.backup_dir.is_dir());
    }

    #[test]
    fn create_scoped_backup_character() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("characters").join("alice")).unwrap();
        fs::write(
            dir.path()
                .join("characters")
                .join("alice")
                .join("card.json"),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("characters").join("bob")).unwrap();
        fs::write(
            dir.path().join("characters").join("bob").join("card.json"),
            "{}",
        )
        .unwrap();
        fs::write(dir.path().join("global.txt"), "g").unwrap();

        let opts = make_opts(
            dir.path(),
            BackupSource::PreDelete,
            BackupScope::Character {
                character_id: "alice".to_string(),
            },
        );
        let created = create_backup(&opts).unwrap();

        let paths: Vec<&str> = created
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(paths.contains(&"characters/alice/card.json"));
        assert!(
            !paths.iter().any(|p| p.contains("characters/bob")),
            "scoped backup 不应包含 bob: {:?}",
            paths
        );
        assert!(
            !paths.contains(&"global.txt"),
            "scoped backup 不应包含全局文件: {:?}",
            paths
        );
        assert_eq!(
            created.manifest.scope,
            BackupScope::Character {
                character_id: "alice".to_string()
            }
        );
    }

    #[test]
    fn create_scoped_backup_session() {
        let dir = tempdir().unwrap();
        let session_path = dir
            .path()
            .join("characters")
            .join("alice")
            .join("sessions")
            .join("sess1");
        fs::create_dir_all(&session_path).unwrap();
        fs::write(session_path.join("current.md"), "x").unwrap();
        // 其他 session
        let other_session = dir
            .path()
            .join("characters")
            .join("alice")
            .join("sessions")
            .join("sess2");
        fs::create_dir_all(&other_session).unwrap();
        fs::write(other_session.join("current.md"), "y").unwrap();

        let opts = make_opts(
            dir.path(),
            BackupSource::PreDelete,
            BackupScope::Session {
                character_id: "alice".to_string(),
                session_id: "sess1".to_string(),
            },
        );
        let created = create_backup(&opts).unwrap();

        let paths: Vec<&str> = created
            .manifest
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect();
        assert!(paths.contains(&"characters/alice/sessions/sess1/current.md"));
        assert!(
            !paths.iter().any(|p| p.contains("sessions/sess2")),
            "session-scoped backup 不应包含 sess2: {:?}",
            paths
        );
    }

    #[test]
    fn workspace_backup_only_contains_fixed_subtree_and_verifies() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("ui").join("workspaces").join("default");
        fs::create_dir_all(workspace.join("revisions").join("7")).unwrap();
        fs::write(
            workspace.join("revisions").join("7").join("workspace.json"),
            "{}",
        )
        .unwrap();
        fs::create_dir_all(dir.path().join("ui").join("workspaces").join("other")).unwrap();
        fs::write(
            dir.path()
                .join("ui")
                .join("workspaces")
                .join("other")
                .join("workspace.json"),
            "other",
        )
        .unwrap();
        fs::write(dir.path().join("outside.txt"), "outside").unwrap();

        let opts = make_opts(
            dir.path(),
            BackupSource::PreMigration,
            BackupScope::Workspace { revision: 7 },
        );
        let created = create_backup(&opts).unwrap();
        let paths: Vec<&str> = created
            .manifest
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect();

        assert_eq!(
            paths,
            vec!["ui/workspaces/default/revisions/7/workspace.json"]
        );
        assert!(verify_backup(dir.path(), &created.backup_id).is_ok());
    }

    #[test]
    fn generic_restore_rejects_workspace_scope() {
        let dir = tempdir().unwrap();
        let workspace = dir.path().join("ui").join("workspaces").join("default");
        fs::create_dir_all(&workspace).unwrap();
        fs::write(workspace.join("CURRENT"), "0\n").unwrap();
        let created = create_backup(&make_opts(
            dir.path(),
            BackupSource::PreMigration,
            BackupScope::Workspace { revision: 0 },
        ))
        .unwrap();

        assert!(matches!(
            restore_backup(dir.path(), &created.backup_id),
            Err(AirpError::BadRequest(message)) if message.contains("Workspace scope")
        ));
        assert_eq!(list_backups(dir.path()).unwrap().len(), 1);
    }

    #[test]
    fn generic_full_restore_cannot_replace_or_remove_workspace_assets() {
        let captured = tempdir().unwrap();
        fs::create_dir_all(captured.path().join("ui/workspaces/default")).unwrap();
        fs::write(
            captured
                .path()
                .join("ui/workspaces/default/current_revision"),
            "1",
        )
        .unwrap();
        let captured_backup = create_backup(&make_opts(
            captured.path(),
            BackupSource::Manual,
            BackupScope::Full,
        ))
        .unwrap();
        assert!(matches!(
            restore_backup(captured.path(), &captured_backup.backup_id),
            Err(AirpError::BadRequest(message)) if message.contains("Workspace assets")
        ));

        let current = tempdir().unwrap();
        fs::write(current.path().join("ordinary.txt"), "before").unwrap();
        let ordinary_backup = create_backup(&make_opts(
            current.path(),
            BackupSource::Manual,
            BackupScope::Full,
        ))
        .unwrap();
        fs::create_dir_all(current.path().join("ui/workspaces/default")).unwrap();
        assert!(matches!(
            restore_backup(current.path(), &ordinary_backup.backup_id),
            Err(AirpError::BadRequest(message)) if message.contains("Workspace assets")
        ));
    }

    #[test]
    fn full_swap_preserves_workspace_while_replacing_other_ui_children() {
        let root = tempdir().unwrap();
        let staging = root.path().join(".restore-staging-race");
        fs::create_dir_all(root.path().join("ui/workspaces/default")).unwrap();
        fs::write(
            root.path().join("ui/workspaces/default/current_revision"),
            "7",
        )
        .unwrap();
        fs::write(root.path().join("ui/obsolete.json"), "old").unwrap();
        fs::create_dir_all(staging.join("ui")).unwrap();
        fs::write(staging.join("ui/replacement.json"), "new").unwrap();

        swap_full_data_root(root.path(), &staging, "rollback-id").unwrap();

        assert_eq!(
            fs::read_to_string(root.path().join("ui/workspaces/default/current_revision")).unwrap(),
            "7"
        );
        assert!(!root.path().join("ui/obsolete.json").exists());
        assert_eq!(
            fs::read_to_string(root.path().join("ui/replacement.json")).unwrap(),
            "new"
        );
    }

    #[test]
    fn verify_backup_passes_for_fresh_backup() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        let (count, tree) = verify_backup(dir.path(), &created.backup_id).unwrap();
        assert_eq!(count, 1);
        assert_eq!(tree, created.manifest.tree_sha256);
    }

    #[test]
    fn verify_backup_detects_tampered_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        // 篡改 backup 内的文件
        fs::write(created.backup_dir.join("files").join("a.txt"), "tampered").unwrap();

        assert!(verify_backup(dir.path(), &created.backup_id).is_err());
    }

    #[test]
    fn verify_backup_detects_missing_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        fs::remove_file(created.backup_dir.join("files").join("a.txt")).unwrap();

        assert!(verify_backup(dir.path(), &created.backup_id).is_err());
    }

    #[test]
    fn verify_backup_detects_extra_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        fs::write(created.backup_dir.join("files").join("extra.txt"), "extra").unwrap();

        assert!(verify_backup(dir.path(), &created.backup_id).is_err());
    }

    #[test]
    fn list_backups_returns_sorted_desc() {
        let dir = tempdir().unwrap();

        // 创建 3 个 backup，时间戳递增
        let mut backups = Vec::new();
        for _ in 0..3 {
            let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
            backups.push(create_backup(&opts).unwrap());
            // 确保时间戳不同（chrono 精度可能不够，加 sleep）
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }

        let listed = list_backups(dir.path()).unwrap();
        assert_eq!(listed.len(), 3);
        // 降序：最新在前
        assert_eq!(listed[0].backup_id, backups[2].backup_id);
        assert_eq!(listed[1].backup_id, backups[1].backup_id);
        assert_eq!(listed[2].backup_id, backups[0].backup_id);
    }

    #[test]
    fn list_backups_skips_staging_and_invalid() {
        let dir = tempdir().unwrap();
        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        create_backup(&opts).unwrap();

        // 创建 staging 残留
        fs::create_dir_all(dir.path().join("backups").join(".staging-xxx")).unwrap();
        // 创建无 manifest 的目录
        fs::create_dir_all(dir.path().join("backups").join("bad")).unwrap();

        let listed = list_backups(dir.path()).unwrap();
        assert_eq!(listed.len(), 1, "应跳过 staging 与无 manifest 目录");
    }

    #[test]
    fn delete_backup_removes_dir() {
        let dir = tempdir().unwrap();
        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        delete_backup(dir.path(), &created.backup_id).unwrap();
        assert!(!created.backup_dir.exists());

        let listed = list_backups(dir.path()).unwrap();
        assert!(listed.is_empty());
    }

    #[test]
    fn delete_backup_rejects_nonexistent() {
        let dir = tempdir().unwrap();
        assert!(delete_backup(dir.path(), "nonexistent").is_err());
    }

    #[test]
    fn read_backup_manifest_returns_not_found_for_missing() {
        let dir = tempdir().unwrap();
        let result = read_backup_manifest(dir.path(), "nonexistent");
        assert!(matches!(result, Err(AirpError::NotFound(_))));
    }

    #[test]
    fn concurrent_backup_creation_serialized() {
        use std::sync::Arc;
        use std::thread;

        let dir = tempdir().unwrap();
        let dir_path = Arc::new(dir.path().to_path_buf());

        let mut handles = Vec::new();
        for _ in 0..3 {
            let p = Arc::clone(&dir_path);
            handles.push(thread::spawn(move || {
                let opts = CreateBackupOptions {
                    data_root: (*p).clone(),
                    source: BackupSource::Manual,
                    scope: BackupScope::Full,
                };
                create_backup(&opts).map(|c| c.backup_id)
            }));
        }

        let mut ids = Vec::new();
        for h in handles {
            ids.push(h.join().unwrap().unwrap());
        }

        // 三个 backup 都应成功创建，id 互异
        assert_eq!(ids.len(), 3);
        let id_set: std::collections::HashSet<&str> = ids.iter().map(|s| s.as_str()).collect();
        assert_eq!(id_set.len(), 3, "backup_id 必须互异");

        let listed = list_backups(&dir_path).unwrap();
        assert_eq!(listed.len(), 3);
    }

    #[test]
    fn backup_manifest_files_paths_are_nfc_and_safe() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("normal.txt"), "x").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        for f in &created.manifest.files {
            // 所有路径必须通过 validate_approved_path
            crate::revision::tree_hash::validate_approved_path(&f.path).unwrap();
        }
    }

    // ── restore 测试 ──────────────────────────────────────────────────────────

    #[test]
    fn restore_full_backup_overwrites_data_root() {
        let dir = tempdir().unwrap();
        // 初始状态：a.txt = "hello", characters/alice/card.json = "{}"
        fs::write(dir.path().join("a.txt"), "hello").unwrap();
        fs::create_dir_all(dir.path().join("characters").join("alice")).unwrap();
        fs::write(
            dir.path()
                .join("characters")
                .join("alice")
                .join("card.json"),
            r#"{"name":"alice"}"#,
        )
        .unwrap();

        // 创建 backup
        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        // 修改 data_root
        fs::write(dir.path().join("a.txt"), "tampered").unwrap();
        fs::write(
            dir.path()
                .join("characters")
                .join("alice")
                .join("card.json"),
            r#"{"name":"changed"}"#,
        )
        .unwrap();
        fs::write(dir.path().join("new_file.txt"), "new").unwrap();

        // restore
        let (restored_from, rollback_id) = restore_backup(dir.path(), &created.backup_id).unwrap();
        assert_eq!(restored_from, created.backup_id);
        assert!(!rollback_id.is_empty());

        // 验证 data_root 恢复到 backup 时的状态
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(
                dir.path()
                    .join("characters")
                    .join("alice")
                    .join("card.json")
            )
            .unwrap(),
            r#"{"name":"alice"}"#
        );
        // new_file.txt 不在 backup 中，应被删除
        assert!(!dir.path().join("new_file.txt").exists());

        // 回滚 backup 存在
        let rollback_dir = dir.path().join("backups").join(&rollback_id);
        assert!(rollback_dir.is_dir());

        // 源 backup 仍存在
        assert!(created.backup_dir.is_dir());

        // backups 列表含源 backup + rollback backup
        let listed = list_backups(dir.path()).unwrap();
        assert_eq!(listed.len(), 2);
    }

    #[test]
    fn restore_creates_rollback_backup_with_correct_source() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "original").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        fs::write(dir.path().join("a.txt"), "modified").unwrap();

        let (_, rollback_id) = restore_backup(dir.path(), &created.backup_id).unwrap();

        // rollback backup 的 source 应为 PreRestoreRollback
        let rollback_manifest = read_backup_manifest(dir.path(), &rollback_id).unwrap();
        assert_eq!(rollback_manifest.source, BackupSource::PreRestoreRollback);
        assert_eq!(rollback_manifest.scope, BackupScope::Full);
        // rollback backup 记录的是 restore 前的状态（"modified"）
        let _rollback_a_txt = rollback_manifest
            .files
            .iter()
            .find(|f| f.path == "a.txt")
            .unwrap();
        let rollback_dir = dir.path().join("backups").join(&rollback_id);
        let content = fs::read_to_string(rollback_dir.join("files").join("a.txt")).unwrap();
        assert_eq!(content, "modified");
    }

    #[test]
    fn restore_rejects_tampered_backup() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        // 篡改 backup 内的文件
        fs::write(created.backup_dir.join("files").join("a.txt"), "tampered").unwrap();

        // restore 应失败（完整性校验不通过）
        assert!(restore_backup(dir.path(), &created.backup_id).is_err());
        // data_root 不应被改动
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn restore_rejects_missing_backup() {
        let dir = tempdir().unwrap();
        assert!(restore_backup(dir.path(), "nonexistent").is_err());
    }

    #[test]
    fn restore_excludes_secrets_from_restored_files() {
        let dir = tempdir().unwrap();
        // 写入 secret 文件 + 普通文件
        fs::write(dir.path().join("secrets.json"), r#"{"key":"secret"}"#).unwrap();
        fs::write(dir.path().join("settings.json"), r#"{"api_key":"x"}"#).unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        // 删除 secret 文件模拟丢失
        fs::remove_file(dir.path().join("secrets.json")).unwrap();
        fs::remove_file(dir.path().join("settings.json")).unwrap();
        fs::write(dir.path().join("a.txt"), "changed").unwrap();

        // restore
        restore_backup(dir.path(), &created.backup_id).unwrap();

        // secret 文件不应被恢复（backup 不含 secret）
        assert!(!dir.path().join("secrets.json").exists());
        assert!(!dir.path().join("settings.json").exists());
        // 普通文件恢复
        assert_eq!(
            fs::read_to_string(dir.path().join("a.txt")).unwrap(),
            "hello"
        );
    }

    #[test]
    fn restore_can_rollback_via_rollback_backup() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "v1").unwrap();

        // 创建 v1 backup (manual)
        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let v1 = create_backup(&opts).unwrap();

        // 修改到 v2
        fs::write(dir.path().join("a.txt"), "v2").unwrap();

        // restore 到 v1（创建 rollback1 = v2 状态）
        let (_, rollback1_id) = restore_backup(dir.path(), &v1.backup_id).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "v1");

        // 再 restore rollback1 回到 v2（创建 rollback2 = v1 状态）
        let (_, rollback2_id) = restore_backup(dir.path(), &rollback1_id).unwrap();
        assert_eq!(fs::read_to_string(dir.path().join("a.txt")).unwrap(), "v2");

        // 现在有 3 个 backup：
        // - v1 manual backup（源 backup，仍存在）
        // - rollback1（PreRestoreRollback，v2 状态，第一次 restore 前创建）
        // - rollback2（PreRestoreRollback，v1 状态，第二次 restore 前创建）
        let listed = list_backups(dir.path()).unwrap();
        assert_eq!(listed.len(), 3, "应有 3 个 backup: {:?}", listed);
        assert!(!rollback2_id.is_empty());
    }

    #[test]
    fn restore_preserves_backups_directory() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "hello").unwrap();

        let opts = make_opts(dir.path(), BackupSource::Manual, BackupScope::Full);
        let created = create_backup(&opts).unwrap();

        fs::write(dir.path().join("a.txt"), "changed").unwrap();

        restore_backup(dir.path(), &created.backup_id).unwrap();

        // backups/ 目录应保留，且包含源 backup + rollback backup
        let backups_dir = dir.path().join("backups");
        assert!(backups_dir.is_dir());
        let listed = list_backups(dir.path()).unwrap();
        assert_eq!(listed.len(), 2);
        // 源 backup 仍存在
        assert!(listed.iter().any(|m| m.backup_id == created.backup_id));
    }

    /// scoped (Character / Session) restore 仅替换目标子树，保留 data_root 下其他
    /// 不相关数据。这是 #342 pre-delete 备份可恢复能力的核心保证：删除 alice 后
    /// 创建的 PreDelete Character-scoped backup 必须能 restore 回 alice，且不影响 bob。
    ///
    /// 参见 `restore_backup` 的 `swap_scoped_subtree` 分支与 §"Scoped restore 不变量"。
    #[test]
    fn restore_scoped_backup_preserves_unrelated_data() {
        let dir = tempdir().unwrap();
        // data_root 下有两个 character，各自有数据
        let alice_dir = dir.path().join("characters").join("alice");
        let bob_dir = dir.path().join("characters").join("bob");
        fs::create_dir_all(&alice_dir).unwrap();
        fs::create_dir_all(&bob_dir).unwrap();
        fs::write(alice_dir.join("card.json"), r#"{"name":"alice"}"#).unwrap();
        fs::write(bob_dir.join("card.json"), r#"{"name":"bob"}"#).unwrap();

        // 创建 alice 的 scoped backup
        let opts = make_opts(
            dir.path(),
            BackupSource::Manual,
            BackupScope::Character {
                character_id: "alice".to_string(),
            },
        );
        let created = create_backup(&opts).unwrap();

        // 模拟删除 alice（直接 remove_dir_all，跳过 PreDelete backup 创建）
        fs::remove_dir_all(&alice_dir).unwrap();
        assert!(!alice_dir.exists(), "alice 应已被删除");

        // restore alice 的 scoped backup —— 应成功
        let (restored_from, _rollback_id) = restore_backup(dir.path(), &created.backup_id).unwrap();
        assert_eq!(restored_from, created.backup_id);

        // 关键不变量：bob 的数据未被改动
        assert!(
            bob_dir.join("card.json").exists(),
            "scoped restore 不应改动 data_root 下其他 character，bob 的数据必须完好"
        );
        let bob_card = fs::read_to_string(bob_dir.join("card.json")).unwrap();
        assert_eq!(bob_card, r#"{"name":"bob"}"#);

        // alice 的数据应被恢复
        assert!(alice_dir.join("card.json").exists());
        let alice_card = fs::read_to_string(alice_dir.join("card.json")).unwrap();
        assert_eq!(alice_card, r#"{"name":"alice"}"#);
    }

    #[test]
    fn restore_preparation_failure_returns_retained_backup_id() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("state.txt"), "captured").unwrap();
        let created = create_backup(&make_opts(
            dir.path(),
            BackupSource::Manual,
            BackupScope::Full,
        ))
        .unwrap();
        fs::write(dir.path().join("state.txt"), "current").unwrap();

        let staging_path = dir
            .path()
            .join(format!(".restore-staging-{}", created.backup_id));
        fs::write(&staging_path, "not a directory").unwrap();

        let error = restore_backup(dir.path(), &created.backup_id).unwrap_err();
        let rollback_id = match error {
            AirpError::BackupRestoreFailed { backup_id } => backup_id,
            other => panic!("expected definite retained-backup failure, got {other:?}"),
        };
        assert_eq!(
            fs::read_to_string(dir.path().join("state.txt")).unwrap(),
            "current"
        );
        let rollback_manifest = read_backup_manifest(dir.path(), &rollback_id).unwrap();
        assert_eq!(rollback_manifest.source, BackupSource::PreRestoreRollback);
        rollback_manifest
            .verify_against_disk(&dir.path().join("backups").join(rollback_id))
            .unwrap();
    }

    #[test]
    fn post_swap_failure_returns_outcome_unknown_with_retained_backup_id() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("state.txt"), "captured").unwrap();
        let created = create_backup(&make_opts(
            dir.path(),
            BackupSource::Manual,
            BackupScope::Full,
        ))
        .unwrap();
        fs::write(dir.path().join("state.txt"), "current").unwrap();

        let error = restore_backup_with_post_swap_hook(dir.path(), &created.backup_id, || {
            Err(AirpError::Internal(
                "injected post-swap failure with private detail".to_string(),
            ))
        })
        .unwrap_err();
        let rollback_id = match error {
            AirpError::BackupRestoreOutcomeUnknown { backup_id } => backup_id,
            other => panic!("expected outcome-unknown retained-backup failure, got {other:?}"),
        };
        assert_eq!(
            fs::read_to_string(dir.path().join("state.txt")).unwrap(),
            "captured"
        );
        let rollback_manifest = read_backup_manifest(dir.path(), &rollback_id).unwrap();
        assert_eq!(rollback_manifest.source, BackupSource::PreRestoreRollback);
        assert_eq!(
            fs::read_to_string(
                dir.path()
                    .join("backups")
                    .join(&rollback_id)
                    .join("files/state.txt")
            )
            .unwrap(),
            "current"
        );
    }

    /// #449: restore swap 阶段失败后必须保留 staging + rollback backup，
    /// 供人工恢复。不清理现场，data_root 不半删。
    ///
    /// 触发方式：scoped restore 的 `swap_scoped_subtree` 在 `create_dir_all(parent)`
    /// 时失败——把 `data_root/characters` 从目录替换为文件，使 `create_dir_all`
    /// 无法创建 `characters/alice` 的父目录 `characters/`。
    #[test]
    fn restore_swap_failure_preserves_staging_and_rollback() {
        let dir = tempdir().unwrap();
        // 创建 character 子树
        let alice_dir = dir.path().join("characters").join("alice");
        fs::create_dir_all(&alice_dir).unwrap();
        fs::write(alice_dir.join("card.json"), r#"{"name":"alice"}"#).unwrap();

        // 创建 Character scope backup
        let opts = make_opts(
            dir.path(),
            BackupSource::Manual,
            BackupScope::Character {
                character_id: "alice".to_string(),
            },
        );
        let created = create_backup(&opts).unwrap();

        // 破坏 data_root：把 characters/ 目录替换成同名文件，
        // 使 swap_scoped_subtree 的 create_dir_all(data_root/characters) 失败
        fs::remove_dir_all(dir.path().join("characters")).unwrap();
        fs::write(dir.path().join("characters"), "not a directory").unwrap();

        // Once swap has been entered, callers receive a conservative
        // outcome-unknown classification plus the retained recovery ID.
        let rollback_id = match restore_backup(dir.path(), &created.backup_id).unwrap_err() {
            AirpError::BackupRestoreOutcomeUnknown { backup_id } => backup_id,
            other => panic!("expected outcome-unknown restore failure, got {other:?}"),
        };

        // 1. staging 目录应保留（.restore-staging-{backup_id}）
        let staging_dir = dir
            .path()
            .join(format!(".restore-staging-{}", created.backup_id));
        assert!(
            staging_dir.exists(),
            "staging dir should be preserved on swap failure: {}",
            staging_dir.display()
        );

        // 2. rollback backup 应存在且可读（PreRestoreRollback source）
        let backups_dir = dir.path().join("backups");
        let manifest = read_backup_manifest(dir.path(), &rollback_id).unwrap();
        assert_eq!(manifest.source, BackupSource::PreRestoreRollback);
        manifest
            .verify_against_disk(&backups_dir.join(&rollback_id))
            .unwrap();

        // 3. data_root 不半删：characters 仍是文件（swap 未完成，alice 未恢复）
        assert!(
            dir.path().join("characters").is_file(),
            "data_root should not be half-deleted: characters should still be the file we created"
        );
        assert!(
            !alice_dir.exists(),
            "alice subtree should not exist (restore did not complete)"
        );
    }
}
