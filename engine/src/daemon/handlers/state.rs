//! Character state / avatar HTTP handlers — read live state + schema + history.
//!
//! #155 PR6：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与磁盘读取；state 文件由 chat pipeline / volume context 写盘。
//!
//! 端点：
//! - `GET /v1/characters/:character_id/avatar` — serve card.png as image/png
//! - `GET /v1/characters/:character_id/state` — 读 live.json
//! - `GET /v1/characters/:character_id/state/schema` — 读 schema.json
//! - `GET /v1/characters/:character_id/state/history?limit=N` — 读 history jsonl（倒序）

use crate::daemon::DaemonState;
use crate::data_dir;
use crate::types::CharacterId;
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
};
use std::fs;
use std::sync::Arc;

/// GET /v1/characters/:character_id/avatar — serve card.png as image/png.
pub(in crate::daemon) async fn get_character_avatar(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let char_dir = data_dir::character_dir_path(&state.data_root, &char_id);
    let png_path = char_dir.join("card.png");
    match fs::read(&png_path) {
        Ok(bytes) => ([(header::CONTENT_TYPE, "image/png")], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state
pub(in crate::daemon) async fn get_character_state(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let live_path = data_dir::char_state_dir(&state.data_root, char_id.as_str()).join("live.json");
    match fs::read_to_string(&live_path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state/schema
pub(in crate::daemon) async fn get_character_state_schema(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let schema_path =
        data_dir::char_state_dir(&state.data_root, char_id.as_str()).join("schema.json");
    match fs::read_to_string(&schema_path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// GET /v1/characters/:character_id/state/history?limit=N
pub(in crate::daemon) async fn get_character_state_history(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let limit: usize = params
        .get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(50)
        .clamp(1, 1000);

    let history_path = data_dir::char_state_history_path(&state.data_root, char_id.as_str());
    let Ok(text) = fs::read_to_string(&history_path) else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let entries: Vec<serde_json::Value> = text
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str(line).ok())
        .take(limit)
        .collect();

    match serde_json::to_string(&entries) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// GET /v1/characters/:character_id/world-events
///
/// 返回角色的世界事件列表（world_events.json）。文件不存在时返回空数组。
pub(in crate::daemon) async fn get_world_events(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Response {
    let char_id = match CharacterId::new(character_id) {
        Ok(id) => id,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let path = state
        .data_root
        .join("characters")
        .join(char_id.as_str())
        .join("world_events.json");
    match fs::read_to_string(&path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => ([(header::CONTENT_TYPE, "application/json")], "[]").into_response(),
    }
}
