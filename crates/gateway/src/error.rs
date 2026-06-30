//! Unified error type for the gateway, with an axum [`IntoResponse`] mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

pub type Result<T> = std::result::Result<T, GatewayError>;

#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("unknown upstream: {0}")]
    UnknownUpstream(String),

    #[error("no route matched: {0} {1}")]
    NoRoute(String, String),

    #[error("upstream transport error: {0}")]
    Transport(String),

    #[error("upstream returned JSON-RPC error {code}: {message}")]
    Upstream { code: i64, message: String },

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("request body too large (limit {0} bytes)")]
    PayloadTooLarge(usize),

    #[error("upstream request timed out after {0:?}")]
    UpstreamTimeout(std::time::Duration),

    #[error("upstream response too large (limit {0} bytes)")]
    ResponseTooLarge(usize),

    #[error("unauthorized")]
    Unauthorized,

    /// A code path that is scaffolded but not yet implemented.
    #[error("not implemented: {0}")]
    Unimplemented(&'static str),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl GatewayError {
    pub fn status(&self) -> StatusCode {
        match self {
            GatewayError::Unauthorized => StatusCode::UNAUTHORIZED,
            GatewayError::BadRequest(_) => StatusCode::BAD_REQUEST,
            GatewayError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            GatewayError::UpstreamTimeout(_) => StatusCode::GATEWAY_TIMEOUT,
            GatewayError::ResponseTooLarge(_) => StatusCode::BAD_GATEWAY,
            GatewayError::NoRoute(_, _) => StatusCode::NOT_FOUND,
            GatewayError::UnknownUpstream(_) => StatusCode::BAD_GATEWAY,
            GatewayError::Transport(_) | GatewayError::Upstream { .. } => StatusCode::BAD_GATEWAY,
            GatewayError::Unimplemented(_) => StatusCode::NOT_IMPLEMENTED,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    /// Whether the full error message is safe to leak to the client.
    /// Internal/transport/io errors may contain upstream paths or internal
    /// details; return a generic message instead and log the detail server-side.
    fn is_client_safe(&self) -> bool {
        matches!(
            self,
            GatewayError::BadRequest(_)
                | GatewayError::PayloadTooLarge(_)
                | GatewayError::UpstreamTimeout(_)
                | GatewayError::ResponseTooLarge(_)
                | GatewayError::Unauthorized
                | GatewayError::NoRoute(_, _)
                | GatewayError::Unimplemented(_)
                | GatewayError::Config(_)
        )
    }
}

impl IntoResponse for GatewayError {
    fn into_response(self) -> Response {
        let status = self.status();
        // Log the full detail server-side; only show safe messages to clients.
        if !self.is_client_safe() {
            tracing::warn!(error = %self, "upstream/internal error");
        }
        let message = if self.is_client_safe() {
            self.to_string()
        } else {
            status.canonical_reason().unwrap_or("error").to_string()
        };
        let body = Json(json!({
            "error": {
                "type": status.canonical_reason().unwrap_or("error"),
                "message": message,
            }
        }));
        (status, body).into_response()
    }
}
