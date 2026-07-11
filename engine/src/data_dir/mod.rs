mod migrations;
mod paths;
mod security;
mod session;
mod utils;

pub use migrations::{migrate_legacy_char_dirs, migrate_legacy_presets};
pub use paths::{
    char_analysis_dir_path,
    char_analysis_file_path,
    char_state_dir,
    char_state_history_path,
    character_dir,
    delete_character,
    ensure_char_analysis_dir,
    ensure_data_dirs,
    ensure_preset_analysis_dir,
    get_character_card,
    list_characters,
    list_presets,
    // M_MS: scene paths
    list_scenes,
    // M_UP: user persona paths
    list_users,
    preset_analysis_dir_path,
    preset_analysis_file_path,
    read_character_card_text,
    resolve_data_root,
    // DX-1: per-user data root
    resolve_effective_root,
    scene_dir,
    scene_history_dir,
    scene_json_path,
    scene_memory_dir,
    scene_world_dir,
    scene_world_lorebook_path,
    user_dir,
    user_persona_lock_path,
    user_persona_path,
    user_state_dir,
    user_state_history_path,
    user_state_live_path,
};
pub(crate) use paths::{
    char_card_dir, char_gating_dir, char_greetings_dir, char_world_dir, char_world_lorebook_path,
    ensure_context_bundle_dir, preset_json_path,
};
pub use security::{safe_resolve_for_write, safe_resolve_under_data_root, validate_id_segment};
pub use session::{
    create_session, delete_session, list_sessions, resolve_session_dir, session_dir,
    session_was_deleted,
};
pub(crate) use utils::strip_utf8_bom;

#[cfg(test)]
mod tests {
    use super::paths::{char_analysis_dir, char_history_dir, preset_dir, preset_regex_dir};
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_validate_id_segment_accepts_normal() {
        assert!(validate_id_segment("alice").is_ok());
        assert!(validate_id_segment("艾米丽").is_ok());
        assert!(validate_id_segment("test_char-01").is_ok());
        assert!(validate_id_segment("v1.2").is_ok());
    }

    #[test]
    fn test_validate_id_segment_rejects_traversal() {
        assert!(validate_id_segment("").is_err());
        assert!(validate_id_segment(".").is_err());
        assert!(validate_id_segment("..").is_err());
        assert!(validate_id_segment("../etc").is_err());
        assert!(validate_id_segment("a/b").is_err());
        assert!(validate_id_segment("a\\b").is_err());
        assert!(validate_id_segment("a:b").is_err());
        assert!(validate_id_segment(".hidden").is_err());
        assert!(validate_id_segment("a\0b").is_err());
        assert!(validate_id_segment("evil..bypass").is_err());
    }

    #[test]
    fn test_safe_resolve_accepts_subpath() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sub = root.join("characters").join("alice");
        fs::create_dir_all(&sub).unwrap();
        let card = sub.join("card.json");
        fs::write(&card, "{}").unwrap();

        let resolved = safe_resolve_under_data_root(root, "characters/alice/card.json")
            .expect("subpath should resolve");
        assert!(resolved.ends_with("card.json"));
    }

    #[test]
    fn test_safe_resolve_rejects_absolute() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        assert!(safe_resolve_under_data_root(root, "/etc/passwd").is_err());
        assert!(
            safe_resolve_under_data_root(root, "\\Windows\\System32\\drivers\\etc\\hosts").is_err()
        );
        assert!(
            safe_resolve_under_data_root(root, "C:\\Windows\\System32\\drivers\\etc\\hosts")
                .is_err()
        );
    }

    #[test]
    fn test_safe_resolve_rejects_traversal_outside_root() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("data");
        fs::create_dir_all(&root).unwrap();
        let outside = tmp.path().join("secret.txt");
        fs::write(&outside, "hush").unwrap();

        let res = safe_resolve_under_data_root(&root, "../secret.txt");
        assert!(
            res.is_err(),
            "expected traversal to be rejected, got {:?}",
            res
        );
    }

    #[test]
    fn test_safe_resolve_rejects_null_byte() {
        let tmp = tempdir().unwrap();
        assert!(safe_resolve_under_data_root(tmp.path(), "ab\0cd").is_err());
    }

    #[test]
    fn test_resolve_session_dir_default_memory_path_when_none() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = resolve_session_dir(root, "alice", None).unwrap();
        assert!(dir.ends_with("memory"), "dir = {:?}", dir);
        assert!(dir.exists());
    }

    #[test]
    fn test_resolve_session_dir_named_memory_path_when_some() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sid = crate::types::SessionId::new();
        let dir = resolve_session_dir(root, "alice", Some(&sid)).unwrap();
        let expected_tail = format!(
            "sessions{sep}{sid}{sep}memory",
            sep = std::path::MAIN_SEPARATOR,
            sid = sid
        );
        assert!(
            dir.to_string_lossy().ends_with(&expected_tail),
            "dir = {:?}, expected tail {:?}",
            dir,
            expected_tail
        );
        assert!(dir.exists());
    }

    #[test]
    fn test_cf3_migrate_legacy_session_to_memory() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("alice");
        let legacy = char_dir.join("session");
        let volumes = legacy.join("volumes");
        fs::create_dir_all(&volumes).unwrap();
        fs::write(legacy.join("current.md"), "old current").unwrap();
        fs::write(legacy.join("index.md"), "old index").unwrap();
        fs::write(legacy.join("turn_counter.txt"), "7").unwrap();
        fs::write(volumes.join("vol_001.md"), "vol1 content").unwrap();

        let new_dir = resolve_session_dir(root, "alice", None).unwrap();
        assert!(new_dir.ends_with("memory"));

        assert_eq!(
            fs::read_to_string(new_dir.join("current.md")).unwrap(),
            "old current"
        );
        assert_eq!(
            fs::read_to_string(new_dir.join("index.md")).unwrap(),
            "old index"
        );
        assert_eq!(
            fs::read_to_string(new_dir.join("turn_counter.txt")).unwrap(),
            "7"
        );
        assert_eq!(
            fs::read_to_string(new_dir.join("volumes").join("vol_001.md")).unwrap(),
            "vol1 content"
        );

        assert!(!legacy.exists(), "迁移后 session/ 空目录应被删除");

        let again = resolve_session_dir(root, "alice", None).unwrap();
        assert_eq!(again, new_dir);
    }

    #[test]
    fn test_cf3_migrate_named_session_to_memory() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sid = crate::types::SessionId::new();
        let session_root = root
            .join("characters")
            .join("alice")
            .join("sessions")
            .join(sid.to_string());
        let volumes = session_root.join("volumes");
        fs::create_dir_all(&volumes).unwrap();
        fs::write(session_root.join("current.md"), "session current").unwrap();
        fs::write(volumes.join("vol_002.md"), "vol2 content").unwrap();

        let new_dir = resolve_session_dir(root, "alice", Some(&sid)).unwrap();
        assert!(new_dir.ends_with("memory"));

        assert_eq!(
            fs::read_to_string(new_dir.join("current.md")).unwrap(),
            "session current"
        );
        assert_eq!(
            fs::read_to_string(new_dir.join("volumes").join("vol_002.md")).unwrap(),
            "vol2 content"
        );

        assert!(session_root.exists(), "sessions/{{uuid}}/ 仍保留");
        assert!(
            !session_root.join("current.md").exists(),
            "current.md 已移走"
        );
        assert!(!session_root.join("volumes").exists(), "volumes/ 已移走");
    }

    #[test]
    fn test_cf3_no_migration_when_new_has_data() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("bob");
        let legacy = char_dir.join("session");
        let new_dir = char_dir.join("memory");
        fs::create_dir_all(&legacy).unwrap();
        fs::create_dir_all(&new_dir).unwrap();
        fs::write(legacy.join("current.md"), "legacy data").unwrap();
        fs::write(new_dir.join("current.md"), "new data").unwrap();

        let resolved = resolve_session_dir(root, "bob", None).unwrap();
        assert_eq!(resolved, new_dir);
        assert_eq!(
            fs::read_to_string(new_dir.join("current.md")).unwrap(),
            "new data"
        );
        assert!(legacy.join("current.md").exists());
    }

    #[test]
    fn test_list_and_create_sessions() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let initial = list_sessions(root, "alice").unwrap();
        assert!(initial.is_empty());

        let sid1 = create_session(root, "alice").unwrap();
        let sid2 = create_session(root, "alice").unwrap();
        assert_ne!(sid1, sid2);

        let listed = list_sessions(root, "alice").unwrap();
        assert_eq!(listed.len(), 2);
        assert!(listed.contains(&sid1));
        assert!(listed.contains(&sid2));
    }

    #[test]
    fn test_list_sessions_ignores_non_uuid_dirs() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let sessions_root = root.join("characters").join("alice").join("sessions");
        fs::create_dir_all(&sessions_root).unwrap();
        fs::create_dir_all(sessions_root.join("not-a-uuid")).unwrap();
        let sid = crate::types::SessionId::new();
        fs::create_dir_all(sessions_root.join(sid.to_string())).unwrap();

        let listed = list_sessions(root, "alice").unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], sid);
    }

    #[test]
    fn test_list_presets_covers_json_and_md_dedup() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let presets = root.join("presets");
        fs::create_dir_all(&presets).unwrap();

        fs::write(presets.join("foo.json"), "{}").unwrap();
        fs::write(presets.join("bar.md"), "# bar").unwrap();
        fs::write(presets.join("baz.json"), "{}").unwrap();
        fs::write(presets.join("baz.md"), "# baz").unwrap();
        fs::write(presets.join("ignore.txt"), "x").unwrap();

        let list = list_presets(root).unwrap();
        assert_eq!(
            list,
            vec!["bar".to_string(), "baz".to_string(), "foo".to_string()]
        );
    }

    #[test]
    fn test_cf1_char_card_dir_creates_and_returns_path() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = char_card_dir(root, "alice").unwrap();
        assert!(dir.ends_with("card"));
        assert!(dir.exists());
        assert!(dir.parent().unwrap().ends_with("alice"));
    }

    #[test]
    fn test_cf1_char_greetings_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = char_greetings_dir(root, "alice").unwrap();
        assert!(dir.ends_with("greetings"));
        assert!(dir.exists());
        assert!(dir.parent().unwrap().ends_with("card"));
    }

    #[test]
    fn test_cf1_char_world_dir_includes_extra() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = char_world_dir(root, "alice").unwrap();
        assert!(dir.ends_with("world"));
        assert!(dir.exists());
        assert!(dir.join("extra").exists(), "world/extra/ 应被自动创建");
    }

    #[test]
    fn test_cf1_char_world_lorebook_path_no_create() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let path = char_world_lorebook_path(root, "alice");
        assert!(path.ends_with("lorebook.json"));
        assert!(!path.exists());
    }

    #[test]
    fn test_cf1_char_history_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = char_history_dir(root, "alice").unwrap();
        assert!(dir.ends_with("history"));
        assert!(dir.exists());
    }

    #[test]
    fn test_cf1_char_analysis_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = char_analysis_dir(root, "alice").unwrap();
        assert!(dir.ends_with("analysis"));
        assert!(dir.exists());
    }

    #[test]
    fn test_cf1_char_gating_dir_initializes_templates() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = char_gating_dir(root, "alice").unwrap();
        assert!(dir.ends_with("gating"));
        assert!(dir.join("checkpoints.md").exists());
        assert!(dir.join("timeline.md").exists());
        let cp = fs::read_to_string(dir.join("checkpoints.md")).unwrap();
        assert!(cp.contains("CP-1"));
    }

    #[test]
    fn test_strip_utf8_bom() {
        assert_eq!(strip_utf8_bom("\u{FEFF}hello"), "hello");
        assert_eq!(strip_utf8_bom("hello"), "hello");
        assert_eq!(strip_utf8_bom(""), "");
        assert_eq!(strip_utf8_bom("\u{FEFF}"), "");
    }

    #[test]
    fn test_cf6_migrate_card_files() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("alice");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("card.png"), b"\x89PNG fake").unwrap();
        fs::write(char_dir.join("card.json"), "{}").unwrap();

        migrate_legacy_char_dirs(root).unwrap();

        assert!(char_dir.join("card").join("card.png").exists());
        assert!(char_dir.join("card").join("card.json").exists());
        assert!(!char_dir.join("card.png").exists());
        assert!(!char_dir.join("card.json").exists());
        assert!(char_dir.join("migration_done.lock").exists());
    }

    #[test]
    fn test_cf6_lock_prevents_repeat() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("bob");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("migration_done.lock"), "prev").unwrap();
        fs::write(char_dir.join("card.png"), b"data").unwrap();

        migrate_legacy_char_dirs(root).unwrap();

        assert!(char_dir.join("card.png").exists());
        assert!(!char_dir.join("card").join("card.png").exists());
    }

    #[test]
    fn test_cf6_triggers_gating_and_session_migration() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let char_dir = root.join("characters").join("carol");
        fs::create_dir_all(&char_dir).unwrap();
        fs::write(char_dir.join("checkpoints.md"), "# 旧 CP\n").unwrap();
        let legacy_session = char_dir.join("session");
        fs::create_dir_all(&legacy_session).unwrap();
        fs::write(legacy_session.join("current.md"), "old").unwrap();

        migrate_legacy_char_dirs(root).unwrap();

        assert!(char_dir.join("gating").join("checkpoints.md").exists());
        assert!(!char_dir.join("checkpoints.md").exists());
        assert!(char_dir.join("memory").join("current.md").exists());
    }

    #[test]
    fn test_cf6_handles_empty_chars_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("characters")).unwrap();
        migrate_legacy_char_dirs(root).unwrap();
    }

    #[test]
    fn test_cf6_handles_missing_chars_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        migrate_legacy_char_dirs(root).unwrap();
    }

    #[test]
    fn test_pr1_preset_dir_creates() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = preset_dir(root, "test_preset").unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with("test_preset"));
        assert!(dir.parent().unwrap().ends_with("presets"));
    }

    #[test]
    fn test_pr1_preset_json_path() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let p = preset_json_path(root, "test_preset");
        assert!(p.ends_with("preset.json"));
        assert!(!p.exists());
    }

    #[test]
    fn test_pr1_preset_regex_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let dir = preset_regex_dir(root, "test_preset").unwrap();
        assert!(dir.ends_with("regex"));
        assert!(dir.exists());
    }

    #[test]
    fn test_pr2_migrate_flat_json_to_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let presets = root.join("presets");
        fs::create_dir_all(&presets).unwrap();
        let flat = presets.join("test_preset.json");
        fs::write(&flat, r#"{"name":"test_preset"}"#).unwrap();

        migrate_legacy_presets(root).unwrap();

        let new_path = presets.join("test_preset").join("preset.json");
        assert!(
            new_path.exists(),
            "test_preset.json 应迁移到 test_preset/preset.json"
        );
        assert!(!flat.exists(), "旧扁平 test_preset.json 应被移走");
        let content = fs::read_to_string(&new_path).unwrap();
        assert_eq!(content, r#"{"name":"test_preset"}"#);
    }

    #[test]
    fn test_pr2_migrate_flat_md_to_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let presets = root.join("presets");
        fs::create_dir_all(&presets).unwrap();
        fs::write(presets.join("test_preset.md"), "# test_preset MD").unwrap();

        migrate_legacy_presets(root).unwrap();

        let new_path = presets.join("test_preset").join("preset.md");
        assert!(
            new_path.exists(),
            "test_preset.md 应迁移到 test_preset/preset.md"
        );
    }

    #[test]
    fn test_pr2_migrate_handles_both_json_and_md() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let presets = root.join("presets");
        fs::create_dir_all(&presets).unwrap();
        fs::write(presets.join("Izumi.json"), r#"{"name":"Izumi"}"#).unwrap();
        fs::write(presets.join("Izumi.md"), "# Izumi").unwrap();

        migrate_legacy_presets(root).unwrap();

        let new_dir = presets.join("Izumi");
        assert!(new_dir.join("preset.json").exists());
        assert!(new_dir.join("preset.md").exists());
    }

    #[test]
    fn test_pr2_idempotent_no_overwrite() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let presets = root.join("presets");
        let new_dir = presets.join("test_preset");
        fs::create_dir_all(&new_dir).unwrap();
        fs::write(new_dir.join("preset.json"), "USER_EDITED").unwrap();
        fs::write(presets.join("test_preset.json"), "STALE").unwrap();

        migrate_legacy_presets(root).unwrap();

        let content = fs::read_to_string(new_dir.join("preset.json")).unwrap();
        assert_eq!(content, "USER_EDITED");
        assert!(presets.join("test_preset.json").exists());
    }

    #[test]
    fn test_pr2_handles_empty_presets_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("presets")).unwrap();
        migrate_legacy_presets(root).unwrap();
    }

    #[test]
    fn test_pr2_handles_missing_presets_dir() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        migrate_legacy_presets(root).unwrap();
    }

    #[test]
    fn test_list_presets_dir_form() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let presets = root.join("presets");
        let d1 = presets.join("dir_preset");
        fs::create_dir_all(&d1).unwrap();
        fs::write(d1.join("preset.json"), "{}").unwrap();
        let d2 = presets.join("md_only");
        fs::create_dir_all(&d2).unwrap();
        fs::write(d2.join("preset.md"), "# md").unwrap();
        fs::create_dir_all(presets.join("empty_dir")).unwrap();
        fs::write(presets.join("flat.json"), "{}").unwrap();

        let list = list_presets(root).unwrap();
        assert!(list.contains(&"dir_preset".to_string()));
        assert!(list.contains(&"md_only".to_string()));
        assert!(list.contains(&"flat".to_string()));
        assert!(!list.contains(&"empty_dir".to_string()), "空目录不应被列出");
    }

    #[test]
    fn parsed_character_card_contract_rejects_invalid_and_non_object_json() {
        let tmp = tempdir().unwrap();
        let character = crate::types::CharacterId::new("contract").unwrap();
        let card_dir = tmp.path().join("characters").join("contract").join("card");
        fs::create_dir_all(&card_dir).unwrap();

        fs::write(card_dir.join("card.json"), "not-json").unwrap();
        assert!(matches!(
            get_character_card(tmp.path(), &character),
            Err(crate::error::AirpError::BadRequest(_))
        ));

        fs::write(card_dir.join("card.json"), "[]").unwrap();
        assert!(matches!(
            get_character_card(tmp.path(), &character),
            Err(crate::error::AirpError::BadRequest(_))
        ));

        fs::write(card_dir.join("card.json"), r#"{"name":"Contract"}"#).unwrap();
        assert_eq!(
            get_character_card(tmp.path(), &character).unwrap()["name"],
            "Contract"
        );
    }
}
