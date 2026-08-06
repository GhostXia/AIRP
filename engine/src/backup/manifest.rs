//! `BackupManifest` schema 与校验（#342 E-P2-1）。
//!
//! backup manifest 是 backup 目录的元数据 sidecar，记录：
//! - `schema`：manifest schema 版本（当前 = 1）
//! - `backup_id`：ULID-like 唯一标识（uuid v4 simple）
//! - `created_at`：RFC3339 UTC 时间戳
//! - `engine_version` / `data_schema_version`：版本兼容性协商
//! - `source`：provenance（manual / pre_delete / pre_restore_rollback）
//! - `scope`：备份覆盖范围（full / character / session）
//! - `secrets_excluded`：v1 恒为 true（排除 secrets.json + settings.json）
//! - `files`：批准文件集合（相对 data_root 路径 + per-file SHA-256 + 字节数）
//! - `tree_sha256`：覆盖 `files` 子树的 `AIRP-TREE-SHA256-v1`
//!
//! 加载时（`from_json_bytes`）强制不变量：
//! 1. `schema == 1`
//! 2. `backup_id` 非空且为合法路径段
//! 3. `data_schema_version` 不超过本引擎支持的最大值
//! 4. `secrets_excluded == true`（v1 强制）
//! 5. `files` 路径经 `validate_approved_path` 校验
//!
//! 注：`tree_sha256` 与 per-file SHA-256 的**重新计算匹配**不在加载阶段做，
//! 而是在 `verify_against_disk` 中针对磁盘 `files/` 子树完整重算并比对。
//! `list_backups` 等只读场景只做加载阶段校验，不重算 hash（性能考虑）。
//! 任一加载/校验失败拒绝该 backup，禁止部分加载或降级。

use crate::error::AirpError;
use crate::revision::manifest::ApprovedFile;
use crate::revision::tree_hash::{compute_tree_sha256, validate_approved_path};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// Manifest schema 版本。当前为 1。
pub(crate) const BACKUP_MANIFEST_SCHEMA: u32 = 1;

/// data schema 版本：backup 内容的 logical schema。
/// v1 = 当前 data_root 结构（characters/ presets/ providers.json 等）。
/// 未来 data 结构大改时递增，restore 时用于拒绝不兼容版本。
pub(crate) const DATA_SCHEMA_VERSION: u32 = 1;

/// v1 恒定排除的 secret 文件（相对 data_root 路径，`/` 分隔）。
/// 这些文件绝不进入 backup `files` 集合。
pub(crate) const SECRET_EXCLUDE_LIST: &[&str] = &["secrets.json", "settings.json"];

/// backup 来源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum BackupSource {
    /// 用户手动创建。
    Manual,
    /// 删除前自动创建。`scope` 字段记录删除目标。
    PreDelete,
    /// restore 前自动创建的回滚点。
    PreRestoreRollback,
}

impl BackupSource {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            BackupSource::Manual => "manual",
            BackupSource::PreDelete => "pre_delete",
            BackupSource::PreRestoreRollback => "pre_restore_rollback",
        }
    }
}

/// backup 覆盖范围。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum BackupScope {
    /// 全 data_root 快照（排除 backups/ 自身 + secret 文件）。
    Full,
    /// 单角色子树：`characters/{character_id}/`。
    Character { character_id: String },
    /// 单会话子树：`characters/{character_id}/sessions/{session_id}/`。
    Session {
        character_id: String,
        session_id: String,
    },
}

impl BackupScope {
    /// 返回该 scope 在 data_root 下的相对子树根路径（`/` 分隔）。
    /// `Full` 返回空串（表示 data_root 自身）。
    pub(crate) fn subtree_prefix(&self) -> String {
        match self {
            BackupScope::Full => String::new(),
            BackupScope::Character { character_id } => {
                format!("characters/{character_id}")
            }
            BackupScope::Session {
                character_id,
                session_id,
            } => format!("characters/{character_id}/sessions/{session_id}"),
        }
    }
}

/// backup manifest schema。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct BackupManifest {
    pub schema: u32,
    pub backup_id: String,
    pub created_at: String,
    pub engine_version: String,
    pub data_schema_version: u32,
    pub source: BackupSource,
    pub scope: BackupScope,
    pub secrets_excluded: bool,
    pub files: Vec<ApprovedFile>,
    pub tree_sha256: String,
}

/// manifest 加载或校验错误。
#[derive(Debug, thiserror::Error)]
pub(crate) enum BackupManifestError {
    #[error("manifest JSON 解析失败: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("manifest schema 版本不兼容: 期望 {expected}, 实际 {actual}")]
    SchemaMismatch { expected: u32, actual: u32 },

    #[error("manifest backup_id 非法: {0}")]
    InvalidBackupId(String),

    #[error("manifest data_schema_version 不兼容: 期望 <= {max_supported}, 实际 {actual}")]
    IncompatibleDataSchema { max_supported: u32, actual: u32 },

    #[error("manifest secrets_excluded 必须为 true（v1 不支持含 secret 的 backup）")]
    SecretsNotExcluded,

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

    #[error("tree_sha256 计算失败: {0}")]
    TreeHashComputation(String),

    #[error("批准文件路径非法 {path:?}: {reason}")]
    InvalidFilePath { path: String, reason: &'static str },

    #[error("I/O 错误: {0}")]
    Io(#[from] std::io::Error),
}

impl From<BackupManifestError> for AirpError {
    fn from(e: BackupManifestError) -> Self {
        AirpError::Internal(format!("backup manifest 错误: {e}"))
    }
}

impl BackupManifest {
    /// 序列化为 pretty JSON bytes。
    pub(crate) fn to_json_bytes(&self) -> Result<Vec<u8>, AirpError> {
        Ok(serde_json::to_vec_pretty(self)?)
    }

    /// 从 JSON bytes 反序列化 + 基础不变量校验。
    pub(crate) fn from_json_bytes(bytes: &[u8]) -> Result<Self, BackupManifestError> {
        let manifest: BackupManifest = serde_json::from_slice(bytes)?;
        if manifest.schema != BACKUP_MANIFEST_SCHEMA {
            return Err(BackupManifestError::SchemaMismatch {
                expected: BACKUP_MANIFEST_SCHEMA,
                actual: manifest.schema,
            });
        }
        validate_backup_id(&manifest.backup_id)?;
        if manifest.data_schema_version > DATA_SCHEMA_VERSION {
            return Err(BackupManifestError::IncompatibleDataSchema {
                max_supported: DATA_SCHEMA_VERSION,
                actual: manifest.data_schema_version,
            });
        }
        // v1 强制 secret 排除
        if !manifest.secrets_excluded {
            return Err(BackupManifestError::SecretsNotExcluded);
        }
        // 校验所有批准文件路径形式
        for file in &manifest.files {
            validate_approved_path(&file.path).map_err(|e| match e {
                crate::revision::tree_hash::TreeHashError::InvalidPath { path, reason } => {
                    BackupManifestError::InvalidFilePath { path, reason }
                }
                _ => BackupManifestError::InvalidFilePath {
                    path: file.path.clone(),
                    reason: "未知路径错误",
                },
            })?;
        }
        Ok(manifest)
    }

    /// 校验 manifest 与磁盘 backup 目录的一致性。
    ///
    /// `backup_dir` 应为 `data_root/backups/{backup_id}/`，
    /// 批准文件位于其下 `files/` 子目录。
    ///
    /// 完整校验：
    /// 1. 磁盘 `files/` 下普通文件集合 == `manifest.files` 集合
    /// 2. 每个文件原始字节 SHA-256 == `manifest.files[].sha256`
    /// 3. `manifest.tree_sha256` == 重新计算的 `AIRP-TREE-SHA256-v1(files)`
    pub(crate) fn verify_against_disk(&self, backup_dir: &Path) -> Result<(), BackupManifestError> {
        let files_root = backup_dir.join("files");
        if !files_root.is_dir() {
            return Err(BackupManifestError::InvalidFilePath {
                path: "files/".to_string(),
                reason: "backup 目录下缺少 files/ 子目录",
            });
        }

        // 1. 枚举磁盘文件集合
        let mut disk_files: HashSet<String> = HashSet::new();
        collect_disk_files(&files_root, &files_root, &mut disk_files)?;
        let manifest_files: HashSet<String> = self.files.iter().map(|f| f.path.clone()).collect();
        let missing: Vec<String> = manifest_files.difference(&disk_files).cloned().collect();
        let extra: Vec<String> = disk_files.difference(&manifest_files).cloned().collect();
        if !missing.is_empty() || !extra.is_empty() {
            return Err(BackupManifestError::FileSetMismatch { missing, extra });
        }

        // 2. 校验每个文件的 SHA-256 + 字节数
        for file in &self.files {
            let abs_path = files_root.join(&file.path);
            let bytes = fs::read(&abs_path)?;
            let actual = crate::revision::manifest::file_sha256_hex(&bytes);
            if actual != file.sha256 {
                return Err(BackupManifestError::FileHashMismatch {
                    path: file.path.clone(),
                    manifest: file.sha256.clone(),
                    actual,
                });
            }
            if bytes.len() as u64 != file.bytes {
                return Err(BackupManifestError::FileHashMismatch {
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

        // 3. 校验 tree_sha256（compute_tree_sha256 会枚举 files/ 目录所有文件）
        let actual_tree = compute_tree_sha256(&files_root)
            .map_err(|e| BackupManifestError::TreeHashComputation(format!("{e}")))?;
        if actual_tree != self.tree_sha256 {
            return Err(BackupManifestError::TreeHashMismatch {
                manifest: self.tree_sha256.clone(),
                actual: actual_tree,
            });
        }

        Ok(())
    }
}

/// 校验 `backup_id` 是否可作为路径段。
///
/// 同时被 manifest 加载（`from_json_bytes`）和 HTTP handler（`validate_backup_id_segment`）
/// 复用，确保两层校验规则单一来源（#450）。
pub(crate) fn validate_backup_id(backup_id: &str) -> Result<(), BackupManifestError> {
    if backup_id.is_empty() {
        return Err(BackupManifestError::InvalidBackupId(backup_id.to_string()));
    }
    if backup_id.contains('/') || backup_id.contains('\\') {
        return Err(BackupManifestError::InvalidBackupId(backup_id.to_string()));
    }
    if backup_id == "." || backup_id == ".." {
        return Err(BackupManifestError::InvalidBackupId(backup_id.to_string()));
    }
    if backup_id.starts_with('.') || backup_id.contains(':') {
        return Err(BackupManifestError::InvalidBackupId(backup_id.to_string()));
    }
    // backup_id 用 uuid v4 simple（32 hex chars），但允许更宽松以兼容未来格式
    if !backup_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(BackupManifestError::InvalidBackupId(backup_id.to_string()));
    }
    Ok(())
}

/// 递归枚举目录下所有普通文件，返回相对路径（`/` 分隔）。
fn collect_disk_files(
    root: &Path,
    current: &Path,
    out: &mut HashSet<String>,
) -> Result<(), BackupManifestError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            return Err(BackupManifestError::InvalidFilePath {
                path: path.to_string_lossy().to_string(),
                reason: "符号链接不允许",
            });
        }
        if metadata.is_dir() {
            collect_disk_files(root, &path, out)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| BackupManifestError::InvalidFilePath {
                    path: path.to_string_lossy().to_string(),
                    reason: "strip_prefix 失败",
                })?
                .to_str()
                .ok_or_else(|| BackupManifestError::InvalidFilePath {
                    path: path.to_string_lossy().to_string(),
                    reason: "路径含非 UTF-8 字节",
                })?
                .replace('\\', "/");
            out.insert(relative);
        } else {
            return Err(BackupManifestError::InvalidFilePath {
                path: path.to_string_lossy().to_string(),
                reason: "非普通文件或目录的特殊入口不允许",
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_manifest() -> BackupManifest {
        BackupManifest {
            schema: BACKUP_MANIFEST_SCHEMA,
            backup_id: "abc123def45678901234567890123456".to_string(),
            created_at: "2026-08-03T00:00:00Z".to_string(),
            engine_version: "0.0.3".to_string(),
            data_schema_version: DATA_SCHEMA_VERSION,
            source: BackupSource::Manual,
            scope: BackupScope::Full,
            secrets_excluded: true,
            files: vec![],
            tree_sha256: "a9682729b0a5609f08a1c9a8b2bf49b68edb9056d9e910fd297f694cc3ee3dbf"
                .to_string(),
        }
    }

    #[test]
    fn manifest_roundtrip_json() {
        let manifest = sample_manifest();
        let bytes = manifest.to_json_bytes().unwrap();
        let parsed = BackupManifest::from_json_bytes(&bytes).unwrap();
        assert_eq!(parsed.schema, BACKUP_MANIFEST_SCHEMA);
        assert_eq!(parsed.backup_id, "abc123def45678901234567890123456");
        assert_eq!(parsed.source, BackupSource::Manual);
        assert_eq!(parsed.scope, BackupScope::Full);
        assert!(parsed.secrets_excluded);
    }

    #[test]
    fn source_serializes_snake_case() {
        let m = sample_manifest();
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["source"]["kind"], "manual");
        assert_eq!(v["scope"]["kind"], "full");
    }

    #[test]
    fn pre_delete_source_with_character_scope_roundtrip() {
        let mut m = sample_manifest();
        m.source = BackupSource::PreDelete;
        m.scope = BackupScope::Character {
            character_id: "alice".to_string(),
        };
        let bytes = m.to_json_bytes().unwrap();
        let parsed = BackupManifest::from_json_bytes(&bytes).unwrap();
        assert_eq!(parsed.source, BackupSource::PreDelete);
        assert_eq!(
            parsed.scope,
            BackupScope::Character {
                character_id: "alice".to_string()
            }
        );
        assert_eq!(parsed.scope.subtree_prefix(), "characters/alice");
    }

    #[test]
    fn session_scope_subtree_prefix() {
        let mut m = sample_manifest();
        m.scope = BackupScope::Session {
            character_id: "alice".to_string(),
            session_id: "deadbeef".to_string(),
        };
        assert_eq!(
            m.scope.subtree_prefix(),
            "characters/alice/sessions/deadbeef"
        );
    }

    #[test]
    fn rejects_schema_mismatch() {
        let mut manifest = sample_manifest();
        manifest.schema = 99;
        let bytes = serde_json::to_vec(&manifest).unwrap();
        let result = BackupManifest::from_json_bytes(&bytes);
        assert!(matches!(
            result,
            Err(BackupManifestError::SchemaMismatch {
                expected: 1,
                actual: 99
            })
        ));
    }

    #[test]
    fn rejects_empty_backup_id() {
        let mut manifest = sample_manifest();
        manifest.backup_id = "".to_string();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(matches!(
            BackupManifest::from_json_bytes(&bytes),
            Err(BackupManifestError::InvalidBackupId(_))
        ));
    }

    #[test]
    fn rejects_backup_id_with_traversal() {
        let mut manifest = sample_manifest();
        manifest.backup_id = "../escape".to_string();
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(matches!(
            BackupManifest::from_json_bytes(&bytes),
            Err(BackupManifestError::InvalidBackupId(_))
        ));
    }

    #[test]
    fn rejects_secrets_not_excluded() {
        let mut manifest = sample_manifest();
        manifest.secrets_excluded = false;
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(matches!(
            BackupManifest::from_json_bytes(&bytes),
            Err(BackupManifestError::SecretsNotExcluded)
        ));
    }

    #[test]
    fn rejects_incompatible_data_schema() {
        let mut manifest = sample_manifest();
        manifest.data_schema_version = DATA_SCHEMA_VERSION + 1;
        let bytes = serde_json::to_vec(&manifest).unwrap();
        assert!(matches!(
            BackupManifest::from_json_bytes(&bytes),
            Err(BackupManifestError::IncompatibleDataSchema { .. })
        ));
    }

    #[test]
    fn verify_empty_files_dir_matches_empty_manifest() {
        let dir = tempdir().unwrap();
        let files_dir = dir.path().join("files");
        fs::create_dir_all(&files_dir).unwrap();
        let manifest = sample_manifest(); // files 为空 + 空目录 tree_sha256
        let result = manifest.verify_against_disk(dir.path());
        assert!(
            result.is_ok(),
            "空 files 目录 + 空 files 应通过校验: {:?}",
            result
        );
    }

    #[test]
    fn verify_rejects_extra_disk_file() {
        let dir = tempdir().unwrap();
        let files_dir = dir.path().join("files");
        fs::create_dir_all(&files_dir).unwrap();
        fs::write(files_dir.join("extra.txt"), "extra").unwrap();
        let manifest = sample_manifest(); // files 为空
        assert!(matches!(
            manifest.verify_against_disk(dir.path()),
            Err(BackupManifestError::FileSetMismatch { .. })
        ));
    }

    #[test]
    fn verify_accepts_valid_backup() {
        let dir = tempdir().unwrap();
        let files_dir = dir.path().join("files");
        fs::create_dir_all(&files_dir).unwrap();
        fs::write(files_dir.join("a.txt"), "x").unwrap();
        let bytes = fs::read(files_dir.join("a.txt")).unwrap();
        let hash = crate::revision::manifest::file_sha256_hex(&bytes);
        let tree = compute_tree_sha256(&files_dir).unwrap();

        let manifest = BackupManifest {
            schema: BACKUP_MANIFEST_SCHEMA,
            backup_id: "test01".to_string(),
            created_at: "2026-08-03T00:00:00Z".to_string(),
            engine_version: "0.0.3".to_string(),
            data_schema_version: DATA_SCHEMA_VERSION,
            source: BackupSource::Manual,
            scope: BackupScope::Full,
            secrets_excluded: true,
            files: vec![ApprovedFile {
                path: "a.txt".to_string(),
                sha256: hash,
                bytes: 1,
            }],
            tree_sha256: tree,
        };
        let result = manifest.verify_against_disk(dir.path());
        assert!(result.is_ok(), "合法 backup 应通过校验: {:?}", result);
    }

    #[test]
    fn verify_rejects_file_hash_mismatch() {
        let dir = tempdir().unwrap();
        let files_dir = dir.path().join("files");
        fs::create_dir_all(&files_dir).unwrap();
        fs::write(files_dir.join("a.txt"), "x").unwrap();

        let manifest = BackupManifest {
            schema: BACKUP_MANIFEST_SCHEMA,
            backup_id: "test01".to_string(),
            created_at: "2026-08-03T00:00:00Z".to_string(),
            engine_version: "0.0.3".to_string(),
            data_schema_version: DATA_SCHEMA_VERSION,
            source: BackupSource::Manual,
            scope: BackupScope::Full,
            secrets_excluded: true,
            files: vec![ApprovedFile {
                path: "a.txt".to_string(),
                sha256: "0".repeat(64),
                bytes: 1,
            }],
            tree_sha256: "0".repeat(64),
        };
        assert!(matches!(
            manifest.verify_against_disk(dir.path()),
            Err(BackupManifestError::FileHashMismatch { .. })
        ));
    }

    #[test]
    fn verify_rejects_missing_files_dir() {
        let dir = tempdir().unwrap();
        // 不创建 files/ 目录
        let manifest = sample_manifest();
        assert!(matches!(
            manifest.verify_against_disk(dir.path()),
            Err(BackupManifestError::InvalidFilePath { .. })
        ));
    }

    #[test]
    fn secret_exclude_list_includes_known_secrets() {
        assert!(SECRET_EXCLUDE_LIST.contains(&"secrets.json"));
        assert!(SECRET_EXCLUDE_LIST.contains(&"settings.json"));
    }
}
