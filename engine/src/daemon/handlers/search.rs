//! Search handlers: FTS5 历史检索 API（4.3）。

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use std::sync::Arc;

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::types::CharacterId;

#[derive(Debug, Deserialize)]
pub struct SearchRequest {
    pub character_id: String,
    pub query: String,
    pub limit: Option<usize>,
}

/// POST /v1/chat/search
pub async fn chat_search(
    State(state): State<Arc<DaemonState>>,
    Json(payload): Json<SearchRequest>,
) -> impl IntoResponse {
    let result = (|| -> Result<Vec<crate::memory::SearchResult>, AirpError> {
        let cid = CharacterId::new(&payload.character_id)?;
        let limit = payload.limit.unwrap_or(10);
        state
            .fts
            .search_history(&state.data_root, &cid, &payload.query, limit)
    })();

    match result {
        Ok(results) => (
            StatusCode::OK,
            Json(serde_json::json!({ "results": results })),
        )
            .into_response(),
        Err(e) => e.into_response(),
    }
}
