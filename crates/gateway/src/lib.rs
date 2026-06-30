//! # AIRP-Gateway
//!
//! Generic, high-performance protocol bridge that links a **frontend**
//! (HTTP / SSE) to one or more **MCP servers** (such as `AIRP-MCP-Server`),
//! which in turn reach `Agent` / inference backends.
//!
//! ```text
//! frontend  ->  AIRP-Gateway  ->  AIRP-MCP-Server  ->  Agent / backend
//! ```
//!
//! ## Design goals
//! - **Pure protocol bridge.** No roleplay / domain logic lives here. The
//!   gateway authenticates, rate-limits, and forwards. Domain behaviour is
//!   owned by the MCP server it talks to.
//! - **Generic & embeddable.** Library-first. Drop this crate into AIRP-Core
//!   or any other project; the binary is a thin runner.
//! - **Transport-agnostic upstream.** Talk to MCP servers over **stdio** or
//!   **HTTP** (streamable) behind a single [`mcp::transport::McpTransport`]
//!   trait, so new transports plug in without touching the bridge.
//! - **Portable architecture.** Layers are kept thin and dependency-light so
//!   the design ports cleanly to a future language if one is ever preferred.
//!
//! ## Layering
//! - [`config`]   declarative, layered configuration (default -> file -> env)
//! - [`server`]   frontend-facing axum HTTP/SSE surface + middleware
//! - [`bridge`]   request -> MCP operation -> response translation
//! - [`mcp`]      MCP client, session, and pluggable transports

pub mod bridge;
pub mod config;
pub mod error;
pub mod mcp;
pub mod server;
pub mod telemetry;

// Optional State-Protocol AgentBus adapter frontend, gated behind the
// `agentbus` feature (default OFF). Hosts that want the `/airp/dispatch` +
// `/airp/stream` SSE surface enable it (see `examples/agentbus_sse.rs`). Pure
// adapter — depends only on the public gateway core, does not modify `bridge`
// or `server`. Per ADR-007: project-specific code is opt-in, never compiled for
// pure-bridge consumers.
#[cfg(feature = "agentbus")]
pub mod agentbus;

pub use config::{GatewayConfig, RouteRule, RouteTarget, TransportConfig, UpstreamConfig};
pub use error::{GatewayError, Result};
pub use server::{Gateway, GatewayState};

// Frontend-agnostic core, re-exported so third parties can build their own
// frontend (any protocol) on top of the shared bridge + MCP client layer.
pub use bridge::{Bridge, DispatchOutcome};
pub use mcp::transport::{connect as connect_transport, McpTransport};
pub use mcp::{McpClient, UpstreamPool};
