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
pub(crate) fn commit_revision(
    staged: &StagedRevision,
    options: &CommitOptions,
) -> Result<PathBuf, AirpError> {
    if staged.content_revision < 1 {
        return Err(AirpError::BadRequest(format!(
            "content_revision 必须 >= 1, 实际 {}",
            staged.content_revision
        )));
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

    // 2. 计算 tree_sha256（覆盖 staging 目录下所有批准文件）
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

    // 5. 同步 staging 目录（best-effort；Windows 上 sync_data 对目录是 no-op）。
    //    注意：必须在 rename 前关闭所有目录句柄，否则 Windows 上 rename 会返回 ACCESS_DENIED。
    {
        let _ = sync_dir(&staging_dir);
    }

    // 6. 原子 rename staging -> revision 目录
    fs::rename(&staging_dir, &revision_dir).map_err(|e| {
        AirpError::Internal(format!(
            "rename staging {} -> revision {} 失败: {e}",
            staging_dir.display(),
            revision_dir.display()
        ))
    })?;

    // 7. 同步 revisions/ 父目录（best-effort）
    {
        let _ = sync_dir(&revisions_dir);
    }

    // 8. 原子替换 current_revision 文件
    let current_revision_path = options.current_revision_path();
    let current_tmp = current_revision_path.with_extension("tmp");
    {
        let mut file = fs::File::create(&current_tmp)?;
        file.write_all(staged.content_revision.to_string().as_bytes())?;
        file.sync_all()?;
    }
    // Windows 上 rename 目标已存在会失败，先尝试 remove
    if current_revision_path.exists() {
        let _ = fs::remove_file(&current_revision_path);
    }
    fs::rename(&current_tmp, &current_revision_path).map_err(|e| {
        AirpError::Internal(format!(
            "rename current_revision tmp {} -> {} 失败: {e}",
            current_tmp.display(),
            current_revision_path.display()
        ))
    })?;

    // 9. 同步 current_revision 父目录（asset_dir，best-effort）
    {
        let _ = sync_dir(&options.asset_dir);
    }

    Ok(revision_dir)
}

/// 读取 `current_revision` 文件，返回当前 revision。
///
/// 文件不存在时返回 `Ok(None)`（asset 未升级到 revision 合同）。
pub(crate) fn read_current_revision(asset_dir: &Path) -> Result<Option<u64>, AirpError> {
    let path = asset_dir.join("current_revision");
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(&path)?;
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let revision: u64 = trimmed.parse().map_err(|e| {
        AirpError::Internal(format!("current_revision 文件内容非法 {:?}: {e}", trimmed))
    })?;
    Ok(Some(revision))
}

/// 同步目录（best-effort；Windows 上 `sync_data` 对目录是 no-op）。
fn sync_dir(path: &Path) -> Result<(), AirpError> {
    let file = fs::File::open(path)?;
    // best-effort sync_data；忽略错误（某些 FS 不支持）
    let _ = file.sync_data();
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
}
