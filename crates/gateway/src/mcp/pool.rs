//! Registry of live MCP clients, keyed by upstream name.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::config::UpstreamConfig;
use crate::error::Result;
use crate::mcp::{client::McpClient, transport};
use crate::Result as GwResult;

#[derive(Clone, Default)]
pub struct UpstreamPool {
    clients: HashMap<String, Arc<McpClient>>,
}

impl UpstreamPool {
    /// Build a client per configured upstream. Transports connect here; the
    /// MCP `initialize` handshake is deferred to first use (see [`McpClient`]).
    ///
    /// If any upstream fails to connect, all previously-built clients are
    /// shut down so we don't leak transports.
    ///
    /// `upstream_timeout_secs` of 0 means no timeout (not recommended in production).
    pub async fn from_config(
        upstreams: &[UpstreamConfig],
        upstream_timeout_secs: u64,
        max_response_bytes: usize,
    ) -> Result<Self> {
        let timeout = if upstream_timeout_secs > 0 {
            Some(Duration::from_secs(upstream_timeout_secs))
        } else {
            None
        };
        let mut clients: Vec<(String, Arc<McpClient>)> = Vec::with_capacity(upstreams.len());
        for up in upstreams {
            let t = match transport::connect(&up.transport, max_response_bytes).await {
                Ok(t) => t,
                Err(e) => {
                    // Rollback: shut down every client we already built.
                    for (_, c) in &clients {
                        let _ = c.shutdown_transport().await;
                    }
                    return Err(e);
                }
            };
            let client =
                McpClient::new(up.name.clone(), Arc::from(t)).with_request_timeout(timeout);
            clients.push((up.name.clone(), Arc::new(client)));
        }
        let map = clients.into_iter().collect();
        Ok(Self { clients: map })
    }

    /// Register a client directly. Useful for composing pools by hand (e.g.
    /// tests with a mock transport, or hosts wiring clients programmatically).
    pub fn insert(&mut self, name: impl Into<String>, client: Arc<McpClient>) {
        self.clients.insert(name.into(), client);
    }

    pub fn get(&self, name: &str) -> Option<Arc<McpClient>> {
        self.clients.get(name).cloned()
    }

    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.clients.keys()
    }

    /// Gracefully shut down every upstream transport in the pool.
    /// Best-effort: errors are logged and ignored so all upstreams get a chance.
    pub async fn shutdown_all(&self) -> GwResult<()> {
        for client in self.clients.values() {
            if let Err(e) = client.shutdown_transport().await {
                tracing::warn!(error = %e, "upstream shutdown error");
            }
        }
        Ok(())
    }
}
