//! Scene HTTP handlers — list / get / create / add-character.
//!
//! #155 PR5：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 scene orchestration；`SceneConfig` 落盘和 `SceneId` 校验在 `scene` / `types` 模块。
//!
//! 端点：
//! - `GET    /v1/scenes` — 列出所有 scene ID
//! - `GET    /v1/scenes/:scene_id` — 返回 scene.json
//! - `POST   /v1/scenes` — 创建或替换 scene
//! - `POST   /v1/scenes/:scene_id/characters` — 向已有 scene 添加角色

use super::DaemonState;
use crate::data_dir;
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use std::fs;
use std::sync::Arc;

/// GET /v1/scenes — list all scene IDs.
pub(in crate::daemon) async fn list_scenes_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<Vec<String>>, crate::error::AirpError> {
    let scenes = data_dir::list_scenes(&state.data_root)?;
    Ok(Json(scenes))
}

/// GET /v1/scenes/:scene_id — return scene.json for a scene.
///
/// AUDIT-2: scene_id is validated once via SceneId::new; downstream path
/// functions take &SceneId so traversal protection is compile-time enforced.
pub(in crate::daemon) async fn get_scene_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(scene_id): axum::extract::Path<String>,
) -> Response {
    let scene_id = match crate::types::SceneId::new(scene_id) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let path = data_dir::scene_json_path(&state.data_root, &scene_id);
    match fs::read_to_string(&path) {
        Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

/// POST /v1/scenes — create or replace a scene from JSON body.
///
/// AUDIT-2: SceneConfig.scene_id is now a `SceneId`; serde Deserialize calls
/// `validate_id_segment` automatically, so a body with an invalid scene_id
/// is rejected at deserialize time (HTTP 400 returned by axum), and the
/// manual check below is no longer needed.
pub(in crate::daemon) async fn create_scene_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(scene): Json<crate::scene::SceneConfig>,
) -> Response {
    match scene.save(&state.data_root) {
        Ok(()) => {
            let path = data_dir::scene_json_path(&state.data_root, &scene.scene_id);
            (
                StatusCode::CREATED,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::json!({"scene_id": scene.scene_id, "path": path}).to_string(),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// POST /v1/scenes/:scene_id/characters — add a character to an existing scene.
pub(in crate::daemon) async fn add_scene_character_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(scene_id): axum::extract::Path<String>,
    Json(body): Json<AddCharacterBody>,
) -> Response {
    let scene_id = match crate::types::SceneId::new(scene_id) {
        Ok(s) => s,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    if data_dir::validate_id_segment(&body.character_id).is_err() {
        return (StatusCode::BAD_REQUEST, "非法 character_id").into_response();
    }
    let mut scene = match crate::scene::SceneConfig::load(&state.data_root, &scene_id) {
        Ok(s) => s,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    scene.characters.push(crate::scene::CharacterEntry {
        character_id: body.character_id,
        role: body.role,
        intro: body.intro,
    });
    match scene.save(&state.data_root) {
        Ok(()) => Json(serde_json::json!({"scene_id": scene_id.as_str(), "character_count": scene.characters.len()})).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
pub(in crate::daemon) struct AddCharacterBody {
    character_id: String,
    #[serde(default)]
    role: crate::scene::CharacterRole,
    #[serde(default)]
    intro: String,
}
