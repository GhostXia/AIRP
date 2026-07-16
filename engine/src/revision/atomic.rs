//! Atomic commit 流程（#115 Phase 2a）。
//!
//! 参考 spec §5.1.3 与 SESSION-DATA-DESIGN.md §5.3：
//! 1. 在 `{asset_dir}/revisions/.staging-{revision_id}/` 同文件系统 staging 目录写完批准文件 + `manifest.json`
//! 2. 逐文件 `sync_data`
//! 3. 同步 staging 目录
//! 4. 全量校验（文件集合 + 每文件 hash + tree hash + manifest 不变量）
//! 5. 原子 rename 为 `{asset_dir}/revisions/{revision_id}/`
//! 6. 同步 `revisions/` 父目录
//! 7. 原子替换 `{asset_dir}/current_revision` 文件内容为 `revision_id` 的十进制字符串
//! 8. 同步 `current_revision` 父目录
//!
//! 任一步失败只留下不被引用的 staging / orphan revision；`current_revision` 永不指向半成品快照。

use crate::error::AirpError;
use crate::revision::manifest::{
    file_sha256_hex, ApprovedFile, AssetKind, AssetSource, RevisionManifest,
};
use crate::revision::tree_hash::compute_tree_sha256;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// 全局 commit 串行化锁。
///
/// `commit_revision` 的 read-current → staging → publish 流程不是无锁原子的：
/// 两个并发 commit 都可能通过 initial read，之后较低 revision 的 rename
/// 可能覆盖较高 revision 的指针（TOCTOU）。Rust std 没有 CAS rename，
/// 因此用进程内全局 Mutex 串行化所有 asset 的 commit。
///
/// 跨进程安全是调用方责任：AIRP daemon 是单进程前台运行（AGENTS.md），
/// 无跨进程 writer；各 asset service 已有 per-asset Mutex 串行化。
///
/// 全局粒度是 Phase 2a 的保守选择；per-asset 锁是后续优化，不影响正确性。
static COMMIT_LOCK: Mutex<()> = Mutex::new(());

/// 已准备的 revision 内容：批准文件 + manifest 元数据。
///
/// 调用方构造 `StagedRevision` 后交给 [`commit_revision`] 写入磁盘。
#[derive(Debug, Clone)]
pub(crate) struct StagedRevision {
    pub content_revision: u64,
    pub asset_kind: AssetKind,
    pub asset_id: String,
    pub created_at: String,
    pub source: AssetSource,
    /// 批准文件集合：(相对路径, 文件内容 bytes)。
    pub files: Vec<(String, Vec<u8>)>,
}

/// commit 选项。
#[derive(Debug, Clone)]
pub(crate) struct CommitOptions {
    /// asset 根目录（如 `characters/{character_id}`）。
    /// 函数会在其下创建 `revisions/{revision_id}/` 和 `current_revision`。
    pub asset_dir: PathBuf,
}

impl CommitOptions {
    pub(crate) fn new(asset_dir: impl Into<PathBuf>) -> Self {
        Self {
            asset_dir: asset_dir.into(),
        }
    }

    fn revisions_dir(&self) -> PathBuf {
        self.asset_dir.join("revisions")
    }

    fn revision_dir(&self, revision: u64) -> PathBuf {
        self.revisions_dir().join(revision.to_string())
    }

    fn staging_dir(&self, revision: u64) -> PathBuf {
        self.revisions_dir().join(format!(".staging-{revision}"))
    }

    fn current_revision_path(&self) -> PathBuf {
        self.asset_dir.join("current_revision")
    }
}

/// 执行 atomic commit。
///
/// 返回最终 revision 目录路径。
///
/// # 不变量
///
/// - `content_revision` 必须 >= 1
/// - `content_revision` 必须 > 现有 `current_revision`（若存在），防止回退
/// - 所有 `staged.files` 路径在写入前预校验（拒绝绝对路径 / `..` / 重复）
/// - `current_revision` 指针通过原子 rename 更新（无 missing-pointer 窗口）
/// - 任一步失败只留下不被引用的 staging / orphan revision
pub(crate) fn commit_revision(
    staged: &StagedRevision,
    options: &CommitOptions,
) -> Result<PathBuf, AirpError> {
    // 进程内串行化：防止并发 commit 的 TOCTOU 导致指针回退。
    // 跨进程安全由调用方负责（AIRP daemon 单进程，无跨进程 writer）。
    let _commit_guard = COMMIT_LOCK
        .lock()
        .map_err(|e| AirpError::Internal(format!("COMMIT_LOCK poisoned: {e}")))?;

    if staged.content_revision < 1 {
        return Err(AirpError::BadRequest(format!(
            "content_revision 必须 >= 1, 实际 {}",
            staged.content_revision
        )));
    }

    // 拒绝低于或等于现有 current_revision 的 commit（防止回退）
    if let Some(existing) = read_current_revision(&options.asset_dir)? {
        if staged.content_revision <= existing {
            return Err(AirpError::BadRequest(format!(
                "content_revision {} 必须 > 现有 current_revision {}，不允许回退",
                staged.content_revision, existing
            )));
        }
    }

    let revisions_dir = options.revisions_dir();
    let staging_dir = options.staging_dir(staged.content_revision);
    let revision_dir = options.revision_dir(staged.content_revision);

    // 如果目标 revision 已存在，拒绝（不可覆盖不可变 revision）
    if revision_dir.exists() {
        return Err(AirpError::BadRequest(format!(
            "revision {} 已存在，不可覆盖",
            staged.content_revision
        )));
    }

    // 预校验所有 staged 文件路径（防止 path traversal / 绝对路径 / 重复）
    validate_staged_paths(&staged.files)?;

    // 清理可能残留的 staging 目录
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).map_err(|e| {
            AirpError::Internal(format!(
                "清理残留 staging 目录 {} 失败: {e}",
                staging_dir.display()
            ))
        })?;
    }

    fs::create_dir_all(&staging_dir)?;

    // 1. 写入批准文件
    let mut approved_files: Vec<ApprovedFile> = Vec::new();
    for (relative_path, content) in &staged.files {
        let abs_path = staging_dir.join(relative_path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(&abs_path)?;
        file.write_all(content)?;
        file.sync_all()?;
        let hash = file_sha256_hex(content);
        approved_files.push(ApprovedFile {
            path: relative_path.clone(),
            sha256: hash,
            bytes: content.len() as u64,
        });
    }

    // 2. 计算 tree_sha256（覆盖 staging 目录下所有批准文件，排除 manifest.json）
    let tree_sha256 = compute_tree_sha256(&staging_dir)?;

    // 3. 构造 manifest 并写入 staging
    let manifest = RevisionManifest {
        schema: crate::revision::manifest::MANIFEST_SCHEMA,
        content_revision: staged.content_revision,
        asset_kind: staged.asset_kind,
        asset_id: staged.asset_id.clone(),
        created_at: staged.created_at.clone(),
        source: staged.source.clone(),
        files: approved_files,
        tree_sha256,
    };
    let manifest_bytes = manifest.to_json_bytes()?;
    let manifest_path = staging_dir.join("manifest.json");
    {
        let mut file = fs::File::create(&manifest_path)?;
        file.write_all(&manifest_bytes)?;
        file.sync_all()?;
    }

    // 4. 全量校验（manifest.verify_against_disk 会校验文件集合 + hash + tree hash）
    manifest.verify_against_disk(&staging_dir).map_err(|e| {
        AirpError::Internal(format!("staging 全量校验失败（不应发生，请报告 bug）: {e}"))
    })?;

    // 5. 同步 staging 目录（必须在 rename 前关闭所有目录句柄，否则 Windows 上 rename 返回 ACCESS_DENIED）。
    //    传播 sync_data 错误：数据完整性场景下不应吞掉 durabilities 错误。
    sync_dir(&staging_dir)?;

    // 6. 原子 rename staging -> revision 目录
    fs::rename(&staging_dir, &revision_dir).map_err(|e| {
        AirpError::Internal(format!(
            "rename staging {} -> revision {} 失败: {e}",
            staging_dir.display(),
            revision_dir.display()
        ))
    })?;

    // 7. 同步 revisions/ 父目录
    sync_dir(&revisions_dir)?;

    // 8. 原子替换 current_revision 文件。
    //    Rust std::fs::rename 在 Windows 上对文件用 MoveFileExW(MOVEFILE_REPLACE_EXISTING)，
    //    在 Unix 上是原子 rename(2)，均可原子替换已存在的目标文件，无需先 remove_file。
    //
    //    使用 per-revision unique temp 文件名（current_revision.{revision_id}.tmp），
    //    防御性设计：即使将来移除全局锁，并发 commit 也不会互相覆盖 temp 文件。
    let current_revision_path = options.current_revision_path();
    let current_tmp = options
        .asset_dir
        .join(format!("current_revision.{}.tmp", staged.content_revision));
    {
        let mut file = fs::File::create(&current_tmp)?;
        file.write_all(staged.content_revision.to_string().as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&current_tmp, &current_revision_path).map_err(|e| {
        AirpError::Internal(format!(
            "rename current_revision tmp {} -> {} 失败: {e}",
            current_tmp.display(),
            current_revision_path.display()
        ))
    })?;

    // 9. 同步 current_revision 父目录（asset_dir）
    sync_dir(&options.asset_dir)?;

    Ok(revision_dir)
}

/// 预校验 staged 文件路径：拒绝绝对路径 / `..` / `.` / 重复 / 空段 / 反斜杠。
fn validate_staged_paths(files: &[(String, Vec<u8>)]) -> Result<(), AirpError> {
    use crate::revision::tree_hash::validate_approved_path;
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (relative_path, _) in files {
        validate_approved_path(relative_path).map_err(|e| {
            AirpError::BadRequest(format!("staged 文件路径非法 {relative_path:?}: {e}"))
        })?;
        if !seen.insert(relative_path.as_str()) {
            return Err(AirpError::BadRequest(format!(
                "staged 文件路径重复: {relative_path:?}"
            )));
        }
    }
    Ok(())
}

/// 读取 `current_revision` 文件，返回当前 revision。
///
/// 语义：
/// - 文件不存在：返回 `Ok(None)`（asset 未升级到 revision 合同）
/// - 文件存在但空或纯空白：返回 `Err`（损坏的指针，不应静默当作"未升级"）
/// - 文件内容解析为 0：返回 `Err`（revision 0 非法）
/// - 文件内容解析为合法 u64 >= 1：返回 `Ok(Some(revision))`
pub(crate) fn read_current_revision(asset_dir: &Path) -> Result<Option<u64>, AirpError> {
    let path = asset_dir.join("current_revision");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Err(AirpError::Internal(format!(
            "current_revision 文件 {:?} 存在但为空，指针损坏",
            path
        )));
    }
    let revision: u64 = trimmed.parse().map_err(|e| {
        AirpError::Internal(format!("current_revision 文件内容非法 {:?}: {e}", trimmed))
    })?;
    if revision == 0 {
        return Err(AirpError::Internal(format!(
            "current_revision 文件 {:?} 内容为 0，revision 0 非法",
            path
        )));
    }
    Ok(Some(revision))
}

/// 计算下次 commit 应使用的 `content_revision`。
///
/// 取 `max(current_revision 指针, 磁盘上已有 revision_dir 最大编号) + 1`。
/// 两者都不存在时返回 `1`（lazy migration 首次写入）。
///
/// # 为什么需要扫描磁盘上的 revision_dir
///
/// `commit_revision` 在第 5 步（staging → revision_dir rename）成功后、第 8 步
/// （current_revision 指针原子替换）前发生失败（I/O 错误、磁盘满、进程被
/// SIGKILL/断电）时，会留下一个**完整的 orphan revision_dir**：内容已写入、
/// manifest 已校验、目录已 rename，但 `current_revision` 文件未更新指向它。
///
/// 此时若调用方仅依据 `current_revision` 推导下次的 `content_revision`，会
/// 得到与 orphan 相同的编号，`commit_revision` 的 `revision_dir.exists()`
/// 不变量检查会拒绝写入，asset 进入**永久不可写**状态。
///
/// 本函数同时扫描 `revisions/` 目录下的数字命名子目录，跳过 orphan revision_dir，
/// 让下次 commit 使用更高的编号。orphan 目录本身保持不动（不可变快照，可能是
/// 合法内容）；调用方或运维可在事后清理。
///
/// # 性能
///
/// `revisions/` 目录通常很小（一个 asset 的历史版本数），`read_dir` 开销可忽略。
/// 写路径已通过 per-asset Mutex 串行化，无并发竞争。
pub(crate) fn next_content_revision(asset_dir: &Path) -> Result<u64, AirpError> {
    let from_pointer = read_current_revision(asset_dir)?.unwrap_or(0);
    let from_disk = max_existing_revision_dir(asset_dir)?.unwrap_or(0);
    Ok(from_pointer.max(from_disk) + 1)
}

/// 扫描 `revisions/` 目录，返回数字命名子目录的最大编号。
///
/// 跳过非目录、非数字命名、`.` 前缀（如 staging 残留 `.staging-{id}/`）的入口。
/// 目录不存在或无合法子目录时返回 `Ok(None)`。
fn max_existing_revision_dir(asset_dir: &Path) -> Result<Option<u64>, AirpError> {
    let revisions_dir = asset_dir.join("revisions");
    if !revisions_dir.exists() {
        return Ok(None);
    }
    let mut max: Option<u64> = None;
    for entry in fs::read_dir(&revisions_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // 跳过 staging 目录（`.staging-{revision_id}`）和其它 dot-prefixed 入口。
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n,
            None => continue,
        };
        if name.starts_with('.') {
            continue;
        }
        if let Ok(rev) = name.parse::<u64>() {
            max = Some(max.map_or(rev, |m| m.max(rev)));
        }
    }
    Ok(max)
}

/// 同步目录（持久化目录元数据，确保 rename 等变更落盘）。
///
/// - Unix：调用 `sync_data` 并传播错误（目录元数据持久化对崩溃恢复至关重要）。
/// - Windows：`sync_data` 对目录句柄返回 `ERROR_ACCESS_DENIED`（操作系统不支持），
///   且打开目录句柄会延迟释放，导致后续 `fs::rename` 返回 ACCESS_DENIED。
///   因此 Windows 上完全不打开目录句柄，直接返回 `Ok(())`；
///   NTFS 的 rename 本身是原子的，目录元数据由文件系统保证一致性。
///
/// 文件内容的持久化由写入时的 `file.sync_all()` 保证，与目录 sync 独立。
fn sync_dir(path: &Path) -> Result<(), AirpError> {
    #[cfg(unix)]
    {
        let file = fs::File::open(path)?;
        file.sync_data()
            .map_err(|e| AirpError::Internal(format!("sync_dir {:?} 失败: {e}", path)))?;
    }
    #[cfg(not(unix))]
    {
        // Windows: 目录 sync_data 不被支持（返回 ACCESS_DENIED），且打开目录句柄
        // 会延迟释放导致后续 rename 失败，因此完全不打开句柄。
        // NTFS rename 原子性由文件系统保证；文件内容已由 sync_all 持久化。
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn staged(revision: u64) -> StagedRevision {
        StagedRevision {
            content_revision: revision,
            asset_kind: AssetKind::Character,
            asset_id: "alice".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource {
                source_kind: "controlled_upload".to_string(),
                ..Default::default()
            },
            files: vec![
                ("card.json".to_string(), b"{\"name\":\"alice\"}".to_vec()),
                ("raw.json".to_string(), b"{\"raw\":true}".to_vec()),
            ],
        }
    }

    #[test]
    fn commit_creates_revision_dir_and_current_pointer() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = staged(1);

        let revision_dir = commit_revision(&staged, &options).unwrap();

        assert!(revision_dir.is_dir(), "revision 目录应存在");
        assert!(revision_dir.join("manifest.json").is_file());
        assert!(revision_dir.join("card.json").is_file());
        assert!(revision_dir.join("raw.json").is_file());

        let current = read_current_revision(dir.path()).unwrap();
        assert_eq!(current, Some(1));

        let current_file = dir.path().join("current_revision");
        let content = fs::read_to_string(&current_file).unwrap();
        assert_eq!(content.trim(), "1");
    }

    #[test]
    fn commit_rejects_revision_zero() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = staged(0);
        let result = commit_revision(&staged, &options);
        assert!(result.is_err());
    }

    #[test]
    fn commit_rejects_existing_revision() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = staged(1);

        commit_revision(&staged, &options).unwrap();
        let result = commit_revision(&staged, &options);
        assert!(result.is_err(), "重复 commit 同一 revision 应失败");
    }

    #[test]
    fn commit_multiple_revisions_advances_pointer() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());

        commit_revision(&staged(1), &options).unwrap();
        assert_eq!(read_current_revision(dir.path()).unwrap(), Some(1));

        commit_revision(&staged(2), &options).unwrap();
        assert_eq!(read_current_revision(dir.path()).unwrap(), Some(2));

        commit_revision(&staged(3), &options).unwrap();
        assert_eq!(read_current_revision(dir.path()).unwrap(), Some(3));

        // 旧 revision 目录应保留（不可变）
        assert!(options.revision_dir(1).is_dir());
        assert!(options.revision_dir(2).is_dir());
        assert!(options.revision_dir(3).is_dir());
    }

    #[test]
    fn read_current_revision_returns_none_when_missing() {
        let dir = tempdir().unwrap();
        let result = read_current_revision(dir.path()).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn read_current_revision_parses_value() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("current_revision"), "42").unwrap();
        let result = read_current_revision(dir.path()).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn read_current_revision_trims_whitespace() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("current_revision"), "  42\n  ").unwrap();
        let result = read_current_revision(dir.path()).unwrap();
        assert_eq!(result, Some(42));
    }

    #[test]
    fn read_current_revision_rejects_empty_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("current_revision"), "   \n  ").unwrap();
        let result = read_current_revision(dir.path());
        assert!(result.is_err(), "空 current_revision 文件应视为损坏");
    }

    #[test]
    fn read_current_revision_rejects_revision_zero() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("current_revision"), "0").unwrap();
        let result = read_current_revision(dir.path());
        assert!(result.is_err(), "revision 0 非法，应视为损坏");
    }

    #[test]
    fn read_current_revision_rejects_non_numeric() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("current_revision"), "abc").unwrap();
        let result = read_current_revision(dir.path());
        assert!(result.is_err(), "非数字内容应视为损坏");
    }

    #[test]
    fn commit_rejects_revision_lower_than_current() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());

        commit_revision(&staged(3), &options).unwrap();
        assert_eq!(read_current_revision(dir.path()).unwrap(), Some(3));

        // 尝试 commit revision 2（低于 current 3），应拒绝
        let result = commit_revision(&staged(2), &options);
        assert!(result.is_err(), "不允许回退到低于 current 的 revision");

        // 指针应仍为 3
        assert_eq!(read_current_revision(dir.path()).unwrap(), Some(3));
    }

    #[test]
    fn commit_rejects_revision_equal_to_current() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());

        commit_revision(&staged(3), &options).unwrap();

        // 删除 revision_dir 3，确保第二次 commit 的失败来自 current_revision equality guard，
        // 而非 existing-directory rejection（否则测试即使移除 equality guard 仍会 green）
        fs::remove_dir_all(options.revision_dir(3)).unwrap();

        // 尝试 commit revision 3（等于 current 3），应拒绝
        // 注意：这与 commit_rejects_existing_revision 不同——后者因 revision_dir 已存在拒绝，
        // 此测试因 current_revision 回退保护拒绝（即使 revision_dir 不存在）
        let result = commit_revision(&staged(3), &options);
        assert!(result.is_err(), "不允许 commit 等于 current 的 revision");
    }

    // ── next_content_revision 单元测试 ──────────────────────────────────────────

    #[test]
    fn next_content_revision_returns_1_when_no_pointer_and_no_dirs() {
        // 全新 asset：无 current_revision 文件、无 revisions/ 目录 → 返回 1（lazy migration）
        let dir = tempdir().unwrap();
        assert_eq!(next_content_revision(dir.path()).unwrap(), 1);
    }

    #[test]
    fn next_content_revision_returns_pointer_plus_1_in_normal_case() {
        // 正常场景：current_revision=3，磁盘上有 revisions/1/2/3/ → 返回 4
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        commit_revision(&staged(1), &options).unwrap();
        commit_revision(&staged(2), &options).unwrap();
        commit_revision(&staged(3), &options).unwrap();
        assert_eq!(next_content_revision(dir.path()).unwrap(), 4);
    }

    #[test]
    fn next_content_revision_skips_orphan_equal_to_pointer() {
        // orphan 场景 1：current_revision 不存在（指针缺失），但磁盘上有 orphan
        // revisions/1/。next_content_revision 应跳过 orphan 返回 2，而非 1
        // （否则 commit_revision 会因 revision_dir 已存在而拒绝）。
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("revisions").join("1")).unwrap();
        assert_eq!(next_content_revision(dir.path()).unwrap(), 2);
    }

    #[test]
    fn next_content_revision_skips_orphan_higher_than_pointer() {
        // orphan 场景 2：current_revision=3，但磁盘上有 orphan revisions/5/
        // （commit_revision 第 5 步成功后崩溃、current_revision 未更新到 5）。
        // next_content_revision 应返回 6（max(3, 5) + 1），而非 4。
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        commit_revision(&staged(3), &options).unwrap();
        fs::create_dir_all(dir.path().join("revisions").join("5")).unwrap();
        assert_eq!(next_content_revision(dir.path()).unwrap(), 6);
    }

    #[test]
    fn next_content_revision_ignores_staging_dirs_and_non_numeric_names() {
        // staging 目录（`.staging-xxx`）和非数字命名目录不应影响编号计算。
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        commit_revision(&staged(2), &options).unwrap();
        fs::create_dir_all(dir.path().join("revisions").join(".staging-abc")).unwrap();
        fs::create_dir_all(dir.path().join("revisions").join("not-a-number")).unwrap();
        fs::create_dir_all(dir.path().join("revisions").join("3.tmp")).unwrap();
        assert_eq!(next_content_revision(dir.path()).unwrap(), 3);
    }

    #[test]
    fn next_content_revision_propagates_io_error_on_corrupted_pointer() {
        // current_revision 文件内容非法（"0"）→ read_current_revision 返回 Err，
        // next_content_revision 应传播错误而非静默降级。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("current_revision"), "0").unwrap();
        assert!(next_content_revision(dir.path()).is_err());
    }

    #[test]
    fn commit_rejects_path_traversal_in_staged_files() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = StagedRevision {
            content_revision: 1,
            asset_kind: AssetKind::Character,
            asset_id: "alice".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource::default(),
            files: vec![("../escape.json".to_string(), b"evil".to_vec())],
        };
        let result = commit_revision(&staged, &options);
        assert!(result.is_err(), "应拒绝 path traversal 路径");

        // 确保没有文件被写到 staging 外。
        // `../escape.json` 从 staging 目录（revisions/.staging-1/）出发，
        // 解析为 revisions/escape.json，而非 tempdir 的 parent。
        assert!(
            !options.revisions_dir().join("escape.json").exists(),
            "path traversal 不应写入 staging 外"
        );
    }

    #[test]
    fn commit_rejects_absolute_path_in_staged_files() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = StagedRevision {
            content_revision: 1,
            asset_kind: AssetKind::Character,
            asset_id: "alice".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource::default(),
            files: vec![("/etc/passwd".to_string(), b"evil".to_vec())],
        };
        let result = commit_revision(&staged, &options);
        assert!(result.is_err(), "应拒绝绝对路径");
    }

    #[test]
    fn commit_rejects_duplicate_paths_in_staged_files() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = StagedRevision {
            content_revision: 1,
            asset_kind: AssetKind::Character,
            asset_id: "alice".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource::default(),
            files: vec![
                ("card.json".to_string(), b"{}".to_vec()),
                ("card.json".to_string(), b"{}".to_vec()),
            ],
        };
        let result = commit_revision(&staged, &options);
        assert!(result.is_err(), "应拒绝重复路径");
    }

    #[test]
    fn commit_staging_cleanup_on_target_exists() {
        // 残留 staging 目录应被清理
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staging = options.staging_dir(1);
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("garbage.txt"), "x").unwrap();

        let result = commit_revision(&staged(1), &options);
        assert!(result.is_ok(), "应清理残留 staging 后正常 commit");
    }

    #[test]
    fn commit_preserves_file_content_via_hash() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = staged(1);

        let revision_dir = commit_revision(&staged, &options).unwrap();

        // 读取 manifest 并验证
        let manifest_bytes = fs::read(revision_dir.join("manifest.json")).unwrap();
        let manifest: RevisionManifest =
            RevisionManifest::from_json_bytes(&manifest_bytes).unwrap();

        // 再次校验
        manifest.verify_against_disk(&revision_dir).unwrap();

        assert_eq!(manifest.files.len(), 2);
        assert_eq!(manifest.content_revision, 1);
    }

    #[test]
    fn commit_with_nested_file_paths() {
        let dir = tempdir().unwrap();
        let options = CommitOptions::new(dir.path());
        let staged = StagedRevision {
            content_revision: 1,
            asset_kind: AssetKind::Worldbook,
            asset_id: "scene1".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource::default(),
            files: vec![
                ("lorebook.json".to_string(), b"{}".to_vec()),
                ("sub/extra.json".to_string(), b"{\"k\":1}".to_vec()),
            ],
        };

        let revision_dir = commit_revision(&staged, &options).unwrap();
        assert!(revision_dir.join("lorebook.json").is_file());
        assert!(revision_dir.join("sub").join("extra.json").is_file());
    }

    #[test]
    fn concurrent_commits_never_roll_back_pointer() {
        // 并发 commit 回归测试（CodeRabbit #1 critical）：
        // 多线程并发 commit 不同 revision，验证 publish 前重新校验（optimistic concurrency control）
        // 防止指针回退。最终指针必须 == 成功 publish 中的最大 revision。
        //
        // 注意：本测试不验证跨进程安全（AIRP daemon 单进程，无跨进程 writer）。
        use std::sync::Arc;
        use std::thread;

        let dir = tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();
        let options = Arc::new(CommitOptions::new(dir_path.clone()));

        // 10 个线程并发 commit revision 1-10
        let mut handles = Vec::new();
        for revision in 1..=10u64 {
            let opts = Arc::clone(&options);
            handles.push(thread::spawn(move || {
                commit_revision(&staged(revision), &opts)
            }));
        }

        let mut successes: Vec<u64> = Vec::new();
        for handle in handles {
            if let Ok(revision_dir) = handle.join().unwrap() {
                let rev: u64 = revision_dir
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .parse()
                    .unwrap();
                successes.push(rev);
            }
        }

        assert!(!successes.is_empty(), "至少应有一个 commit 成功");

        let max_success = *successes.iter().max().unwrap();
        let current = read_current_revision(&dir_path).unwrap();

        // 指针应等于成功 publish 中的最大 revision（不回退）
        assert_eq!(
            current,
            Some(max_success),
            "指针 {:?} 应等于最大成功 revision {}，不允许回退",
            current,
            max_success
        );

        // 指针指向的 revision_dir 应存在（不可变快照）
        assert!(
            options.revision_dir(max_success).is_dir(),
            "指针指向的 revision_dir 应存在"
        );
    }
}
