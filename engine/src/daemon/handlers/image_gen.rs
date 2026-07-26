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
use crate::image_gen::{
    default_size, default_style, download_image_to_session, generate_image, ImageGenRequest,
    ImageMeta,
};
use crate::types::CharacterId;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
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
            match download_image_to_session(
                &state.http_client,
                &state.data_root,
                cid.as_str(),
                req.session_id.as_deref(),
                url,
                prompt,
            )
            .await
            {
                Ok((m, path)) => {
                    // CodeRabbit #4：直接用 download_image_to_session 返回的
                    // ImageMeta，不再 re-read index.last()——并发下可能读到他人条目。
                    meta = Some(m);
                    image_path = Some(path);
                    // 已下载到本地，不回传短期 URL 防止客户端缓存过期 URL
                    resp.image_url = None;
                }
                Err(e) => {
                    // CodeRabbit #1：下载失败不 abort 整个 handler。上游图片生成
                    // 已计费/限流，保留 resp.image_url 返回 URL-only 响应，让用户
                    // 在短期 URL 有效期内仍能使用已生成的图。
                    tracing::warn!("image download failed, returning URL only: {e}");
                }
            }
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

/// 校验图片文件名，防 path traversal。
///
/// 仅允许 `[A-Za-z0-9_.-]+\.png`，拒绝含路径分隔符、`..`、空字节、Windows
/// 驱动器前缀分隔符 `:` 等一切非白名单字符的文件名。
///
/// CodeRabbit 二次审计指出：原实现用黑名单（`contains('/')` 等）漏掉了 `:`。
/// 在 Windows 上 `PathBuf::join` 会把带 prefix 无 root 的输入（如 `C:foo.png`）
/// 视为驱动器相对路径并替换整个 base，可逃逸 `data_root`。改用白名单从根上杜绝。
fn validate_image_filename(filename: &str) -> Result<(), AirpError> {
    if filename.is_empty() || !filename.ends_with(".png") {
        return Err(AirpError::BadRequest(format!(
            "invalid image filename: {filename}"
        )));
    }
    // 白名单：仅允许字母、数字、下划线、连字符、点。其余（含 `/` `\` `:` `\0`
    // `..` 虽由 `.` 单字符允许，但 `..` 作为整段路径仍需在 join 后再校验，见下）
    // 一律拒绝。
    if !filename
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        return Err(AirpError::BadRequest(format!(
            "invalid image filename (contains non-whitelisted char): {filename}"
        )));
    }
    // 即便单字符都合法，`..foo.png` / `foo..png` / `..` 仍可能被 PathBuf 解释为
    // 父目录引用。显式拒绝任何 `..` 段。
    if filename.contains("..") {
        return Err(AirpError::BadRequest(format!(
            "invalid image filename (parent directory reference): {filename}"
        )));
    }
    Ok(())
}

/// 内部：读取并返回图片字节，Content-Type 固定 `image/png`（当前下载流程
/// 统一存为 `.png`）。
fn serve_image_file(filepath: std::path::PathBuf) -> Result<Response, AirpError> {
    if !filepath.exists() {
        return Err(AirpError::NotFound(format!(
            "image not found: {filepath:?}"
        )));
    }
    let bytes = std::fs::read(&filepath)?;
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/png")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, max-age=3600"),
            ),
        ],
        Body::from(bytes),
    )
        .into_response())
}

/// `GET /v1/characters/:character_id/images/:filename`
///
/// 服务角色级（非 session 绑定）已生成图片的字节流。webui `<img src>` 需要
/// 此端点才能显示图片——`ServeDir` fallback 指向 webui 静态目录，无法服务
/// `data_root/characters/...` 下的资产（CodeRabbit #2）。
pub(in crate::daemon) async fn serve_image_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, filename)): Path<(String, String)>,
) -> Result<Response, AirpError> {
    let cid = CharacterId::new(&character_id)?;
    validate_image_filename(&filename)?;
    let filepath = state
        .data_root
        .join("characters")
        .join(cid.as_str())
        .join("images")
        .join(&filename);
    serve_image_file(filepath)
}

/// `GET /v1/characters/:character_id/sessions/:session_id/images/:filename`
///
/// 服务 session 绑定的已生成图片字节流（CodeRabbit #2）。
pub(in crate::daemon) async fn serve_session_image_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, session_id, filename)): Path<(String, String, String)>,
) -> Result<Response, AirpError> {
    let cid = CharacterId::new(&character_id)?;
    // 校验 session_id 是合法 UUID（防 path traversal / 非法字符），校验后用原字符串构造路径。
    let _ = crate::types::SessionId::parse(&session_id)?;
    validate_image_filename(&filename)?;
    let filepath = state
        .data_root
        .join("characters")
        .join(cid.as_str())
        .join("sessions")
        .join(&session_id)
        .join("images")
        .join(&filename);
    serve_image_file(filepath)
}

#[cfg(test)]
mod tests {
    use super::validate_image_filename;

    #[test]
    fn accepts_normal_png_filename() {
        assert!(validate_image_filename("1690000000000.png").is_ok());
        assert!(validate_image_filename("image_1.png").is_ok());
        assert!(validate_image_filename("img-2.3.png").is_ok());
    }

    #[test]
    fn rejects_empty_and_non_png() {
        assert!(validate_image_filename("").is_err());
        assert!(validate_image_filename("image.jpg").is_err());
        assert!(validate_image_filename("image").is_err());
    }

    #[test]
    fn rejects_path_separators_and_null() {
        for bad in ["a/b.png", "a\\b.png", "a\0b.png", "a\nb.png"] {
            assert!(
                validate_image_filename(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_parent_dir_traversal() {
        for bad in ["..png", "../etc.png", "a/../b.png", "a..b.png", "...png"] {
            assert!(
                validate_image_filename(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    /// CodeRabbit 二次审计指出：Windows 上 `PathBuf::join` 把 `C:foo.png`（带
    /// prefix 无 root）当作驱动器相对路径并替换整个 base，可逃逸 `data_root`。
    /// 必须拒绝 `:`。
    #[test]
    fn rejects_windows_drive_prefix_colon() {
        for bad in ["C:foo.png", "D:evil.png", "a:b.png", ":hidden.png"] {
            assert!(
                validate_image_filename(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }

    #[test]
    fn rejects_other_non_whitelisted_chars() {
        // 白名单仅 [A-Za-z0-9_.-]，其余一律拒
        for bad in ["a b.png", "a;b.png", "a|b.png", "a(b).png", "中文.png"] {
            assert!(
                validate_image_filename(bad).is_err(),
                "should reject {bad:?}"
            );
        }
    }
}
