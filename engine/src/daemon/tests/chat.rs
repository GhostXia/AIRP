// Chat history / rollback / regen endpoint tests, plus A1b persona-aware
// `/v1/chat/completions` validation.
//
// Moved verbatim from `daemon::tests`. The A6 cases assert that `session_id`
// is accepted (no longer 422 unknown field) and that legacy vs session-scoped
// logs diverge on disk; O1 asserts the response shape exposes
// `scope_session_id` only for session-scoped reads; A1b asserts the chat
// pipeline's persona fail-closed (404 for unknown persona_id, OK for the
// virtual `default`).
//
// PR4-A1（#160 审计遗留项）：`pr75_chat_history_returns_message_timestamps`
// 原为 `handlers.rs` 内联测试，使用其私有的 `make_state_for_http_test`。
// `make_state_no_key` 与之等价（同一 DaemonState fixture），借此把测试
// 迁出 handler 模块、回归到 `daemon/tests/chat.rs` 路由级测试位置。

use super::*;

// ── A6: chat/history 支持 session_id 字段 ──────────────────────────────

#[tokio::test]
async fn test_a6_chat_history_accepts_session_id_field() {
    // A6 修复前：deny_unknown_fields 拒绝 session_id → 422
    // A6 修复后：session_id 被接受，返回 200 + ChatLog
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    let body = serde_json::json!({"character_id": "alice", "session_id": "550e8400-e29b-41d4-a716-446655440000"});
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // 200 即证明 session_id 被接受（修复前是 422 unknown field）
}

#[tokio::test]
async fn test_a6_chat_history_session_scoped_vs_legacy_diverge() {
    // A6 核心验证：同一 character_id 下，
    //   1. 不传 session_id → legacy per-character log
    //   2. 传 session_id → session-scoped log
    // 两个 log 写到不同路径（legacy 在 characters/{id}/history/，
    // scoped 在 characters/{id}/sessions/{sid}/history/）
    let (state, tmp) = make_state_no_key();
    let app = create_router(state.clone());

    // (1) 不传 session_id → legacy log
    let body1 = serde_json::json!({"character_id": "alice"});
    let resp1 = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body1).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp1.status(), StatusCode::OK);

    // (2) 传 session_id → session-scoped log
    let scoped_sid = "550e8400-e29b-41d4-a716-446655440000";
    let body2 = serde_json::json!({"character_id": "alice", "session_id": scoped_sid});
    let resp2 = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body2).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);

    // 核心断言：两个 log 写到不同路径
    // legacy: characters/alice/history/chat_log.jsonl
    // scoped: characters/alice/sessions/{sid}/history/chat_log.jsonl
    let legacy_jsonl = tmp
        .path()
        .join("characters")
        .join("alice")
        .join("history")
        .join("chat_log.jsonl");
    let scoped_jsonl = tmp
        .path()
        .join("characters")
        .join("alice")
        .join("sessions")
        .join(scoped_sid)
        .join("history")
        .join("chat_log.jsonl");
    assert!(
        legacy_jsonl.exists(),
        "legacy log 必须存在: {:?}",
        legacy_jsonl
    );
    assert!(
        scoped_jsonl.exists(),
        "session-scoped log 必须存在: {:?}",
        scoped_jsonl
    );
    assert_ne!(legacy_jsonl, scoped_jsonl, "A6 核心断言：两个路径必须不同");
}

#[tokio::test]
async fn test_a6_chat_rollback_accepts_session_id() {
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    // 直接 rollback 一个空 session（messages 为空，rollback_to(0) 应安全）
    let body = serde_json::json!({
        "character_id": "alice",
        "message_index": 0,
        "session_id": "550e8400-e29b-41d4-a716-446655440000"
    });
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/rollback")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    // 期望 200（A6 修复前会 422 unknown field session_id）
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_a6_chat_regen_accepts_session_id() {
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    let body = serde_json::json!({
        "character_id": "alice",
        "session_id": "550e8400-e29b-41d4-a716-446655440000"
    });
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/regen")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    // 期望 200（A6 修复前会 422 unknown field session_id）
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_a6_chat_history_without_session_id_still_works() {
    // 回退兼容：不传 session_id 时仍然走 legacy 路径，不能因 A6 改动破坏旧客户端
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    let body = serde_json::json!({"character_id": "alice"});
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

// ── O1 (#86): ChatLog.scope_session_id 在 HTTP 响应中暴露 ──────────────

#[tokio::test]
async fn test_o1_session_scoped_history_exposes_scope_session_id() {
    // 传 session_id 调 history → 响应应包含 scope_session_id 字段，值与传入一致
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    let scope_id = "550e8400-e29b-41d4-a716-446655440000";
    let body = serde_json::json!({"character_id": "alice", "session_id": scope_id});
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        v["scope_session_id"].as_str(),
        Some(scope_id),
        "session-scoped 响应必须暴露 scope_session_id 且与传入值一致"
    );
}

#[tokio::test]
async fn test_o1_legacy_history_omits_scope_session_id() {
    // 不传 session_id 调 history → 响应不应包含 scope_session_id 字段（None skip）
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    let body = serde_json::json!({"character_id": "alice"});
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_string(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(
        v.get("scope_session_id").is_none() || v["scope_session_id"].is_null(),
        "legacy 响应不应包含 scope_session_id 字段"
    );
}

// ── A1b: chat_pipeline persona activation ────────────────────────────────

#[tokio::test]
async fn a1b_chat_completions_returns_404_for_nonexistent_persona_id() {
    // Explicit `persona_id` that does not exist must fail closed with 404
    // before any upstream LLM call. This mirrors the plural GET contract.
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_id": "alice",
                        "persona_id": "ghost",
                        "user_profile": { "name": "Alice", "variables": {} },
                        "message": "hi"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::NOT_FOUND,
        "nonexistent persona_id must return 404, got {}",
        resp.status()
    );
}

#[tokio::test]
async fn a1b_chat_completions_accepts_default_persona_id_for_fresh_user() {
    // `default` is a virtual profile that always resolves (initial snapshot
    // when no file exists). The request should reach the streaming stage,
    // failing only at the upstream LLM call (http://localhost) — not at
    // persona resolution. We assert the response is NOT a 404.
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_id": "alice",
                        "persona_id": "default",
                        "user_profile": { "name": "Alice", "variables": {} },
                        "message": "hi",
                        "messages_history": []
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::OK,
        "default persona must reach the streaming response, got {}",
        resp.status()
    );
}

// ── W-01 (#75 PR4-A1 / #160 审计遗留项)：/v1/chat/history 返回 JSON 包含
//     message_timestamps 字段，且长度等于 messages。 ──────────────────────

#[tokio::test]
async fn pr75_chat_history_returns_message_timestamps() {
    let (state, tmp) = make_state_no_key();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("characters").join("ts_http_char")).unwrap();

    // 用 ChatLog API 写入 2 条消息（产生 ts）
    let mut log = crate::chat_store::ChatLog::new("ts_http_char");
    log.append(
        root,
        crate::adapter::ChatMessage {
            role: crate::adapter::MessageRole::User,
            content: "hello".to_string(),
        },
    )
    .unwrap();
    log.append(
        root,
        crate::adapter::ChatMessage {
            role: crate::adapter::MessageRole::Assistant,
            content: "hi".to_string(),
        },
    )
    .unwrap();

    let app = create_router(state.clone());
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "character_id": "ts_http_char" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    // 无分页字段时必须保持 legacy ChatLog 响应形状。
    assert_eq!(v["character_id"], "ts_http_char");
    assert!(v["session_id"].is_string());
    // messages 数组长度 = 2
    assert_eq!(v["messages"].as_array().unwrap().len(), 2);
    assert_eq!(v["message_ids"].as_array().unwrap().len(), 2);
    // message_timestamps 字段存在且长度等于 messages
    let tss = v["message_timestamps"].as_array().unwrap();
    assert_eq!(tss.len(), 2, "message_timestamps 长度应等于 messages");
    // 每条都有 ts（非 null）
    assert!(tss[0].is_string(), "ts[0] 应为字符串");
    assert!(tss[1].is_string(), "ts[1] 应为字符串");

    // 显式 limit 才切换到分页窗口响应，并保留完整 total。
    let app = create_router(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/history")
                .header("Content-Type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "character_id": "ts_http_char",
                        "limit": 1
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page["messages"].as_array().unwrap().len(), 1);
    assert_eq!(page["total"], 2);
    assert_eq!(page["has_more"], true);
}
