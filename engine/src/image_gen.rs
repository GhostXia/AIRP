//! Phase 3.3: 场景插图生成。
//!
//! 调用 OpenAI-compatible 图片生成 API（DALL-E / Stable Diffusion），
//! 在关键剧情节点自动或手动触发，图片存入 session 资产。
//!
//! ## 端点
//! - `POST /v1/image/generate` — 手动触发图片生成
//!
//! ## 存储
//! - 图片保存在 `characters/{id}/sessions/{sid}/images/{timestamp}.png`
//! - 元数据保存在 `characters/{id}/sessions/{sid}/images/index.json`

use crate::error::AirpError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 图片生成请求。
#[derive(Debug, Clone, Deserialize)]
pub struct ImageGenRequest {
    /// 角色 ID。
    pub character_id: String,
    /// 可选 session ID。
    pub session_id: Option<String>,
    /// 图片描述 prompt。
    pub prompt: String,
    /// 图片尺寸（默认 1024x1024）。
    #[serde(default = "default_size")]
    pub size: String,
    /// 风格（vivid / natural）。
    #[serde(default = "default_style")]
    pub style: String,
}

fn default_size() -> String {
    "1024x1024".to_string()
}
fn default_style() -> String {
    "vivid".to_string()
}

/// 图片生成响应。
#[derive(Debug, Clone, Serialize)]
pub struct ImageGenResponse {
    /// 是否成功。
    pub success: bool,
    /// 图片本地路径（相对于 data_root）。
    pub image_path: Option<String>,
    /// 图片 URL（如果 API 返回 URL 而非 base64）。
    pub image_url: Option<String>,
    /// 修订后的 prompt。
    pub revised_prompt: Option<String>,
}

/// 图片元数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageMeta {
    pub filename: String,
    pub prompt: String,
    pub timestamp: u64,
    pub size: String,
}

fn images_dir(data_root: &Path, character_id: &str, session_id: Option<&str>) -> PathBuf {
    let base = data_root.join("characters").join(character_id);
    match session_id {
        Some(sid) => base.join("sessions").join(sid).join("images"),
        None => base.join("images"),
    }
}

/// 返回 `index.json` 路径（供 handler 读取已写入的元数据列表）。
pub fn images_index_path(
    data_root: &Path,
    character_id: &str,
    session_id: Option<&str>,
) -> PathBuf {
    images_dir(data_root, character_id, session_id).join("index.json")
}

/// 调用 OpenAI-compatible 图片生成 API。
pub async fn generate_image(
    client: &reqwest::Client,
    endpoint: &str,
    api_key: Option<&str>,
    model: &str,
    req: &ImageGenRequest,
) -> Result<ImageGenResponse, AirpError> {
    // 构建图片生成 API URL（兼容 OpenAI images/generations）
    let base_url = endpoint.trim_end_matches('/');
    let url = if base_url.contains("/images") {
        base_url.to_string()
    } else {
        // 从 chat/completions endpoint 推导 images endpoint
        base_url
            .replace("/chat/completions", "/images/generations")
            .replace("/v1/chat/completions", "/v1/images/generations")
    };

    let mut request = client.post(&url).json(&serde_json::json!({
        "model": model,
        "prompt": req.prompt,
        "n": 1,
        "size": req.size,
        "style": req.style,
        "response_format": "url"
    }));

    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        request = request.bearer_auth(key);
    }

    let response = request
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| AirpError::Upstream {
            status: 0,
            body: format!("image generation request failed: {e}"),
        })?;

    let status = response.status();
    let body = response.text().await.unwrap_or_default();

    if !status.is_success() {
        return Err(AirpError::Upstream {
            status: status.as_u16(),
            body,
        });
    }

    let payload: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| AirpError::Internal(format!("image generation response parse error: {e}")))?;

    // 解析 OpenAI 格式响应
    let data = payload.get("data").and_then(|d| d.as_array());
    let first = data.and_then(|arr| arr.first());

    let image_url = first
        .and_then(|item| item.get("url"))
        .and_then(|u| u.as_str())
        .map(String::from);

    let revised_prompt = first
        .and_then(|item| item.get("revised_prompt"))
        .and_then(|p| p.as_str())
        .map(String::from);

    if image_url.is_none() {
        return Ok(ImageGenResponse {
            success: false,
            image_path: None,
            image_url: None,
            revised_prompt: None,
        });
    }

    Ok(ImageGenResponse {
        success: true,
        image_path: None, // URL 模式不保存本地
        image_url,
        revised_prompt,
    })
}

/// 下载图片到本地 session 资产目录。
pub async fn download_image_to_session(
    client: &reqwest::Client,
    data_root: &Path,
    character_id: &str,
    session_id: Option<&str>,
    image_url: &str,
    prompt: &str,
) -> Result<String, AirpError> {
    let dir = images_dir(data_root, character_id, session_id);
    std::fs::create_dir_all(&dir)?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("{}.png", timestamp);
    let filepath = dir.join(&filename);

    // 下载图片
    let response = client
        .get(image_url)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| AirpError::Upstream {
            status: 0,
            body: format!("image download failed: {e}"),
        })?;

    if !response.status().is_success() {
        return Err(AirpError::Upstream {
            status: response.status().as_u16(),
            body: "image download returned non-success status".to_string(),
        });
    }

    let bytes = response.bytes().await.map_err(|e| AirpError::Upstream {
        status: 0,
        body: format!("image download body read failed: {e}"),
    })?;

    crate::data_dir::replace_file(&filepath, &bytes)?;

    // 更新 index.json
    let index_path = dir.join("index.json");
    let mut index: Vec<ImageMeta> = std::fs::read_to_string(&index_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    index.push(ImageMeta {
        filename: filename.clone(),
        prompt: prompt.to_string(),
        timestamp,
        size: format!("{} bytes", bytes.len()),
    });
    let index_json = serde_json::to_string_pretty(&index)
        .map_err(|e| AirpError::Internal(format!("index serialize: {e}")))?;
    crate::data_dir::replace_file(&index_path, index_json.as_bytes())?;

    // 返回相对路径
    let relative = Path::new("characters")
        .join(character_id)
        .join(
            session_id
                .map(|s| Path::new("sessions").join(s))
                .unwrap_or_default(),
        )
        .join("images")
        .join(&filename);
    Ok(relative.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn images_dir_with_session() {
        let dir = images_dir(Path::new("/data"), "hero", Some("sess1"));
        assert_eq!(
            dir,
            PathBuf::from("/data/characters/hero/sessions/sess1/images")
        );
    }

    #[test]
    fn images_dir_without_session() {
        let dir = images_dir(Path::new("/data"), "hero", None);
        assert_eq!(dir, PathBuf::from("/data/characters/hero/images"));
    }

    #[test]
    fn default_request_params() {
        let req: ImageGenRequest =
            serde_json::from_str(r#"{"character_id":"hero","prompt":"a sunset"}"#).unwrap();
        assert_eq!(req.size, "1024x1024");
        assert_eq!(req.style, "vivid");
        assert!(req.session_id.is_none());
    }
}
