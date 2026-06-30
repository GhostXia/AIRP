//! Example: AIRP State-Protocol AgentBus adapter over SSE.
//!
//! Mounts the optional `agentbus` frontend onto the gateway core and serves
//! it alongside the built-in HTTP surface. This is the binary AIRP-State-Protocol
//! UI connects to for the minimal closed loop (see `docs/` and the requirement
//! doc §8).
//!
//! ## What it does
//! 1. Builds `GatewayState` with one stdio upstream (the `mock_mcp_stdio`
//!    example) and one route `POST /v1/chat.send` → tool `echo`.
//! 2. Builds the adapter state with a `chat.send → w-chat` scope fallback
//!    and an initial blueprint + empty chat state.
//! 3. Serves a merged router:
//!    - `GET  /health`, `GET /version`           (gateway core, public)
//!    - `POST /v1/chat.send` + fallback          (gateway core catch-all, auth'd)
//!    - `POST /airp/dispatch`, `GET /airp/stream` (adapter, auth'd)
//!
//! ## Run
//! ```sh
//! # Build the mock MCP server first (the stdio upstream).
//! cargo build --example mock_mcp_stdio
//!
//! # Run the adapter example.
//! AIRP_MCP_BIN=target/debug/examples/mock_mcp_stdio \
//!   cargo run --example agentbus_sse
//!
//! # Then point the UI's SSEBus at http://127.0.0.1:8080/airp
//! ```
//!
//! Adjust bind / upstream / routes via `AIRP_BIND`, `AIRP_MCP_BIN` env vars.

use axum::Router;

use airp_gateway::agentbus::{adapter_router, build_state, AdapterConfig, IntentScopeFallback};
use airp_gateway::config::{
    RateLimitConfig, RouteRule, RouteTarget, TransportConfig, UpstreamConfig,
};
use airp_gateway::{Gateway, GatewayConfig, Result};

#[tokio::main]
async fn main() -> Result<()> {
    airp_gateway::telemetry::init();

    let bind = std::env::var("AIRP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let mcp_bin = std::env::var("AIRP_MCP_BIN")
        .unwrap_or_else(|_| "target/debug/examples/mock_mcp_stdio".into());

    // --- Gateway core config: one stdio upstream, one chat.send route. ---
    let upstream = UpstreamConfig {
        name: "chat-mcp".into(),
        transport: TransportConfig::Stdio {
            command: mcp_bin.clone(),
            args: vec![],
            cwd: None,
        },
    };
    let route = RouteRule {
        path: "/v1/chat.send".into(),
        method: "POST".into(),
        upstream: "chat-mcp".into(),
        target: RouteTarget::Tool {
            name: "echo".into(),
            stream: false,
        },
    };
    let gateway_config = GatewayConfig {
        bind: bind.clone(),
        // Open in the example; set AIRP_ACCESS_KEY to require a bearer.
        access_key: std::env::var("AIRP_ACCESS_KEY").ok(),
        rate_limit: RateLimitConfig {
            enabled: false,
            ..Default::default()
        },
        upstreams: vec![upstream],
        routes: vec![route],
        // Allow the mock binary path (basename match).
        allowed_commands: vec!["mock_mcp_stdio".into()],
        ..Default::default()
    };

    // --- Build gateway core + adapter on top. ---
    let gateway = Gateway::build(gateway_config).await?;
    let gateway_state = gateway.state();

    let adapter_config = AdapterConfig {
        default_scope: Some("w-default".into()),
        fallback_scopes: vec![IntentScopeFallback {
            intent: "chat.send".into(),
            scope: "w-chat".into(),
        }],
        route_prefix: "/v1/".into(),
        initial_blueprint: Some(airp_gateway::agentbus::Blueprint {
            version: "bp-1".into(),
            layout: serde_json::json!({
                "type": "dock",
                "areas": [{"id": "main", "widgets": ["w-chat"]}]
            }),
            widgets: vec![serde_json::json!({
                "id": "w-chat", "type": "core.chat", "state": "w-chat"
            })],
        }),
        initial_manifests: vec![],
        initial_state: std::collections::HashMap::from([(
            "w-chat".into(),
            serde_json::json!({"messages": []}),
        )]),
    };
    let adapter_state = build_state(gateway_state, adapter_config);

    // Clone out the pool before adapter_state is moved into the router.
    let pool_for_shutdown = adapter_state.gateway.pool.clone();

    // --- Merge routers: gateway core + adapter. ---
    // The gateway's own router owns the catch-all dispatch (auth'd) and the
    // public health/version routes. We merge the adapter sub-router (scoped
    // to /airp) on top. Both share their respective state types via with_state.
    let gateway_router = gateway.router();
    let adapter = adapter_router().with_state(adapter_state);

    // Nest the adapter under /airp so its /dispatch, /stream become
    // /airp/dispatch, /airp/stream. The gateway's own RequestBodyLimit +
    // CORS layers do not cross into nested routes, but the adapter envelope
    // bodies are tiny so the default axum limit (2 MiB) is plenty.
    let app: Router = gateway_router.nest("/airp", adapter);

    // --- Serve. ---
    let addr: std::net::SocketAddr = bind
        .parse()
        .map_err(|e| airp_gateway::GatewayError::Config(format!("invalid bind address: {e}")))?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("agentbus-sse example listening on http://{addr}");
    tracing::info!("UI surface:");
    tracing::info!("  POST http://{addr}/airp/dispatch   (upstream Envelope)");
    tracing::info!("  GET  http://{addr}/airp/stream     (downstream SSE Envelopes)");
    tracing::info!("upstream: stdio `{mcp_bin}` (tool `echo`)");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        shutdown_signal().await;
        tracing::info!("shutting down upstream transports");
        let _ = pool_for_shutdown.shutdown_all().await;
    })
    .await?;

    Ok(())
}

/// Re-implemented here rather than re-exported to keep the example self-contained.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("install Ctrl-C handler");
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    tracing::info!("shutdown signal received");
}
