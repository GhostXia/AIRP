//! The translation layer: frontend request -> MCP operation -> response.
//!
//! The bridge is deliberately thin and domain-agnostic. It matches an incoming
//! request against the declarative [`RouteRule`]s, forwards the JSON body to
//! the configured MCP tool/resource, and hands the result back. It contains no
//! knowledge of what any tool *means*.

use serde_json::Value;
use std::sync::Arc;

use crate::config::{RouteRule, RouteTarget};
use crate::error::{GatewayError, Result};
use crate::mcp::UpstreamPool;

/// Result of dispatching a matched route.
pub enum DispatchOutcome {
    /// A complete JSON payload to return to the frontend.
    Json(Value),
    /// A streaming response (SSE). Scaffolded; see [`Bridge::dispatch`].
    Stream,
}

pub struct Bridge {
    pool: Arc<UpstreamPool>,
    routes: Vec<RouteRule>,
}

impl Bridge {
    pub fn new(pool: Arc<UpstreamPool>, routes: Vec<RouteRule>) -> Self {
        Self { pool, routes }
    }

    /// Find a route matching the request method + path.
    pub fn match_route(&self, method: &str, path: &str) -> Option<&RouteRule> {
        self.routes
            .iter()
            .find(|r| r.path == path && r.method.eq_ignore_ascii_case(method))
    }

    /// Execute a matched route against its upstream MCP server.
    pub async fn dispatch(&self, rule: &RouteRule, body: Value) -> Result<DispatchOutcome> {
        let client = self
            .pool
            .get(&rule.upstream)
            .ok_or_else(|| GatewayError::UnknownUpstream(rule.upstream.clone()))?;

        match &rule.target {
            RouteTarget::Tool { name, stream } => {
                if *stream {
                    // TODO(streaming): use a streaming transport call and map
                    // MCP partial results onto frontend SSE events.
                    return Err(GatewayError::Unimplemented("streaming tool dispatch"));
                }
                let result = client.call_tool(name, body).await?;
                Ok(DispatchOutcome::Json(result))
            }
            RouteTarget::Resource { uri } => {
                let result = client.read_resource(uri).await?;
                Ok(DispatchOutcome::Json(result))
            }
        }
    }
}
