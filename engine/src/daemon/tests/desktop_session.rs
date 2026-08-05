//! C-P0 desktop session token（bearer 注入通道）行为测试。

use super::*;
use crate::daemon::desktop_session::{
    clear_desktop_session_tokens_for_test, mint_desktop_session_token, token_test_lock,
    validate_desktop_session_token, DESKTOP_SESSION_TTL_SECS,
};
use axum::http::{header, Request, StatusCode};

#[test]
fn mint_then_validate_roundtrip() {
    let _token_lock = token_test_lock();
    clear_desktop_session_tokens_for_test();
    let (token, expires_in) = mint_desktop_session_token();
    assert_eq!(expires_in, DESKTOP_SESSION_TTL_SECS);
    assert_eq!(token.len(), 32); // uuid v4 simple hex
    assert!(validate_desktop_session_token(&token));
}

#[test]
fn unknown_or_empty_tokens_are_rejected() {
    let _token_lock = token_test_lock();
    clear_desktop_session_tokens_for_test();
    assert!(!validate_desktop_session_token(""));
    assert!(!validate_desktop_session_token("not-a-minted-token"));
}

#[tokio::test]
async fn desktop_session_endpoint_mints_token_behind_access_key() {
    let _token_lock = token_test_lock();
    clear_desktop_session_tokens_for_test();
    let (state, _guard) = make_state_with_key(Some("desk-key-123"));
    let router = Router::new()
        .route(
            "/v1/desktop-session",
            post(crate::daemon::desktop_session::desktop_session_endpoint),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    // 无凭据 → 401（auth 层拒绝，端点不可匿名探测）
    let anon = router
        .clone()
        .oneshot(
            Request::post("/v1/desktop-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anon.status(), StatusCode::UNAUTHORIZED);

    // access key → 200 + token 形状
    let ok = router
        .clone()
        .oneshot(
            Request::post("/v1/desktop-session")
                .header(header::AUTHORIZATION, "Bearer desk-key-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(ok.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(body["expires_in"], DESKTOP_SESSION_TTL_SECS);
    let token = body["token"].as_str().unwrap().to_string();
    assert!(validate_desktop_session_token(&token));
}

#[tokio::test]
async fn desktop_session_endpoint_fail_closed_without_access_key() {
    let _token_lock = token_test_lock();
    clear_desktop_session_tokens_for_test();
    // local-webui 便携模式（无 key、无鉴权）：auth 层放行，端点自身 403 fail-closed。
    let (state, _guard) = make_state_no_key();
    let router = Router::new()
        .route(
            "/v1/desktop-session",
            post(crate::daemon::desktop_session::desktop_session_endpoint),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);
    let resp = router
        .oneshot(
            Request::post("/v1/desktop-session")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("desktop_session_unavailable"));
}

#[tokio::test]
async fn auth_middleware_accepts_desktop_token_and_rejects_foreign_token() {
    let _token_lock = token_test_lock();
    clear_desktop_session_tokens_for_test();
    let (state, _guard) = make_state_with_key(Some("desk-key-456"));
    let router = make_router_for_test(state);

    // 未 mint 的 token → 401
    let rejected = router
        .clone()
        .oneshot(
            Request::get("/v1/ping")
                .header(
                    header::AUTHORIZATION,
                    "Bearer ffffffffffffffffffffffffffffffff",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);

    // mint 后的 token → 200，且 access key 仍可用
    let (token, _) = mint_desktop_session_token();
    for bearer in [format!("Bearer {token}"), "Bearer desk-key-456".to_string()] {
        let ok = router
            .clone()
            .oneshot(
                Request::get("/v1/ping")
                    .header(header::AUTHORIZATION, bearer)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(ok.status(), StatusCode::OK);
    }
}

#[tokio::test]
async fn desktop_webui_router_reports_desktop_mode_and_keeps_hard_csp() {
    let (state, _guard) = make_state_with_key(Some("desk-key-789"));
    let webui = tempfile::tempdir().unwrap();
    std::fs::write(webui.path().join("index.html"), "<h1>AIRP desktop</h1>").unwrap();
    let router = create_desktop_webui_router(state, webui.path().to_path_buf());

    let runtime = router
        .clone()
        .oneshot(
            Request::get("/runtime-config.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runtime.status(), StatusCode::OK);
    assert!(runtime.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'none'"));
    let body = axum::body::to_bytes(runtime.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("mode: 'desktop'"));

    // local router 行为不回退：仍报 mode 'local'
    let (state2, _guard2) = make_state_no_key();
    let local = create_local_webui_router(state2, webui.path().to_path_buf());
    let runtime_local = local
        .oneshot(
            Request::get("/runtime-config.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body_local = axum::body::to_bytes(runtime_local.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body_local).contains("mode: 'local'"));
}
