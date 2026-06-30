//! Frontend-facing HTTP handlers.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{Method, Uri};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::bridge::DispatchOutcome;
use crate::error::GatewayError;
use crate::server::GatewayState;

/// Liveness probe.
pub async fn health() -> &'static str {
    "ok"
}

/// Diagnostic metadata (unauthenticated by design — see router wiring).
pub async fn version(State(state): State<Arc<GatewayState>>) -> Json<Value> {
    Json(json!({
        "name": "airp-gateway",
        "version": env!("CARGO_PKG_VERSION"),
        "upstreams": state.bridge_upstreams(),
    }))
}

/// Catch-all: match the request against configured routes and forward to MCP.
pub async fn dispatch(
    State(state): State<Arc<GatewayState>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> Response {
    let path = uri.path();

    // Defense-in-depth: cap inbound body size. tower-http's RequestBodyLimitLayer
    // is the primary gate (see server::mod), but we re-check here so callers
    // using the bridge directly (custom frontends) also get the bound.
    if body.len() > state.config.max_request_bytes {
        return GatewayError::PayloadTooLarge(state.config.max_request_bytes).into_response();
    }

    let rule = match state.bridge.match_route(method.as_str(), path) {
        Some(r) => r.clone(),
        None => return GatewayError::NoRoute(method.to_string(), path.to_string()).into_response(),
    };

    // Empty body is a valid empty arguments object.
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return GatewayError::BadRequest(e.to_string()).into_response(),
        }
    };

    match state.bridge.dispatch(&rule, args).await {
        Ok(DispatchOutcome::Json(v)) => Json(v).into_response(),
        Ok(DispatchOutcome::Stream) => {
            // TODO(streaming): return an SSE response here.
            GatewayError::Unimplemented("SSE response").into_response()
        }
        Err(e) => e.into_response(),
    }
}
