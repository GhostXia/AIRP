//! Phase 4.1: 角色卡模板库 HTTP handlers。
//!
//! 端点：
//! - `GET  /v1/character-templates` — 列出所有模板元数据
//! - `GET  /v1/character-templates/:id` — 返回完整模板角色卡 JSON
//! - `POST /v1/character-templates/:id/instantiate` — 基于模板创建角色，
//!   复用 `import_card_to_disk` 落盘流程

use crate::character_templates::{
    template_card_json, InstantiateRequest, InstantiateResponse, TemplateMeta, TEMPLATE_METAS,
};
use crate::daemon::DaemonState;
use crate::error::AirpError;
use axum::extract::{Path, State};
use axum::Json;
use std::sync::Arc;

use crate::daemon::handlers::import_card_to_disk;

/// `GET /v1/character-templates`
///
/// 返回所有内建模板元数据，按 id 字典序排列。
pub(in crate::daemon) async fn list_templates_endpoint(
    State(_state): State<Arc<DaemonState>>,
) -> Result<Json<Vec<TemplateMeta>>, AirpError> {
    let mut list: Vec<TemplateMeta> = TEMPLATE_METAS.to_vec();
    list.sort_by_key(|t| t.id);
    Ok(Json(list))
}

/// `GET /v1/character-templates/:id`
///
/// 返回指定模板的完整角色卡 JSON（SillyTavern V2 兼容）。
pub(in crate::daemon) async fn get_template_endpoint(
    State(_state): State<Arc<DaemonState>>,
    Path(template_id): Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let json_str = template_card_json(&template_id)?;
    let value: serde_json::Value = serde_json::from_str(&json_str)
        .map_err(|e| AirpError::Internal(format!("template json parse failed: {e}")))?;
    Ok(Json(value))
}

/// `POST /v1/character-templates/:id/instantiate`
///
/// 基于模板创建角色。复用 `import_card_to_disk` 的 `card_json` 路径落盘。
/// 若请求体 `name_override` 提供，则覆盖模板 name 后再导入。
pub(in crate::daemon) async fn instantiate_template_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(template_id): Path<String>,
    Json(req): Json<InstantiateRequest>,
) -> Result<Json<InstantiateResponse>, AirpError> {
    let json_str = template_card_json(&template_id)?;

    // 可选 name 覆盖：解析 → 替换 data.name → 重新序列化
    let final_json = if let Some(ref name) = req.name_override {
        let name_trimmed = name.trim();
        if name_trimmed.is_empty() {
            return Err(AirpError::BadRequest(
                "name_override must not be empty".to_string(),
            ));
        }
        let mut v: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| AirpError::Internal(format!("template json parse failed: {e}")))?;
        v["data"]["name"] = serde_json::Value::String(name_trimmed.to_string());
        serde_json::to_string(&v)
            .map_err(|e| AirpError::Internal(format!("template json re-serialize failed: {e}")))?
    } else {
        json_str
    };

    // 可选 character_id 校验
    if let Some(ref id) = req.character_id {
        let _ = crate::types::CharacterId::new(id)?;
    }

    let (final_id, card_format, _json) = import_card_to_disk(
        &state.data_root,
        req.character_id.as_deref(),
        None,
        Some(final_json),
        None,
    )?;

    Ok(Json(InstantiateResponse {
        character_id: final_id,
        template_id,
        card_format,
    }))
}
