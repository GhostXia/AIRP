//! Streamable-HTTP MCP transport.
//!
//! Implements the request/response path against an MCP server's HTTP endpoint
//! (e.g. an `/mcp/v1` mounted by rmcp's streamable-http server). Handles the
//! parts the spec requires of a client:
//! - captures the `Mcp-Session-Id` returned on `initialize` and echoes it on
//!   every subsequent request;
//! - captures the *negotiated* `protocolVersion` and sends it as the
//!   `MCP-Protocol-Version` header on subsequent requests;
//! - accepts either `application/json` or `text/event-stream` (SSE) responses.
//!
//! ## Robustness
//! - The `reqwest::Client` is built with configurable connect/read timeouts.
//! - `shutdown` is a no-op (HTTP has no persistent resource to close).

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::{GatewayError, Result};
use crate::mcp::transport::McpTransport;
use crate::mcp::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

/// Default connect timeout for the HTTP client.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default request timeout (connect + read).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

pub struct HttpTransport {
    client: reqwest::Client,
    url: String,
    auth_token: Option<String>,
    /// Set from the `Mcp-Session-Id` response header on `initialize`.
    session_id: Mutex<Option<String>>,
    /// Negotiated protocol version (server's reply to `initialize`).
    protocol_version: Mutex<Option<String>>,
    /// Maximum response body size in bytes. Bodies exceeding this are rejected.
    max_response_bytes: usize,
}

impl HttpTransport {
    pub fn new(url: &str, auth_token: Option<String>) -> Result<Self> {
        Self::with_max_response(url, auth_token, 10 * 1024 * 1024)
    }

    /// Create with a specific `max_response_bytes` limit.
    pub fn with_max_response(
        url: &str,
        auth_token: Option<String>,
        max_response_bytes: usize,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| GatewayError::Transport(e.to_string()))?;
        Ok(Self {
            client,
            url: url.to_string(),
            auth_token,
            session_id: Mutex::new(None),
            protocol_version: Mutex::new(None),
            max_response_bytes,
        })
    }

    /// Add the standard headers (auth, content negotiation, session, version).
    /// Guards are cloned out and dropped here — none are held across `.await`.
    fn decorate(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let mut rb = rb
            .header("content-type", "application/json")
            .header("accept", "application/json, text/event-stream");
        if let Some(t) = &self.auth_token {
            rb = rb.bearer_auth(t);
        }
        if let Some(sid) = self.session_id.lock().unwrap().clone() {
            rb = rb.header("Mcp-Session-Id", sid);
        }
        if let Some(ver) = self.protocol_version.lock().unwrap().clone() {
            rb = rb.header("MCP-Protocol-Version", ver);
        }
        rb
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let rb = self.decorate(self.client.post(&self.url).json(&req));
        let resp = rb
            .send()
            .await
            .map_err(|e| GatewayError::Transport(format!("upstream HTTP request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            return Err(GatewayError::Transport(format!("upstream HTTP {status}")));
        }

        // Capture session id (sent on the initialize response) before consuming body.
        if let Some(sid) = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
        {
            *self.session_id.lock().unwrap() = Some(sid);
        }
        let is_sse = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|c| c.contains("text/event-stream"))
            .unwrap_or(false);

        // Pre-check Content-Length header to reject oversized responses before
        // allocating memory. This is a fast path; the actual body size is
        // verified after streaming read below.
        if let Some(cl) = resp
            .headers()
            .get(reqwest::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
        {
            if cl > self.max_response_bytes {
                return Err(GatewayError::ResponseTooLarge(self.max_response_bytes));
            }
        }

        // Stream the body chunk-by-chunk, accumulating with a size cap.
        // This avoids loading an unbounded response into memory.
        let mut body_bytes = Vec::new();
        let mut total: usize = 0;
        let mut stream = resp.bytes_stream();
        use futures_util::StreamExt;
        while let Some(chunk) = stream.next().await {
            let chunk =
                chunk.map_err(|e| GatewayError::Transport(format!("read upstream body: {e}")))?;
            total += chunk.len();
            if total > self.max_response_bytes {
                return Err(GatewayError::ResponseTooLarge(self.max_response_bytes));
            }
            body_bytes.extend_from_slice(&chunk);
        }
        let text = String::from_utf8(body_bytes)
            .map_err(|e| GatewayError::Transport(format!("non-utf8 upstream body: {e}")))?;

        let parsed = if is_sse {
            parse_sse(&text)?
        } else {
            serde_json::from_str::<JsonRpcResponse>(&text)
                .map_err(|e| GatewayError::Transport(format!("decode JSON-RPC: {e}")))?
        };

        // Capture the negotiated protocol version from the initialize result.
        if let Some(ver) = parsed
            .result
            .as_ref()
            .and_then(|r| r.get("protocolVersion"))
            .and_then(|v| v.as_str())
        {
            *self.protocol_version.lock().unwrap() = Some(ver.to_string());
        }

        Ok(parsed)
    }

    async fn notify(&self, note: JsonRpcNotification) -> Result<()> {
        let rb = self.decorate(self.client.post(&self.url).json(&note));
        rb.send()
            .await
            .map_err(|e| GatewayError::Transport(format!("upstream HTTP notify: {e}")))?;
        Ok(())
    }

    // HTTP transport has no persistent resource to close; shutdown is a no-op.
}

/// Extract the first JSON-RPC response from an SSE body. Concatenates the
/// `data:` lines of each event and returns the first that parses to a
/// response carrying a `result` or `error`.
fn parse_sse(text: &str) -> Result<JsonRpcResponse> {
    fn try_parse(buf: &str) -> Option<JsonRpcResponse> {
        serde_json::from_str::<JsonRpcResponse>(buf.trim())
            .ok()
            .filter(|r| r.result.is_some() || r.error.is_some())
    }

    let mut event = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            if !event.is_empty() {
                event.push('\n');
            }
            event.push_str(rest.trim_start());
        } else if line.trim().is_empty() {
            if let Some(r) = try_parse(&event) {
                return Ok(r);
            }
            event.clear();
        }
    }
    try_parse(&event)
        .ok_or_else(|| GatewayError::Transport("no JSON-RPC response in SSE stream".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_event_sse() {
        let body =
            "event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n\n";
        let resp = parse_sse(body).expect("should parse");
        assert_eq!(resp.result.unwrap()["ok"], serde_json::json!(true));
    }

    #[test]
    fn parses_multiline_data_sse() {
        let body = "data: {\"jsonrpc\":\"2.0\",\"id\":1,\ndata: \"result\":{\"v\":2}}\n\n";
        let resp = parse_sse(body).expect("should parse");
        assert_eq!(resp.result.unwrap()["v"], serde_json::json!(2));
    }

    #[test]
    fn errors_when_no_response_frame() {
        let body = "event: ping\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"x\"}\n\n";
        assert!(parse_sse(body).is_err());
    }
}
