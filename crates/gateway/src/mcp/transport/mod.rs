//! Transport abstraction for talking to an upstream MCP server.
//!
//! A single trait, [`McpTransport`], hides whether the server is reached over
//! a stdio child process or streamable HTTP. The bridge and client never
//! branch on transport kind — add a new transport by implementing this trait.

use async_trait::async_trait;

use crate::config::TransportConfig;
use crate::error::Result;
use crate::mcp::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

pub mod http;
pub mod stdio;

/// A bidirectional JSON-RPC channel to one MCP server.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a request and await its matching response.
    async fn request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse>;

    /// Fire a notification (no response expected).
    async fn notify(&self, note: JsonRpcNotification) -> Result<()>;

    /// Gracefully shut down the transport (e.g. close stdin, wait for child exit).
    /// Default is a no-op; transports that own resources should override.
    async fn shutdown(&self) {}

    // TODO(streaming): a `request_stream` returning a `Stream<Item = ServerEvent>`
    // for tools whose results arrive incrementally (mapped to frontend SSE).
}

/// Construct a transport from its declarative config.
///
/// `max_response_bytes` is applied to HTTP transports to cap the upstream
/// response body size. stdio transports do their own line-by-line framing
/// and are not subject to this limit (the reader task drains on EOF).
pub async fn connect(
    cfg: &TransportConfig,
    max_response_bytes: usize,
) -> Result<Box<dyn McpTransport>> {
    match cfg {
        TransportConfig::Stdio { command, args, cwd } => {
            let t = stdio::StdioTransport::connect(command, args, cwd.as_deref()).await?;
            Ok(Box::new(t))
        }
        TransportConfig::Http { url, auth_token } => {
            let t = http::HttpTransport::with_max_response(
                url,
                auth_token.clone(),
                max_response_bytes,
            )?;
            Ok(Box::new(t))
        }
    }
}
