// Auth (DX-2), CORS, production cache policy, rate-limit key extractor (DX-4),
// and constant-time comparison (A2-5) tests.
//
// Originally split across `daemon::tests` (CORS / cache / DX-2 / A2-5) and the
// separate `daemon::tests_dx4` module (UserOrIpKeyExtractor behavior). The
// audit plan (#155 PR 1) asks for the two to be merged here, since both cover
// the same security surface. `use super::*;` pulls in `daemon` items (via
// `tests/mod.rs`'s `use super::*`), so the original `super::allowed_cors_origins`
// and `super::constant_time_eq` call paths still resolve (they now refer to
// `tests::allowed_cors_origins` / `tests::constant_time_eq`, re-exported from
// `daemon` by the glob in `tests/mod.rs`).

use super::*;
use axum::extract::ConnectInfo;
use axum::http::Request;
use std::net::SocketAddr;
use tower_governor::key_extractor::KeyExtractor;

#[tokio::test]
async fn cors_allows_bundled_webui_origin() {
    let app = create_router(make_state_with_key(None));
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .header(header::ORIGIN, "http://127.0.0.1:9001")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&HeaderValue::from_static("http://127.0.0.1:9001"))
    );
}

#[tokio::test]
async fn production_cache_policy_keeps_streams_unbuffered_and_other_responses_unstored() {
    let state = make_state_with_key(None);
    {
        let mut cfg = state.config.write().unwrap();
        cfg.deployment_mode = DeploymentMode::Production;
        cfg.public_origin = Some("https://airp.example.com".to_string());
    }
    let app = Router::new()
        .route(
            "/json",
            get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
        )
        .route(
            "/stream",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/event-stream")],
                    "data: ok\n\n",
                )
            }),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            production_cache_policy,
        ))
        .with_state(state);

    let json = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        json.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-store"))
    );

    let stream = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/stream")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        stream.headers().get(header::CACHE_CONTROL),
        Some(&HeaderValue::from_static("no-cache"))
    );
}

#[tokio::test]
async fn cors_rejects_unlisted_origin() {
    let app = create_router(make_state_with_key(None));
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/health")
                .header(header::ORIGIN, "https://attacker.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

#[test]
fn configured_cors_origins_extend_built_in_defaults() {
    let origins = super::allowed_cors_origins(
        crate::config::DeploymentMode::Development,
        None,
        Some("https://example.test, https://example.test, invalid origin"),
    );
    assert!(origins.contains(&HeaderValue::from_static("http://127.0.0.1:9001")));
    assert!(origins.contains(&HeaderValue::from_static("https://example.test")));
    assert_eq!(
        origins
            .iter()
            .filter(|origin| *origin == "https://example.test")
            .count(),
        1
    );
}

#[test]
fn production_cors_uses_only_the_public_origin() {
    let origins = super::allowed_cors_origins(
        crate::config::DeploymentMode::Production,
        Some("https://airp.example.com"),
        Some("https://operator-added.example,http://127.0.0.1:9001"),
    );
    assert_eq!(
        origins,
        vec![HeaderValue::from_static("https://airp.example.com")]
    );
}

#[test]
fn test_a2_5_constant_time_eq() {
    assert!(super::constant_time_eq(b"secret", b"secret"));
    assert!(!super::constant_time_eq(b"secret", b"secreT"));
    assert!(!super::constant_time_eq(b"secret", b"secre")); // length differs
    assert!(super::constant_time_eq(b"", b""));
}

#[tokio::test]
async fn test_dx2_no_key_all_pass() {
    let state = make_state_with_key(None);
    let app = make_router_for_test(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_dx2_correct_key_passes() {
    let state = make_state_with_key(Some("secret-key"));
    let app = make_router_for_test(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/ping")
                .header("Authorization", "Bearer secret-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_dx2_wrong_key_returns_401() {
    let state = make_state_with_key(Some("secret-key"));
    let app = make_router_for_test(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/ping")
                .header("Authorization", "Bearer wrong-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_dx2_missing_header_returns_401() {
    let state = make_state_with_key(Some("secret-key"));
    let app = make_router_for_test(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_dx2_bearer_prefix_required() {
    let state = make_state_with_key(Some("secret-key"));
    let app = make_router_for_test(state);
    let resp = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/ping")
                .header("Authorization", "secret-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

// ── DX-4: UserOrIpKeyExtractor ─────────────────────────────────────────────
//
// Originally `daemon::tests_dx4`. Merged here per #155 PR 1: both modules cover
// the same security surface, and the audit plan asks for `tests_dx4` to be
// folded into `security.rs`. The helper builders stay file-local (no fixture
// reuse outside this module).

fn req_no_auth() -> Request<()> {
    Request::builder().body(()).unwrap()
}

fn req_with_auth(token: &str) -> Request<()> {
    Request::builder()
        .header("Authorization", format!("Bearer {}", token))
        .body(())
        .unwrap()
}

fn req_with_connect_info(ip: &str) -> Request<()> {
    let addr: SocketAddr = format!("{}:12345", ip).parse().unwrap();
    let mut req = Request::builder().body(()).unwrap();
    req.extensions_mut().insert(ConnectInfo(addr));
    req
}

#[test]
fn rate_limit_matches_ten_requests_per_second_with_twenty_burst() {
    assert_eq!(RATE_LIMIT_PERIOD, std::time::Duration::from_millis(100));
    assert_eq!(RATE_LIMIT_BURST, 20);
}

#[test]
fn test_dx4_bearer_token_used_as_key() {
    let ext = UserOrIpKeyExtractor;
    let req = req_with_auth("my-secret-token");
    let key = ext.extract(&req).unwrap();
    assert!(key.starts_with("k:"), "expected k: prefix, got: {}", key);
    assert!(key.contains("my-secret-token"));
}

#[test]
fn test_dx4_ip_used_when_no_auth_header() {
    let ext = UserOrIpKeyExtractor;
    let req = req_with_connect_info("10.0.0.1");
    let key = ext.extract(&req).unwrap();
    assert!(key.starts_with("ip:"), "expected ip: prefix, got: {}", key);
    assert!(key.contains("10.0.0.1"));
}

#[test]
fn test_a2_7_falls_back_to_fixed_key_without_auth_or_connect_info() {
    // A2-7: previously this errored (UnableToExtractKey). Now that the
    // governor covers every route, erroring would turn ConnectInfo-less
    // requests (in-process tests) into hard failures. The extractor falls
    // back to a fixed "ip:unknown" key instead — never errors.
    let ext = UserOrIpKeyExtractor;
    let req = req_no_auth();
    let key = ext.extract(&req).expect("must not error");
    assert_eq!(key, "ip:unknown");
}

#[test]
fn test_dx4_long_token_truncated_to_32_chars() {
    let ext = UserOrIpKeyExtractor;
    let long_token = "x".repeat(64);
    let req = req_with_auth(&long_token);
    let key = ext.extract(&req).unwrap();
    assert_eq!(key.len(), 34, "k: prefix (2) + 32 chars = 34; got: {}", key);
}

#[test]
fn test_a4_multibyte_token_does_not_panic() {
    // A4: 审计假设多字节 token 能到达 `&token[..32]` 切片路径，从而触发
    // "byte index 32 is not a char boundary" panic。实测：HTTP HeaderValue
    // 的 `to_str()` 在任意字节 ≥ 0x80 时返回 Err（HTTP/1.1 头值规范只允许
    // visible ASCII），因此非 ASCII token 会落回 IP key，永远到不了切片那行。
    // 结论：A4 描述的 panic 在当前代码路径下不可达；但 `chars().take(32)`
    // 仍是更安全的 Rust 写法（defense-in-depth），保留。
    // 本测试固化"多字节 token 落回 ip:unknown、不 panic"的现状，防止未来
    // 有人放松 `to_str()` 检查时悄悄把 panic 引回来。
    let ext = UserOrIpKeyExtractor;
    let multibyte_token = "🛡".repeat(40); // U+1F6E1，单码点 4 字节，40 个 = 160 字节
    let req = req_with_auth(&multibyte_token);
    let key = ext
        .extract(&req)
        .expect("must not panic on multibyte token");
    assert_eq!(
        key, "ip:unknown",
        "multibyte token rejected by to_str(); expected IP fallback"
    );
}
