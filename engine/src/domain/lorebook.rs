//! Lorebook (worldbook) domain service: read/write character-scoped lorebook
//! with v3→v4 selective migration and unified revision contract.
//!
//! Extracted from `domain/mod.rs` (E-P1-1 slice 2). Zero behavior change.

use std::fs;
use std::path::PathBuf;

use crate::data_dir;
use crate::error::AirpError;
use crate::revision::atomic::{
    commit_revision, next_content_revision, CommitOptions, StagedRevision,
};
use crate::revision::manifest::{AssetKind, AssetSource};
use crate::types::CharacterId;

use super::lock_order;
use super::locks::{character_lock, state_lock};

#[derive(Clone, Debug)]
pub struct LorebookService {
    data_root: PathBuf,
}

impl LorebookService {
    pub fn new(data_root: impl AsRef<std::path::Path>) -> Self {
        Self {
            data_root: data_root.as_ref().to_path_buf(),
        }
    }

    pub fn read(
        &self,
        character_id: &CharacterId,
    ) -> Result<crate::orchestrator::Lorebook, AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().unwrap_or_else(|p| p.into_inner());
        let resource = state_lock(character_id.as_str());
        let _resource_guard = resource.lock().unwrap_or_else(|p| p.into_inner());
        let _state_track = lock_order::track_state();
        let path = data_dir::char_world_lorebook_path(&self.data_root, character_id.as_str());
        if !path.exists() {
            return Err(AirpError::NotFound(format!(
                "lorebook for character {character_id} not found"
            )));
        }
        let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path)?)?;
        migrate_lorebook_v3_selective(&mut value)?;
        Ok(serde_json::from_value(value)?)
    }

    pub fn write(
        &self,
        character_id: &CharacterId,
        lorebook: &crate::orchestrator::Lorebook,
    ) -> Result<(), AirpError> {
        let character = character_lock(character_id.as_str());
        let _guard = character.read().unwrap_or_else(|p| p.into_inner());
        let resource = state_lock(character_id.as_str());
        let _resource_guard = resource.lock().unwrap_or_else(|p| p.into_inner());
        let _state_track = lock_order::track_state();
        let world_dir = data_dir::char_world_dir(&self.data_root, character_id.as_str())?;
        let path = data_dir::char_world_lorebook_path(&self.data_root, character_id.as_str());
        let lorebook_bytes = serde_json::to_vec_pretty(lorebook)?;
        data_dir::replace_file(&path, &lorebook_bytes)?;

        // #115 Phase 2d：Worldbook 接入统一 revision 合同。
        // 工作副本 `lorebook.json` 已原子写入；下面在 `characters/{id}/world/` 下
        // 创建 `revisions/{content_revision}/` + `current_revision` 不可变快照。
        // 使用 next_content_revision 跳过 orphan revision_dir（详见 atomic::next_content_revision 文档）。
        let content_revision = next_content_revision(&world_dir)?;
        let source_hash_hex = {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(&lorebook_bytes);
            format!("{:x}", hasher.finalize())
        };
        let now = chrono::Utc::now().to_rfc3339();
        let staged = StagedRevision {
            content_revision,
            asset_kind: AssetKind::Worldbook,
            asset_id: character_id.to_string(),
            created_at: now.clone(),
            source: AssetSource {
                source_kind: "controlled_upload".to_string(),
                source_hash: Some(source_hash_hex),
                source_filename: None,
                converter_version: None,
                imported_at: Some(now),
                parent_revision: if content_revision > 1 {
                    Some(content_revision - 1)
                } else {
                    None
                },
            },
            files: vec![("lorebook.json".to_string(), lorebook_bytes)],
        };
        let commit_opts = CommitOptions::new(&world_dir);
        commit_revision(&staged, &commit_opts)?;
        Ok(())
    }
}

/// v3 persisted `selective` under `extensions`. Preserve field presence before
/// deserializing v4's defaulted bool so extension-only `true` is not lost.
fn migrate_lorebook_v3_selective(value: &mut serde_json::Value) -> Result<(), AirpError> {
    let Some(entries) = value
        .get_mut("entries")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return Ok(());
    };
    for entry in entries {
        let Some(entry) = entry.as_object_mut() else {
            continue;
        };
        let extension_selective = entry
            .get("extensions")
            .and_then(serde_json::Value::as_object)
            .and_then(|extensions| extensions.get("selective"))
            .cloned();
        if extension_selective
            .as_ref()
            .is_some_and(|selective| !selective.is_boolean())
        {
            return Err(AirpError::Internal(
                "lorebook extensions.selective must be a boolean".to_string(),
            ));
        }
        if !entry.contains_key("selective") {
            if let Some(selective) = extension_selective.as_ref() {
                entry.insert("selective".to_string(), selective.clone());
            }
        }
        let extensions_empty = entry
            .get_mut("extensions")
            .and_then(serde_json::Value::as_object_mut)
            .is_some_and(|extensions| {
                extensions.remove("selective");
                extensions.is_empty()
            });
        if extensions_empty {
            entry.remove("extensions");
        }
    }
    Ok(())
}
