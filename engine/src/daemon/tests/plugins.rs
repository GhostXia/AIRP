//! Trusted Plugin MVP（#498 §6.4 / §7.3）集成测试：
//! 反代路由（转发 / 透传 / 错误映射 / 鉴权层外）+ `GET /v1/plugins` 列表。

use super::*;
use crate::plugins::TrustedPluginManifest;
use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{header, Request, StatusCode};
use futures_util::StreamExt;
use tower::util::ServiceExt;

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// loopback peer 地址（B4 修复后反代 fail-closed，测试需显式注入）。
fn loopback_addr() -> std::net::SocketAddr {
    "127.0.0.1:0".parse().unwrap()
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
            let body = String::from_utf8_lossy(
                &axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap(),
            )
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
                .extension(ConnectInfo(loopback_addr()))
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
                .extension(ConnectInfo(loopback_addr()))
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
                .extension(ConnectInfo(loopback_addr()))
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
                .extension(ConnectInfo(loopback_addr()))
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

    // 无 token 反代 → 200（loopback ConnectInfo 注入，B4 fail-closed）
    let proxied = router
        .clone()
        .oneshot(
            Request::get("/api/plugins/com.example.echo/ping")
                .extension(ConnectInfo(loopback_addr()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(proxied.status(), StatusCode::OK);

    // 无 token 列表 → 401
    let listed = router
        .oneshot(Request::get("/v1/plugins").body(Body::empty()).unwrap())
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

/// SSE 上游：单帧后挂起（模拟心跳长连接）。
async fn spawn_sse_plugin() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route(
        "/stream",
        axum::routing::get(|| async {
            let stream = futures_util::stream::iter(vec![Ok::<bytes::Bytes, std::io::Error>(
                bytes::Bytes::from_static(b"data: hello\n\n"),
            )])
            .chain(futures_util::stream::pending());
            (
                [(header::CONTENT_TYPE, "text/event-stream")],
                axum::body::Body::from_stream(stream),
            )
        }),
    );
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .unwrap_or_else(|e| panic!("fake sse plugin serve failed: {e}"));
    });
    (port, handle)
}

/// 审计 W1/W2：`text/event-stream` 分块透传（不整体缓冲），shutdown 广播后
/// 流立即 EOF（否则在飞 SSE 会阻塞 daemon 优雅退出，且整体缓冲会在 30s
/// 超时后断流）。
#[tokio::test]
async fn proxy_streams_sse_and_stops_on_shutdown() {
    let (port, _plugin) = spawn_sse_plugin().await;
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.sse", port));
    let router = create_router(state.clone());

    let resp = router
        .oneshot(
            Request::get("/api/plugins/com.example.sse/stream")
                .extension(ConnectInfo(loopback_addr()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "text/event-stream");

    let mut stream = resp.into_body().into_data_stream();
    // 第一帧必须立即到达：整体缓冲实现会在 30s 代理超时后才断流。
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("sse first frame should arrive promptly (no full buffering)")
        .expect("sse stream should not be empty")
        .expect("sse stream should not error");
    assert!(
        String::from_utf8_lossy(&first).contains("data: hello"),
        "first frame content: {:?}",
        String::from_utf8_lossy(&first)
    );

    // shutdown 广播 → 流提前 EOF。
    state.shutdown.send(true).unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("sse stream should end after shutdown broadcast");
    assert!(
        ended.is_none(),
        "sse stream must EOF once shutdown is broadcast"
    );
}

/// CodeRabbit：非流式响应体上限 2MB（此处经 Content-Length 预检拒绝）。
#[tokio::test]
async fn proxy_rejects_oversized_response() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = axum::Router::new().route(
        "/big",
        axum::routing::get(|| async {
            (
                StatusCode::OK,
                axum::body::Body::from(vec![0u8; 3 * 1024 * 1024]),
            )
        }),
    );
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .unwrap_or_else(|e| panic!("fake big plugin serve failed: {e}"));
    });

    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.big", port));
    let router = create_router(state);
    let resp = router
        .oneshot(
            Request::get("/api/plugins/com.example.big/big")
                .extension(ConnectInfo(loopback_addr()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "plugin_response_too_large");
    handle.abort();
}

/// 审计 W7：反代 loopback-only——非 loopback peer 直接 403
/// （0.0.0.0 监听时远程请求不得直达插件）。
#[tokio::test]
async fn proxy_rejects_remote_clients() {
    let (port, _plugin) = spawn_fake_plugin().await;
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.echo", port));
    let router = create_router(state);

    let mut req = Request::get("/api/plugins/com.example.echo/ping")
        .body(Body::empty())
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "203.0.113.1:4321".parse::<std::net::SocketAddr>().unwrap(),
    ));
    let resp = router.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "plugin_remote_forbidden");
}

/// 审计 B4：无 ConnectInfo 时 fail-closed（非 fail-open）——
/// 反代挂鉴权层外，无 peer 信息时必须拒绝，不应放行。
#[tokio::test]
async fn proxy_rejects_missing_connect_info() {
    let (port, _plugin) = spawn_fake_plugin().await;
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.echo", port));
    let router = create_router(state);

    // 不注入 ConnectInfo → 403（fail-closed）
    let resp = router
        .oneshot(
            Request::get("/api/plugins/com.example.echo/ping")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "plugin_remote_forbidden");
}

/// 审计 W5：RawPathParams 保留原始编码（%2F / %23 / %20 不被二次解码），
/// 裸非 ASCII（中文）补码为 %XX，插件收到的路径与浏览器语义一致。
#[tokio::test]
async fn proxy_preserves_encoding_and_reencodes_raw_utf8() {
    let (port, _plugin) = spawn_fake_plugin().await;
    let (state, _tmp) = make_state_no_key();
    state
        .plugins
        .write()
        .unwrap()
        .push(manifest_with_port("com.example.echo", port));
    let router = create_router(state);

    // 已编码输入原样保留。
    let resp = router
        .clone()
        .oneshot(
            Request::get("/api/plugins/com.example.echo/a%2Fb%23c%20d?q=%2F%23%20x")
                .extension(ConnectInfo(loopback_addr()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["path"], "/a%2Fb%23c%20d");
    assert_eq!(body["query"], "q=%2F%23%20x");

    // 裸非 ASCII 补码（避免上游解析歧义）。
    let resp = router
        .oneshot(
            Request::get("/api/plugins/com.example.echo/你好")
                .extension(ConnectInfo(loopback_addr()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["path"], "/%E4%BD%A0%E5%A5%BD");
}
