use crate::error::AirpError;
use std::fs;
use std::path::Path;

#[inline]
pub(crate) fn strip_utf8_bom(s: &str) -> &str {
    s.strip_prefix('\u{FEFF}').unwrap_or(s)
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
