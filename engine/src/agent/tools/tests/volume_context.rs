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
    let character = CharacterId::new("alice").unwrap();
    let state_service = StateService::new(&state.data_root);
    state_service
        .write(&character, &serde_json::json!({"hp": 4}))
        .unwrap();
    let live_path = crate::data_dir::char_state_dir(&state.data_root, "alice").join("live.json");
    let stale_live = std::fs::read(&live_path).unwrap();
    state_service
        .write(&character, &serde_json::json!({"hp": 9}))
        .unwrap();
    std::fs::write(&live_path, stale_live).unwrap();
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

/// #442 / #438 W-04：`run_seal_flow` 持 `character_lock.read()` 期间，
/// `delete_character`（需 `character_lock.write()`）必须串行化，不得读到
/// character 目录的半删状态。
///
/// 测试策略与其他 R1 回归一致：两个独立 OS worker 经 `Barrier` 同时放行，
/// worker A 运行真实 `run_seal_flow`（wiremock 提供 OpenAI-compatible SSE），
/// worker B 调用 `delete_character`，外层用 30s timeout 检测残留死锁。R1 只保证
/// 串行化而不保证顺序，因此 `Ok` / `NotFound` / `Io(NotFound)` 都是合法结果；
/// Windows 在删除后访问 pending-deletion 路径时可能返回 `Io(PermissionDenied)`，
/// 这是已知 OS quirk。`Internal` 表示读到了半删状态，必须升级为测试失败。
#[tokio::test(flavor = "current_thread")]
async fn run_seal_flow_and_delete_character_serialized_by_character_lock() {
    use std::sync::Arc;
    use std::time::Duration;
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

    let character_id = "seal_r1_delete";
    let tmp = tempdir().unwrap();
    let state = make_state(tmp.path().to_path_buf());
    crate::data_dir::ensure_data_dirs(&state.data_root).unwrap();
    let card_dir = state
        .data_root
        .join("characters")
        .join(character_id)
        .join("card");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(card_dir.join("card.json"), r#"{"name":"Seal R1"}"#).unwrap();
    let memory =
        crate::data_dir::resolve_session_dir(&state.data_root, character_id, None).unwrap();
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
    let _ = crate::domain::take_test_character_lock_events(character_id);
    let (read_acquired, write_attempted, gate) =
        crate::domain::install_test_character_lock_gate(character_id);
    let (start_delete_tx, start_delete_rx) = tokio::sync::oneshot::channel();
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let worker_state = state.clone();
    let memory_for_seal = memory.clone();

    let join_handle = tokio::task::spawn_blocking(move || {
        std::thread::scope(|scope| {
            // Worker A: run_seal_flow（写盘段持 character_lock.read()）。
            let handle_a = {
                let state = worker_state.clone();
                let barrier = barrier.clone();
                let memory = memory_for_seal.clone();
                let provider = provider.clone();
                scope.spawn(move || -> Result<(), String> {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .map_err(|error| format!("runtime build: {error}"))?;
                    barrier.wait();
                    let result = runtime.block_on(crate::volume_manager::run_seal_flow(
                        &state.http_client,
                        &memory,
                        Some(character_id),
                        None,
                        provider,
                        params,
                    ));

                    // 合法串行化结果：
                    // - Ok：run_seal_flow 先完成写盘，delete_character 随后删除目录；
                    // - NotFound / Io(NotFound)：delete_character 先完成，seal 看到已删目录；
                    // - Windows Io(PermissionDenied)：pending-deletion 文件句柄的 OS quirk。
                    // Internal 则表示读到半删数据（R1 TOCTOU 防护失效）。
                    match result {
                        Ok(_) => Ok(()),
                        Err(crate::error::AirpError::NotFound(_)) => Ok(()),
                        Err(crate::error::AirpError::Io(error))
                            if error.kind() == std::io::ErrorKind::NotFound
                                || (cfg!(windows)
                                    && error.kind() == std::io::ErrorKind::PermissionDenied) =>
                        {
                            Ok(())
                        }
                        Err(error) => Err(format!(
                            "run_seal_flow failed with non-serialized error \
                             (R1 TOCTOU protection may have failed): {error:?}"
                        )),
                    }
                })
            };

            // Worker B: delete_character（需 character_lock.write()）。
            let handle_b = {
                let state = worker_state.clone();
                let barrier = barrier.clone();
                scope.spawn(move || -> Result<bool, String> {
                    let chat = crate::domain::ChatService::new(&state.data_root);
                    let cid = crate::types::CharacterId::new(character_id)
                        .map_err(|error| format!("character id: {error}"))?;
                    barrier.wait();
                    start_delete_rx
                        .blocking_recv()
                        .map_err(|error| format!("start delete: {error}"))?;
                    let result = chat.delete_character(&cid, true);
                    Ok(result.is_ok())
                })
            };

            handle_a
                .join()
                .map_err(|error| format!("run_seal_flow worker join: {error:?}"))??;
            let _delete_succeeded = handle_b
                .join()
                .map_err(|error| format!("delete_character worker join: {error:?}"))??;
            Ok::<(), String>(())
        })
    });

    tokio::time::timeout(Duration::from_secs(30), read_acquired)
        .await
        .expect("run_seal_flow did not acquire character read lock")
        .expect("run_seal_flow read-acquired notification dropped");
    start_delete_tx
        .send(())
        .expect("delete worker start notification dropped");
    tokio::time::timeout(Duration::from_secs(30), write_attempted)
        .await
        .expect("delete_character did not attempt character write lock")
        .expect("delete_character write-attempt notification dropped");
    // Keep the read gate alive until the writer has definitely attempted its
    // acquire. Dropping it now releases both operations in a known order.
    drop(gate);

    match tokio::time::timeout_at(deadline, join_handle).await {
        Ok(Ok(Ok(()))) => {}
        Ok(Ok(Err(error))) => panic!("R1 worker error: {error}"),
        Ok(Err(error)) => panic!("R1 spawn_blocking join error: {error:?}"),
        Err(_) => panic!(
            "run_seal_flow + delete_character deadlocked: workers exceeded 30 seconds \
             (R1 character_lock serialization failed — see #442 / #438)"
        ),
    }

    let events = crate::domain::take_test_character_lock_events(character_id);
    assert_eq!(
        events.len(),
        4,
        "unexpected R1 lock event timeline: {events:?}"
    );
    assert_eq!(events[0].kind, crate::domain::TestCharacterLockKind::Read);
    assert_eq!(
        events[0].phase,
        crate::domain::TestCharacterLockPhase::Acquired
    );
    assert_eq!(events[1].kind, crate::domain::TestCharacterLockKind::Read);
    assert_eq!(
        events[1].phase,
        crate::domain::TestCharacterLockPhase::Released
    );
    assert_eq!(events[2].kind, crate::domain::TestCharacterLockKind::Write);
    assert_eq!(
        events[2].phase,
        crate::domain::TestCharacterLockPhase::Acquired
    );
    assert_eq!(events[3].kind, crate::domain::TestCharacterLockKind::Write);
    assert_eq!(
        events[3].phase,
        crate::domain::TestCharacterLockPhase::Released
    );
}
