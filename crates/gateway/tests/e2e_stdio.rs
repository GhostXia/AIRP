//! Real cross-process end-to-end test over stdio.
//!
//! Launches an actual MCP server as a child process and drives it through the
//! full gateway stack. Verifies what a mock cannot: subprocess spawn,
//! newline-delimited JSON-RPC framing over real pipes, the live MCP
//! `initialize` handshake, and a real `tools/list` + `tools/call`.
//!
//! Server-agnostic by design (the gateway binds to no project): it discovers
//! whatever tool the server exposes, then routes a frontend request to it.
//! CI points `AIRP_MCP_BIN` at this crate's own `mock_mcp_stdio` example, so the
//! test depends on nothing external. Point it at any MCP server to interop-test.
//!
//! Skipped (no-op pass) when `AIRP_MCP_BIN` is unset.

use airp_gateway::config::{RateLimitConfig, TransportConfig, UpstreamConfig};
use airp_gateway::{Gateway, GatewayConfig, RouteRule, RouteTarget};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

fn stdio_upstream(bin: &str, data_dir: &str) -> UpstreamConfig {
    UpstreamConfig {
        name: "srv".into(),
        transport: TransportConfig::Stdio {
            command: bin.into(),
            // Args suit a real `airp-mcp`; the mock example ignores argv.
            args: vec!["mcp".into(), "--data-dir".into(), data_dir.into()],
            cwd: None,
        },
    }
}

fn base_config(bin: &str, data_dir: &str, routes: Vec<RouteRule>) -> GatewayConfig {
    GatewayConfig {
        rate_limit: RateLimitConfig {
            enabled: false,
            ..Default::default()
        },
        upstreams: vec![stdio_upstream(bin, data_dir)],
        routes,
        ..Default::default()
    }
}

#[tokio::test]
async fn tool_roundtrip_via_real_stdio_server() {
    let Ok(bin) = std::env::var("AIRP_MCP_BIN") else {
        eprintln!("AIRP_MCP_BIN not set; skipping real stdio e2e");
        return;
    };

    let data_dir = std::env::temp_dir().join(format!("airp-gw-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&data_dir).unwrap();
    let data_dir = data_dir.to_string_lossy().into_owned();

    // --- Discover the server's tools over a real subprocess. ---
    // Proves: spawn + NDJSON framing + initialize handshake + tools/list.
    let discover = Gateway::build(base_config(&bin, &data_dir, vec![]))
        .await
        .expect("build gateway (discover)");
    let client = discover.state().pool.get("srv").expect("upstream present");
    let listed = client
        .list_tools()
        .await
        .expect("tools/list over real stdio (initialize handshake must succeed)");
    let names: Vec<String> = listed["tools"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| t["name"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    eprintln!("server exposes {} tools: {names:?}", names.len());
    let tool = names
        .first()
        .cloned()
        .expect("server should expose at least one tool");

    // --- Full chain through the router to the discovered tool. ---
    // Proves: frontend HTTP -> bridge -> McpClient -> real subprocess -> result.
    let route = RouteRule {
        path: "/v1/call".into(),
        method: "POST".into(),
        upstream: "srv".into(),
        target: RouteTarget::Tool {
            name: tool.clone(),
            stream: false,
        },
    };
    let app = Gateway::build(base_config(&bin, &data_dir, vec![route]))
        .await
        .expect("build gateway (dispatch)")
        .router();

    let req = Request::builder()
        .method("POST")
        .uri("/v1/call")
        .header("content-type", "application/json")
        .body(Body::from(json!({}).to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();

    assert_eq!(
        status,
        StatusCode::OK,
        "dispatch `{tool}` failed; body={body}"
    );
    assert_ne!(body["isError"], Value::Bool(true), "tool errored: {body}");

    let _ = std::fs::remove_dir_all(&data_dir);
}
