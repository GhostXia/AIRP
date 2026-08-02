use crate::error::AirpError;
use crate::types::{SceneId, SessionId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SceneLockKey {
    root: PathBuf,
    scene_id: String,
}

type SceneLockRegistry = Mutex<HashMap<SceneLockKey, Weak<Mutex<()>>>>;

static SCENE_WRITE_LOCKS: OnceLock<SceneLockRegistry> = OnceLock::new();

fn scene_write_lock(root: &Path, scene_id: &SceneId) -> Arc<Mutex<()>> {
    // LOCK-ORDER: scene advisory 锁（§1.3 / R5）。独立，不与资源锁嵌套。
    // 合同：docs/LOCK-ORDER-CONTRACT.md §1.3 / §3 R5。
    let key = SceneLockKey {
        root: root.to_path_buf(),
        scene_id: scene_id.as_str().to_string(),
    };
    let mut locks = SCENE_WRITE_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    locks.retain(|_, weak| weak.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

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
    /// #343: 指向该 scene 当前活跃的群聊 Conversation。WebUI 刷新恢复时
    /// 优先读此字段（权威），localStorage 仅作浏览器缓存。旧 scene.json 无此
    /// 字段时反序列化为 `None`，向后兼容。创建新 Conversation 时由
    /// `create_scene_conversation_endpoint` 自动更新。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_conversation_id: Option<SessionId>,
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
        let lock = scene_write_lock(root, &self.scene_id);
        let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
        self.save_unlocked(root)
    }

    fn save_unlocked(&self, root: &Path) -> Result<(), AirpError> {
        let scene_dir = crate::data_dir::scene_dir(root, &self.scene_id);
        std::fs::create_dir_all(&scene_dir)?;
        let path = scene_dir.join("scene.json");
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}

/// Serialize a scene load → mutate → save sequence under its per-path lock.
///
/// The lock key includes the data root and scene ID, so unrelated scenes can
/// still update concurrently while all scene manifest writers for one path
/// observe a single read-modify-write order.
pub fn update_scene<T, F>(root: &Path, scene_id: &SceneId, update: F) -> Result<T, AirpError>
where
    F: FnOnce(&mut SceneConfig) -> Result<T, AirpError>,
{
    let lock = scene_write_lock(root, scene_id);
    let _guard = lock.lock().unwrap_or_else(|p| p.into_inner());
    let mut scene = SceneConfig::load(root, scene_id)?;
    let result = update(&mut scene)?;
    scene.save_unlocked(root)?;
    Ok(result)
}

/// #343: 在 scene manifest 中记录当前活跃的群聊 Conversation。
///
/// 写入失败时 scene.json 不会半写（save 是整体覆盖），但已创建的 Conversation
/// 仍存在——调用方应将 Conversation 创建视为权威，active_conversation_id 视为
/// 指向标记，标记失败时不影响 Conversation 本身。
pub fn set_active_conversation(
    root: &Path,
    scene_id: &SceneId,
    conversation_id: SessionId,
) -> Result<(), AirpError> {
    update_scene(root, scene_id, |scene| {
        scene.active_conversation_id = Some(conversation_id);
        Ok(())
    })
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
            active_conversation_id: None,
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
            active_conversation_id: None,
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
            active_conversation_id: None,
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

    // ── #343: active_conversation_id 持久化 ──────────────────────────────────

    #[test]
    fn test_343_active_conversation_id_roundtrip() {
        let tmp = tempdir().unwrap();
        let mut sc = sample_scene();
        let conv = SessionId::new();
        sc.active_conversation_id = Some(conv);
        sc.save(tmp.path()).unwrap();

        let loaded = SceneConfig::load(tmp.path(), &SceneId::new("tavern").unwrap()).unwrap();
        assert_eq!(loaded.active_conversation_id, Some(conv));
    }

    #[test]
    fn test_343_legacy_scene_json_without_active_conversation_id_loads_as_none() {
        // 旧 scene.json 不含 active_conversation_id 字段，反序列化必须为 None。
        let tmp = tempdir().unwrap();
        let scene_dir = tmp.path().join("scenes").join("legacy");
        std::fs::create_dir_all(&scene_dir).unwrap();
        let legacy_json = r#"{
            "scene_id": "legacy",
            "description": "legacy scene",
            "characters": [],
            "narrator_style": "",
            "lorebook_merge": "union",
            "format_hint": ""
        }"#;
        std::fs::write(scene_dir.join("scene.json"), legacy_json).unwrap();

        let loaded = SceneConfig::load(tmp.path(), &SceneId::new("legacy").unwrap()).unwrap();
        assert_eq!(loaded.active_conversation_id, None);
    }

    #[test]
    fn test_343_none_active_conversation_id_omitted_from_serialized_json() {
        // None 时序列化不写入字段，保证旧客户端/工具不受影响。
        let sc = sample_scene();
        let json = serde_json::to_string(&sc).unwrap();
        assert!(
            !json.contains("active_conversation_id"),
            "None active_conversation_id should be skipped, got: {json}"
        );
    }

    #[test]
    fn test_343_set_active_conversation_helper_updates_scene_manifest() {
        let tmp = tempdir().unwrap();
        let scene_id = SceneId::new("tavern").unwrap();
        sample_scene().save(tmp.path()).unwrap();

        let conv = SessionId::new();
        set_active_conversation(tmp.path(), &scene_id, conv).unwrap();

        let loaded = SceneConfig::load(tmp.path(), &scene_id).unwrap();
        assert_eq!(loaded.active_conversation_id, Some(conv));
        // 其他字段保持不变
        assert_eq!(loaded.characters.len(), 2);
        assert_eq!(loaded.description, "A tavern scene");
    }

    #[test]
    fn test_343_set_active_conversation_overwrites_previous_value() {
        let tmp = tempdir().unwrap();
        let scene_id = SceneId::new("tavern").unwrap();
        sample_scene().save(tmp.path()).unwrap();

        let conv1 = SessionId::new();
        set_active_conversation(tmp.path(), &scene_id, conv1).unwrap();
        let conv2 = SessionId::new();
        set_active_conversation(tmp.path(), &scene_id, conv2).unwrap();

        let loaded = SceneConfig::load(tmp.path(), &scene_id).unwrap();
        assert_eq!(loaded.active_conversation_id, Some(conv2));
        assert_ne!(loaded.active_conversation_id, Some(conv1));
    }

    #[test]
    fn test_343_set_active_conversation_errors_when_scene_missing() {
        let tmp = tempdir().unwrap();
        let scene_id = SceneId::new("no_such_scene").unwrap();
        let conv = SessionId::new();
        let result = set_active_conversation(tmp.path(), &scene_id, conv);
        assert!(
            result.is_err(),
            "setting active conversation on missing scene should error"
        );
    }

    #[test]
    fn test_376_update_scene_preserves_independent_mutations() {
        let tmp = tempdir().unwrap();
        let scene_id = SceneId::new("tavern").unwrap();
        sample_scene().save(tmp.path()).unwrap();

        update_scene(tmp.path(), &scene_id, |scene| {
            scene.active_conversation_id = Some(SessionId::new());
            Ok(())
        })
        .unwrap();
        update_scene(tmp.path(), &scene_id, |scene| {
            scene.characters.push(CharacterEntry {
                character_id: "carol".to_string(),
                role: CharacterRole::Npc,
                intro: "The bard".to_string(),
            });
            Ok(())
        })
        .unwrap();

        let loaded = SceneConfig::load(tmp.path(), &scene_id).unwrap();
        assert!(loaded.active_conversation_id.is_some());
        assert_eq!(loaded.characters.len(), 3);
        assert_eq!(loaded.characters[2].character_id, "carol");
    }
}
