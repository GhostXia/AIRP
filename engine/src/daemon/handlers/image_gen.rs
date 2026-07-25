//! Phase 3.3: 场景插图生成 HTTP handlers。
//!
//! 端点：
//! - `POST /v1/image/generate` — 手动触发图片生成，可选下载到 session 资产
//! - `GET  /v1/characters/:character_id/images` — 列出已生成的图片元数据
//!
//! 复用 `crate::image_gen` 业务逻辑；handler 仅做参数解析、DaemonState 读取
//! 与响应包装。

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::image_gen::{download_image_to_session, generate_image, ImageGenRequest, ImageMeta};
use crate::types::CharacterId;
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// `POST /v1/image/generate` 请求体。继承 `ImageGenRequest`，附加可选 `download`。
#[derive(Debug, Deserialize)]
pub struct GenerateImageEndpointRequest {
    pub character_id: String,
    pub session_id: Option<String>,
    pub prompt: String,
    #[serde(default = "default_size")]
    pub size: String,
    #[serde(default = "default_style")]
    pub style: String,
    /// 是否下载图片到本地 session 资产目录（默认 false，仅返回 URL）。
    #[serde(default)]
    pub download: bool,
    /// 可选图片生成 model（覆盖 settings 中的 `model`）。便于调用 dall-e-3 等。
    pub image_model: Option<String>,
}

fn default_size() -> String {
    "1024x1024".to_string()
}
fn default_style() -> String {
    "vivid".to_string()
}

/// `POST /v1/image/generate` 响应体。
#[derive(Debug, Serialize)]
pub struct GenerateImageEndpointResponse {
    pub success: bool,
    /// 本地相对路径（仅当 `download=true` 且下载成功时存在）。
    pub image_path: Option<String>,
    /// 上游 API 返回的图片 URL（短期有效）。
    pub image_url: Option<String>,
    pub revised_prompt: Option<String>,
    /// 图片元数据（仅当 `download=true` 时返回最新条目）。
    pub meta: Option<ImageMeta>,
}

/// `GET /v1/characters/:character_id/images?session_id=...` 查询参数。
#[derive(Debug, Deserialize)]
pub struct ListImagesQuery {
    pub session_id: Option<String>,
}

/// `POST /v1/image/generate`
///
/// 调用 OpenAI-compatible 图片生成 API（DALL-E / Stable Diffusion）。
/// 当 `download=true` 时下载到 `characters/{id}/sessions/{sid}/images/`。
pub(in crate::daemon) async fn generate_image_endpoint(
    State(state): State<Arc<DaemonState>>,
    Json(req): Json<GenerateImageEndpointRequest>,
) -> Result<Json<GenerateImageEndpointResponse>, AirpError> {
    // 参数校验：character_id 必须合法；session_id（若提供）也走 SessionId 解析。
    let cid = CharacterId::new(&req.character_id)?;
    if let Some(ref sid) = req.session_id {
        let _ = crate::types::SessionId::parse(sid)?;
    }

    // prompt 不能为空
    let prompt = req.prompt.trim();
    if prompt.is_empty() {
        return Err(AirpError::BadRequest(
            "prompt must not be empty".to_string(),
        ));
    }

    // 读取上游 LLM provider 配置（endpoint / api_key）。image_model 可覆盖默认 model。
    let snapshot = state
        .config
        .read()
        .map_err(|_| AirpError::Internal("config lock poisoned".to_string()))?
        .clone();

    let model = req
        .image_model
        .clone()
        .unwrap_or_else(|| snapshot.model.clone());

    let inner_req = ImageGenRequest {
        character_id: cid.as_str().to_string(),
        session_id: req.session_id.clone(),
        prompt: prompt.to_string(),
        size: req.size.clone(),
        style: req.style.clone(),
    };

    let mut resp = generate_image(
        &state.http_client,
        &snapshot.endpoint,
        snapshot.api_key.as_deref(),
        &model,
        &inner_req,
    )
    .await?;

    if !resp.success {
        return Ok(Json(GenerateImageEndpointResponse {
            success: false,
            image_path: None,
            image_url: None,
            revised_prompt: None,
            meta: None,
        }));
    }

    // 可选下载到本地
    let mut meta = None;
    let mut image_path = None;
    if req.download {
        if let Some(ref url) = resp.image_url {
            let path = download_image_to_session(
                &state.http_client,
                &state.data_root,
                cid.as_str(),
                req.session_id.as_deref(),
                url,
                prompt,
            )
            .await?;
            // 读取刚写入的 index.json 末条元数据回填响应
            let index_path = crate::image_gen::images_index_path(
                &state.data_root,
                cid.as_str(),
                req.session_id.as_deref(),
            );
            if let Some(index) = std::fs::read_to_string(&index_path)
                .ok()
                .and_then(|s| serde_json::from_str::<Vec<ImageMeta>>(&s).ok())
            {
                meta = index.last().cloned();
            }
            image_path = Some(path);
            // 已下载到本地，不回传短期 URL 防止客户端缓存过期 URL
            resp.image_url = None;
        }
    }

    Ok(Json(GenerateImageEndpointResponse {
        success: true,
        image_path,
        image_url: resp.image_url,
        revised_prompt: resp.revised_prompt,
        meta,
    }))
}

/// `GET /v1/characters/:character_id/images?session_id=...`
///
/// 返回指定角色（可选 session）下已生成图片的元数据列表。
pub(in crate::daemon) async fn list_images_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Query(query): Query<ListImagesQuery>,
) -> Result<Json<Vec<ImageMeta>>, AirpError> {
    let cid = CharacterId::new(&character_id)?;
    if let Some(ref sid) = query.session_id {
        let _ = crate::types::SessionId::parse(sid)?;
    }

    let index_path = crate::image_gen::images_index_path(
        &state.data_root,
        cid.as_str(),
        query.session_id.as_deref(),
    );

    let list: Vec<ImageMeta> = std::fs::read_to_string(&index_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    Ok(Json(list))
}
