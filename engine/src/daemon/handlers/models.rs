//! Models proxy HTTP handler — proxy upstream provider's /models endpoint.
//!
//! #155 PR5：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 upstream orchestration；URL 推导、endpoint 脱敏、error shape 在本模块私有 helper。
//!
//! 端点：
//! - `GET /v1/models` — 代理上游 provider 的 /models，带 timeout / redirect / 脱敏

use crate::daemon::DaemonState;
use crate::error::AirpError;
use axum::{
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

const MODELS_PROXY_TIMEOUT_DEFAULT: Duration = Duration::from_secs(5);

/// #42 F-6：/v1/models 上游超时。默认 5s，可用 `AIRP_MODELS_PROXY_TIMEOUT_MS`
/// 覆盖（跨境 provider 偏慢时无需重编译；测试也借此走快速超时路径）。
fn models_proxy_timeout() -> Duration {
    std::env::var("AIRP_MODELS_PROXY_TIMEOUT_MS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|ms| *ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(MODELS_PROXY_TIMEOUT_DEFAULT)
}

/// GET /v1/models — proxy the upstream provider's /models endpoint.
pub(in crate::daemon) async fn list_models(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Response {
    let (endpoint, api_key) = {
        // #42 F-2：与 get_settings/update_settings 一致，poisoned lock 恢复而非 panic。
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        (cfg.endpoint.clone(), cfg.api_key.clone())
    };

    let models_url = match models_url_from_endpoint(&endpoint) {
        Some(url) => url,
        None => {
            let redacted = redact_endpoint_for_error(&endpoint);
            tracing::warn!(endpoint = %redacted, "models proxy: endpoint cannot be mapped to a /models URL");
            return models_proxy_error(
                StatusCode::BAD_GATEWAY,
                "invalid_endpoint",
                "provider endpoint cannot be mapped to a /models URL",
                None,
                None,
                Some(redacted),
            );
        }
    };

    let timeout = models_proxy_timeout();
    let mut req = state.http_client.get(&models_url).timeout(timeout);
    if let Some(key) = &api_key {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
    }

    match req.send().await {
        Ok(resp) => {
            // #117 A：redirect 必须先于 success/non-success 分流判定，给 typed 脱敏文案。
            if let Some(classified) = crate::outbound::classify_redirect_response(&resp) {
                let upstream_status = match &classified {
                    AirpError::Upstream { status, .. } => *status,
                    _ => unreachable!("redirect classifier must return AirpError::Upstream"),
                };
                tracing::warn!(
                    upstream_status,
                    "models proxy: upstream redirected; outbound policy rejected"
                );
                return models_proxy_error(
                    StatusCode::BAD_GATEWAY,
                    "upstream_redirect_rejected",
                    format!(
                        "model provider /models redirected; outbound policy rejected to protect credentials: {}",
                        classified
                    ),
                    Some(upstream_status),
                    None,
                    None,
                );
            }
            let status = resp.status();
            match resp.bytes().await {
                Ok(body) if status.is_success() => (
                    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::OK),
                    [(header::CONTENT_TYPE, "application/json")],
                    body,
                )
                    .into_response(),
                Ok(body) => {
                    // #42 F-3：非 2xx 上游留痕，便于诊断（body 已截断脱敏后进响应）。
                    tracing::warn!(
                        upstream_status = status.as_u16(),
                        "models proxy: upstream returned non-success status"
                    );
                    models_proxy_error(
                        StatusCode::BAD_GATEWAY,
                        "upstream_status",
                        format!("model provider /models returned HTTP {}", status.as_u16()),
                        Some(status.as_u16()),
                        Some(truncate_error_text(&String::from_utf8_lossy(&body))),
                        None,
                    )
                }
                Err(e) => {
                    tracing::warn!(upstream_status = status.as_u16(), error = %e, "models proxy: failed to read upstream body");
                    models_proxy_error(
                        StatusCode::BAD_GATEWAY,
                        "upstream_body_read_failed",
                        "failed to read model provider /models response body",
                        Some(status.as_u16()),
                        None,
                        Some(e.to_string()),
                    )
                }
            }
        }
        Err(e) if e.is_timeout() => {
            tracing::warn!(
                timeout_ms = timeout.as_millis() as u64,
                "models proxy: upstream request timed out"
            );
            models_proxy_error(
                StatusCode::GATEWAY_TIMEOUT,
                "upstream_timeout",
                format!(
                    "model provider /models timed out after {}ms",
                    timeout.as_millis()
                ),
                None,
                None,
                None,
            )
        }
        Err(e) => {
            tracing::warn!(error = %e, "models proxy: upstream request failed");
            models_proxy_error(
                StatusCode::BAD_GATEWAY,
                "upstream_request_failed",
                "model provider /models request failed",
                None,
                None,
                None,
            )
        }
    }
}

/// 从 chat endpoint 推导 /models URL。
///
/// #42 F-1：改为基于 URL 解析推导，杜绝字符串 rfind('/') 在无路径 endpoint
/// （如 `http://example.com`）上命中 scheme 分隔符产生 `http://models` 之类
/// 丢失 host 的畸形 URL。规则：
/// - 非 http(s) 或无 host → None（走 invalid_endpoint 类型化错误）；
/// - 路径含 `/v1/` → 前缀 + `/v1/models`（OpenAI 兼容主路径）；
/// - 否则保守 fallback：把最后一个路径段替换为 `models`；无路径段则 None。
///
/// 推导结果一律剥离 query/fragment，避免把 endpoint 上的凭据带去 /models。
fn models_url_from_endpoint(endpoint: &str) -> Option<String> {
    let mut url = reqwest::Url::parse(endpoint).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    let path = url.path().to_string();
    let new_path = if let Some(pos) = path.find("/v1/") {
        format!("{}/v1/models", &path[..pos])
    } else {
        let trimmed = path.trim_end_matches('/');
        let pos = trimmed.rfind('/')?;
        if trimmed[pos + 1..].is_empty() {
            // 无有效路径段（如 "http://example.com" 或 "http://example.com/"）
            return None;
        }
        format!("{}/models", &trimmed[..pos])
    };
    url.set_path(&new_path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn redact_endpoint_for_error(endpoint: &str) -> String {
    if let Ok(mut url) = reqwest::Url::parse(endpoint) {
        if !url.username().is_empty() {
            let _ = url.set_username("redacted");
        }
        if url.password().is_some() {
            let _ = url.set_password(Some("redacted"));
        }
        if url.query().is_some() {
            url.set_query(Some("redacted"));
        }
        // #40 建议 2：fragment 虽不发往服务端，但用户可能误把 secret 放在 # 后。
        if url.fragment().is_some() {
            url.set_fragment(Some("redacted"));
        }
        return url.to_string();
    }
    if let Some(pos) = endpoint.find(['?', '#']) {
        return format!("{}?redacted", &endpoint[..pos]);
    }
    endpoint.to_string()
}

#[derive(Debug, Serialize)]
struct ModelsProxyError {
    error: ModelsProxyErrorBody,
}

#[derive(Debug, Serialize)]
struct ModelsProxyErrorBody {
    code: &'static str,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    upstream_body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

fn models_proxy_error(
    status: StatusCode,
    code: &'static str,
    message: impl Into<String>,
    upstream_status: Option<u16>,
    upstream_body: Option<String>,
    detail: Option<String>,
) -> Response {
    (
        status,
        Json(ModelsProxyError {
            error: ModelsProxyErrorBody {
                code,
                message: message.into(),
                upstream_status,
                upstream_body,
                detail,
            },
        }),
    )
        .into_response()
}

fn truncate_error_text(text: &str) -> String {
    const MAX_ERROR_BODY_CHARS: usize = 2048;
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= MAX_ERROR_BODY_CHARS {
            out.push_str("...");
            return out;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_url_v1_endpoint_maps_to_v1_models() {
        assert_eq!(
            models_url_from_endpoint("https://api.example.com/v1/chat/completions"),
            Some("https://api.example.com/v1/models".to_string())
        );
    }

    #[test]
    fn models_url_no_path_endpoint_returns_none() {
        // #42 F-1：旧实现产生丢失 host 的 "http://models"，现在必须拒绝。
        assert_eq!(models_url_from_endpoint("http://example.com"), None);
        assert_eq!(models_url_from_endpoint("http://example.com/"), None);
    }

    #[test]
    fn models_url_non_http_scheme_returns_none() {
        assert_eq!(models_url_from_endpoint("file:///etc/passwd"), None);
        assert_eq!(models_url_from_endpoint("not-a-url"), None);
    }

    #[test]
    fn models_url_fallback_replaces_last_segment() {
        assert_eq!(
            models_url_from_endpoint("https://api.example.com/api/chat/completions"),
            Some("https://api.example.com/api/chat/models".to_string())
        );
    }

    #[test]
    fn models_url_strips_query_and_fragment() {
        assert_eq!(
            models_url_from_endpoint(
                "https://api.example.com/v1/chat/completions?api_key=secret#frag"
            ),
            Some("https://api.example.com/v1/models".to_string())
        );
    }

    #[test]
    fn redact_endpoint_clears_userinfo_password_query_fragment() {
        let redacted = redact_endpoint_for_error(
            "https://user:hunter2@api.example.com/v1/chat?api_key=secret#token=secret2",
        );
        assert!(!redacted.contains("hunter2"), "password leaked: {redacted}");
        assert!(!redacted.contains("user:"), "username leaked: {redacted}");
        assert!(
            !redacted.contains("secret"),
            "query/fragment leaked: {redacted}"
        );
        assert!(redacted.contains("api.example.com"));
    }

    #[test]
    fn redact_endpoint_unparseable_with_fragment() {
        assert_eq!(
            redact_endpoint_for_error("not-a-url#token=secret"),
            "not-a-url?redacted"
        );
    }
}
