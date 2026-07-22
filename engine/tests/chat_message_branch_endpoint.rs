//! PR #270 F-4 / F-3：HTTP `/v1/chat/message` (PUT) + `/v1/chat/branch/switch` (POST)
//! 端到端测试。
//!
//! 验证 axum router 上的完整契约：
//!   1. PUT /v1/chat/message — 编辑 user 消息内容，返回 ChatLog
//!   2. PUT /v1/chat/message — 编辑 assistant 消息 → 400（只能编辑 user 消息）
//!   3. PUT /v1/chat/message — 不存在的 message_id → 400
//!   4. PUT /v1/chat/message — 非法 ULID 格式 → 400
//!   5. POST /v1/chat/branch/switch — 切换到目标叶节点 → 200 + ChatLog
//!   6. POST /v1/chat/branch/switch — 不存在的 target_leaf_id → 400
//!   7. PUT /v1/chat/message — body > 2MB → 413 Payload Too Large（#277）

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

/// 构造一个带 ConnectInfo 的请求（rate-limiter 需要 IP key）。
fn build_request(method: Method, uri: &str, json_body: &str) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(json_body.to_owned()))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9999u16))));
    req
}

/// 构造一个带 ConnectInfo 的请求，body 直接为字节切片（用于 413 测试）。
fn build_request_bytes(method: Method, uri: &str, body: Vec<u8>) -> Request<Body> {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9999u16))));
    req
}

/// 准备一个角色 + 1 条 user 消息 + 1 条 assistant 消息。
/// 返回 (character_id, user_message_id, assistant_message_id)。
fn setup_character_with_messages(data_root: &std::path::Path) -> (CharacterId, String, String) {
    let character = CharacterId::new("edit-test-char").unwrap();
    let service = ChatService::new(data_root);
    let (log, _) = service
        .append(
            &character,
            None,
            ChatMessage {
                role: MessageRole::User,
                content: "hello".into(),
            },
        )
        .unwrap();
    let user_id = log.message_ids[0].clone();
    let (log, _) = service
        .append(
            &character,
            None,
            ChatMessage {
                role: MessageRole::Assistant,
                content: "hi there".into(),
            },
        )
        .unwrap();
    let assistant_id = log.message_ids[1].clone();
    (character, user_id, assistant_id)
}

// ── #278: PUT /v1/chat/message 端到端测试 ────────────────────────────────────

#[tokio::test]
async fn edit_message_updates_content_and_returns_chat_log() {
    let (state, _tmp) = setup().await;
    let (character, user_id, _) = setup_character_with_messages(&state.data_root);

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": user_id,
        "content": "edited content",
    });
    let req = build_request(Method::PUT, "/v1/chat/message", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    // 返回 ChatLog，messages 数组第一条应是编辑后的 user 消息
    let messages = json["messages"].as_array().expect("messages array");
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0]["content"], "edited content");
    assert_eq!(messages[0]["role"], "user");
    // ChatMessage 无 id 字段；ID 在 ChatLog.message_ids 平行数组中
    let message_ids = json["message_ids"].as_array().expect("message_ids array");
    assert_eq!(message_ids[0], user_id);
}

#[tokio::test]
async fn edit_message_rejects_assistant_message() {
    let (state, _tmp) = setup().await;
    let (character, _, assistant_id) = setup_character_with_messages(&state.data_root);

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": assistant_id,
        "content": "should fail",
    });
    let req = build_request(Method::PUT, "/v1/chat/message", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    // assistant 消息不能编辑，应返回 400
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn edit_message_invalid_id_returns_400() {
    let (state, _tmp) = setup().await;
    let (character, _, _) = setup_character_with_messages(&state.data_root);

    let router = create_router(state);
    // 合法 ULID 格式但实际不存在
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
        "content": "edited",
    });
    let req = build_request(Method::PUT, "/v1/chat/message", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn edit_message_malformed_id_returns_400() {
    let (state, _tmp) = setup().await;
    let (character, _, _) = setup_character_with_messages(&state.data_root);

    let router = create_router(state);
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": "not-a-valid-ulid",
        "content": "edited",
    });
    let req = build_request(Method::PUT, "/v1/chat/message", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── #278: POST /v1/chat/branch/switch 端到端测试 ─────────────────────────────

/// 构造一个含两个分支的会话：
///   user1 → assistant1 (leaf=a1, 原分支)
///        → user2 (leaf=u2, 新分支，active)
/// 返回 (character_id, a1_leaf_id, u2_leaf_id)。
fn setup_character_with_branches(data_root: &std::path::Path) -> (CharacterId, String, String) {
    let character = CharacterId::new("branch-test-char").unwrap();
    let service = ChatService::new(data_root);
    // user1
    let (log, _) = service
        .append(
            &character,
            None,
            ChatMessage {
                role: MessageRole::User,
                content: "first user".into(),
            },
        )
        .unwrap();
    let user1_id = log.message_ids[0].clone();
    // assistant1（原分支叶节点）
    let (log, _) = service
        .append(
            &character,
            None,
            ChatMessage {
                role: MessageRole::Assistant,
                content: "first reply".into(),
            },
        )
        .unwrap();
    let a1_id = log.message_ids[1].clone();
    // 从 user1 分叉，创建新分支：user2（新分支叶节点）
    let (log, _) = service
        .append_with_branch(
            &character,
            None,
            ChatMessage {
                role: MessageRole::User,
                content: "second user (branch)".into(),
            },
            Some(user1_id),
        )
        .unwrap();
    let u2_id = log.message_ids[2].clone();
    (character, a1_id, u2_id)
}

#[tokio::test]
async fn switch_branch_to_target_leaf_returns_200() {
    let (state, _tmp) = setup().await;
    let (character, a1_leaf_id, u2_leaf_id) = setup_character_with_branches(&state.data_root);

    let router = create_router(state.clone());
    // 当前 active 是 u2（最后追加的）。切换回 a1 分支。
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "target_leaf_id": a1_leaf_id,
    });
    let req = build_request(Method::POST, "/v1/chat/branch/switch", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 8192).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    // ChatLog（非 HistoryWindow）只有 active_leaf 字段，没有 active_path 字段。
    // 切换到 a1 分支后 active_leaf 应等于 a1_leaf_id
    assert_eq!(
        json["active_leaf"], a1_leaf_id,
        "切换到 a1 分支后 active_leaf 应为 a1_leaf_id"
    );
    // messages 应包含所有消息（含非 active path 的 sibling 分支）
    let messages = json["messages"].as_array().expect("messages array");
    assert!(
        messages.len() >= 2,
        "messages 应至少包含 user1 + assistant1"
    );

    // 再切回 u2 分支
    let router2 = create_router(state);
    let body2 = serde_json::json!({
        "character_id": character.as_str(),
        "target_leaf_id": u2_leaf_id,
    });
    let req2 = build_request(Method::POST, "/v1/chat/branch/switch", &body2.to_string());
    let resp2 = router2.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let body_bytes2 = axum::body::to_bytes(resp2.into_body(), 8192).await.unwrap();
    let json2: serde_json::Value = serde_json::from_slice(&body_bytes2).unwrap();
    assert_eq!(
        json2["active_leaf"], u2_leaf_id,
        "切回 u2 分支后 active_leaf 应为 u2_leaf_id"
    );
}

#[tokio::test]
async fn switch_branch_invalid_leaf_returns_400() {
    let (state, _tmp) = setup().await;
    let (character, _, _) = setup_character_with_branches(&state.data_root);

    let router = create_router(state);
    // 合法 ULID 格式但实际不存在
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "target_leaf_id": "01ARZ3NDEKTSV4RRFFQ69G5FAV",
    });
    let req = build_request(Method::POST, "/v1/chat/branch/switch", &body.to_string());
    let resp = router.oneshot(req).await.unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

// ── #277: PUT /v1/chat/message body limit 413 ────────────────────────────────

#[tokio::test]
async fn edit_message_oversized_body_returns_413() {
    let (state, _tmp) = setup().await;
    let (character, user_id, _) = setup_character_with_messages(&state.data_root);

    let router = create_router(state);
    // 构造 > 2MB 的 body：content 字段填充至 2.5MB
    // 2MB = 2 * 1024 * 1024 = 2_097_152 bytes
    // 2.5MB content + JSON 包裹 ≈ 2.5MB > 2MB limit
    let oversized_content = "x".repeat(2_621_440); // 2.5MB
    let body = serde_json::json!({
        "character_id": character.as_str(),
        "message_id": user_id,
        "content": oversized_content,
    });
    let body_bytes = serde_json::to_vec(&body).unwrap();
    assert!(
        body_bytes.len() > 2 * 1024 * 1024,
        "test body should exceed 2MB, got {} bytes",
        body_bytes.len()
    );

    let req = build_request_bytes(Method::PUT, "/v1/chat/message", body_bytes);
    let resp = router.oneshot(req).await.unwrap();

    // 413 Payload Too Large
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}
