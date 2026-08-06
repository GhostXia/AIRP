//! Phase 2.4: 剧情弧 HTTP handlers。
//!
//! 端点：
//! - `GET /v1/characters/:character_id/plot-arc` — 读取剧情弧
//! - `PUT /v1/characters/:character_id/plot-arc` — 保存剧情弧

use crate::daemon::DaemonState;
use crate::domain::{PlotArc, PlotService};
use crate::error::AirpError;
use crate::types::CharacterId;
use axum::extract::{Path, State};
use axum::Json;
use std::sync::Arc;

/// GET /v1/characters/:character_id/plot-arc
///
/// `PlotService::load_arc` 是同步文件 IO；在 async handler 中用
/// `spawn_blocking` 包装避免阻塞 tokio worker 线程（#433）。
pub(in crate::daemon) async fn get_plot_arc(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
) -> Result<Json<PlotArc>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let data_root = state.data_root.clone();
    let cid_str = cid.as_str().to_string();
    let arc = tokio::task::spawn_blocking(move || PlotService::new(&data_root).load_arc(&cid_str))
        .await
        .map_err(|e| AirpError::Internal(format!("plot load_arc join failed: {e}")))??;
    Ok(Json(arc))
}

/// PUT /v1/characters/:character_id/plot-arc
///
/// `PlotService::save_arc` 是同步文件 IO；在 async handler 中用
/// `spawn_blocking` 包装避免阻塞 tokio worker 线程（#433）。
pub(in crate::daemon) async fn update_plot_arc(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Json(arc): Json<PlotArc>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let data_root = state.data_root.clone();
    let cid_str = cid.as_str().to_string();
    tokio::task::spawn_blocking(move || PlotService::new(&data_root).save_arc(&cid_str, &arc))
        .await
        .map_err(|e| AirpError::Internal(format!("plot save_arc join failed: {e}")))??;
    Ok(Json(serde_json::json!({ "success": true })))
}
