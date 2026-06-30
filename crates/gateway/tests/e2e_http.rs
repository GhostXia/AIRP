//! End-to-end test over real HTTP, fully self-contained (no external project).
//!
//! Spins up an in-process axum mock MCP server (streamable-HTTP style) on a
//! random port, then drives a tool call through the full gateway stack over
//! real TCP/HTTP. The mock *enforces* that, after `initialize`, the client
//! echoes both `Mcp-Session-Id` and `MCP-Protocol-Version` — so a passing test
//! proves the HTTP transport implements session + version propagation.

use airp_gateway::config::{RateLimitConfig, TransportConfig, UpstreamConfig};
use airp_gateway::{Gateway, GatewayConfig, RouteRule, RouteTarget};
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tower::ServiceExt;

async fn mcp_handler(headers: HeaderMap, body: Bytes) -> Response {
    let msg: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let method = msg["method"].as_str().unwrap_or("");

    // Notifications (no id) just get accepted.
    let Some(id) = msg.get("id").cloned() else {
        return StatusCode::ACCEPTED.into_response();
    };

    // Everything after initialize must carry session + negotiated version.
    let has_ctx =
        headers.contains_key("mcp-session-id") && headers.contains_key("mcp-protocol-version");
    let err = |id: Value, m: &str| {
        Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":-32600,"message":m}})).into_response()
    };

    match method {
        "initialize" => {
            let result = json!({
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "mock-http", "version": "0.0.0" }
            });
            let mut hm = HeaderMap::new();
            hm.insert("mcp-session-id", HeaderValue::from_static("sess-abc"));
            (hm, Json(json!({"jsonrpc":"2.0","id":id,"result":result}))).into_response()
        }
        "tools/list" => {
            if !has_ctx {
                return err(id, "missing session/version headers");
            }
            Json(json!({
                "jsonrpc":"2.0","id":id,
                "result":{"tools":[{"name":"echo","inputSchema":{"type":"object"}}]}
            }))
            .into_response()
        }
        "tools/call" => {
            if !has_ctx {
                return err(id, "missing session/version headers");
            }
            Json(json!({
                "jsonrpc":"2.0","id":id,
                "result":{"content":[{"type":"text","text":"ok"}],"isError":false}
            }))
            .into_response()
        }
        _ => Json(json!({"jsonrpc":"2.0","id":id,"result":{}})).into_response(),
    }
}

/// Start the mock MCP server on a random port; returns its `/mcp/v1` URL.
async fn start_mock() -> String {
    let app = Router::new().route("/mcp/v1", post(mcp_handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}/mcp/v1")
}

#[tokio::test]
async fn tool_roundtrip_via_http() {
    let url = start_mock().await;

    let config = GatewayConfig {
        rate_limit: RateLimitConfig {
            enabled: false,
            ..Default::default()
        },
        // Upstream is an in-process mock on 127.0.0.1. Loopback is allowed by
        // default (SSRF guard still blocks private/link-local), so no opt-out.
        upstreams: vec![UpstreamConfig {
            name: "http".into(),
            transport: TransportConfig::Http {
                url,
                auth_token: None,
            },
        }],
        routes: vec![RouteRule {
            path: "/v1/echo".into(),
            method: "POST".into(),
            upstream: "http".into(),
            target: RouteTarget::Tool {
                name: "echo".into(),
                stream: false,
            },
        }],
        ..Default::default()
    };

    let app = Gateway::build(config)
        .await
        .expect("build gateway")
        .router();

    let req = Request::builder()
        .method("POST")
        .uri("/v1/echo")
        .header("content-type", "application/json")
        .body(Body::from(json!({}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    // Reaching OK proves: initialize captured session+version, and the
    // tools/call carried them (the mock 400s/errors otherwise).
    assert_eq!(status, StatusCode::OK, "http dispatch failed; body={body}");
    assert_eq!(body["isError"], Value::Bool(false), "tool errored: {body}");
}
