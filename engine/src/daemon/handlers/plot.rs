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
pub(in crate::daemon) async fn get_plot_arc(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
) -> Result<Json<PlotArc>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let arc = PlotService::new(&state.data_root).load_arc(cid.as_str())?;
    Ok(Json(arc))
}

/// PUT /v1/characters/:character_id/plot-arc
pub(in crate::daemon) async fn update_plot_arc(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Json(arc): Json<PlotArc>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    PlotService::new(&state.data_root).save_arc(cid.as_str(), &arc)?;
    Ok(Json(serde_json::json!({ "success": true })))
}
