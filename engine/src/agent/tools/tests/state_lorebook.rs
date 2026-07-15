// State & lorebook family tests for `agent::tools`.
//
// 从 `tools/tests/mod.rs` 原 inline 测试原样迁移，不改断言逻辑。
// 测试通过 `default_registry` 端到端验证 6 个 state/lorebook 工具的
// roundtrip / SillyTavern 归一化 / 拒绝覆写 / apply+merge readonly 行为。

use super::*;
use crate::domain::LorebookService;
use crate::types::CharacterId;
use tempfile::tempdir;

#[tokio::test]
async fn state_and_lorebook_tools_roundtrip_with_confirmation() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    let update_state = reg.get("update_character_state").unwrap();
    let updated = update_state
        .call(
            serde_json::json!({"character_id": "alice", "state": {"hp": 90}}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(updated.output["revision"], 1);

    let get_state = reg.get("get_character_state").unwrap();
    let current = get_state
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    assert_eq!(current.output["hp"], 90);

    let lorebook = serde_json::json!({
        "entries": [{
            "keys": ["AIRP"],
            "content": "Open runtime",
            "enabled": true,
            "priority": 10,
            "comment": null
        }]
    });
    let update_lorebook = reg.get("update_lorebook").unwrap();
    let preview = update_lorebook
        .call(
            serde_json::json!({"character_id": "alice", "lorebook": lorebook.clone()}),
            false,
        )
        .await
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.output["requires"], "confirm=true");
    assert!(!crate::data_dir::char_world_lorebook_path(&state.data_root, "alice").exists());

    let written = update_lorebook
        .call(
            serde_json::json!({"character_id": "alice", "lorebook": lorebook}),
            true,
        )
        .await
        .unwrap();
    assert!(!written.dry_run);
    assert_eq!(written.output["entries"], 1);

    let get_lorebook = reg.get("get_lorebook").unwrap();
    let current = get_lorebook
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    assert_eq!(current.output["entries"][0]["content"], "Open runtime");
}

/// #126: Verify update_lorebook tool accepts SillyTavern form and normalizes
/// via the shared WorldbookNormalizer, preserving ST-only fields in extensions.
#[tokio::test]
async fn update_lorebook_normalizes_sillytavern_form() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    // ST character_book form: object-map entries with ST aliases
    let st_lorebook = serde_json::json!({
        "entries": {
            "0": {
                "keys": ["moon gate"],
                "keysecondary": ["night"],
                "content": "The moon gate opens at night.",
                "disable": false,
                "order": 10,
                "constant": false,
                "selective": true,
                "position": "before_char"
            },
            "1": {
                "keys": [],
                "content": "Constant world fact.",
                "disable": false,
                "order": 30,
                "constant": true
            }
        }
    });

    let update_lorebook = reg.get("update_lorebook").unwrap();

    // Preview (dry_run) should show import_report
    let preview = update_lorebook
        .call(
            serde_json::json!({
                "character_id": "alice",
                "lorebook": st_lorebook.clone()
            }),
            false,
        )
        .await
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.output["entries"], 2);
    assert_eq!(preview.output["import_report"]["total_input"], 2);
    assert_eq!(preview.output["import_report"]["converted"], 2);
    assert_eq!(preview.output["import_report"]["aliases_normalized"], 2);

    // Confirm write
    let written = update_lorebook
        .call(
            serde_json::json!({
                "character_id": "alice",
                "lorebook": st_lorebook
            }),
            true,
        )
        .await
        .unwrap();
    assert!(!written.dry_run);
    assert_eq!(written.output["entries"], 2);

    // Read back and verify canonical form
    let get_lorebook = reg.get("get_lorebook").unwrap();
    let current = get_lorebook
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();

    // After priority sort (descending): constant (30) > moon gate (10)
    let entries = current.output["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);

    // entries[0] = constant world fact (priority 30)
    assert_eq!(entries[0]["content"], "Constant world fact.");
    assert_eq!(entries[0]["enabled"], true); // disable=false → enabled=true
    assert_eq!(entries[0]["priority"], 30);
    assert_eq!(entries[0]["constant"], true);

    // entries[1] = moon gate (priority 10) — ST aliases normalized
    assert_eq!(entries[1]["content"], "The moon gate opens at night.");
    assert_eq!(entries[1]["enabled"], true);
    assert_eq!(entries[1]["priority"], 10);
    assert_eq!(entries[1]["constant"], false);
    assert_eq!(entries[1]["secondary_keys"], serde_json::json!(["night"]));

    // ST-only fields preserved in extensions
    let ext = &entries[1]["extensions"];
    assert_eq!(ext["selective"], true);
    assert_eq!(ext["position"], "before_char");
}

#[tokio::test]
async fn update_lorebook_rejects_all_invalid_entries_without_overwrite() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let character = CharacterId::new("alice").unwrap();
    let original = crate::orchestrator::Lorebook {
        entries: vec![crate::orchestrator::LorebookEntry {
            keys: vec!["safe".to_string()],
            content: "keep me".to_string(),
            enabled: Some(true),
            priority: Some(10),
            constant: None,
            comment: None,
            secondary_keys: Vec::new(),
            case_sensitive: None,
            extensions: None,
        }],
    };
    LorebookService::new(&state.data_root)
        .write(&character, &original)
        .unwrap();

    let registry = default_registry(state.clone());
    let tool = registry.get("update_lorebook").unwrap();
    let err = tool
        .call(
            serde_json::json!({
                "character_id": "alice",
                "lorebook": {"entries": [{"keys": ["bad"], "content": 42}]}
            }),
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));

    let current = LorebookService::new(&state.data_root)
        .read(&character)
        .unwrap();
    assert_eq!(current.entries.len(), 1);
    assert_eq!(current.entries[0].content, "keep me");
}

#[tokio::test]
async fn lorebook_apply_and_merge_are_readonly() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let service = LorebookService::new(&state.data_root);
    for (character, entries) in [
        (
            "alice",
            vec![crate::orchestrator::LorebookEntry {
                keys: vec!["moon".to_string()],
                content: "Moon knowledge".to_string(),
                enabled: Some(true),
                priority: Some(20),
                constant: None,
                comment: None,
                secondary_keys: Vec::new(),
                case_sensitive: None,
                extensions: None,
            }],
        ),
        (
            "bob",
            vec![
                crate::orchestrator::LorebookEntry {
                    keys: vec!["moon".to_string()],
                    content: "Moon knowledge".to_string(),
                    enabled: Some(true),
                    priority: Some(20),
                    constant: None,
                    comment: None,
                    secondary_keys: Vec::new(),
                    case_sensitive: None,
                    extensions: None,
                },
                crate::orchestrator::LorebookEntry {
                    keys: vec!["gate".to_string()],
                    content: "Gate knowledge".to_string(),
                    enabled: Some(true),
                    priority: Some(10),
                    constant: None,
                    comment: None,
                    secondary_keys: Vec::new(),
                    case_sensitive: None,
                    extensions: None,
                },
            ],
        ),
    ] {
        service
            .write(
                &CharacterId::new(character).unwrap(),
                &crate::orchestrator::Lorebook { entries },
            )
            .unwrap();
    }
    let reg = default_registry(state);

    let applied = reg
        .get("apply_lorebook")
        .unwrap()
        .call(
            serde_json::json!({"character_id": "alice", "text": "the moon rises"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(applied.output["matched"], true);
    assert!(applied.output["context"]
        .as_str()
        .unwrap()
        .contains("Moon knowledge"));

    let empty = reg
        .get("apply_lorebook")
        .unwrap()
        .call(
            serde_json::json!({"character_id": "charlie", "text": "moon"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(empty.output["matched"], false);

    let merged = reg
        .get("merge_lorebooks")
        .unwrap()
        .call(
            serde_json::json!({"character_ids": ["alice", "bob"], "strategy": "union"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(merged.output["entries"], 2);
    assert!(!merged.dry_run);

    let merged_with_missing = reg
        .get("merge_lorebooks")
        .unwrap()
        .call(
            serde_json::json!({"character_ids": ["alice", "charlie"], "strategy": "union"}),
            false,
        )
        .await
        .unwrap();
    assert_eq!(merged_with_missing.output["entries"], 1);
}
