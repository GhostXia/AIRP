//! Minimal mock MCP server over stdio — for the gateway's own cross-process
//! e2e test, so verification depends on **no external project** (the gateway
//! binds to nothing).
//!
//! Speaks newline-delimited JSON-RPC 2.0, just enough to exercise the gateway's
//! stdio transport: `initialize`, `notifications/initialized` (ignored),
//! `tools/list` (one tool `echo`), `tools/call` (echoes its arguments).
//! This is NOT a real MCP implementation.

use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };

        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();

        let result = match method {
            "initialize" => Some(serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "mock-mcp-stdio", "version": "0.0.0" }
            })),
            "tools/list" => Some(serde_json::json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echoes the call arguments back.",
                    "inputSchema": { "type": "object" }
                }]
            })),
            "tools/call" => {
                let args = msg
                    .pointer("/params/arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                Some(serde_json::json!({
                    "content": [{ "type": "text", "text": "ok" }],
                    "structuredContent": args,
                    "isError": false
                }))
            }
            // Notifications (no id) and anything else: no response.
            _ => None,
        };

        // Only requests (with an id) get a response; notifications do not.
        if let (Some(id), Some(result)) = (id, result) {
            let resp = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
            if writeln!(out, "{}", serde_json::to_string(&resp).unwrap()).is_err() {
                break;
            }
            let _ = out.flush();
        }
    }
}
