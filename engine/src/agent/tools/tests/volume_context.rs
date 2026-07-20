// Volume & context bundle family tests for `agent::tools`.
//
// 从 `tools/tests/mod.rs` 原 inline 测试原样迁移，不改断言逻辑。
// 测试通过 `default_registry` 端到端验证 seal_volume 的 dry-run→confirm
// 流程与 export_context_bundle 的 isolated subagent 输出格式。

use super::*;
use crate::domain::{LorebookService, StateService};
use crate::types::CharacterId;
use tempfile::tempdir;

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
                    selective: false,
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

// #160 A3：无效 preset 不得修改既有 bundle。
// 顺序回归：原实现先清理 preset_raw.json/extensions.json，再验证 preset 路径。
// preset 缺失时返回 NotFound，但 sidecar 已被删除、context.md 仍是旧版本，
// 形成不一致快照。修复后 preset 验证前置，NotFound 时 bundle 目录不被触碰。
#[tokio::test]
async fn export_context_bundle_invalid_preset_does_not_modify_existing_bundle() {
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

    // 1) 先用合法 preset 导出一次，建立 baseline bundle。
    let preset_dir = state.data_root.join("presets/story");
    std::fs::create_dir_all(&preset_dir).unwrap();
    std::fs::write(preset_dir.join("preset.json"), r#"{"prompts":[]}"#).unwrap();
    let first = default_registry(state.clone())
        .get("export_context_bundle")
        .unwrap()
        .call(
            serde_json::json!({
                "character_id": "alice",
                "preset_id": "story",
            }),
            false,
        )
        .await
        .unwrap();
    assert!(!first.dry_run);

    let bundle = state.data_root.join("exports/context-bundles/alice");
    let baseline_context = std::fs::read_to_string(bundle.join("context.md")).unwrap();
    let baseline_preset_raw = std::fs::read_to_string(bundle.join("preset_raw.json")).unwrap();
    let baseline_extensions = std::fs::read_to_string(bundle.join("extensions.json")).unwrap();

    // 2) 用不存在 preset 再导出，必须返回 NotFound 且 bundle 文件保持 baseline。
    let second = default_registry(state.clone())
        .get("export_context_bundle")
        .unwrap()
        .call(
            serde_json::json!({
                "character_id": "alice",
                "preset_id": "does-not-exist",
            }),
            false,
        )
        .await;
    assert!(
        matches!(second, Err(ref e) if matches!(e, crate::error::AirpError::NotFound(_))),
        "expected NotFound for missing preset, got {:?}",
        second
    );

    // 3) 既有 bundle 必须未被修改：context.md / preset_raw.json / extensions.json
    //    三者都保持 baseline 字节。这是 #160 A3 回归保护的核心断言。
    assert_eq!(
        std::fs::read_to_string(bundle.join("context.md")).unwrap(),
        baseline_context,
        "context.md must not change when preset validation fails"
    );
    assert_eq!(
        std::fs::read_to_string(bundle.join("preset_raw.json")).unwrap(),
        baseline_preset_raw,
        "preset_raw.json must not be deleted when preset validation fails"
    );
    assert_eq!(
        std::fs::read_to_string(bundle.join("extensions.json")).unwrap(),
        baseline_extensions,
        "extensions.json must not be deleted when preset validation fails"
    );
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
