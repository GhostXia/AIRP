//! `AIRP-TREE-SHA256-v1` 实现（参考 `docs/SESSION-DATA-DESIGN.md` §4 第 5 条）。
//!
//! 算法：
//! 1. ASCII 域分隔符 `AIRP-TREE-SHA256\0v1\0`
//! 2. 对每个批准文件（按路径 UTF-8 字节序升序）：
//!    - `u64be(path_utf8_length)` || `path_utf8` || `u64be(file_length)` || `raw_file_bytes`
//! 3. 输出 SHA-256 的小写十六进制
//!
//! 路径要求：`/` 分隔、无空段 / `.` / `..` / 反斜杠、Unicode NFC、UTF-8 字节序升序。
//! 文件要求：仅普通文件，拒绝符号链接 / junction / reparse point / 设备文件。

use crate::error::AirpError;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// 算法标识，写入 manifest 用于版本协商。
pub(crate) const TREE_HASH_ALGORITHM: &str = "AIRP-TREE-SHA256-v1";

/// ASCII 域分隔符：`AIRP-TREE-SHA256\0v1\0`。
const DOMAIN_SEPARATOR: &[u8] = b"AIRP-TREE-SHA256\0v1\0";

/// tree hash 计算错误。
#[derive(Debug, thiserror::Error)]
pub(crate) enum TreeHashError {
    /// 路径形式非法（含 `..`、空段、反斜杠、非 NFC 等）。
    #[error("非法批准文件路径 {path:?}: {reason}")]
    InvalidPath { path: String, reason: &'static str },

    /// 入口非普通文件（符号链接 / 目录 / 设备文件）。
    #[error("批准入口 {path:?} 不是普通文件")]
    NotRegularFile { path: PathBuf },

    /// 读文件失败。
    #[error("读取批准文件 {path:?} 失败: {source}")]
    ReadFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// 校验单个批准文件路径形式。
///
/// 规则（参考 SESSION-DATA-DESIGN.md §4 第 5 条）：
/// - 必须是相对路径
/// - 以 `/` 分隔
/// - 无空段、无 `.`、无 `..`、无反斜杠
/// - Unicode NFC（实现层面：输入已是 `&str`，调用方负责 NFC；本函数只做结构性校验）
pub(crate) fn validate_approved_path(path: &str) -> Result<(), TreeHashError> {
    if path.is_empty() {
        return Err(TreeHashError::InvalidPath {
            path: path.to_string(),
            reason: "路径为空",
        });
    }
    if path.starts_with('/') || path.ends_with('/') {
        return Err(TreeHashError::InvalidPath {
            path: path.to_string(),
            reason: "路径不得以 / 开头或结尾",
        });
    }
    if path.contains('\\') {
        return Err(TreeHashError::InvalidPath {
            path: path.to_string(),
            reason: "路径不得包含反斜杠",
        });
    }
    for segment in path.split('/') {
        if segment.is_empty() {
            return Err(TreeHashError::InvalidPath {
                path: path.to_string(),
                reason: "路径含空段",
            });
        }
        if segment == "." || segment == ".." {
            return Err(TreeHashError::InvalidPath {
                path: path.to_string(),
                reason: "路径含 . 或 .. 段",
            });
        }
    }
    Ok(())
}

/// 计算目录的 `AIRP-TREE-SHA256-v1` tree hash。
///
/// 入参 `revision_dir` 应为已写好的 revision 目录（如 `.../revisions/3/`）。
/// 函数会：
/// 1. 递归枚举目录下所有普通文件
/// 2. 路径相对于 `revision_dir`
/// 3. 按 UTF-8 字节序升序排序
/// 4. 按 algorithm 注入 SHA-256
///
/// 拒绝符号链接 / junction / reparse point（跨平台：Unix 用 `symlink_metadata`，
/// Windows 用 `fs_metadata` 并检查 `is_file`；当前实现统一用 `symlink_metadata`
/// 拒绝符号链接，Windows junction 由 `is_file` 兜底拒绝）。
pub(crate) fn compute_tree_sha256(revision_dir: &Path) -> Result<String, AirpError> {
    if !revision_dir.is_dir() {
        return Err(AirpError::Internal(format!(
            "revision 目录不存在或非目录: {}",
            revision_dir.display()
        )));
    }

    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    collect_approved_files(revision_dir, revision_dir, &mut entries)?;

    // 按 UTF-8 字节序升序排序（与 SESSION-DATA-DESIGN.md §4 第 5 条一致）
    entries.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    let mut hasher = Sha256::new();
    hasher.update(DOMAIN_SEPARATOR);

    for (relative_path, abs_path) in &entries {
        validate_approved_path(relative_path)?;

        let path_bytes = relative_path.as_bytes();
        let path_len = path_bytes.len() as u64;
        hasher.update(path_len.to_be_bytes());
        hasher.update(path_bytes);

        let metadata =
            std::fs::symlink_metadata(abs_path).map_err(|e| TreeHashError::ReadFile {
                path: abs_path.clone(),
                source: e,
            })?;
        if !metadata.is_file() {
            return Err(TreeHashError::NotRegularFile {
                path: abs_path.clone(),
            }
            .into());
        }

        let file_len = metadata.len();
        hasher.update(file_len.to_be_bytes());

        let mut file = fs::File::open(abs_path).map_err(|e| TreeHashError::ReadFile {
            path: abs_path.clone(),
            source: e,
        })?;
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).map_err(|e| TreeHashError::ReadFile {
                path: abs_path.clone(),
                source: e,
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
    }

    let digest = hasher.finalize();
    Ok(format!("{:x}", digest))
}

fn collect_approved_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), AirpError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)?;
        // 拒绝符号链接（跨平台一致）
        if metadata.file_type().is_symlink() {
            return Err(TreeHashError::NotRegularFile { path }.into());
        }
        if metadata.is_dir() {
            collect_approved_files(root, &path, out)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| {
                    AirpError::Internal(format!(
                        "路径 strip_prefix 失败: {} vs {}",
                        path.display(),
                        root.display()
                    ))
                })?
                .to_string_lossy()
                .replace('\\', "/");
            out.push((relative, path));
        } else {
            return Err(TreeHashError::NotRegularFile { path }.into());
        }
    }
    Ok(())
}

impl From<TreeHashError> for AirpError {
    fn from(e: TreeHashError) -> Self {
        AirpError::Internal(format!("tree hash 错误: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn empty_directory_matches_known_vector() {
        // SESSION-DATA-DESIGN.md §4 第 5 条规定的空目录测试向量
        let dir = tempdir().unwrap();
        let hash = compute_tree_sha256(dir.path()).unwrap();
        assert_eq!(
            hash,
            "a9682729b0a5609f08a1c9a8b2bf49b68edb9056d9e910fd297f694cc3ee3dbf"
        );
    }

    #[test]
    fn single_file_matches_known_vector() {
        // SESSION-DATA-DESIGN.md §4 第 5 条规定的单文件测试向量
        // 文件 a.txt 内容 "x"
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        let hash = compute_tree_sha256(dir.path()).unwrap();
        assert_eq!(
            hash,
            "cfa2887973ce5ecc1f2bc57b00ad0130a39aae4d4bf67adae0431ccd3a3ae189"
        );
    }

    #[test]
    fn rejects_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target.txt");
        fs::write(&target, "data").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, dir.path().join("link.txt")).unwrap();
            let result = compute_tree_sha256(dir.path());
            assert!(result.is_err(), "应拒绝符号链接");
        }
        #[cfg(not(unix))]
        {
            // Windows 上跳过符号链接测试，但确保其他测试通过
            let hash = compute_tree_sha256(dir.path()).unwrap();
            assert_ne!(hash, "");
        }
    }

    #[test]
    fn multiple_files_sorted_by_utf8_bytes() {
        let dir = tempdir().unwrap();
        // 故意乱序写入，验证排序
        fs::write(dir.path().join("z.md"), "z").unwrap();
        fs::write(dir.path().join("a.md"), "a").unwrap();
        fs::write(dir.path().join("m.md"), "m").unwrap();

        let hash1 = compute_tree_sha256(dir.path()).unwrap();

        // 重新以同内容写入不同顺序，hash 应一致（因为算法内部排序）
        let dir2 = tempdir().unwrap();
        fs::write(dir2.path().join("a.md"), "a").unwrap();
        fs::write(dir2.path().join("m.md"), "m").unwrap();
        fs::write(dir2.path().join("z.md"), "z").unwrap();
        let hash2 = compute_tree_sha256(dir2.path()).unwrap();

        assert_eq!(hash1, hash2, "排序后应与写入顺序无关");
    }

    #[test]
    fn nested_subdirectories() {
        let dir = tempdir().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub").join("a.txt"), "alpha").unwrap();
        fs::write(dir.path().join("root.txt"), "root").unwrap();

        let hash = compute_tree_sha256(dir.path()).unwrap();
        assert_ne!(hash, "");
        // 验证与空目录 hash 不同
        let empty = tempdir().unwrap();
        let empty_hash = compute_tree_sha256(empty.path()).unwrap();
        assert_ne!(hash, empty_hash);
    }

    #[test]
    fn validate_path_rejects_empty() {
        assert!(validate_approved_path("").is_err());
    }

    #[test]
    fn validate_path_rejects_absolute() {
        assert!(validate_approved_path("/etc/passwd").is_err());
        assert!(validate_approved_path("foo/").is_err());
    }

    #[test]
    fn validate_path_rejects_dot_segments() {
        assert!(validate_approved_path("./foo").is_err());
        assert!(validate_approved_path("foo/.").is_err());
        assert!(validate_approved_path("../foo").is_err());
        assert!(validate_approved_path("foo/..").is_err());
        assert!(validate_approved_path("foo//bar").is_err());
    }

    #[test]
    fn validate_path_rejects_backslash() {
        assert!(validate_approved_path("foo\\bar").is_err());
    }

    #[test]
    fn validate_path_accepts_normal_relative() {
        assert!(validate_approved_path("a.txt").is_ok());
        assert!(validate_approved_path("sub/a.txt").is_ok());
        assert!(validate_approved_path("sub/deep/a.txt").is_ok());
    }
}
