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
    // L1 修复（PR #220）：保留原扩展名而非替换为 .json.tmp/.json.bak。
    // `with_extension("json.tmp")` 会把 `current.md` 变成 `current.json.tmp`，
    // `chat_log.jsonl` 变成 `chat_log.json.tmp`，导致文件名污染。
    // 改为追加 .tmp/.bak 后缀，保留原扩展名：`current.md.tmp` / `chat_log.jsonl.bak`。
    //
    // PR #227 审计修复（gemini + coderabbit）：
    // - 无扩展名文件（如 `current`）不能 fallback 成 `"tmp"`，否则会变成
    //   `current.tmp.tmp`；应用 `with_extension("tmp")` 直接替换为 `current.tmp`。
    // - 使用 `OsString` 避免 non-UTF-8 路径的 lossy 转换。
    let (temporary, backup) = match path.extension() {
        Some(ext) => {
            let mut tmp_ext = ext.to_os_string();
            tmp_ext.push(".tmp");
            let mut bak_ext = ext.to_os_string();
            bak_ext.push(".bak");
            (path.with_extension(tmp_ext), path.with_extension(bak_ext))
        }
        None => (path.with_extension("tmp"), path.with_extension("bak")),
    };
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
    //
    // Q-A2 fix: 复用 `revision::atomic::sync_dir`，避免两处实现漂移。
    // Unix 上打开目录并 sync_data；Windows 上 no-op（NTFS rename 原子性
    // 由文件系统保证，且打开目录句柄会触发 ACCESS_DENIED 延迟释放）。
    if let Some(parent) = path.parent() {
        crate::revision::atomic::sync_dir(parent)?;
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

    /// #220 L4 回归：非 .json 文件（如 .md / .jsonl）的 tmp/backup 必须保留原扩展名，
    /// 不能变成 `.json.tmp`。验证 `current.md` 替换后无残留且内容正确。
    ///
    /// PR #227 审计修复（coderabbit）：注入 backup-cleanup 失败，直接断言 backup
    /// 文件名为 `current.md.bak` 而非 `current.json.bak`，让测试真正观察文件名
    /// 而非只检查"无残留"（无残留会被旧 bug 通过——cleanup 把错误命名的 backup
    /// 也删掉了）。
    #[test]
    fn replace_file_preserves_non_json_extension() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("current.md");

        replace_file(&path, b"# v1").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"# v1");

        // 注入 backup-cleanup 失败，让 backup 文件留下来以便断言其命名。
        // 失败的 cleanup 不影响主路径成功（replace_file 仍返回 Ok）。
        replace_file_with_backup_cleanup_for_test(&path, b"# v2", |_| {
            Err(std::io::Error::other("keep backup for naming assertion"))
        })
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"# v2");

        // backup 文件名必须是 current.md.bak（旧 bug 会产生 current.json.bak）
        assert!(
            tmp.path().join("current.md.bak").exists(),
            "backup must preserve .md extension; dir: {:?}",
            fs::read_dir(tmp.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect::<Vec<_>>()
        );
        assert!(
            !tmp.path().join("current.json.bak").exists(),
            "current.json.bak must not exist (old bug)"
        );
        assert!(
            !tmp.path().join("current.md.tmp").exists(),
            "tmp file must be renamed into place, not left behind"
        );
        assert!(
            !tmp.path().join("current.json.tmp").exists(),
            "current.json.tmp must not exist (old bug)"
        );
    }

    /// PR #227 审计修复（coderabbit）：无扩展名文件（如 `current`）的 tmp/backup
    /// 必须是 `current.tmp` / `current.bak`，不能是 `current.tmp.tmp`。
    #[test]
    fn replace_file_handles_extensionless_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("current");

        replace_file(&path, b"v1").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"v1");

        // 注入失败以观察 backup 文件名
        replace_file_with_backup_cleanup_for_test(&path, b"v2", |_| {
            Err(std::io::Error::other("keep backup for naming assertion"))
        })
        .unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"v2");

        assert!(
            tmp.path().join("current.bak").exists(),
            "extensionless backup must be current.bak, not current.tmp.bak"
        );
        assert!(
            !tmp.path().join("current.tmp.bak").exists(),
            "current.tmp.bak must not exist (bug from unwrap_or(\"tmp\") fallback)"
        );
        assert!(
            !tmp.path().join("current.tmp.tmp").exists(),
            "current.tmp.tmp must not exist (bug from unwrap_or(\"tmp\") fallback)"
        );
    }
}
