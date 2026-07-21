// Memory API route-level tests（审计 B1/B2/W1/W3 验证）。
//
// PR #271 审计修复后的覆盖：
// - GET /v1/memory/resident?character_id=... 读取空文件返回空 content + char_count=0
// - PUT /v1/memory/resident 写入后 GET 能读回
// - PUT /v1/memory/resident 拒绝路径遍历 character_id（CharacterId::new 校验）
// - PUT /v1/memory/user-model 拒绝路径遍历 user_id（UserId serde 反序列化校验，审计 B1）
// - PUT /v1/memory/* 超过 2MB body 返回 413 Payload Too Large（审计 B2）
// - GET /v1/memory/user-model 读取不存在用户返回空 content
//
// 注意：UserId 走 serde 反序列化路径校验：
// - GET 路径用 `Query` 提取器，非法 user_id 返回 400 BadRequest
// - PUT 路径用 `Json` 提取器，非法 user_id 返回 422 Unprocessable Entity
// 两者都会阻止路径遍历进入 handler，差异是 axum 标准行为，不需要"修正"。

use super::*;

/// GET /v1/memory/resident?character_id=alice 在没有任何记忆文件时应返回空 content。
#[tokio::test]
async fn get_resident_memory_returns_empty_when_absent() {
    let (state, _tmp) = make_state_with_key(None);
    // 先建一个 character 目录，否则 resolve_session_dir 可能返回 NotFound
    std::fs::create_dir_all(state.data_root.join("characters").join("alice")).unwrap();
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/memory/resident?character_id=alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["content"], "");
    assert_eq!(v["char_count"], 0);
    assert_eq!(v["capacity"], 2000);
}

/// PUT /v1/memory/resident 写入后 GET 能读回相同内容。
#[tokio::test]
async fn put_resident_memory_roundtrip() {
    let (state, _tmp) = make_state_with_key(None);
    std::fs::create_dir_all(state.data_root.join("characters").join("alice")).unwrap();
    let app = create_router(state);

    let put_resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/memory/resident")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "character_id": "alice",
                        "session_id": null,
                        "content": "- 用户喜欢猫\n- 角色叫艾莉娅"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::OK);

    let get_resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/memory/resident?character_id=alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get_resp.into_body(), 4096)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["content"], "- 用户喜欢猫\n- 角色叫艾莉娅");
    // char_count 用 chars().count()，对中文每字算 1：
    // "- 用户喜欢猫\n- 角色叫艾莉娅" = 2 + 5 + 1 + 2 + 6 = 16
    assert_eq!(v["char_count"], 16);
}

/// PUT /v1/memory/resident 拒绝路径遍历 character_id。
#[tokio::test]
async fn put_resident_memory_rejects_traversal_character_id() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/memory/resident")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "character_id": "../../../etc/passwd",
                        "session_id": null,
                        "content": "evil"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // CharacterId::new 在 handler 内显式调用，返回 BadRequest
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// PUT /v1/memory/user-model 拒绝路径遍历 user_id（审计 B1）。
///
/// UserId 走 serde 反序列化路径，非法字符串在 `Json<UpdateUserModelRequest>`
/// 解析时即被拒绝，不会进入 handler body。
///
/// 注意：axum 的 `Json` 提取器对反序列化失败返回 422 Unprocessable Entity
/// （请求体是合法 JSON 但语义校验失败），而 `Query` 提取器返回 400 BadRequest。
/// 因此 PUT 路径（Json）预期 422，GET 路径（Query）预期 400。这是 axum 的标准行为，
/// 不需要"修正"为 400——两者都能阻止路径遍历进入 handler。
#[tokio::test]
async fn put_user_model_rejects_traversal_user_id() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/memory/user-model")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_id": "../../../etc/passwd",
                        "content": "evil"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    // axum Json 提取器对 serde 反序列化失败返回 422，不是 400
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
}

/// PUT /v1/memory/user-model 正常写入后，GET 能读回。
#[tokio::test]
async fn user_model_roundtrip() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let put_resp = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/memory/user-model")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_id": "alice",
                        "content": "- 偏好简洁回复"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(put_resp.status(), StatusCode::OK);

    let get_resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/memory/user-model?user_id=alice")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get_resp.into_body(), 4096)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["content"], "- 偏好简洁回复");
}

/// PUT /v1/memory/resident 超过 2MB body 返回 413（审计 B2）。
#[tokio::test]
async fn put_resident_memory_rejects_oversized_body() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    // 构造 > 2MB 的 content：2 * 1024 * 1024 + 64 字节
    let huge_content = "x".repeat(2 * 1024 * 1024 + 64);
    let body_str = serde_json::json!({
        "character_id": "alice",
        "session_id": null,
        "content": huge_content,
    })
    .to_string();
    let body_len = body_str.len();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/memory/resident")
                .header("content-type", "application/json")
                .body(Body::from(body_str))
                .unwrap(),
        )
        .await
        .unwrap();
    // 413 Payload Too Large
    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "body of {body_len} bytes should be rejected as 413"
    );
}

/// PUT /v1/memory/user-model 超过 2MB body 返回 413（审计 B2）。
#[tokio::test]
async fn put_user_model_rejects_oversized_body() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let huge_content = "x".repeat(2 * 1024 * 1024 + 64);
    let body_str = serde_json::json!({
        "user_id": "alice",
        "content": huge_content,
    })
    .to_string();

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/memory/user-model")
                .header("content-type", "application/json")
                .body(Body::from(body_str))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

/// GET /v1/memory/user-model?user_id=nonexistent 读取不存在用户返回空 content。
#[tokio::test]
async fn get_user_model_returns_empty_when_absent() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/memory/user-model?user_id=nonexistent_user")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["content"], "");
}

/// GET /v1/memory/user-model?user_id=../etc 路径遍历在 query 反序列化时被拒绝（审计 B1）。
#[tokio::test]
async fn get_user_model_rejects_traversal_user_id_in_query() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/memory/user-model?user_id=..%2Fetc")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}

/// PUT /v1/memory/user-model 后，磁盘上的 user_model.md 应只有目标文件，无 .tmp / .bak 残留（审计 W1）。
#[tokio::test]
async fn user_model_write_leaves_no_temp_or_bak_residue() {
    let (state, _tmp) = make_state_with_key(None);
    let data_root = state.data_root.clone();
    let app = create_router(state);

    // 两次写入，触发 backup + cleanup 路径
    for content in ["first", "second"] {
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/memory/user-model")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "user_id": "alice",
                            "content": content,
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    let user_dir = data_root.join("users").join("alice");
    let entries: Vec<String> = std::fs::read_dir(&user_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(entries, vec!["user_model.md".to_string()]);
    assert_eq!(
        std::fs::read_to_string(user_dir.join("user_model.md")).unwrap(),
        "second"
    );
}
