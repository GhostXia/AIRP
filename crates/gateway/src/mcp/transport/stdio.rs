//! stdio MCP transport.
//!
//! Launches the MCP server as a child process and exchanges newline-delimited
//! JSON-RPC messages over its stdin/stdout. A background reader task matches
//! responses to in-flight requests by `id`.
//!
//! ## Robustness
//! - When the child exits (EOF on stdout), the reader task drains all pending
//!   oneshot senders with an error so callers don't hang forever.
//! - A graceful shutdown sequence is provided: close stdin → wait for exit →
//!   kill on timeout. `Drop` performs a best-effort kill.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin};
use tokio::sync::{broadcast, Mutex};

use crate::error::{GatewayError, Result};
use crate::mcp::transport::McpTransport;
use crate::mcp::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};

type Pending = Arc<Mutex<HashMap<String, tokio::sync::oneshot::Sender<JsonRpcResponse>>>>;

/// How long to wait for the child to exit after closing stdin before killing it.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// Maximum line length from child stdout. Lines exceeding this are treated as
/// an error (prevents a malicious/buggy upstream from OOM'ing the gateway).
const MAX_LINE_BYTES: usize = 1024 * 1024; // 1 MiB

pub struct StdioTransport {
    stdin: Mutex<Option<ChildStdin>>,
    pending: Pending,
    /// Broadcast channel for server-initiated notifications (no `id`).
    /// Receivers can subscribe via [`Self::subscribe_notifications`].
    /// Capacity is bounded; slow consumers drop oldest messages.
    notification_tx: broadcast::Sender<JsonRpcNotification>,
    // Keep the child alive for the lifetime of the transport.
    child: Mutex<Child>,
}

impl StdioTransport {
    pub async fn connect(command: &str, args: &[String], cwd: Option<&str>) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| GatewayError::Transport(format!("spawn `{command}`: {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| GatewayError::Transport("child stdin unavailable".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| GatewayError::Transport("child stdout unavailable".into()))?;

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        let (notification_tx, _) = broadcast::channel(64);

        // Reader task: route each response line to its waiting caller.
        // On EOF / error, drain all pending senders with an error so
        // callers don't hang forever.
        let pending_reader = pending.clone();
        let notification_tx_r = notification_tx.clone();
        tokio::spawn(async move {
            // Buffered, chunk-at-a-time read: scan each fill_buf chunk for a
            // newline (no per-byte await), enforcing MAX_LINE_BYTES across
            // chunks so a newline-less flood can't OOM the gateway.
            let mut reader = BufReader::new(stdout);
            let mut line: Vec<u8> = Vec::with_capacity(4096);
            let mut oversized = false;
            loop {
                let (consumed, have_newline) = {
                    let buf = match reader.fill_buf().await {
                        Ok(b) => b,
                        Err(_) => break, // I/O error — treat as EOF.
                    };
                    if buf.is_empty() {
                        break; // EOF
                    }
                    match buf.iter().position(|&b| b == b'\n') {
                        Some(pos) => {
                            line.extend_from_slice(&buf[..pos]);
                            (pos + 1, true)
                        }
                        None => {
                            line.extend_from_slice(buf);
                            (buf.len(), false)
                        }
                    }
                };
                reader.consume(consumed);

                if line.len() > MAX_LINE_BYTES {
                    tracing::warn!(
                        "stdio line exceeded {} bytes, killing upstream",
                        MAX_LINE_BYTES
                    );
                    drain_pending(&pending_reader, "upstream line too large").await;
                    oversized = true;
                    break;
                }

                if have_newline {
                    let line_str = String::from_utf8_lossy(&line);
                    let trimmed = line_str.trim();
                    if !trimmed.is_empty() {
                        // Try to parse as a response (has `id`).
                        if let Ok(resp) = serde_json::from_str::<JsonRpcResponse>(trimmed) {
                            let key = id_key(&resp.id);
                            if let Some(tx) = pending_reader.lock().await.remove(&key) {
                                let _ = tx.send(resp);
                            }
                        }
                        // Otherwise try to parse as a notification (no `id`).
                        else if let Ok(note) =
                            serde_json::from_str::<JsonRpcNotification>(trimmed)
                        {
                            let _ = notification_tx_r.send(note);
                        }
                        // Unparseable line — ignore.
                    }
                    line.clear();
                }
            }

            // On EOF/error, drain pending so callers don't hang (unless we
            // already drained due to an oversized line).
            if !oversized {
                drain_pending(&pending_reader, "upstream process exited").await;
            }
        });

        Ok(Self {
            stdin: Mutex::new(Some(stdin)),
            pending,
            notification_tx,
            child: Mutex::new(child),
        })
    }

    /// Subscribe to server-initiated notifications (e.g. `notifications/progress`).
    /// Late subscribers only see notifications emitted after subscription.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<JsonRpcNotification> {
        self.notification_tx.subscribe()
    }

    /// Graceful shutdown: close stdin (signals the child to exit), wait up to
    /// [`SHUTDOWN_TIMEOUT`], then kill if still alive.
    pub async fn shutdown(&self) {
        // Close stdin to signal the child.
        {
            let mut stdin_guard = self.stdin.lock().await;
            let _ = stdin_guard.take();
        }

        // Wait for the child to exit, with a timeout.
        let exited = {
            let mut child = self.child.lock().await;
            match tokio::time::timeout(SHUTDOWN_TIMEOUT, child.wait()).await {
                Ok(Ok(_status)) => true,
                Ok(Err(_)) => true, // error querying — assume gone
                Err(_) => false,    // timed out
            }
        };

        if !exited {
            let mut child = self.child.lock().await;
            let _ = child.start_kill();
        }
    }

    async fn write_line(&self, payload: &str) -> Result<()> {
        let mut stdin_guard = self.stdin.lock().await;
        let stdin = stdin_guard
            .as_mut()
            .ok_or_else(|| GatewayError::Transport("upstream stdin already closed".into()))?;
        stdin
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| GatewayError::Transport(e.to_string()))?;
        stdin
            .write_all(b"\n")
            .await
            .map_err(|e| GatewayError::Transport(e.to_string()))?;
        stdin
            .flush()
            .await
            .map_err(|e| GatewayError::Transport(e.to_string()))?;
        Ok(())
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // Best-effort kill on drop. We can't do async here, so just start_kill.
        // The child's stdin was already taken or will be dropped here.
        let mut child = match self.child.try_lock() {
            Ok(c) => c,
            Err(_) => return, // someone else holds the lock — best effort
        };
        let _ = child.start_kill();
    }
}

/// Send a synthetic JSON-RPC error to every in-flight request and clear the
/// pending map, so callers awaiting a response don't hang when the reader stops.
async fn drain_pending(pending: &Pending, message: &str) {
    let remaining = {
        let mut map = pending.lock().await;
        std::mem::take(&mut *map)
    };
    for (_, tx) in remaining {
        let _ = tx.send(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: serde_json::Value::Null,
            result: None,
            error: Some(crate::mcp::types::JsonRpcError {
                code: -32000,
                message: message.to_string(),
                data: None,
            }),
        });
    }
}

fn id_key(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn request(&self, req: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let key = id_key(&req.id);
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.pending.lock().await.insert(key.clone(), tx);

        let line = serde_json::to_string(&req)?;
        if let Err(e) = self.write_line(&line).await {
            self.pending.lock().await.remove(&key);
            return Err(e);
        }

        rx.await
            .map_err(|_| GatewayError::Transport("upstream closed before responding".into()))
    }

    async fn notify(&self, note: JsonRpcNotification) -> Result<()> {
        let line = serde_json::to_string(&note)?;
        self.write_line(&line).await
    }

    async fn shutdown(&self) {
        StdioTransport::shutdown(self).await;
    }
}
