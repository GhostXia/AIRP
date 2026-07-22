//! #252 §2.B3：HTTP `/v1/chat/swipe` 端点端到端测试。
//!
//! 验证 `POST /v1/chat/swipe` 在 axum router 上的完整契约：
//!   1. 切换到候选 0 / 1 → 返回 SwipeResponse 增量响应（#252 D3）
//!   2. 越界 index → 400 BadRequest
//!   3. 不存在的 message_id → 400 BadRequest
//!   4. 无候选的消息 → 400 BadRequest
//!
//! 与 `chat_pipeline::tests::tests_b1_finalize_empty_stripped` 互补：
//! 那边测 finalize 层的候选回灌逻辑，这边测 HTTP 层的 switch_swipe 契约。

use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use std::net::SocketAddr;
use tower::ServiceExt;

use airp_core::adapter::{BackendEngine, ChatMessage, MessageRole, Provider};
use airp_core::config::VolumeConfig;
use airp_core::daemon::{create_router, DaemonState, MutableConfig};
use airp_core::domain::ChatService;
use airp_core::quota::QuotaConfig;
use airp_core::types::CharacterId;

/// 构造一份最小可用的 `DaemonState`，data_root 指向临时目录。
async fn setup() -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().to_path_buf();
    std::fs::create_dir_all(data_root.join("characters")).unwrap();
    std::fs::create_dir_all(data_root.join("presets")).unwrap();
    std::fs::create_dir_all(data_root.join("sessions")).unwrap();

    let state = Arc::new(DaemonState {
        data_root: data_root.clone(),
        http_client: reqwest::Client::new(),
        fts: Default::default(),
        settings_update: Default::default(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint: "https://example.test/v1/chat/completions".to_string(),
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: QuotaConfig::default(),
            deployment_mode: Default::default(),
            public_origin: None,
        }),
    });
    (state, tmp)
}

/// 构造一个带 ConnectInfo 的 POST 请求（rate-limiter 需要 IP key）。
fn build_post_request(uri: &str, json_body: &str) -> Request<Body> {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(json_body.to_owned()))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9999u16))));
    req
}

/// 准备一个角色 + 1 条 user 消息 + 1 条带 2 个候选的 assistant 消息。
/// 返回 (data_root, character_id, assistant_message_id)。
fn setup_character_with_candidates(data_root: &std::path::Path) -> (CharacterId, String) {
    let character = CharacterId::new("swipe-test-char").unwrap();
    let service = ChatService::new(data_root);
    // 写入 user 消息
    service
        .append(
            &character,
            None,
            ChatMessage {
                role: MessageRole::User,
                content: "hello".into(),
            },
        )
        .unwrap();
    // 写入带 2 个候选的 assistant 消息
    let log = service
        .append_with_candidates(
            &character,
            None,
            vec!["reply-a".to_string(), "reply-b".to_string()],
        )
        .unwrap();
    let assistant_id = log.message_ids[1].clone();
    (character, assistant_id)
}

#[tokio::test]
async fn swipe_switches_to_index_zero() {
    let (state, _tmp) = setup().await;
    let (character, message_id) = setup_character_with_candidates(&state.data_root);

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": message_id,
        "index": 0,
    });
    let req = build_post_request("/v1/chat/swipe", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["message_id"], message_id);
    assert_eq!(json["index"], 0);
    assert_eq!(json["content"], "reply-a");
    assert_eq!(json["role"], "assistant");
    assert_eq!(json["candidates_count"], 2);
}

#[tokio::test]
async fn swipe_switches_to_index_one() {
    let (state, _tmp) = setup().await;
    let (character, message_id) = setup_character_with_candidates(&state.data_root);

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": message_id,
        "index": 1,
    });
    let req = build_post_request("/v1/chat/swipe", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(json["index"], 1);
    assert_eq!(json["content"], "reply-b");
    assert_eq!(json["candidates_count"], 2);
}

#[tokio::test]
async fn swipe_out_of_range_index_returns_400() {
    let (state, _tmp) = setup().await;
    let (character, message_id) = setup_character_with_candidates(&state.data_root);

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": message_id,
        "index": 99,
    });
    let req = build_post_request("/v1/chat/swipe", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn swipe_invalid_message_id_returns_400() {
    let (state, _tmp) = setup().await;
    let (character, _message_id) = setup_character_with_candidates(&state.data_root);

    let router = create_router(state);
    // 合法 ULID 格式但实际不存在
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "index": 0,
    });
    let req = build_post_request("/v1/chat/swipe", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn swipe_malformed_message_id_returns_400() {
    let (state, _tmp) = setup().await;
    let (character, _message_id) = setup_character_with_candidates(&state.data_root);

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": "not-a-valid-ulid",
        "index": 0,
    });
    let req = build_post_request("/v1/chat/swipe", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn swipe_message_without_candidates_returns_400() {
    let (state, _tmp) = setup().await;
    // 只写 user 消息，不写带候选的 assistant 消息
    let character = CharacterId::new("no-candidates-char").unwrap();
    let service = ChatService::new(&state.data_root);
    service
        .append(
            &character,
            None,
            ChatMessage {
                role: MessageRole::User,
                content: "hello".into(),
            },
        )
        .unwrap();
    // 再写一条无候选的 assistant 消息
    let (log, _) = service
        .append(
            &character,
            None,
            ChatMessage {
                role: MessageRole::Assistant,
                content: "single reply".into(),
            },
        )
        .unwrap();
    let assistant_id = log.message_ids[1].clone();

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": assistant_id,
        "index": 0,
    });
    let req = build_post_request("/v1/chat/swipe", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
