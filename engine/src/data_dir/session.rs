use crate::error::AirpError;
use std::fs;
use std::path::{Path, PathBuf};

fn migrate_legacy_volume_files(legacy: &Path, new_dir: &Path) -> Result<(), AirpError> {
    if !legacy.exists() || legacy == new_dir || new_dir.join("current.md").exists() {
        return Ok(());
    }
    let entries = ["current.md", "index.md", "turn_counter.txt"];
    let has_any =
        entries.iter().any(|f| legacy.join(f).exists()) || legacy.join("volumes").exists();
    if !has_any {
        return Ok(());
    }

    fs::create_dir_all(new_dir)?;

    for fname in entries {
        let src = legacy.join(fname);
        if src.exists() {
            super::utils::move_path(&src, &new_dir.join(fname))?;
        }
    }
    let vol_src = legacy.join("volumes");
    if vol_src.exists() {
        super::utils::move_path(&vol_src, &new_dir.join("volumes"))?;
    }

    tracing::info!(legacy = ?legacy, new = ?new_dir, "CF-3: 迁移 volume 文件到 memory/");

    let _ = fs::remove_dir(legacy);
    Ok(())
}

pub fn session_dir(root: &Path, character_id: &str) -> Result<PathBuf, AirpError> {
    let char_dir = super::paths::character_dir(root, character_id)?;
    let new_dir = char_dir.join("memory");
    let legacy = char_dir.join("session");
    migrate_legacy_volume_files(&legacy, &new_dir)?;
    crate::volume_store::ensure_session_dirs(&new_dir)?;
    Ok(new_dir)
}

pub fn resolve_session_dir(
    root: &Path,
    character_id: &str,
    session_id: Option<&crate::types::SessionId>,
) -> Result<PathBuf, AirpError> {
    match session_id {
        None => session_dir(root, character_id),
        Some(sid) => {
            let char_dir = super::paths::character_dir(root, character_id)?;
            let session_root = char_dir.join("sessions").join(sid.to_string());
            let new_dir = session_root.join("memory");
            migrate_legacy_volume_files(&session_root, &new_dir)?;
            crate::volume_store::ensure_session_dirs(&new_dir)?;
            Ok(new_dir)
        }
    }
}

pub fn list_sessions(
    root: &Path,
    character_id: &str,
) -> Result<Vec<crate::types::SessionId>, AirpError> {
    let sessions_root = root.join("characters").join(character_id).join("sessions");
    if !sessions_root.exists() {
        return Ok(vec![]);
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(&sessions_root)? {
        let entry = entry?;
        if !entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(sid) = crate::types::SessionId::parse(name) {
                result.push(sid);
            }
        }
    }
    result.sort_by_key(|s| s.to_string());
    Ok(result)
}

pub fn create_session(
    root: &Path,
    character_id: &str,
) -> Result<crate::types::SessionId, AirpError> {
    let sid = crate::types::SessionId::new();
    let _ = resolve_session_dir(root, character_id, Some(&sid))?;
    Ok(sid)
}

fn deleted_session_marker(
    root: &Path,
    character_id: &str,
    session_id: &crate::types::SessionId,
) -> PathBuf {
    root.join("characters")
        .join(character_id)
        .join("deleted_sessions")
        .join(session_id.to_string())
}

/// Distinguish a deleted named session from a never-seen ID. Legacy clients may
/// address a fresh valid ID directly and rely on lazy creation; a tombstone is
/// therefore required to prevent only explicitly deleted sessions from reviving.
pub fn session_was_deleted(
    root: &Path,
    character_id: &str,
    session_id: &crate::types::SessionId,
) -> bool {
    deleted_session_marker(root, character_id, session_id).is_file()
}

/// #35：删除一个命名会话目录（`characters/{id}/sessions/{sid}/`）。
///
/// 会话不存在 → `NotFound`。destructive：调用方负责确认。删除的是整个会话目录
/// （memory + volumes + history + meta），不可恢复——与 `delete_character` 同边界。
pub fn delete_session(
    root: &Path,
    character_id: &str,
    session_id: &crate::types::SessionId,
) -> Result<(), AirpError> {
    super::security::validate_id_segment(character_id)?;
    let dir = root
        .join("characters")
        .join(character_id)
        .join("sessions")
        .join(session_id.to_string());
    if !dir.is_dir() {
        return Err(AirpError::NotFound(format!(
            "session {session_id} for character {character_id} not found"
        )));
    }
    let marker = deleted_session_marker(root, character_id, session_id);
    let marker_parent = marker
        .parent()
        .ok_or_else(|| AirpError::Internal("deleted session marker has no parent".to_string()))?;
    fs::create_dir_all(marker_parent)?;
    fs::File::create(marker)?.sync_all()?;
    fs::remove_dir_all(&dir)?;
    Ok(())
}
