//! # AIRP State-Protocol AgentBus adapter
//!
//! An **optional frontend** that exposes the gateway core (`GatewayState` +
//! `Bridge`) over the State-Protocol `Envelope` wire format, transported on
//! SSE. This is the integration surface AIRP-State-Protocol's UI talks to.
//!
//! Per ADR-007 ("the adapter layer is not part of the core; it is an optional
//! crate/example on top of it"), this module:
//! - depends only on already-public exports (`GatewayState`, `Bridge`,
//!   `DispatchOutcome`) and existing dependencies (`axum`, `tokio`,
//!   `futures-util`, `serde`);
//! - does **not** modify `bridge/mod.rs` or `server/mod.rs`;
//! - is mounted by a host that owns process startup (see
//!   `examples/agentbus_sse.rs`).
//!
//! ## Surface
//! - `POST /airp/dispatch` — body is a JSON `Envelope` (upstream: hello /
//!   intent / subscribe / ack)
//! - `GET  /airp/stream`   — `text/event-stream`; each `data:` line is a JSON
//!   `Envelope` (downstream: blueprint / state / manifest / error)
//!
//! ## Mapping (the only designed part)
//! - `intent name=X params=P` → `Bridge::match_route("POST", "/v1/X")` →
//!   `dispatch` → MCP result → `state op:set` on the resolved scope.
//! - scope resolution: `intent.source` wins; else the per-intent
//!   `fallback_scopes` table; else `default_scope`; else an `error` envelope.
//! - `hello` → emit configured initial blueprint / manifests / per-scope state.
//! - `subscribe scopes=[...]` → filter this connection's SSE stream.
//!
//! See `envelope.rs` for the wire types, `config.rs` for the mapping table,
//! `bus.rs` for the broadcast fan-out, and `handlers.rs` for the axum routes.

pub mod bus;
pub mod config;
pub mod envelope;
pub mod handlers;

use std::sync::Arc;

use axum::routing::{get, post};
use axum::Router;

use crate::server::GatewayState;

pub use bus::Bus;
pub use config::{AdapterConfig, IntentScopeFallback, DEFAULT_ROUTE_PREFIX};
pub use envelope::{
    Blueprint, BlueprintOp, Body, Envelope, EnvelopeError, Manifest, ManifestOp, PatchOp, StateOp,
    ENVELOPE_VERSION, SRC_GATEWAY,
};
pub use handlers::AdapterState;

/// Build the adapter state shared by both handlers.
///
/// The host calls this with an already-built [`GatewayState`] (the gateway
/// core) and an [`AdapterConfig`] (the State-Protocol mapping table), then
/// mounts [`adapter_router`] onto its axum app — typically merged with the
/// gateway's own `Gateway::router()` so `/health`, `/version`, and the
/// bridge's catch-all dispatch remain available alongside `/airp/*`.
pub fn build_state(gateway: Arc<GatewayState>, config: AdapterConfig) -> Arc<AdapterState> {
    Arc::new(AdapterState {
        gateway,
        bus: Bus::new(),
        config,
    })
}

/// The axum router for the State-Protocol SSE surface.
///
/// Mount this as a sub-router and `.with_state(adapter_state)` it, or merge
/// it with the gateway's router. The routes are:
/// - `POST /dispatch` (relative; mount under `/airp` → `/airp/dispatch`)
/// - `GET  /stream`
pub fn adapter_router() -> Router<Arc<AdapterState>> {
    Router::new()
        .route("/dispatch", post(handlers::dispatch))
        .route("/stream", get(handlers::stream))
}
