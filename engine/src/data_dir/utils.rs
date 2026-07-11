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
    if backup.exists() {
        fs::remove_file(backup)?;
    }
    Ok(())
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
