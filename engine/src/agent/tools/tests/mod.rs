// Tests for `agent::tools` — declared as `#[cfg(test)] mod tests;` in `tools.rs`.
//
// 本文件是 `tools::tests` 子模块的 hub：
// - `use super::*` 从 facade（`tools`）拉入全部 public + private 项
//   （`default_registry` / `ToolRegistry` / `EchoTool` / PR 3 tool struct /
//   `read_lorebook_or_empty` / `enhance_md_via_llm_shared` 等）。
// - `make_state` fixture 为 `pub(super)`，对 `registry` / `session` /
//   `character` 三个测试子模块可见，绝不外泄到 production。
// - `MAX_RECENT_CONTEXT` 从 `tools::session`（production）re-import，
//   供 `tests::session` 的边界断言使用。
// - PR 3 范围（state / lorebook / volume / context / analysis）的 8 个测试
//   留在本文件内联，因为这些 tool struct 仍在 facade；PR 3 会一并迁出。

use super::*;
use crate::adapter::{BackendEngine, Provider};
use crate::config::VolumeConfig;
use crate::daemon::MutableConfig;
use crate::domain::{LorebookService, StateService};
use crate::types::CharacterId;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;

// 从 production `tools::session` re-import，供 `tests::session` 边界断言。
use super::session::MAX_RECENT_CONTEXT;

mod character;
mod registry;
mod session;

/// 最小可运行 DaemonState，data_root 指向临时目录（照 chat_pipeline/tests 模板）。
pub(super) fn make_state(data_root: PathBuf) -> Arc<DaemonState> {
    Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint: "https://example.test/v1/chat/completions".to_string(),
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: crate::quota::QuotaConfig::default(),
            deployment_mode: Default::default(),
            public_origin: None,
        }),
    })
}

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

#[tokio::test]
async fn export_context_bundle_output_directs_isolated_subagent() {
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let card_dir = state.data_root.join("characters/alice/card");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(
        card_dir.join("card.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "spec": "chara_card_v2",
            "spec_version": "2.0",
            "data": {
                "name": "Alice",
                "description": "A test character",
                "personality": "Curious",
                "scenario": "An observatory",
                "extensions": {"depth_prompt": "raw extension"}
            }
        }))
        .unwrap(),
    )
    .unwrap();
    StateService::new(&state.data_root)
        .write(
            &CharacterId::new("alice").unwrap(),
            &serde_json::json!({"hp": 9}),
        )
        .unwrap();
    LorebookService::new(&state.data_root)
        .write(
            &CharacterId::new("alice").unwrap(),
            &crate::orchestrator::Lorebook {
                entries: vec![crate::orchestrator::LorebookEntry {
                    keys: vec!["observatory".to_string()],
                    content: "Stable world fact".to_string(),
                    enabled: Some(true),
                    priority: Some(10),
                    constant: None,
                    comment: None,
                    secondary_keys: Vec::new(),
                    case_sensitive: None,
                    extensions: None,
                }],
            },
        )
        .unwrap();
    let preset_dir = state.data_root.join("presets/story");
    std::fs::create_dir_all(&preset_dir).unwrap();
    std::fs::write(preset_dir.join("preset.json"), r#"{"prompts":[]}"#).unwrap();

    let result = default_registry(state.clone())
        .get("export_context_bundle")
        .unwrap()
        .call(
            serde_json::json!({
                "character_id": "alice",
                "preset_id": "story",
                "include_lorebook": true,
                "thinking_mode_text": "Stay immersed"
            }),
            false,
        )
        .await
        .unwrap();
    assert!(!result.dry_run);
    let bundle = state.data_root.join("exports/context-bundles/alice");
    let context = std::fs::read_to_string(bundle.join("context.md")).unwrap();
    assert!(context.contains("ISOLATED subagent"));
    assert!(context.contains("fresh context"));
    assert!(context.contains("Stable world fact"));
    assert!(context.contains("\"hp\": 9"));
    assert!(
        context.find("Stable character context").unwrap()
            < context.find("Current state (volatile").unwrap()
    );
    assert!(bundle.join("preset_raw.json").exists());
    assert!(bundle.join("extensions.json").exists());
}

#[tokio::test]
async fn seal_volume_dry_run_then_confirm() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let archive = "<卷索引>\n- 卷标题: Test\n</卷索引>\n<卷内容>\nArchived scene\n</卷内容>\n<全局index更新>\n</全局index更新>";
    let event = serde_json::json!({"choices": [{"delta": {"content": archive}}]});
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(format!("data: {event}\n\ndata: [DONE]\n\n")),
        )
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    state.config.write().unwrap().endpoint = format!("{}/v1/chat/completions", server.uri());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let card_dir = state.data_root.join("characters/alice/card");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(card_dir.join("card.json"), r#"{"name":"Alice"}"#).unwrap();
    let memory = crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
    crate::volume_store::append_to_current(&memory, "A scene to archive").unwrap();
    let reg = default_registry(state);
    let tool = reg.get("seal_volume").unwrap();

    let preview = tool
        .call(serde_json::json!({"character_id": "alice"}), false)
        .await
        .unwrap();
    assert!(preview.dry_run);
    assert_eq!(preview.output["requires"], "confirm=true");
    assert!(crate::volume_store::list_volume_numbers(&memory).is_empty());

    let sealed = tool
        .call(serde_json::json!({"character_id": "alice"}), true)
        .await
        .unwrap();
    assert!(!sealed.dry_run);
    assert_eq!(sealed.output["volume"], 1);
    assert_eq!(crate::volume_store::list_volume_numbers(&memory), vec![1]);
    assert!(crate::volume_store::read_current(&memory)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn enhance_analysis_returns_preview_and_rejects_world_book() {
    // L2 修复（issue #92）：用 wiremock mock LLM upstream。
    // L3：enhance 真正调 LLM，测试需 mock，否则烧 token + DNS 失败。
    // A1：enhance 只读返回 diff 预览，不写盘
    // A2：world_book/ 前缀拒绝
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let mock_server = MockServer::start().await;
    let enhanced_content = "# Basic Info\n\nName: Alice\nDescription: A brave knight\n";
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":{}}}}}]}}\n\ndata: [DONE]\n\n",
            serde_json::to_string(enhanced_content).unwrap()
        )))
        .mount(&mock_server)
        .await;

    let tmp = tempdir().unwrap();
    let state = Arc::new(DaemonState {
        data_root: tmp.path().to_path_buf(),
        http_client: reqwest::Client::new(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint: format!("{}/v1/chat/completions", mock_server.uri()),
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: crate::quota::QuotaConfig::default(),
            deployment_mode: Default::default(),
            public_origin: None,
        }),
    });
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    // 写一个 fixture analysis MD 文件
    let analysis_dir = state
        .data_root
        .join("characters")
        .join("alice")
        .join("analysis");
    std::fs::create_dir_all(&analysis_dir).unwrap();
    let original = "# Basic Info\n\nName: Alice\n";
    std::fs::write(analysis_dir.join("basic_info.md"), original).unwrap();

    let enhance = reg.get("enhance_analysis").unwrap();
    let r = enhance
        .call(
            serde_json::json!({"character_id": "alice", "filename": "basic_info.md"}),
            false,
        )
        .await
        .unwrap();
    assert!(!r.dry_run, "enhance is readonly, never dry-run");
    assert_eq!(r.output["filename"], "basic_info.md");
    assert_eq!(r.output["original_md"], original);
    // L3：enhanced_md 来自 LLM mock，has_changes=true
    // 注意：enhance 会 trim LLM 输出，故比较时用 trim
    assert_eq!(
        r.output["enhanced_md"].as_str().unwrap().trim(),
        enhanced_content.trim()
    );
    assert_eq!(r.output["has_changes"], true);

    // enhance 不写盘（A1：readonly）
    assert_eq!(
        std::fs::read_to_string(analysis_dir.join("basic_info.md")).unwrap(),
        original,
        "enhance is readonly — must not write to disk"
    );

    // A2: world_book/ 前缀拒绝
    let err = enhance
        .call(
            serde_json::json!({"character_id": "alice", "filename": "world_book/entry_001.md"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));

    // 不存在文件 → NotFound
    let err = enhance
        .call(
            serde_json::json!({"character_id": "alice", "filename": "ghost.md"}),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::NotFound(_)));
}

#[tokio::test]
async fn apply_enhanced_analysis_dry_run_then_confirm() {
    // A1：apply 是 destructive → dry-run 默认，confirm=true 才写盘
    // A2：world_book/ 前缀拒绝
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let reg = default_registry(state.clone());

    let analysis_dir = state
        .data_root
        .join("characters")
        .join("alice")
        .join("analysis");
    std::fs::create_dir_all(&analysis_dir).unwrap();
    std::fs::write(analysis_dir.join("personality.md"), "old content").unwrap();

    let apply = reg.get("apply_enhanced_analysis").unwrap();
    let enhanced = "# Personality\n\nBrave and curious\n";

    // dry-run → 不写盘
    let r = apply
        .call(
            serde_json::json!({
                "character_id": "alice",
                "filename": "personality.md",
                "enhanced_md": enhanced,
            }),
            false,
        )
        .await
        .unwrap();
    assert!(r.dry_run);
    assert_eq!(r.output["action"], "apply_enhanced_analysis");
    assert_eq!(r.output["requires"], "confirm=true");
    assert_eq!(
        std::fs::read_to_string(analysis_dir.join("personality.md")).unwrap(),
        "old content",
        "dry-run must not write to disk"
    );

    // confirm=true → 写盘
    let r = apply
        .call(
            serde_json::json!({
                "character_id": "alice",
                "filename": "personality.md",
                "enhanced_md": enhanced,
            }),
            true,
        )
        .await
        .unwrap();
    assert!(!r.dry_run);
    assert_eq!(r.output["status"], "applied");
    assert_eq!(
        std::fs::read_to_string(analysis_dir.join("personality.md")).unwrap(),
        enhanced,
        "confirm=true must write enhanced_md to disk"
    );

    // A2: world_book/ 前缀拒绝
    let err = apply
        .call(
            serde_json::json!({
                "character_id": "alice",
                "filename": "world_book/entry_001.md",
                "enhanced_md": "evil",
            }),
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AirpError::BadRequest(_)));
}
