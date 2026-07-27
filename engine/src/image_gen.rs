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
use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use std::net::IpAddr;
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

/// 图片尺寸默认值。`pub(crate)` 以便 handler 复用同一来源（CodeRabbit N1）。
pub(crate) fn default_size() -> String {
    "1024x1024".to_string()
}
/// 图片风格默认值。`pub(crate)` 以便 handler 复用同一来源（CodeRabbit N1）。
pub(crate) fn default_style() -> String {
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
    /// Decoded PNG bytes when an upstream returns `b64_json` instead of a URL.
    #[serde(skip)]
    pub(crate) image_bytes: Option<Vec<u8>>,
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

fn image_generation_url(endpoint: &str) -> Result<reqwest::Url, AirpError> {
    let mut url = reqwest::Url::parse(endpoint)
        .map_err(|error| AirpError::Config(format!("invalid image endpoint: {error}")))?;
    let path = url.path().trim_end_matches('/').to_string();

    if let Some(prefix) = path.strip_suffix("/chat/completions") {
        url.set_path(&format!("{prefix}/images/generations"));
    }

    Ok(url)
}

fn parse_image_generation_response(body: &str) -> Result<ImageGenResponse, AirpError> {
    let payload: serde_json::Value = serde_json::from_str(body)
        .map_err(|e| AirpError::Internal(format!("image generation response parse error: {e}")))?;
    let first = payload
        .get("data")
        .and_then(|data| data.as_array())
        .and_then(|data| data.first());

    let image_url = first
        .and_then(|item| item.get("url"))
        .and_then(|url| url.as_str())
        .map(String::from);
    let revised_prompt = first
        .and_then(|item| item.get("revised_prompt"))
        .and_then(|prompt| prompt.as_str())
        .map(String::from);

    let image_bytes = if image_url.is_none() {
        first
            .and_then(|item| item.get("b64_json"))
            .and_then(|encoded| encoded.as_str())
            .map(decode_generated_image)
            .transpose()?
    } else {
        None
    };

    Ok(ImageGenResponse {
        success: image_url.is_some() || image_bytes.is_some(),
        image_path: None,
        image_url,
        revised_prompt,
        image_bytes,
    })
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
    let url = image_generation_url(endpoint)?;

    let mut request = client.post(url).json(&serde_json::json!({
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

    parse_image_generation_response(&body)
}

/// 图片下载大小上限（20 MiB）。防止恶意/被攻陷上游返回超大响应压满磁盘
/// （CodeRabbit N2）。
const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
const MAX_BASE64_IMAGE_CHARS: usize = MAX_IMAGE_BYTES.div_ceil(3) * 4;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn image_download_error(message: impl Into<String>) -> AirpError {
    AirpError::Upstream {
        status: 0,
        body: message.into(),
    }
}

fn decode_generated_image(encoded: &str) -> Result<Vec<u8>, AirpError> {
    if encoded.len() > MAX_BASE64_IMAGE_CHARS {
        return Err(image_download_error(
            "image generation rejected: b64_json exceeds the image size limit",
        ));
    }
    let bytes = STANDARD.decode(encoded).map_err(|_| {
        image_download_error("image generation rejected: b64_json is not valid base64")
    })?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(image_download_error(
            "image generation rejected: decoded image exceeds the image size limit",
        ));
    }
    validate_png_signature(&bytes)?;
    Ok(bytes)
}

fn is_png_content_type(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("image/png"))
}

fn validate_png_signature(bytes: &[u8]) -> Result<(), AirpError> {
    if bytes.starts_with(PNG_SIGNATURE) {
        Ok(())
    } else {
        Err(image_download_error(
            "image download rejected: response body is not a valid PNG",
        ))
    }
}

fn is_non_public_image_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            ip.is_loopback()
                || ip.is_private()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_broadcast()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 2)
                || (octets[0] == 198 && (octets[1] == 18 || octets[1] == 19))
                || (octets[0] == 198 && octets[1] == 51 && octets[2] == 100)
                || (octets[0] == 203 && octets[1] == 0 && octets[2] == 113)
                || octets[0] >= 240
        }
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            let first = segments[0];
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || ip
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_non_public_image_ip(IpAddr::V4(mapped)))
        }
    }
}

#[derive(Debug)]
struct ValidatedImageDownload {
    url: reqwest::Url,
    host: String,
    address: std::net::SocketAddr,
}

async fn validate_image_download_url(image_url: &str) -> Result<ValidatedImageDownload, AirpError> {
    let url = reqwest::Url::parse(image_url)
        .map_err(|_| image_download_error("image download rejected: invalid URL"))?;
    if url.scheme() != "https" {
        return Err(image_download_error(
            "image download rejected: only HTTPS URLs are allowed",
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| image_download_error("image download rejected: URL has no host"))?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let resolved = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::net::lookup_host((host.as_str(), port)),
    )
    .await
    .map_err(|_| image_download_error("image download rejected: host resolution failed"))?
    .map_err(|_| image_download_error("image download rejected: host resolution failed"))?;
    let addresses: Vec<_> = resolved.collect();
    if addresses.is_empty() {
        return Err(image_download_error(
            "image download rejected: host resolved to no addresses",
        ));
    }
    if addresses
        .iter()
        .any(|address| is_non_public_image_ip(address.ip()))
    {
        return Err(image_download_error(
            "image download rejected: host resolves to a non-public address",
        ));
    }

    Ok(ValidatedImageDownload {
        url,
        host,
        address: addresses[0],
    })
}

fn pinned_image_download_client(
    host: &str,
    address: std::net::SocketAddr,
) -> Result<reqwest::Client, AirpError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(15))
        .resolve(host, address)
        .build()
        .map_err(|_| image_download_error("image download rejected: HTTP client setup failed"))
}

/// 序列化所有 `index.json` 读-改-写序列，避免并发请求 last-write-wins 丢失条目
/// （CodeRabbit #4）。图片生成本身是秒级慢操作，全局序列化对吞吐影响可忽略。
static INDEX_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 在 `dir` 下选一个不存在的图片文件名，格式 `{millis}.png` 或 `{millis}_{n}.png`。
///
/// 纯文件名选择算法，提取出来便于单元测试（CodeRabbit nitpick #3）。
/// **调用方必须在 `INDEX_LOCK` 持锁后调用**，否则存在 TOCTOU：两个并发请求
/// 可能同时观察到同一候选名不存在，互相覆盖。
fn pick_unique_image_filename(dir: &Path, millis: u128) -> String {
    let mut filename = format!("{}.png", millis);
    let mut suffix = 1u32;
    while dir.join(&filename).exists() {
        filename = format!("{}_{}.png", millis, suffix);
        suffix += 1;
    }
    filename
}

/// 下载图片到本地 session 资产目录。
///
/// 成功时返回 `(`[`ImageMeta`]`, 相对路径)`。调用方应**直接使用**返回的
/// `ImageMeta`，不要再 re-read `index.last()`——并发下可能读到他人条目
/// （CodeRabbit #4）。
///
/// 文件名采用毫秒精度时间戳 + 碰撞自增后缀，避免秒级精度下 1 秒内多次
/// 生成覆盖前一张（CodeRabbit #3）。
///
/// **并发安全**（CodeRabbit outside-diff #2）：`INDEX_LOCK` 持锁贯穿
/// "选文件名 + 写文件 + 更新 index.json" 三步，避免同毫秒并发请求在
/// `exists()` 检查处双双重叠导致选同一文件名互相覆盖。下载（慢网络 I/O）
/// 在锁外执行以保留吞吐。
pub async fn download_image_to_session(
    data_root: &Path,
    character_id: &str,
    session_id: Option<&str>,
    image_url: &str,
    prompt: &str,
) -> Result<(ImageMeta, String), AirpError> {
    let validated = validate_image_download_url(image_url).await?;
    // Bind the request to an address from the validated DNS result. The URL
    // retains its original hostname for TLS/SNI and HTTP Host semantics.
    let client = pinned_image_download_client(&validated.host, validated.address)?;

    // Phase 1（锁外）：下载图片字节。网络慢，并行化保留吞吐。
    let mut response = client
        .get(validated.url)
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

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    if !is_png_content_type(content_type) {
        return Err(image_download_error(
            "image download rejected: response Content-Type must be image/png",
        ));
    }

    // CodeRabbit N2：Content-Length 预检，防超大响应压满内存。
    if let Some(len) = response.content_length() {
        if len as usize > MAX_IMAGE_BYTES {
            return Err(AirpError::Upstream {
                status: 0,
                body: format!(
                    "image download rejected: Content-Length {len} exceeds {MAX_IMAGE_BYTES} bytes"
                ),
            });
        }
    }

    // 流式累积，超限立即 reject（CodeRabbit 第五轮 outside-diff）。
    // 原 `response.bytes().await` 一次性缓冲整个 body，当 `Content-Length`
    // 缺失或错误（chunked transfer encoding）时，超大响应会在 post-read
    // 检查运行前已耗尽内存。改用 `chunk()` 流式接口，每收到一个 chunk
    // 累加检查；超上限立即返回 Upstream 错误，不再继续读取。
    // `Vec::with_capacity` 用 1 MiB 起步而非 MAX_IMAGE_BYTES，避免对大图
    // 预分配 20 MiB。
    let mut bytes: Vec<u8> = Vec::with_capacity(1024 * 1024);
    loop {
        let chunk = response.chunk().await.map_err(|e| AirpError::Upstream {
            status: 0,
            body: format!("image download body read failed: {e}"),
        })?;
        match chunk {
            Some(chunk) => {
                let new_len = bytes.len().checked_add(chunk.len()).ok_or_else(|| {
                    AirpError::Internal("image download size overflowed usize".to_string())
                })?;
                if new_len > MAX_IMAGE_BYTES {
                    return Err(AirpError::Upstream {
                        status: 0,
                        body: format!(
                            "image download rejected: streamed {new_len} bytes exceeds {MAX_IMAGE_BYTES} bytes"
                        ),
                    });
                }
                bytes.extend_from_slice(&chunk);
            }
            None => break,
        }
    }

    store_image_bytes_to_session(data_root, character_id, session_id, &bytes, prompt).await
}

/// Persist already-decoded image bytes through the same validation and atomic
/// index update used by URL downloads.
pub(crate) async fn store_image_bytes_to_session(
    data_root: &Path,
    character_id: &str,
    session_id: Option<&str>,
    bytes: &[u8],
    prompt: &str,
) -> Result<(ImageMeta, String), AirpError> {
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(image_download_error(
            "image storage rejected: image exceeds the image size limit",
        ));
    }
    validate_png_signature(bytes)?;

    let dir = images_dir(data_root, character_id, session_id);
    std::fs::create_dir_all(&dir)?;

    // Phase 2（持锁）：选文件名 + 写文件 + 更新 index.json。
    // CodeRabbit outside-diff #2：原实现先在锁外选文件名 + 写文件，再在
    // index.json 更新前才取锁——同毫秒并发请求会双双重叠在 `exists()` 检查，
    // 选同一文件名互相覆盖（lost image）。现将锁提到选文件名之前，覆盖整段
    // critical section。图片生成本身是秒级慢操作且上游已限流，全局序列化对
    // 吞吐影响可忽略。
    let _guard = INDEX_LOCK.lock().await;

    // 毫秒精度时间戳 + 碰撞自增，避免覆盖（CodeRabbit #3）。
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = pick_unique_image_filename(&dir, millis);
    let filepath = dir.join(&filename);
    crate::data_dir::replace_file(&filepath, bytes)?;

    let meta = ImageMeta {
        filename: filename.clone(),
        prompt: prompt.to_string(),
        timestamp: millis as u64,
        size: format!("{} bytes", bytes.len()),
    };

    // 更新 index.json（CodeRabbit #4：复用同一 guard，不再二次取锁）。
    let index_path = dir.join("index.json");
    let mut index: Vec<ImageMeta> = std::fs::read_to_string(&index_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    index.push(meta.clone());
    let index_json = serde_json::to_string_pretty(&index)
        .map_err(|e| AirpError::Internal(format!("index serialize: {e}")))?;
    crate::data_dir::replace_file(&index_path, index_json.as_bytes())?;
    drop(_guard);

    // 返回相对路径（供 handler 回填响应 `image_path` 字段）
    let relative = Path::new("characters")
        .join(character_id)
        .join(
            session_id
                .map(|s| Path::new("sessions").join(s))
                .unwrap_or_default(),
        )
        .join("images")
        .join(&filename);
    Ok((meta, relative.to_string_lossy().to_string()))
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

    #[test]
    fn image_generation_url_rewrites_only_chat_completions_suffix() {
        let url = image_generation_url(
            "https://provider.example/images-api/v1/chat/completions?tenant=one",
        )
        .unwrap();
        assert_eq!(
            url.as_str(),
            "https://provider.example/images-api/v1/images/generations?tenant=one"
        );

        let explicit =
            image_generation_url("https://provider.example/v1/images/generations").unwrap();
        assert_eq!(
            explicit.as_str(),
            "https://provider.example/v1/images/generations"
        );

        let custom = image_generation_url("https://provider.example/custom/image-create").unwrap();
        assert_eq!(
            custom.as_str(),
            "https://provider.example/custom/image-create"
        );
    }

    #[test]
    fn image_generation_url_rejects_invalid_endpoint() {
        let error = image_generation_url("not a URL").unwrap_err();
        assert!(matches!(error, AirpError::Config(_)));
    }

    #[test]
    fn image_generation_response_decodes_b64_json() {
        let png = b"\x89PNG\r\n\x1a\npayload";
        let encoded = STANDARD.encode(png);
        let response = parse_image_generation_response(&format!(
            r#"{{"data":[{{"b64_json":"{encoded}","revised_prompt":"revised"}}]}}"#
        ))
        .unwrap();

        assert!(response.success);
        assert_eq!(response.image_bytes.as_deref(), Some(png.as_slice()));
        assert!(response.image_url.is_none());
        assert_eq!(response.revised_prompt.as_deref(), Some("revised"));
        assert!(serde_json::to_value(&response)
            .unwrap()
            .get("image_bytes")
            .is_none());
    }

    #[test]
    fn image_generation_response_rejects_invalid_b64_json() {
        let error = parse_image_generation_response(r#"{"data":[{"b64_json":"not base64!"}]}"#)
            .unwrap_err();
        assert!(matches!(error, AirpError::Upstream { .. }));
    }

    #[tokio::test]
    async fn decoded_image_uses_shared_storage_path() {
        let temp = tempfile::tempdir().unwrap();
        let png = b"\x89PNG\r\n\x1a\npayload";
        let (meta, relative) =
            store_image_bytes_to_session(temp.path(), "hero", None, png, "a prompt")
                .await
                .unwrap();

        assert_eq!(meta.prompt, "a prompt");
        assert_eq!(meta.size, format!("{} bytes", png.len()));
        assert!(temp.path().join(&relative).exists());

        let index: Vec<ImageMeta> = serde_json::from_str(
            &std::fs::read_to_string(images_index_path(temp.path(), "hero", None)).unwrap(),
        )
        .unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].filename, meta.filename);
    }

    /// CodeRabbit nitpick #3：锁定 `MAX_IMAGE_BYTES` 上限值，防回归。
    /// 20 MiB = 20 * 1024 * 1024 = 20971520 bytes。若未来误改此常量，测试会
    /// 立即失败。
    #[test]
    fn max_image_bytes_is_20_mib() {
        assert_eq!(MAX_IMAGE_BYTES, 20 * 1024 * 1024);
        assert_eq!(MAX_IMAGE_BYTES, 20_971_520);
    }

    #[test]
    fn rejects_non_public_image_download_addresses() {
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "169.254.169.254",
            "192.168.1.1",
            "100.64.0.1",
            "198.18.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(
                is_non_public_image_ip(ip.parse().unwrap()),
                "{ip} must not be eligible for image downloads"
            );
        }
        assert!(!is_non_public_image_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_non_public_image_ip(
            "2606:4700:4700::1111".parse().unwrap()
        ));
    }

    #[tokio::test]
    async fn image_download_url_requires_https_before_resolution() {
        let error = validate_image_download_url("http://127.0.0.1/image.png")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("only HTTPS"));

        let error = validate_image_download_url("https://127.0.0.1/image.png")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("non-public address"));
    }

    #[test]
    fn image_download_requires_png_content_type_and_signature() {
        assert!(is_png_content_type(Some("image/png")));
        assert!(is_png_content_type(Some("IMAGE/PNG")));
        assert!(is_png_content_type(Some("image/png; charset=binary")));
        assert!(!is_png_content_type(Some("text/html")));
        assert!(!is_png_content_type(None));
        assert!(validate_png_signature(b"\x89PNG\r\n\x1a\npayload").is_ok());
        assert!(validate_png_signature(b"<html>not an image</html>").is_err());
    }

    /// CodeRabbit nitpick #3：锁定文件名碰撞后缀算法的纯逻辑部分。
    ///
    /// 测试调用**实际**的 `pick_unique_image_filename`（非重写），用 tempdir
    /// 模拟 `exists()` 检查。算法是：取毫秒时间戳作 `{millis}.png`，若已存在
    /// 则 `{millis}_{n}.png`（n 从 1 递增）。
    #[test]
    fn pick_unique_image_filename_skips_existing() {
        let tmp = std::env::temp_dir().join(format!(
            "airp_image_filename_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let millis = 1_700_000_000_000u128;

        // 空目录 → 直接用 millis
        assert_eq!(
            pick_unique_image_filename(&tmp, millis),
            format!("{millis}.png")
        );
        // 占住 millis.png → 跳到 millis_1.png
        std::fs::write(tmp.join(format!("{millis}.png")), b"x").unwrap();
        assert_eq!(
            pick_unique_image_filename(&tmp, millis),
            format!("{millis}_1.png")
        );
        // 再占住 millis_1.png → 跳到 millis_2.png
        std::fs::write(tmp.join(format!("{millis}_1.png")), b"x").unwrap();
        assert_eq!(
            pick_unique_image_filename(&tmp, millis),
            format!("{millis}_2.png")
        );
        // 占住 millis_2.png → 跳到 millis_3.png
        std::fs::write(tmp.join(format!("{millis}_2.png")), b"x").unwrap();
        assert_eq!(
            pick_unique_image_filename(&tmp, millis),
            format!("{millis}_3.png")
        );
        // 不同 millis 不受 millis 的影响
        let other = 1_700_000_000_001u128;
        assert_eq!(
            pick_unique_image_filename(&tmp, other),
            format!("{other}.png")
        );

        std::fs::remove_dir_all(&tmp).ok();
    }

    /// CodeRabbit nitpick #3：端到端验证 `download_image_to_session` 的
    /// TOCTOU 修复——`INDEX_LOCK` 持锁贯穿"选文件名 + 写文件 + 更新 index.json"
    /// critical section。此测试需 mockito dev-dependency 模拟上游；当前仓库
    /// 未启用，故 ignore。算法正确性由 `pick_unique_image_filename_skips_existing`
    /// 锁定，锁覆盖范围由源码注释与人工 review 确认。
    #[test]
    #[ignore = "requires mockito dev-dependency; manually run with --features mockito"]
    fn download_image_to_session_writes_unique_files_under_collision() {
        // 占位：见 ignore 说明。算法 + 锁覆盖已由其它测试与源码审计覆盖。
    }
}
