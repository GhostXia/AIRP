// Analysis enhance/apply family tests for `agent::tools`.
//
// 从 `tools/tests/mod.rs` 原 inline 测试原样迁移，不改断言逻辑。
// 测试通过 `default_registry` 端到端验证 enhance_analysis 的 readonly diff
// 预览 + world_book 拒绝，以及 apply_enhanced_analysis 的 dry-run→confirm 流程。

use super::*;
use std::sync::Arc;
use tempfile::tempdir;

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

#[tokio::test]
async fn analysis_validation_preserves_existing_error_precedence() {
    let tmp = tempdir().unwrap();
    let reg = default_registry(make_state(tmp.path().to_path_buf()));

    // Both tools historically check required field presence before validating
    // the character id value. Reusing required_character_id must not move that
    // value validation ahead of family-specific missing-field errors.
    let enhance_err = reg
        .get("enhance_analysis")
        .unwrap()
        .call(serde_json::json!({"character_id": ".bad"}), false)
        .await
        .unwrap_err();
    assert!(matches!(
        enhance_err,
        AirpError::BadRequest(ref message) if message == "missing filename"
    ));

    let apply_err = reg
        .get("apply_enhanced_analysis")
        .unwrap()
        .call(
            serde_json::json!({
                "character_id": ".bad",
                "filename": "personality.md",
            }),
            false,
        )
        .await
        .unwrap_err();
    assert!(matches!(
        apply_err,
        AirpError::BadRequest(ref message) if message == "missing enhanced_md"
    ));
}
