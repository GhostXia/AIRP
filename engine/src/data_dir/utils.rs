use crate::error::AirpError;
use std::fs;
use std::io::Write;
use std::path::Path;

#[inline]
pub(crate) fn strip_utf8_bom(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
}

/// Replace a JSON file without exposing a partially written value.
/// Callers must serialize concurrent writes to the same path.
pub(crate) fn replace_file(path: &Path, bytes: &[u8]) -> Result<(), AirpError> {
    replace_file_with_backup_cleanup(path, bytes, |backup| fs::remove_file(backup))
}

fn replace_file_with_backup_cleanup<F>(
    path: &Path,
    bytes: &[u8],
    cleanup_backup: F,
) -> Result<(), AirpError>
where
    F: FnOnce(&Path) -> std::io::Result<()>,
{
    let temporary = path.with_extension("json.tmp");
    let backup = path.with_extension("json.bak");
    {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    if path.exists() {
        let _ = fs::remove_file(&backup);
        fs::rename(path, &backup)?;
    }
    if let Err(error) = fs::rename(&temporary, path) {
        if backup.exists() {
            let _ = fs::rename(&backup, path);
        }
        return Err(error.into());
    }
    // D7: rename is atomic in-memory but the directory entry update is not
    // durable until the parent directory is fsync'd. Without this, a crash
    // after `rename` can leave the file appearing with stale or absent
    // content on disk, undermining every caller that depends on
    // `replace_file` for crash-atomic updates (lorebook, state, character
    // card, revisions, etc.).
    if let Some(parent) = path.parent() {
        sync_dir(parent)?;
    }
    if backup.exists() {
        if let Err(error) = cleanup_backup(&backup) {
            tracing::warn!(
                path = %backup.display(),
                %error,
                "replacement committed but stale backup cleanup failed"
            );
        }
    }
    Ok(())
}

/// fsync a directory so that rename/create/unlink operations on its entries
/// become durable. On Unix we open the directory and call `sync_data`; on
/// Windows we intentionally do nothing because opening a directory handle
/// returns ACCESS_DENIED and NTFS rename atomicity is provided by the
/// filesystem. Mirrors `revision::atomic::sync_dir`.
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

#[cfg(test)]
pub(crate) fn replace_file_with_backup_cleanup_for_test<F>(
    path: &Path,
    bytes: &[u8],
    cleanup_backup: F,
) -> Result<(), AirpError>
where
    F: FnOnce(&Path) -> std::io::Result<()>,
{
    replace_file_with_backup_cleanup(path, bytes, cleanup_backup)
}

pub(crate) fn move_path(src: &Path, dst: &Path) -> Result<(), AirpError> {
    if fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    if src.is_dir() {
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let e = entry?;
            move_path(&e.path(), &dst.join(e.file_name()))?;
        }
        fs::remove_dir(src)?;
    } else {
        fs::copy(src, dst)?;
        fs::remove_file(src)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_file_atomically_swaps_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("target.json");

        replace_file(&path, b"first").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"first");

        replace_file(&path, b"second").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"second");
    }

    /// D7 回归：`replace_file` 必须在 `rename` 后对父目录调用 `sync_dir`，
    /// 否则 rename 在崩溃后可能不持久。我们无法在用户态直接断言 fsync 行为，
    /// 但可以验证：(1) 父目录 sync 不会失败，(2) 替换后内容可见，
    /// (3) 没有残留的 tmp / bak 文件污染目录。
    #[test]
    fn replace_file_syncs_parent_dir_and_leaves_no_residue() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("a/b/c");
        fs::create_dir_all(&nested).unwrap();
        let path = nested.join("lorebook.json");

        replace_file(&path, b"v1").unwrap();
        replace_file(&path, b"v2").unwrap();
        replace_file(&path, b"v3").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"v3");

        // 没有残留 .tmp / .bak 文件
        let entries: Vec<String> = fs::read_dir(&nested)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            entries,
            vec!["lorebook.json".to_string()],
            "no stale .tmp/.bak files should remain after successful replace_file; got {entries:?}"
        );
    }

    #[test]
    fn replace_file_creates_file_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fresh.json");
        replace_file(&path, b"created").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"created");
    }
}
