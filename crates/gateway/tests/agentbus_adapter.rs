//! Integration tests for the optional State-Protocol AgentBus adapter.
//!
//! Proves the minimal closed loop end-to-end (requirement §8) **without** a
//! subprocess: a mock MCP transport at the `McpTransport` seam, an in-process
//! adapter state, and a real axum `oneshot` against the adapter's `dispatch`
//! handler. The downstream `state op:set` envelope is captured off the
//! adapter's broadcast bus.
//!
//! What a passing test proves:
//! 1. `intent chat.send { text: "hello" }` is parsed from the State-Protocol
//!    `Envelope` JSON.
//! 2. The adapter maps the intent name → route path `/v1/chat.send` and
//!    matches it against the gateway core's `RouteRule`.
//! 3. `Bridge::dispatch` → `McpClient::call_tool` reaches the mock transport
//!    (initialize handshake + tools/call).
//! 4. The MCP result is wrapped as a `state op:set` envelope on scope `w-chat`
//!    and broadcast.
//! 5. A subscriber to the bus receives that envelope.

use std::sync::Arc;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt; // for `oneshot`

use airp_gateway::agentbus::{
    adapter_router, build_state, AdapterConfig, Body as EnvelopeBody, IntentScopeFallback,
};
use airp_gateway::config::{RateLimitConfig, RouteRule, RouteTarget};
use airp_gateway::mcp::types::{JsonRpcNotification, JsonRpcRequest, JsonRpcResponse};
use airp_gateway::{Gateway, GatewayConfig, GatewayState, McpTransport};

/// Mock MCP transport: initialize + tools/call. The call result carries
/// `structuredContent` so we can verify the adapter picks it over the raw
/// result.
struct MockTransport;

#[async_trait]
impl McpTransport for MockTransport {
    async fn request(&self, req: JsonRpcRequest) -> airp_gateway::Result<JsonRpcResponse> {
        let result = match req.method.as_str() {
            "initialize" => json!({
                "protocolVersion": "2025-06-18",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "mock", "version": "0.0.0" }
            }),
            "tools/call" => {
                let args = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("arguments"))
                    .cloned()
                    .unwrap_or(Value::Null);
                // Real MCP servers return { content, structuredContent, isError }.
                // We surface the echoed args under structuredContent so the
                // adapter's "prefer structuredContent" path is exercised.
                json!({
                    "content": [{ "type": "text", "text": "ok" }],
                    "structuredContent": { "reply": "echo", "received": args },
                    "isError": false
                })
            }
            other => json!({ "unhandled": other }),
        };
        Ok(JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id: req.id,
            result: Some(result),
            error: None,
        })
    }

    async fn notify(&self, _note: JsonRpcNotification) -> airp_gateway::Result<()> {
        Ok(())
    }
}

/// Build an adapter state on top of a gateway core whose single upstream is
/// the in-process `MockTransport`. The route `POST /v1/chat.send` targets the
/// mock `echo` tool. Adapter config maps `chat.send` → scope `w-chat`.
fn adapter_state() -> Arc<airp_gateway::agentbus::AdapterState> {
    let client = Arc::new(airp_gateway::McpClient::new(
        "mock",
        Arc::new(MockTransport),
    ));
    let mut pool = airp_gateway::UpstreamPool::default();
    pool.insert("mock", client);
    let pool = Arc::new(pool);

    let route = RouteRule {
        path: "/v1/chat.send".into(),
        method: "POST".into(),
        upstream: "mock".into(),
        target: RouteTarget::Tool {
            name: "echo".into(),
            stream: false,
        },
    };
    let bridge = airp_gateway::Bridge::new(pool.clone(), vec![route.clone()]);

    let config = GatewayConfig {
        rate_limit: RateLimitConfig {
            enabled: false,
            ..Default::default()
        },
        routes: vec![route],
        ..Default::default()
    };
    let gateway_state = Arc::new(GatewayState {
        config,
        bridge,
        pool,
    });
    let gateway = Gateway::from_state(gateway_state.clone());
    let _ = gateway; // gateway not served; we only need its state.

    let adapter_config = AdapterConfig {
        default_scope: Some("w-default".into()),
        fallback_scopes: vec![IntentScopeFallback {
            intent: "chat.send".into(),
            scope: "w-chat".into(),
        }],
        route_prefix: "/v1/".into(),
        ..Default::default()
    };
    build_state(gateway_state, adapter_config)
}

/// Subscribe a fresh receiver to the adapter bus, returning (conn_id, rx).
fn subscribe(
    state: &airp_gateway::agentbus::AdapterState,
) -> (
    String,
    tokio::sync::broadcast::Receiver<airp_gateway::agentbus::Envelope>,
) {
    state.bus.subscribe()
}

#[tokio::test]
async fn intent_roundtrips_to_state_patch() {
    let state = adapter_state();

    // Subscribe BEFORE dispatch so we don't miss the broadcast.
    let (conn_id, mut rx) = subscribe(&state);

    // POST an intent envelope to /dispatch.
    let envelope = json!({
        "v": 1, "id": "ui-1", "ts": 1700000000000u64, "src": "ui",
        "body": { "kind": "intent", "name": "chat.send",
                  "params": { "text": "hello" },
                  "source": "w-chat" }
    });
    let app = adapter_router().with_state(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/dispatch")
        .header("content-type", "application/json")
        .header("x-airp-conn", conn_id.to_string())
        .body(Body::from(envelope.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // The handler must have broadcast a `state op:set` on scope w-chat.
    let downstream = rx.recv().await.expect("downstream envelope");

    match downstream.body {
        EnvelopeBody::State { scope, op, patch } => {
            assert_eq!(scope, "w-chat");
            assert!(matches!(op, airp_gateway::agentbus::StateOp::Set));
            // The patch replaces the scope root with the MCP structuredContent.
            assert_eq!(patch.len(), 1);
            assert_eq!(patch[0].op, "replace");
            let value = patch[0].value.as_ref().expect("value present");
            // structuredContent echoed back by the mock.
            assert_eq!(value["reply"], "echo");
            assert_eq!(value["received"]["text"], "hello");
        }
        other => panic!("expected State envelope, got {other:?}"),
    }

    // The correlation ref should point back at the intent id.
    assert_eq!(downstream.ref_.as_deref(), Some("ui-1"));
    assert_eq!(downstream.src, "gateway");
}

#[tokio::test]
async fn unknown_intent_returns_error_envelope_downstream() {
    let state = adapter_state();
    let (_conn_id, mut rx) = subscribe(&state);

    // An intent with no matching route.
    let envelope = json!({
        "v": 1, "id": "ui-2", "ts": 0, "src": "ui",
        "body": { "kind": "intent", "name": "no.such.intent",
                  "params": {} }
    });
    let app = adapter_router().with_state(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/dispatch")
        .header("content-type", "application/json")
        .body(Body::from(envelope.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    // HTTP 200 (the handler returns JSON {ok:false}); the error is downstream.
    assert_eq!(resp.status(), StatusCode::OK);

    let downstream = rx.recv().await.expect("error envelope downstream");
    match downstream.body {
        EnvelopeBody::Error {
            ref_,
            code,
            message,
        } => {
            assert_eq!(ref_.as_deref(), Some("ui-2"));
            assert_eq!(code, "no_route");
            assert!(message.contains("/v1/no.such.intent"));
        }
        other => panic!("expected Error envelope, got {other:?}"),
    }
}

#[tokio::test]
async fn intent_without_scope_falls_back_to_table() {
    // intent chat.send with no source → fallback table → scope w-chat.
    let state = adapter_state();
    let (conn_id, mut rx) = subscribe(&state);

    let envelope = json!({
        "v": 1, "id": "ui-3", "ts": 0, "src": "ui",
        "body": { "kind": "intent", "name": "chat.send",
                  "params": { "text": "hi" } }
    });
    let app = adapter_router().with_state(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/dispatch")
        .header("content-type", "application/json")
        .header("x-airp-conn", conn_id.to_string())
        .body(Body::from(envelope.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let downstream = rx.recv().await.expect("downstream envelope");
    match downstream.body {
        EnvelopeBody::State { scope, .. } => {
            // No source → fallback table → w-chat (not w-default).
            assert_eq!(scope, "w-chat");
        }
        other => panic!("expected State envelope, got {other:?}"),
    }
}

#[tokio::test]
async fn hello_pushes_initial_state_downstream() {
    // Adapter config with initial blueprint + initial_state for w-chat.
    // Rebuild on the same gateway state to verify the hello path emits envelopes.
    let state = adapter_state();
    let cfg = airp_gateway::agentbus::AdapterConfig {
        default_scope: Some("w-default".into()),
        fallback_scopes: vec![IntentScopeFallback {
            intent: "chat.send".into(),
            scope: "w-chat".into(),
        }],
        route_prefix: "/v1/".into(),
        initial_blueprint: Some(airp_gateway::agentbus::Blueprint {
            version: "bp-1".into(),
            layout: json!({"type": "dock"}),
            widgets: vec![json!({"id": "w-chat", "type": "core.chat"})],
        }),
        initial_manifests: vec![],
        initial_state: std::collections::HashMap::from([(
            "w-chat".into(),
            json!({"messages": []}),
        )]),
    };
    let gw_state = state.gateway.clone();
    let state2 = build_state(gw_state, cfg);
    let (_conn_id, mut rx) = subscribe(&state2);

    let envelope = json!({
        "v": 1, "id": "ui-0", "ts": 0, "src": "ui",
        "body": { "kind": "hello", "client": "airp-ui", "version": "0.1.0",
                  "accept": ["core.chat"] }
    });
    let app = adapter_router().with_state(state2.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/dispatch")
        .header("content-type", "application/json")
        .body(Body::from(envelope.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // hello should emit blueprint + state set (w-chat). Order matches send order.
    let mut saw_blueprint = false;
    let mut saw_state = false;
    // Drain a bounded number of envelopes (blueprint, state — at most a couple).
    for _ in 0..4 {
        match rx.try_recv() {
            Ok(env) => match env.body {
                EnvelopeBody::Blueprint { blueprint, .. } => {
                    saw_blueprint = true;
                    assert_eq!(blueprint.version, "bp-1");
                }
                EnvelopeBody::State { scope, patch, .. } => {
                    saw_state = true;
                    assert_eq!(scope, "w-chat");
                    assert_eq!(patch[0].value.as_ref().unwrap()["messages"], json!([]));
                }
                _ => {}
            },
            Err(_) => break,
        }
    }
    assert!(saw_blueprint, "hello should emit a blueprint envelope");
    assert!(saw_state, "hello should emit initial w-chat state");
}

#[tokio::test]
async fn unsupported_envelope_version_rejected() {
    let state = adapter_state();
    // v: 99 — wrong version. The handler returns {ok:false}.
    let envelope = json!({
        "v": 99, "id": "ui-x", "ts": 0, "src": "ui",
        "body": { "kind": "hello", "client": "x", "version": "0", "accept": [] }
    });
    let app = adapter_router().with_state(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/dispatch")
        .header("content-type", "application/json")
        .body(Body::from(envelope.to_string()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap().contains("version"));
}
