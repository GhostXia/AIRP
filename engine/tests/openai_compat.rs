//! DX-7 / DX-10: OpenAI compatibility + multi-user acceptance tests.
//!
//! DX-7 (openai_compat): end-to-end `/v1/chat/completions` via in-process router.
//!   1. Valid AIRP request → HTTP 200, `content-type: text/event-stream`.
//!   2. SSE stream has `event: message` frames.
//!   3. Inline character card JSON accepted without a disk file.
//!   4. Upstream errors propagate as `event: error` SSE frames.
//!
//! DX-10 (acceptance): multi-user isolation + quota enforcement end-to-end.
//!   5. Two users with quota=1 each get exactly one successful request.
//!   6. Second request per user returns HTTP 429.
//!   7. Users are isolated: alice's quota does not affect bob's.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use std::net::SocketAddr;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use airp_core::adapter::{BackendEngine, Provider};
use airp_core::config::VolumeConfig;
use airp_core::daemon::{create_router, DaemonState, MutableConfig};
use airp_core::quota::QuotaConfig;

/// Minimal Tavern V2 character card JSON.
fn inline_card() -> &'static str {
    r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"TestChar","description":"A test character","personality":"","scenario":"","first_mes":"Hello!","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#
}

/// Build OpenAI SSE body with a few delta tokens.
fn build_sse_body(tokens: &[&str]) -> String {
    let mut out = String::new();
    for tk in tokens {
        let escaped = tk.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
            escaped
        ));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

/// Build a POST request with a fake peer `ConnectInfo` so the rate-limiter's
/// `UserOrIpKeyExtractor` can extract an IP key from the extension.
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

fn build_get_request(uri: &str) -> Request<Body> {
    let mut req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9999u16))));
    req
}

/// Spin up `DaemonState` with wiremock upstream, quota limit, and a tmp data root.
async fn setup_with_quota(
    upstream_url: &str,
    max_requests_per_day: u32,
) -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().to_path_buf();

    std::fs::create_dir_all(data_root.join("characters")).unwrap();
    std::fs::create_dir_all(data_root.join("presets")).unwrap();
    std::fs::create_dir_all(data_root.join("sessions")).unwrap();

    let endpoint = format!("{}/v1/chat/completions", upstream_url);
    let state = Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint,
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: QuotaConfig {
                max_requests_per_day,
                max_tokens_per_day: 0,
            },
        }),
    });
    (state, tmp)
}

/// Spin up a `DaemonState` with a wiremock upstream and a tmp data root.
async fn setup(upstream_url: &str) -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().to_path_buf();

    std::fs::create_dir_all(data_root.join("characters")).unwrap();
    std::fs::create_dir_all(data_root.join("presets")).unwrap();
    std::fs::create_dir_all(data_root.join("sessions")).unwrap();

    let endpoint = format!("{}/v1/chat/completions", upstream_url);
    let state = Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint,
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: QuotaConfig::default(),
        }),
    });
    (state, tmp)
}

async fn setup_with_endpoint(endpoint: String) -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().to_path_buf();

    std::fs::create_dir_all(data_root.join("characters")).unwrap();
    std::fs::create_dir_all(data_root.join("presets")).unwrap();
    std::fs::create_dir_all(data_root.join("sessions")).unwrap();

    let state = Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint,
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: QuotaConfig::default(),
        }),
    });
    (state, tmp)
}

#[tokio::test]
async fn test_models_proxy_success_passthrough() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "object": "list",
            "data": [{ "id": "test-model" }]
        })))
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let router = create_router(state);
    let resp = router
        .oneshot(build_get_request("/v1/models"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["data"][0]["id"], "test-model");
}

#[tokio::test]
async fn test_models_proxy_upstream_status_returns_typed_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": { "message": "bad provider key" }
        })))
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let router = create_router(state);
    let resp = router
        .oneshot(build_get_request("/v1/models"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "upstream_status");
    assert_eq!(json["error"]["upstream_status"], 401);
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("HTTP 401"));
    assert!(json["error"]["upstream_body"]
        .as_str()
        .unwrap()
        .contains("bad provider key"));
}

#[tokio::test]
async fn test_models_proxy_invalid_endpoint_returns_typed_error() {
    let (state, _tmp) = setup_with_endpoint("not-a-url".to_string()).await;
    let router = create_router(state);
    let resp = router
        .oneshot(build_get_request("/v1/models"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "invalid_endpoint");
    assert_eq!(json["error"]["detail"], "not-a-url");
}

#[tokio::test]
async fn test_models_proxy_invalid_endpoint_redacts_query_detail() {
    let (state, _tmp) = setup_with_endpoint("not-a-url?api_key=secret".to_string()).await;
    let router = create_router(state);
    let resp = router
        .oneshot(build_get_request("/v1/models"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "invalid_endpoint");
    assert_eq!(json["error"]["detail"], "not-a-url?redacted");
    assert!(!body.windows(b"secret".len()).any(|w| w == b"secret"));
}

/// #42 F-1：无路径 endpoint（如 `http://example.com`）不得推导出丢失 host 的
/// 畸形 URL，必须走 invalid_endpoint 类型化错误。
#[tokio::test]
async fn test_models_proxy_no_path_endpoint_returns_invalid_endpoint() {
    let (state, _tmp) = setup_with_endpoint("http://example.com".to_string()).await;
    let router = create_router(state);
    let resp = router
        .oneshot(build_get_request("/v1/models"))
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "invalid_endpoint");
}

/// #40 建议 3：上游 hang → 504 + upstream_timeout。
/// 通过 `AIRP_MODELS_PROXY_TIMEOUT_MS` 走快速超时路径，避免测试等满默认 5s。
#[tokio::test]
async fn test_models_proxy_upstream_timeout_returns_504() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"data": []}))
                .set_delay(std::time::Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    std::env::set_var("AIRP_MODELS_PROXY_TIMEOUT_MS", "200");
    let (state, _tmp) = setup(&server.uri()).await;
    let router = create_router(state);
    let resp = router
        .oneshot(build_get_request("/v1/models"))
        .await
        .unwrap();
    std::env::remove_var("AIRP_MODELS_PROXY_TIMEOUT_MS");

    assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["error"]["code"], "upstream_timeout");
}

#[tokio::test]
async fn test_dx7_completions_returns_sse_200() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_sse_body(&["Hello", " world"])),
        )
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let router = create_router(state);

    let body = serde_json::json!({
        "message": "Hi!",
        "character_card_id": inline_card(),
        "user_profile": { "name": "Tester", "variables": {} }
    });

    let req = build_post_request(
        "/v1/chat/completions",
        &serde_json::to_string(&body).unwrap(),
    );

    let resp = router.oneshot(req).await.unwrap();
    if resp.status() != StatusCode::OK {
        let b = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        panic!(
            "expected 200, got 500: {}",
            std::str::from_utf8(&b).unwrap_or("?")
        );
    }

    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.contains("text/event-stream"),
        "expected text/event-stream, got: {}",
        ct
    );
}

#[tokio::test]
async fn test_dx7_completions_sse_has_message_events() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_sse_body(&["Greetings"])),
        )
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let router = create_router(state);

    let body = serde_json::json!({
        "message": "Hello",
        "character_card_id": inline_card(),
        "user_profile": { "name": "User", "variables": {} }
    });

    let req = build_post_request(
        "/v1/chat/completions",
        &serde_json::to_string(&body).unwrap(),
    );

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body_str = std::str::from_utf8(&bytes).unwrap();
    assert!(
        body_str.contains("event:message") || body_str.contains("event: message"),
        "SSE body should contain message events, got:\n{}",
        &body_str[..body_str.len().min(500)]
    );
}

#[tokio::test]
async fn test_dx7_upstream_error_propagated_as_error_event() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let router = create_router(state);

    let body = serde_json::json!({
        "message": "Hello",
        "character_card_id": inline_card(),
        "user_profile": { "name": "User", "variables": {} }
    });

    let req = build_post_request(
        "/v1/chat/completions",
        &serde_json::to_string(&body).unwrap(),
    );

    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body_str = std::str::from_utf8(&bytes).unwrap();
    assert!(
        body_str.contains("event:error") || body_str.contains("event: error"),
        "upstream error should propagate as SSE error event, got:\n{}",
        &body_str[..body_str.len().min(500)]
    );
}

// ─── DX-10: Multi-user acceptance ────────────────────────────────────────────

/// DX-10a: quota=1 enforced — first request succeeds, second returns 429.
#[tokio::test]
async fn test_dx10_quota_enforced_per_user() {
    let server = MockServer::start().await;
    // Mount unlimited responses; quota cuts in before upstream is even called on the 2nd.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_sse_body(&["ok"])),
        )
        .expect(1) // wiremock will assert exactly 1 upstream call
        .mount(&server)
        .await;

    let (state, _tmp) = setup_with_quota(&server.uri(), 1).await;

    let body = serde_json::json!({
        "message": "Hi",
        "character_card_id": inline_card(),
        "user_profile": { "name": "QuotaUser", "variables": {} },
        "user_id": "quota_test_user"
    });
    let body_str = serde_json::to_string(&body).unwrap();

    // First request — should succeed.
    let resp1 = create_router(state.clone())
        .oneshot(build_post_request("/v1/chat/completions", &body_str))
        .await
        .unwrap();
    assert_eq!(
        resp1.status(),
        StatusCode::OK,
        "first request should be OK within quota"
    );
    // Drain body so response is consumed.
    axum::body::to_bytes(resp1.into_body(), 4096).await.unwrap();

    // Second request — quota exceeded → 429.
    let resp2 = create_router(state.clone())
        .oneshot(build_post_request("/v1/chat/completions", &body_str))
        .await
        .unwrap();
    assert_eq!(
        resp2.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "second request should be rejected with 429"
    );
}

/// DX-10b: two users share same quota limit but counters are isolated.
/// alice and bob each get one successful request; each is denied on the second.
#[tokio::test]
async fn test_dx10_multi_user_quota_isolation() {
    let server = MockServer::start().await;
    // Upstream will be called exactly twice (once per user).
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_sse_body(&["hi"])),
        )
        .expect(2)
        .mount(&server)
        .await;

    let (state, _tmp) = setup_with_quota(&server.uri(), 1).await;

    let make_body = |uid: &str| {
        serde_json::to_string(&serde_json::json!({
            "message": "Hey",
            "character_card_id": inline_card(),
            "user_profile": { "name": uid, "variables": {} },
            "user_id": uid
        }))
        .unwrap()
    };

    // alice first request → 200.
    let r1 = create_router(state.clone())
        .oneshot(build_post_request(
            "/v1/chat/completions",
            &make_body("alice_dx10"),
        ))
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK, "alice first req should be OK");
    axum::body::to_bytes(r1.into_body(), 4096).await.unwrap();

    // bob first request → 200 (independent quota).
    let r2 = create_router(state.clone())
        .oneshot(build_post_request(
            "/v1/chat/completions",
            &make_body("bob_dx10"),
        ))
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK, "bob first req should be OK");
    axum::body::to_bytes(r2.into_body(), 4096).await.unwrap();

    // alice second request → 429.
    let r3 = create_router(state.clone())
        .oneshot(build_post_request(
            "/v1/chat/completions",
            &make_body("alice_dx10"),
        ))
        .await
        .unwrap();
    assert_eq!(
        r3.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "alice second req should be 429"
    );

    // bob second request → 429.
    let r4 = create_router(state.clone())
        .oneshot(build_post_request(
            "/v1/chat/completions",
            &make_body("bob_dx10"),
        ))
        .await
        .unwrap();
    assert_eq!(
        r4.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "bob second req should be 429"
    );
}
