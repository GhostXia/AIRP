// Session route-level characterization tests.
//
// #155 PR4：sessions handler family 拆入 `handlers/sessions.rs` 后，补齐此前
// 缺失的 HTTP 路由层直接覆盖。domain 层已有完整单测（domain.rs 1652+），
// 此文件只验证 route table → handler → service 接线，不重复 domain 逻辑。
//
// 覆盖：
// - create → list 可见
// - delete → list 不可见
// - 非法 character_id（含路径遍历字符）→ 400 BadRequest
// - 非法 session_id（非 UUID）→ 400 BadRequest
// - 删除不存在的 session → 404 NotFound

use super::*;

/// POST /v1/sessions/:character_id 创建 → GET list 能看到该 session。
#[tokio::test]
async fn create_session_then_list_shows_it() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let create_resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/sessions/alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create_resp.into_body(), 256)
        .await
        .unwrap();
    let created: String = serde_json::from_slice(&body).unwrap();

    let list_resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/sessions/alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list_resp.into_body(), 1024)
        .await
        .unwrap();
    let sessions: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(
        sessions.contains(&created),
        "created session {created} should appear in list: {sessions:?}"
    );
}

/// DELETE /v1/sessions/:character_id/:session_id 删除 → GET list 不再包含。
#[tokio::test]
async fn delete_session_removes_it_from_list() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let create_resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/sessions/bob")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(create_resp.into_body(), 256)
        .await
        .unwrap();
    let created: String = serde_json::from_slice(&body).unwrap();

    let delete_resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/v1/sessions/bob/{}", created))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(delete_resp.status(), StatusCode::OK);

    let list_resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/sessions/bob")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list_resp.into_body(), 1024)
        .await
        .unwrap();
    let sessions: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert!(
        !sessions.contains(&created),
        "deleted session {created} should not appear in list: {sessions:?}"
    );
}

/// GET /v1/sessions/:character_id — 非法 character_id（含 `..`）→ 400。
#[tokio::test]
async fn list_sessions_rejects_path_traversal_character_id() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/sessions/..%2Fetc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// POST /v1/sessions/:character_id — 非法 character_id → 400。
#[tokio::test]
async fn create_session_rejects_invalid_character_id() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/sessions/..")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// DELETE /v1/sessions/:character_id/:session_id — 非法 session_id（非 UUID）→ 400。
#[tokio::test]
async fn delete_session_rejects_invalid_session_id() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/v1/sessions/alice/not-a-uuid")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// DELETE /v1/sessions/:character_id/:session_id — 不存在的 session → 404。
#[tokio::test]
async fn delete_nonexistent_session_returns_404() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let valid_uuid = uuid::Uuid::new_v4().to_string();
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri(format!("/v1/sessions/alice/{}", valid_uuid))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}
