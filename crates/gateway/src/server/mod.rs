//! Frontend-facing server: builds the axum app, wires middleware, and runs it.

pub mod handlers;
pub mod middleware;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::routing::get;
use axum::Router;

use crate::bridge::Bridge;
use crate::config::GatewayConfig;
use crate::error::{GatewayError, Result};
use crate::mcp::UpstreamPool;

/// Shared application state handed to every handler.
pub struct GatewayState {
    pub config: GatewayConfig,
    pub bridge: Bridge,
    pub pool: Arc<UpstreamPool>,
}

impl GatewayState {
    /// Connect upstream transports, assemble the bridge, and return shared state.
    ///
    /// This is the reusable core. A third party that wants a **different frontend**
    /// (gRPC, WebSocket, AIRP-State-Protocol's AgentBus, a custom protocol — not the
    /// built-in axum HTTP surface) can build state here and dispatch via
    /// [`GatewayState::bridge`] without using the [`Gateway`] server at all.
    pub async fn build(config: GatewayConfig) -> Result<Arc<Self>> {
        // Security gate: reject disallowed stdio commands before spawning anything.
        config.validate()?;
        let pool = Arc::new(
            UpstreamPool::from_config(
                &config.upstreams,
                config.upstream_timeout_secs,
                config.max_response_bytes,
            )
            .await?,
        );
        let bridge = Bridge::new(pool.clone(), config.routes.clone());
        Ok(Arc::new(GatewayState {
            config,
            bridge,
            pool,
        }))
    }

    pub fn bridge_upstreams(&self) -> Vec<String> {
        self.pool.names().cloned().collect()
    }
}

/// The built-in HTTP/SSE frontend over a [`GatewayState`].
///
/// This is *one* frontend. The core ([`GatewayState`] + [`Bridge`] + the MCP
/// client layer) is frontend-agnostic — see [`GatewayState::build`] to drive it
/// from any other protocol.
pub struct Gateway {
    state: Arc<GatewayState>,
}

impl Gateway {
    /// Build the gateway with the default HTTP frontend.
    pub async fn build(config: GatewayConfig) -> Result<Self> {
        Ok(Self {
            state: GatewayState::build(config).await?,
        })
    }

    /// Wrap pre-built shared state (e.g. constructed via [`GatewayState::build`]
    /// and shared with a custom frontend) in the default HTTP frontend.
    pub fn from_state(state: Arc<GatewayState>) -> Self {
        Self { state }
    }

    /// Access the shared state: config, [`Bridge`], and upstream pool. Lets a
    /// host mount its own routes/handlers that dispatch through the same bridge.
    pub fn state(&self) -> Arc<GatewayState> {
        self.state.clone()
    }

    /// Construct the axum router with middleware applied.
    pub fn router(&self) -> Router {
        let state = self.state.clone();

        // Authenticated API surface (everything except the public routes below).
        let api =
            Router::new()
                .fallback(handlers::dispatch)
                .layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    middleware::auth,
                ));

        // Public, unauthenticated routes.
        let public = Router::new()
            .route("/health", get(handlers::health))
            .route("/version", get(handlers::version));

        let mut app = public
            .merge(api)
            .layer(tower_http::limit::RequestBodyLimitLayer::new(
                self.state.config.max_request_bytes,
            ))
            .layer(middleware::cors_layer(&self.state.config.cors));

        if self.state.config.rate_limit.enabled {
            // Per-IP token bucket shared across all routes.
            let conf = Arc::new(
                tower_governor::governor::GovernorConfigBuilder::default()
                    .per_second(self.state.config.rate_limit.per_second)
                    .burst_size(self.state.config.rate_limit.burst)
                    .finish()
                    .expect("valid governor config"),
            );
            app = app.layer(tower_governor::GovernorLayer { config: conf });
        }

        app.with_state(state)
    }

    /// Bind and serve until the process is terminated or a shutdown signal
    /// (SIGTERM / SIGINT / Ctrl-C) is received. On shutdown, gracefully drain
    /// in-flight requests and shut down all upstream transports.
    pub async fn run(self) -> Result<()> {
        let addr: SocketAddr = self
            .state
            .config
            .bind
            .parse()
            .map_err(|e| GatewayError::Config(format!("invalid bind address: {e}")))?;

        // Warn loudly about the dangerous combo: reachable off-host with no auth.
        if !addr.ip().is_loopback() {
            if self.state.config.access_key.is_none() {
                tracing::warn!(
                    %addr,
                    "airp-gateway bound to a NON-LOOPBACK address WITHOUT access_key — \
                     it is exposed to the network unauthenticated; set access_key or bind 127.0.0.1"
                );
            } else {
                tracing::info!(%addr, "airp-gateway exposed off-host (auth enabled)");
            }
        }

        let app = self.router();
        let listener = tokio::net::TcpListener::bind(addr).await?;
        tracing::info!("airp-gateway listening on http://{addr}");

        let pool = self.state.pool.clone();
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            tracing::info!("airp-gateway shutting down upstream transports");
            if let Err(e) = pool.shutdown_all().await {
                tracing::warn!(error = %e, "error during upstream shutdown");
            }
        })
        .await?;
        Ok(())
    }
}

/// Wait for a termination signal (SIGTERM / SIGINT on Unix, Ctrl-C on Windows).
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
