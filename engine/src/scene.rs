use crate::error::AirpError;
use crate::types::SceneId;
use serde::{Deserialize, Serialize};
use std::path::Path;

// ── MS-2: Scene data types ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CharacterRole {
    Primary,
    #[default]
    Npc,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterEntry {
    pub character_id: String,
    #[serde(default)]
    pub role: CharacterRole,
    #[serde(default)]
    pub intro: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LorebookMerge {
    #[default]
    Union,
    PrimaryOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneConfig {
    /// AUDIT-2: validated newtype — `validate_id_segment` runs at serde
    /// deserialize time, so any inbound SceneConfig has a safe scene_id.
    pub scene_id: SceneId,
    #[serde(default)]
    pub description: String,
    pub characters: Vec<CharacterEntry>,
    #[serde(default)]
    pub narrator_style: String,
    #[serde(default)]
    pub lorebook_merge: LorebookMerge,
    #[serde(default)]
    pub format_hint: String,
}

impl SceneConfig {
    pub fn primary(&self) -> Option<&CharacterEntry> {
        self.characters
            .iter()
            .find(|c| c.role == CharacterRole::Primary)
    }

    pub fn load(root: &Path, scene_id: &SceneId) -> Result<Self, AirpError> {
        let path = crate::data_dir::scene_json_path(root, scene_id);
        let json = std::fs::read_to_string(&path)?;
        Ok(serde_json::from_str(&json)?)
    }

    pub fn save(&self, root: &Path) -> Result<(), AirpError> {
        let scene_dir = crate::data_dir::scene_dir(root, &self.scene_id);
        std::fs::create_dir_all(&scene_dir)?;
        let path = scene_dir.join("scene.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_scene() -> SceneConfig {
        SceneConfig {
            scene_id: SceneId::new("tavern").unwrap(),
            description: "A tavern scene".to_string(),
            characters: vec![
                CharacterEntry {
                    character_id: "alice".to_string(),
                    role: CharacterRole::Primary,
                    intro: "The hero".to_string(),
                },
                CharacterEntry {
                    character_id: "bob".to_string(),
                    role: CharacterRole::Npc,
                    intro: "The innkeeper".to_string(),
                },
            ],
            narrator_style: "third_person_limited".to_string(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: "Name: dialogue".to_string(),
        }
    }

    #[test]
    fn test_ms2_scene_config_roundtrip() {
        let tmp = tempdir().unwrap();
        let sc = sample_scene();
        sc.save(tmp.path()).unwrap();

        let loaded = SceneConfig::load(tmp.path(), &SceneId::new("tavern").unwrap()).unwrap();
        assert_eq!(loaded.scene_id.as_str(), "tavern");
        assert_eq!(loaded.characters.len(), 2);
        assert_eq!(loaded.characters[0].role, CharacterRole::Primary);
    }

    #[test]
    fn test_ms2_primary_finds_primary_character() {
        let sc = sample_scene();
        assert_eq!(sc.primary().map(|c| c.character_id.as_str()), Some("alice"));
    }

    #[test]
    fn test_ms2_scene_defaults() {
        let json = r#"{"scene_id":"s1","characters":[]}"#;
        let sc: SceneConfig = serde_json::from_str(json).unwrap();
        assert_eq!(sc.scene_id.as_str(), "s1");
        assert_eq!(sc.lorebook_merge, LorebookMerge::Union);
        assert!(sc.description.is_empty());
        assert!(sc.primary().is_none());
    }

    #[test]
    fn test_ms2_list_scenes_empty_when_no_dir() {
        let tmp = tempdir().unwrap();
        let list = crate::data_dir::list_scenes(tmp.path()).unwrap();
        assert!(list.is_empty());
    }

    #[test]
    fn test_ms2_list_scenes_returns_saved() {
        let tmp = tempdir().unwrap();
        sample_scene().save(tmp.path()).unwrap();
        let list = crate::data_dir::list_scenes(tmp.path()).unwrap();
        assert_eq!(list, vec!["tavern"]);
    }

    // AUDIT-4: edge case coverage for scene module

    #[test]
    fn test_audit_4_load_nonexistent_scene_errors() {
        let tmp = tempdir().unwrap();
        let result = SceneConfig::load(tmp.path(), &SceneId::new("no_such_scene").unwrap());
        assert!(result.is_err(), "loading nonexistent scene should error");
    }

    #[test]
    fn test_audit_4_primary_returns_none_when_only_npcs() {
        let sc = SceneConfig {
            scene_id: SceneId::new("npc_only").unwrap(),
            description: String::new(),
            characters: vec![
                CharacterEntry {
                    character_id: "a".to_string(),
                    role: CharacterRole::Npc,
                    intro: String::new(),
                },
                CharacterEntry {
                    character_id: "b".to_string(),
                    role: CharacterRole::Npc,
                    intro: String::new(),
                },
            ],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };
        assert!(sc.primary().is_none());
    }

    #[test]
    fn test_audit_4_primary_returns_first_when_multiple() {
        // Behavior: if multiple Primary characters defined (config error),
        // primary() returns the first one. Documents existing semantics.
        let sc = SceneConfig {
            scene_id: SceneId::new("multi").unwrap(),
            description: String::new(),
            characters: vec![
                CharacterEntry {
                    character_id: "first".to_string(),
                    role: CharacterRole::Primary,
                    intro: String::new(),
                },
                CharacterEntry {
                    character_id: "second".to_string(),
                    role: CharacterRole::Primary,
                    intro: String::new(),
                },
            ],
            narrator_style: String::new(),
            lorebook_merge: LorebookMerge::Union,
            format_hint: String::new(),
        };
        assert_eq!(sc.primary().map(|c| c.character_id.as_str()), Some("first"));
    }

    #[test]
    fn test_audit_4_save_creates_scene_dir_if_missing() {
        let tmp = tempdir().unwrap();
        // No scenes/ dir exists
        let sc = sample_scene();
        sc.save(tmp.path()).unwrap();
        let expected = tmp.path().join("scenes").join("tavern").join("scene.json");
        assert!(expected.exists(), "save should create nested dirs");
    }

    #[test]
    fn test_audit_4_lorebook_merge_serializes_snake_case() {
        let json = serde_json::to_string(&LorebookMerge::PrimaryOnly).unwrap();
        assert_eq!(json, "\"primary_only\"");
        let json = serde_json::to_string(&LorebookMerge::Union).unwrap();
        assert_eq!(json, "\"union\"");
    }

    #[test]
    fn test_audit_4_character_role_serializes_snake_case() {
        let json = serde_json::to_string(&CharacterRole::Primary).unwrap();
        assert_eq!(json, "\"primary\"");
        let json = serde_json::to_string(&CharacterRole::Npc).unwrap();
        assert_eq!(json, "\"npc\"");
    }

    #[test]
    fn test_audit_4_scene_load_rejects_malformed_json() {
        let tmp = tempdir().unwrap();
        let scene_dir = tmp.path().join("scenes").join("broken");
        std::fs::create_dir_all(&scene_dir).unwrap();
        std::fs::write(scene_dir.join("scene.json"), "{not json").unwrap();
        let result = SceneConfig::load(tmp.path(), &SceneId::new("broken").unwrap());
        assert!(result.is_err(), "malformed JSON should error");
    }
}
