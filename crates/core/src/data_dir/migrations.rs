use crate::error::AirpError;
use std::fs;
use std::path::{Path, PathBuf};

pub fn migrate_legacy_char_dirs(root: &Path) -> Result<(), AirpError> {
    let chars = root.join("characters");
    if !chars.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(&chars)? {
        let entry = entry?;
        let char_dir = entry.path();
        if !char_dir.is_dir() {
            continue;
        }
        let lock = char_dir.join("migration_done.lock");
        if lock.exists() {
            continue;
        }
        let char_id = match entry.file_name().to_str() {
            Some(s) => s.to_string(),
            None => continue,
        };
        if let Err(e) = migrate_one_char_dir(root, &char_dir, &char_id) {
            tracing::warn!(char = %char_id, err = %e, "CF-6: 单角色迁移失败");
            continue;
        }
        let stamp = format!("CF-6 migrated at {}\n", chrono::Utc::now().to_rfc3339());
        if let Err(e) = fs::write(&lock, stamp) {
            tracing::warn!(path = ?lock, err = %e, "CF-6: 写 lock 失败");
        }
    }
    Ok(())
}

fn migrate_one_char_dir(root: &Path, char_dir: &Path, char_id: &str) -> Result<(), AirpError> {
    let card_dir = char_dir.join("card");
    for fname in ["card.png", "card.json"] {
        let old = char_dir.join(fname);
        let new = card_dir.join(fname);
        if old.exists() && !new.exists() {
            fs::create_dir_all(&card_dir)?;
            super::utils::move_path(&old, &new)?;
            tracing::info!(old = ?old, new = ?new, "CF-6: 迁移到 card/");
        }
    }

    let _ = super::paths::character_dir(root, char_id)?;
    let _ = super::session::session_dir(root, char_id)?;
    for sid in super::session::list_sessions(root, char_id).unwrap_or_default() {
        let _ = super::session::resolve_session_dir(root, char_id, Some(&sid))?;
    }
    Ok(())
}

pub fn migrate_legacy_presets(root: &Path) -> Result<(), AirpError> {
    let presets = root.join("presets");
    if !presets.exists() {
        return Ok(());
    }
    let mut flat_files: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(&presets)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            flat_files.push(path);
        }
    }
    for path in flat_files {
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        let target_name = match ext {
            "json" => "preset.json",
            "md" => "preset.md",
            _ => continue,
        };
        let new_dir = presets.join(&stem);
        if let Err(e) = fs::create_dir_all(&new_dir) {
            tracing::warn!(path = ?new_dir, err = %e, "M_PR: 创建预设目录失败");
            continue;
        }
        let new_path = new_dir.join(target_name);
        if new_path.exists() {
            continue;
        }
        if let Err(e) = super::utils::move_path(&path, &new_path) {
            tracing::warn!(old = ?path, new = ?new_path, err = %e, "M_PR: 迁移预设失败");
            continue;
        }
        tracing::info!(new = ?new_path, "M_PR: 迁移扁平预设到目录结构");
    }
    Ok(())
}
