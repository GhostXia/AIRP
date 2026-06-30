//! Frontend-facing middleware: bearer auth (constant-time) and CORS.
//! Rate limiting is wired in [`super::build_router`] via `tower_governor`.

use std::sync::Arc;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tower_http::cors::{Any, CorsLayer};

use crate::config::CorsConfig;
use crate::error::GatewayError;
use crate::server::GatewayState;

/// Bearer-token gate. No-op when `access_key` is unset.
pub async fn auth(State(state): State<Arc<GatewayState>>, req: Request, next: Next) -> Response {
    let Some(expected) = state.config.access_key.as_deref() else {
        return next.run(req).await;
    };

    let presented = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => {
            next.run(req).await
        }
        _ => GatewayError::Unauthorized.into_response(),
    }
}

/// Length-aware constant-time comparison to avoid leaking the key via timing.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Build the CORS layer from config.
pub fn cors_layer(cfg: &CorsConfig) -> CorsLayer {
    if cfg.allow_any || cfg.allow_origins.is_empty() {
        CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(Any)
    } else {
        let origins = cfg
            .allow_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect::<Vec<_>>();
        CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(origins)
    }
}
