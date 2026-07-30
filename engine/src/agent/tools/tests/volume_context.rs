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

/// #283 回归测试：seal_volume 在 LLM streaming 期间若有并发 append（模拟
/// npc_action），baseline 校验必须返回 Conflict，且 current.md 保留全部内容
/// （不执行 clear_current），不产生孤儿卷。
///
/// 时序（确定性）：run_seal_flow 记录 baseline → 进入 LLM streaming（mock
/// 延迟 300ms）→ 主 task 轮询 `server.received_requests()` 直到确认 mock
/// 收到 chat completion 请求（说明已进入 streaming）→ append_to_current
/// 模拟 npc_action → LLM 返回后 baseline 校验发现 current.md 已变 → Conflict。
#[tokio::test]
async fn seal_volume_returns_conflict_on_concurrent_modification() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
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
                .set_body_string(format!("data: {event}\n\ndata: [DONE]\n\n"))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let card_dir = state.data_root.join("characters/alice/card");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(card_dir.join("card.json"), r#"{"name":"Alice"}"#).unwrap();
    let memory = crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
    crate::volume_store::append_to_current(&memory, "A scene to archive").unwrap();

    let provider = Arc::new(crate::adapter::ProviderConfig {
        provider: crate::adapter::Provider::OpenAI,
        endpoint: format!("{}/v1/chat/completions", server.uri()),
        api_key: Some("test-key".to_string()),
    });
    let params = crate::adapter::GenerationParams {
        model: "test-model".to_string(),
        temperature: Some(0.7),
        max_tokens: None,
    };
    let client = state.http_client.clone();
    let memory_for_seal = memory.clone();

    let seal_handle = tokio::spawn(async move {
        crate::volume_manager::run_seal_flow(
            &client,
            &memory_for_seal,
            Some("alice"),
            None,
            provider,
            params,
        )
        .await
    });

    // 确定性等待：轮询 mock server 直到它收到 chat completion 请求，
    // 说明 run_seal_flow 已记录 baseline 并进入 LLM streaming（mock 还在
    // 300ms 延迟内）。比固定 sleep 更稳定，避免 CI 上调度抖动导致的 flaky。
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let count = server
            .received_requests()
            .await
            .map(|v| v.len())
            .unwrap_or(0);
        if count >= 1 {
            break;
        }
        if Instant::now() > deadline {
            panic!("mock server did not receive chat completion within 2s");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // 模拟 npc_action 在 LLM streaming 期间并发 append。
    crate::volume_store::append_to_current(&memory, "\n[NPC行动: 盗贼] 潜行\n").unwrap();

    let result = seal_handle.await.unwrap();
    assert!(
        matches!(result, Err(crate::error::AirpError::Conflict(_))),
        "expected Conflict on concurrent modification, got {:?}",
        result
    );

    // current.md 必须保留原始内容 + NPC append（clear_current 未执行）。
    let remaining = crate::volume_store::read_current(&memory).unwrap();
    assert!(
        remaining.contains("A scene to archive"),
        "original content lost: {remaining}"
    );
    assert!(
        remaining.contains("[NPC行动: 盗贼]"),
        "NPC append lost: {remaining}"
    );
    assert!(
        crate::volume_store::list_volume_numbers(&memory).is_empty(),
        "no volume should be written on Conflict"
    );
}

/// #283 补充回归测试：seal_volume 在 LLM streaming 期间若有并发 index.md
/// 改写（模拟 run_maintenance 在另一轮 finalize 触发），baseline 校验必须
/// 返回 Conflict，且 index.md 保留新内容（不被 seal 的 new_index 覆盖），
/// 不产生孤儿卷。
///
/// 覆盖 CodeRabbit 指出的 gap：原 baseline 只校验 current.md，index.md
/// 的 read-modify-write 仍可被并发 seal 静默覆盖。修复后 baseline 同时
/// 校验 index.md。
#[tokio::test]
async fn seal_volume_returns_conflict_on_concurrent_index_modification() {
    use std::sync::Arc;
    use std::time::{Duration, Instant};
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
                .set_body_string(format!("data: {event}\n\ndata: [DONE]\n\n"))
                .set_delay(Duration::from_millis(300)),
        )
        .mount(&server)
        .await;

    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let card_dir = state.data_root.join("characters/alice/card");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(card_dir.join("card.json"), r#"{"name":"Alice"}"#).unwrap();
    let memory = crate::data_dir::resolve_session_dir(&state.data_root, "alice", None).unwrap();
    crate::volume_store::append_to_current(&memory, "A scene to archive").unwrap();
    // 建立初始 index.md baseline（非空，便于检测并发改写）。
    crate::volume_store::write_index(&memory, "- 初始卷索引条目\n").unwrap();

    let provider = Arc::new(crate::adapter::ProviderConfig {
        provider: crate::adapter::Provider::OpenAI,
        endpoint: format!("{}/v1/chat/completions", server.uri()),
        api_key: Some("test-key".to_string()),
    });
    let params = crate::adapter::GenerationParams {
        model: "test-model".to_string(),
        temperature: Some(0.7),
        max_tokens: None,
    };
    let client = state.http_client.clone();
    let memory_for_seal = memory.clone();

    let seal_handle = tokio::spawn(async move {
        crate::volume_manager::run_seal_flow(
            &client,
            &memory_for_seal,
            Some("alice"),
            None,
            provider,
            params,
        )
        .await
    });

    // 确定性等待 mock 收到请求（已进入 streaming）。
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let count = server
            .received_requests()
            .await
            .map(|v| v.len())
            .unwrap_or(0);
        if count >= 1 {
            break;
        }
        if Instant::now() > deadline {
            panic!("mock server did not receive chat completion within 2s");
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    // 模拟 run_maintenance 在 LLM streaming 期间并发改写 index.md
    // （跨卷实体晋升等）。
    crate::volume_store::write_index(&memory, "- 初始卷索引条目\n- 并发维护新增条目\n").unwrap();

    let result = seal_handle.await.unwrap();
    assert!(
        matches!(result, Err(crate::error::AirpError::Conflict(_))),
        "expected Conflict on concurrent index.md modification, got {:?}",
        result
    );

    // index.md 必须保留维护写入的新内容（seal 的 write_index 未执行）。
    let remaining_index = crate::volume_store::read_index(&memory).unwrap();
    assert!(
        remaining_index.contains("并发维护新增条目"),
        "concurrent maintenance index entry lost: {remaining_index}"
    );
    // current.md 未被并发改，保留原始内容。
    let remaining_current = crate::volume_store::read_current(&memory).unwrap();
    assert!(
        remaining_current.contains("A scene to archive"),
        "original current content lost: {remaining_current}"
    );
    assert!(
        crate::volume_store::list_volume_numbers(&memory).is_empty(),
        "no volume should be written on Conflict"
    );
}
