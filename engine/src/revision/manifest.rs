//! `RevisionManifest` schema 与加载校验（#115 Phase 2a）。
//!
//! manifest 是 revision 目录的元数据 sidecar，记录：
//! - `content_revision`：u64 非负整数，从 1 起
//! - `asset_kind` / `asset_id`：归属 asset
//! - `source`：provenance（source_kind / source_hash / imported_at / parent_revision）
//! - `files`：批准文件集合（相对路径 + per-file SHA-256 + 字节数）
//! - `tree_sha256`：覆盖 `files` 子树的 `AIRP-TREE-SHA256-v1`
//!
//! 加载时强制 5 项不变量（参考 spec §5.1.2）：
//! 1. 磁盘普通文件集合 == `files` 集合
//! 2. 每个文件原始字节 SHA-256 == `files[].sha256`
//! 3. `tree_sha256` == 重新计算的 `AIRP-TREE-SHA256-v1(files)`
//! 4. `content_revision` >= 1
//! 5. `asset_kind` ∈ 枚举集合
//!
//! 任一失败拒绝该 revision，禁止回退到工作副本或部分加载。

use crate::error::AirpError;
use crate::revision::tree_hash::validate_approved_path;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Manifest schema 版本。当前为 1。
pub(crate) const MANIFEST_SCHEMA: u32 = 1;

/// asset 类型枚举。新增 asset 类型时需同步更新此枚举与 `AssetKind::as_str`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AssetKind {
    Character,
    Preset,
    Worldbook,
    State,
    Memory,
    Persona,
}

impl AssetKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            AssetKind::Character => "character",
            AssetKind::Preset => "preset",
            AssetKind::Worldbook => "worldbook",
            AssetKind::State => "state",
            AssetKind::Memory => "memory",
            AssetKind::Persona => "persona",
        }
    }
}

/// revision manifest schema。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RevisionManifest {
    pub schema: u32,
    pub content_revision: u64,
    pub asset_kind: AssetKind,
    pub asset_id: String,
    pub created_at: String,
    pub source: AssetSource,
    pub files: Vec<ApprovedFile>,
    pub tree_sha256: String,
}

/// asset provenance。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(crate) struct AssetSource {
    pub source_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub converter_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<u64>,
}

/// 批准文件记录。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ApprovedFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
}

/// manifest 加载或校验错误。
#[derive(Debug, thiserror::Error)]
pub(crate) enum RevisionManifestError {
    #[error("manifest JSON 解析失败: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("manifest schema 版本不兼容: 期望 {expected}, 实际 {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },

    #[error("manifest content_revision 非法: {0}（应 >= 1）")]
    InvalidRevision(u64),

    #[error("manifest asset_kind 非法: {0}")]
    InvalidAssetKind(String),

    #[error("manifest asset_id 非法: {0}")]
    InvalidAssetId(String),

    #[error("磁盘文件集合与 manifest.files 不一致：缺失 {missing:?}, 额外 {extra:?}")]
    FileSetMismatch {
        missing: Vec<String>,
        extra: Vec<String>,
    },

    #[error("文件 {path:?} 的 SHA-256 不匹配: manifest={manifest}, 实际={actual}")]
    FileHashMismatch {
        path: String,
        manifest: String,
        actual: String,
    },

    #[error("manifest.tree_sha256 不匹配: manifest={manifest}, 实际={actual}")]
    TreeHashMismatch { manifest: String, actual: String },

    #[error("批准文件路径非法 {path:?}: {reason}")]
    InvalidFilePath { path: String, reason: &'static str },

    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),
}

impl From<RevisionManifestError> for AirpError {
    fn from(e: RevisionManifestError) -> Self {
        AirpError::Internal(format!("revision manifest 错误: {e}"))
    }
}

impl RevisionManifest {
    /// 序列化为 pretty JSON bytes。
    pub(crate) fn to_json_bytes(&self) -> Result<Vec<u8>, AirpError> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// 从 JSON bytes 反序列化。
    pub(crate) fn from_json_bytes(bytes: &[u8]) -> Result<Self, RevisionManifestError> {
        let manifest: RevisionManifest = serde_json::from_slice(bytes)?;
        if manifest.schema != MANIFEST_SCHEMA {
            return Err(RevisionManifestError::SchemaMismatch {
                expected: MANIFEST_SCHEMA,
                actual: manifest.schema,
            });
        }
        if manifest.content_revision < 1 {
            return Err(RevisionManifestError::InvalidRevision(
                manifest.content_revision,
            ));
        }
        // asset_kind 已通过 serde 反序列化校验枚举合法性
        // asset_id 用于构造文件系统路径，必须通过路径校验（拒绝空 / `..` / 绝对路径 / 反斜杠等）
        validate_asset_id(&manifest.asset_id)?;
        // 校验所有批准文件路径形式
        for file in &manifest.files {
            validate_approved_path(&file.path).map_err(|e| match e {
                crate::revision::tree_hash::TreeHashError::InvalidPath { path, reason } => {
                    RevisionManifestError::InvalidFilePath { path, reason }
                }
                _ => RevisionManifestError::InvalidFilePath {
                    path: file.path.clone(),
                    reason: "未知路径错误",
                },
            })?;
        }
        Ok(manifest)
    }

    /// 计算指定 revision 目录的 tree_sha256，并对比 manifest 记录值。
    ///
    /// 完整不变量校验流程（spec §5.1.2）：
    /// 1. 磁盘普通文件集合（排除 `manifest.json` sidecar）== `files` 集合
    /// 2. 每个文件原始字节 SHA-256 == `files[].sha256`
    /// 3. `tree_sha256` == 重新计算的 `AIRP-TREE-SHA256-v1(files)`
    ///
    /// `manifest.json` 是元数据 sidecar，不纳入批准文件集合；
    /// 它自身的完整性由加载时 schema 校验和 tree_sha256 字段间接保护
    ///（tree_sha256 覆盖批准文件，manifest 又记录 tree_sha256，篡改任一会失配）。
    pub(crate) fn verify_against_disk(
        &self,
        revision_dir: &Path,
    ) -> Result<(), RevisionManifestError> {
        // 1. 枚举磁盘文件集合（排除 manifest.json sidecar）
        let mut disk_files: HashSet<String> = HashSet::new();
        collect_disk_files(revision_dir, revision_dir, &mut disk_files)?;
        disk_files.remove("manifest.json");

        let manifest_files: HashSet<String> = self.files.iter().map(|f| f.path.clone()).collect();

        let missing: Vec<String> = manifest_files.difference(&disk_files).cloned().collect();
        let extra: Vec<String> = disk_files.difference(&manifest_files).cloned().collect();
        if !missing.is_empty() || !extra.is_empty() {
            return Err(RevisionManifestError::FileSetMismatch { missing, extra });
        }

        // 2. 校验每个文件的 SHA-256
        for file in &self.files {
            let abs_path = revision_dir.join(&file.path);
            let bytes = fs::read(&abs_path)?;
            let mut hasher = Sha256::new();
            hasher.update(&bytes);
            let actual = format!("{:x}", hasher.finalize());
            if actual != file.sha256 {
                return Err(RevisionManifestError::FileHashMismatch {
                    path: file.path.clone(),
                    manifest: file.sha256.clone(),
                    actual,
                });
            }
            if bytes.len() as u64 != file.bytes {
                return Err(RevisionManifestError::FileHashMismatch {
                    path: file.path.clone(),
                    manifest: file.sha256.clone(),
                    actual: format!(
                        "bytes mismatch: manifest={}, actual={}",
                        file.bytes,
                        bytes.len()
                    ),
                });
            }
        }

        // 3. 校验 tree_sha256（compute_tree_sha256 会枚举目录所有文件，包括 manifest.json；
        //    为保持与 commit 时一致，临时移除 manifest.json 后计算）
        let actual_tree = compute_tree_sha256_excluding_manifest(revision_dir)?;
        if actual_tree != self.tree_sha256 {
            return Err(RevisionManifestError::TreeHashMismatch {
                manifest: self.tree_sha256.clone(),
                actual: actual_tree,
            });
        }

        Ok(())
    }
}

/// 计算目录的 tree_sha256，排除 `manifest.json` sidecar。
///
/// revision 目录中的 `manifest.json` 是元数据，不应纳入 tree hash。
/// 本函数枚举目录所有普通文件，排除 `manifest.json` 后按 `AIRP-TREE-SHA256-v1` 计算。
fn compute_tree_sha256_excluding_manifest(
    revision_dir: &Path,
) -> Result<String, RevisionManifestError> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    use std::path::PathBuf;

    const DOMAIN_SEPARATOR: &[u8] = b"AIRP-TREE-SHA256\0v1\0";

    // 枚举磁盘文件并排除 manifest.json
    let mut disk_files: HashSet<String> = HashSet::new();
    collect_disk_files(revision_dir, revision_dir, &mut disk_files)?;
    disk_files.remove("manifest.json");

    let mut entries: Vec<(String, PathBuf)> = disk_files
        .into_iter()
        .map(|relative| (relative.clone(), revision_dir.join(&relative)))
        .collect();
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_SEPARATOR);

    for (relative_path, abs_path) in &entries {
        validate_approved_path(relative_path).map_err(|e| match e {
            crate::revision::tree_hash::TreeHashError::InvalidPath { path, reason } => {
                RevisionManifestError::InvalidFilePath { path, reason }
            }
            _ => RevisionManifestError::InvalidFilePath {
                path: relative_path.clone(),
                reason: "未知路径错误",
            },
        })?;

        let path_bytes = relative_path.as_bytes();
        let path_len = path_bytes.len() as u64;
        hasher.update(path_len.to_be_bytes());
        hasher.update(path_bytes);

        let metadata = std::fs::symlink_metadata(abs_path)?;
        if !metadata.is_file() {
            return Err(RevisionManifestError::InvalidFilePath {
                path: relative_path.clone(),
                reason: "非普通文件",
            });
        }
        let file_len = metadata.len();
        hasher.update(file_len.to_be_bytes());

        let mut file = fs::File::open(abs_path)?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn collect_disk_files(
    root: &Path,
    current: &Path,
    out: &mut HashSet<String>,
) -> Result<(), RevisionManifestError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(RevisionManifestError::InvalidFilePath {
                path: path.to_string_lossy().to_string(),
                reason: "符号链接不允许",
            });
        }
        if metadata.is_dir() {
            collect_disk_files(root, &path, out)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| RevisionManifestError::InvalidFilePath {
                    path: path.to_string_lossy().to_string(),
                    reason: "strip_prefix 失败",
                })?
                .to_str()
                .ok_or_else(|| RevisionManifestError::InvalidFilePath {
                    path: path.to_string_lossy().to_string(),
                    reason: "路径含非 UTF-8 字节",
                })?
                .replace('\\', "/");
            out.insert(relative);
        } else {
            // 拒绝设备文件、FIFO、socket 等特殊入口
            return Err(RevisionManifestError::InvalidFilePath {
                path: path.to_string_lossy().to_string(),
                reason: "非普通文件或目录的特殊入口不允许",
            });
        }
    }
    Ok(())
}

/// 校验 `asset_id` 是否可作为路径段。
///
/// `asset_id` 用于构造文件系统路径（如 `characters/{asset_id}/revisions/`），
/// 必须通过路径校验防止 traversal / 绝对路径 / 反斜杠等注入。
/// 单段 id 不允许含 `/`（与批准文件路径的多段规则不同）。
fn validate_asset_id(asset_id: &str) -> Result<(), RevisionManifestError> {
    if asset_id.is_empty() {
        return Err(RevisionManifestError::InvalidAssetId(asset_id.to_string()));
    }
    if asset_id.contains('/') || asset_id.contains('\\') {
        return Err(RevisionManifestError::InvalidAssetId(asset_id.to_string()));
    }
    if asset_id == "." || asset_id == ".." {
        return Err(RevisionManifestError::InvalidAssetId(asset_id.to_string()));
    }
    // 拒绝绝对路径前缀（Windows 盘符或 Unix /）
    if asset_id.starts_with('/') || asset_id.contains(':') {
        return Err(RevisionManifestError::InvalidAssetId(asset_id.to_string()));
    }
    Ok(())
}

/// 计算单个文件的 SHA-256 hex。
pub(crate) fn file_sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_manifest() -> RevisionManifest {
        RevisionManifest {
            schema: MANIFEST_SCHEMA,
            content_revision: 1,
            asset_kind: AssetKind::Character,
            asset_id: "alice".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource {
                source_kind: "controlled_upload".to_string(),
                ..Default::default()
            },
            files: vec![],
            tree_sha256: "a9682729b0a5609f08a1c9a8b2bf49b68edb9056d9e910fd297f694cc3ee3dbf"
                .to_string(),
        }
    }

    #[test]
    fn manifest_roundtrip_json() {
        let manifest = sample_manifest();
        let bytes = manifest.to_json_bytes().unwrap();
        let parsed = RevisionManifest::from_json_bytes(&bytes).unwrap();
        assert_eq!(parsed.schema, MANIFEST_SCHEMA);
        assert_eq!(parsed.content_revision, 1);
        assert_eq!(parsed.asset_kind, AssetKind::Character);
        assert_eq!(parsed.asset_id, "alice");
    }

    #[test]
    fn rejects_schema_mismatch() {
        let mut manifest = sample_manifest();
        manifest.schema = 99;
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = RevisionManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(RevisionManifestError::SchemaMismatch {
                expected: 1,
                actual: 99
            })
        ));
    }

    #[test]
    fn rejects_revision_zero() {
        let mut manifest = sample_manifest();
        manifest.content_revision = 0;
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = RevisionManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(RevisionManifestError::InvalidRevision(0))
        ));
    }

    #[test]
    fn rejects_empty_asset_id() {
        let mut manifest = sample_manifest();
        manifest.asset_id = "".to_string();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = RevisionManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(RevisionManifestError::InvalidAssetId(_))
        ));
    }

    #[test]
    fn rejects_asset_id_with_traversal() {
        let mut manifest = sample_manifest();
        manifest.asset_id = "../other".to_string();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = RevisionManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(RevisionManifestError::InvalidAssetId(_))
        ));
    }

    #[test]
    fn rejects_asset_id_absolute_path() {
        let mut manifest = sample_manifest();
        manifest.asset_id = "/etc/passwd".to_string();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = RevisionManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(RevisionManifestError::InvalidAssetId(_))
        ));
    }

    #[test]
    fn rejects_asset_id_with_backslash() {
        let mut manifest = sample_manifest();
        manifest.asset_id = "foo\\bar".to_string();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = RevisionManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(RevisionManifestError::InvalidAssetId(_))
        ));
    }

    #[test]
    fn rejects_asset_id_dot_segment() {
        let mut manifest = sample_manifest();
        manifest.asset_id = ".".to_string();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = RevisionManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(RevisionManifestError::InvalidAssetId(_))
        ));
    }

    #[test]
    fn asset_kind_serializes_lowercase() {
        let manifest = sample_manifest();
        let value = serde_json::to_value(&manifest).unwrap();
        assert_eq!(value["asset_kind"], "character");
    }

    #[test]
    fn verify_empty_revision_dir_matches() {
        let dir = tempdir().unwrap();
        let manifest = sample_manifest();
        // 空目录 + 空 files + 空目录 tree_sha256
        let result = manifest.verify_against_disk(dir.path());
        assert!(result.is_ok(), "空 revision 目录应通过校验");
    }

    #[test]
    fn verify_rejects_extra_disk_file() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("extra.txt"), "extra").unwrap();
        let manifest = sample_manifest(); // files 为空
        let result = manifest.verify_against_disk(dir.path());
        assert!(matches!(
            result,
            Err(RevisionManifestError::FileSetMismatch { .. })
        ));
    }

    #[test]
    fn verify_rejects_missing_disk_file() {
        let dir = tempdir().unwrap();
        let mut manifest = sample_manifest();
        manifest.files = vec![ApprovedFile {
            path: "card.json".to_string(),
            sha256: "abc".to_string(),
            bytes: 3,
        }];
        let result = manifest.verify_against_disk(dir.path());
        assert!(matches!(
            result,
            Err(RevisionManifestError::FileSetMismatch { .. })
        ));
    }

    #[test]
    fn verify_rejects_file_hash_mismatch() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let bytes = fs::read(dir.path().join("a.txt")).unwrap();
        let wrong_hash = "0".repeat(64);
        let mut manifest = sample_manifest();
        manifest.files = vec![ApprovedFile {
            path: "a.txt".to_string(),
            sha256: wrong_hash,
            bytes: bytes.len() as u64,
        }];
        let result = manifest.verify_against_disk(dir.path());
        assert!(matches!(
            result,
            Err(RevisionManifestError::FileHashMismatch { .. })
        ));
    }

    #[test]
    fn verify_accepts_valid_revision() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let bytes = fs::read(dir.path().join("a.txt")).unwrap();
        let hash = file_sha256_hex(&bytes);

        // 计算正确的 tree_sha256
        let tree = crate::revision::tree_hash::compute_tree_sha256(dir.path()).unwrap();

        let manifest = RevisionManifest {
            schema: MANIFEST_SCHEMA,
            content_revision: 1,
            asset_kind: AssetKind::Character,
            asset_id: "alice".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource::default(),
            files: vec![ApprovedFile {
                path: "a.txt".to_string(),
                sha256: hash,
                bytes: 1,
            }],
            tree_sha256: tree,
        };

        let result = manifest.verify_against_disk(dir.path());
        assert!(result.is_ok(), "合法 revision 应通过校验: {:?}", result);
    }

    #[test]
    fn verify_rejects_tree_hash_mismatch() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let bytes = fs::read(dir.path().join("a.txt")).unwrap();
        let hash = file_sha256_hex(&bytes);

        let manifest = RevisionManifest {
            schema: MANIFEST_SCHEMA,
            content_revision: 1,
            asset_kind: AssetKind::Character,
            asset_id: "alice".to_string(),
            created_at: "2026-07-16T00:00:00Z".to_string(),
            source: AssetSource::default(),
            files: vec![ApprovedFile {
                path: "a.txt".to_string(),
                sha256: hash,
                bytes: 1,
            }],
            tree_sha256: "0".repeat(64), // 错误的 tree hash
        };

        let result = manifest.verify_against_disk(dir.path());
        assert!(matches!(
            result,
            Err(RevisionManifestError::TreeHashMismatch { .. })
        ));
    }
}
