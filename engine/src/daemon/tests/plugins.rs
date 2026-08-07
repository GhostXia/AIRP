//! Trusted Plugin MVP（#498 §6.4 / §7.3）集成测试：
//! 反代路由（转发 / 透传 / 错误映射 / 鉴权层外）+ `GET /v1/plugins` 列表。

use super::*;
use crate::plugins::TrustedPluginManifest;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use tower::util::ServiceExt;

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn manifest_with_port(id: &str, port: u16) -> TrustedPluginManifest {
    TrustedPluginManifest {
        id: id.to_string(),
        version: "1.0.0".to_string(),
        command: "./server".to_string(),
        args: vec![],
        port,
        host_api: "1".to_string(),
    }
}

/// 起一个假的 trusted plugin（echo server）在随机 loopback 端口，
/// 返回 (端口, join handle)。
async fn spawn_fake_plugin() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route(
        "/*path",
        axum::routing::any(|req: Request<Body>| async move {
            let method = req.method().to_string();
            let path = req.uri().path().to_string();
            let query = req.uri().query().unwrap_or("").to_string();
            let body =
                String::from_utf8_lossy(&axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap())
                .to_string();
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({ "method": method, "path": path, "query": query, "body": body })
                    .to_string(),
            )
        }),
    );
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .unwrap_or_else(|e| panic!("fake plugin serve failed: {e}"));
    });
    (port, handle)
}

/// 反代 GET：path / query 透传。
#[tokio::test]
async fn proxy_forwards_get_with_query() {
    let (port, _plugin) = spawn_fake_plugin().await;
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.echo", port));
    let router = create_router(state);

    let resp = router
        .oneshot(
            Request::get("/api/plugins/com.example.echo/hello?x=1&y=2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["method"], "GET");
    assert_eq!(body["path"], "/hello");
    assert_eq!(body["query"], "x=1&y=2");
}

/// 反代 POST：method / body 透传。
#[tokio::test]
async fn proxy_forwards_post_with_body() {
    let (port, _plugin) = spawn_fake_plugin().await;
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.echo", port));
    let router = create_router(state);

    let resp = router
        .oneshot(
            Request::post("/api/plugins/com.example.echo/speak")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"text":"hi"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["method"], "POST");
    assert_eq!(body["path"], "/speak");
    assert_eq!(body["body"], r#"{"text":"hi"}"#);
}

/// 未知 plugin id → 404 plugin_not_found。
#[tokio::test]
async fn proxy_unknown_id_returns_404() {
    let (state, _tmp) = make_state_no_key();
    let router = create_router(state);
    let resp = router
        .oneshot(
            Request::get("/api/plugins/com.example.ghost/anything")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "plugin_not_found");
}

/// 插件端口未监听（子进程未起/已崩）→ 502 plugin_unreachable。
#[tokio::test]
async fn proxy_unreachable_port_returns_502() {
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.dead", 1));
    let router = create_router(state);
    let resp = router
        .oneshot(
            Request::get("/api/plugins/com.example.dead/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "plugin_unreachable");
}

/// 架构决策锁：反代在鉴权层外（§6.4 不做 caller 限制），带 access key 时
/// 无 token 的 /api/plugins 请求仍可转发；同 state 下 /v1/plugins 无 token
/// 必须 401（列表面仍走统一鉴权）。
#[tokio::test]
async fn proxy_is_outside_auth_layer_while_list_is_inside() {
    let (port, _plugin) = spawn_fake_plugin().await;
    let (state, _tmp) = make_state_with_key(Some("secret"));
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.echo", port));
    let router = create_router(state);

    // 无 token 反代 → 200
    let proxied = router
        .clone()
        .oneshot(
            Request::get("/api/plugins/com.example.echo/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(proxied.status(), StatusCode::OK);

    // 无 token 列表 → 401
    let listed = router
        .oneshot(
            Request::get("/v1/plugins").body(Body::empty()).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::UNAUTHORIZED);
}

/// GET /v1/plugins：返回已安装 manifest（id/version/host_api/status）。
/// 无子进程 → status=stopped（不做探活，§6.6）。
#[tokio::test]
async fn list_plugins_reports_manifests() {
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.one", 8001));
    let router = create_router(state);

    let resp = router
        .oneshot(Request::get("/v1/plugins").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let items = body["plugins"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], "com.example.one");
    assert_eq!(items[0]["version"], "1.0.0");
    assert_eq!(items[0]["host_api"], "1");
    assert_eq!(items[0]["status"], "stopped");
}
