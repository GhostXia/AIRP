//! Minimal **custom frontend** example — no `agentbus`, no built-in HTTP server.
//!
//! Demonstrates the core extension point (see `docs/CUSTOMIZATION.md` §3): build
//! the shared [`GatewayState`] and drive the same [`Bridge`] directly from your
//! own frontend code. The built-in axum HTTP surface is just *one* frontend;
//! here we skip it entirely and call `bridge.dispatch` ourselves.
//!
//! Run it against this repo's bundled mock MCP server:
//! ```text
//! cargo build --example mock_mcp_stdio
//! # then point AIRP_MCP_BIN at the built binary, e.g.:
//! #   target/debug/examples/mock_mcp_stdio    (or .exe on Windows)
//! AIRP_MCP_BIN=target/debug/examples/mock_mcp_stdio cargo run --example custom_frontend
//! ```

use airp_gateway::config::{TransportConfig, UpstreamConfig};
use airp_gateway::{DispatchOutcome, GatewayConfig, GatewayState, RouteRule, RouteTarget};
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Any MCP server works; default to the bundled mock's conventional name.
    let command = std::env::var("AIRP_MCP_BIN").unwrap_or_else(|_| "mock_mcp_stdio".to_string());

    let config = GatewayConfig {
        upstreams: vec![UpstreamConfig {
            name: "backend".into(),
            transport: TransportConfig::Stdio {
                command,
                args: vec![],
                cwd: None,
            },
        }],
        routes: vec![RouteRule {
            path: "/v1/echo".into(),
            method: "POST".into(),
            upstream: "backend".into(),
            target: RouteTarget::Tool {
                name: "echo".into(),
                stream: false,
            },
        }],
        ..Default::default()
    };

    // Build the shared core only — NOT the built-in axum frontend.
    let state = GatewayState::build(config).await?;

    // --- Your custom frontend lives here ---
    // Whatever protocol your frontend speaks (CLI, WS, gRPC, …), the pattern is
    // the same: resolve a route, then dispatch through the shared bridge.
    let rule = state
        .bridge
        .match_route("POST", "/v1/echo")
        .expect("route configured above")
        .clone();

    match state
        .bridge
        .dispatch(&rule, json!({"hello": "world"}))
        .await?
    {
        DispatchOutcome::Json(result) => println!("MCP result: {result}"),
        DispatchOutcome::Stream => println!("(streaming dispatch — not shown in this example)"),
    }

    Ok(())
}
