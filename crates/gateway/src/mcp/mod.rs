//! MCP client side of the gateway.
//!
//! The gateway is an **MCP client**: it speaks JSON-RPC to upstream MCP
//! servers. The wire types live in [`types`], a single transport abstraction
//! in [`transport`], the per-server session/handshake in [`client`], and a
//! name-keyed registry of live clients in [`pool`].

pub mod client;
pub mod pool;
pub mod transport;
pub mod types;

pub use client::McpClient;
pub use pool::UpstreamPool;
