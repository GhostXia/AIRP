//! Trusted Plugin HTTP 面（#498 §6.4 / §7.3）：反代路由与列表查询。
//!
//! - `GET/POST /api/plugins/:id/*path` → 反代到 `127.0.0.1:<port>/*path`
//!   （挂在鉴权层外：widget iframe 沙箱无 daemon token，且 §6.4 不做 caller
//!   限制——loopback 上任何进程都能调，trusted plugin 自己校验请求）。
//! - `GET /v1/plugins`（鉴权层内，webui 查询用）→ 已安装列表。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use serde_json::{json, Value};

use crate::daemon::DaemonState;
use crate::plugins::TrustedPluginManifest;

/// 反代超时（复用 plugin_tool 的超时量级，#498 §6.4）。
const PROXY_TIMEOUT_SECS: u64 = 30;

fn error_body(status: StatusCode, code: &str, message: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message.into() } })),
    )
        .into_response()
}

/// 从 state 中按 id 找 manifest（id 不存在 → 404）。
fn find_manifest(state: &DaemonState, id: &str) -> Option<TrustedPluginManifest> {
    state
        .plugins
        .read()
        .ok()?
        .iter()
        .find(|m| m.id == id)
        .cloned()
}

/// `GET/POST /api/plugins/:plugin_id/*path` — 反向代理到插件子进程。
///
/// 只透传 method / path / query / body / Content-Type；**不透传** daemon 的
/// Authorization / Cookie / Origin 等头（daemon 凭据不泄漏给插件；插件应
/// 自行校验请求来源与 body schema，见 #498 §7.2）。
pub async fn proxy_plugin(
    Path((plugin_id, path)): Path<(String, String)>,
    State(state): State<Arc<DaemonState>>,
    req: axum::extract::Request,
) -> Response {
    let Some(manifest) = find_manifest(&state, &plugin_id) else {
        return error_body(
            StatusCode::NOT_FOUND,
            "plugin_not_found",
            format!("trusted plugin {plugin_id} is not installed"),
        );
    };

    // axum wildcard `*path` 捕获值不带前导 `/`（0.7 语义）；补回后与 query 拼接。
    let mut url = format!("http://127.0.0.1:{}/{}", manifest.port, path);
    if let Some(query) = req.uri().query() {
        url.push('?');
        url.push_str(query);
    }

    let method = req.method().clone();
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| header::HeaderValue::from_static("application/octet-stream"));
    // 请求 body 受 axum 默认 2MB 上限约束（反代路由未放大）。
    let body = match axum::body::to_bytes(req.into_body(), 2 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(e) => {
            return error_body(
                StatusCode::BAD_REQUEST,
                "plugin_bad_request",
                format!("failed to read request body: {e}"),
            );
        }
    };

    let forwarded = state
        .http_client
        .request(method, &url)
        .header(header::CONTENT_TYPE, content_type)
        .body(body)
        .timeout(std::time::Duration::from_secs(PROXY_TIMEOUT_SECS))
        .send()
        .await;

    match forwarded {
        Ok(resp) => {
            let status = resp.status();
            let resp_content_type = resp
                .headers()
                .get(header::CONTENT_TYPE)
                .cloned()
                .unwrap_or_else(|| header::HeaderValue::from_static("application/octet-stream"));
            match resp.bytes().await {
                Ok(resp_body) => (status, [(header::CONTENT_TYPE, resp_content_type)], resp_body)
                    .into_response(),
                Err(e) => {
                    tracing::error!(plugin = %plugin_id, %e, "trusted plugin response body read failed");
                    error_body(
                        StatusCode::BAD_GATEWAY,
                        "plugin_unreachable",
                        format!("failed to read plugin response: {e}"),
                    )
                }
            }
        }
        Err(e) => {
            // 连接失败 / 超时：插件未起、端口冲突或已崩溃（§6.5/§6.7）。
            tracing::warn!(plugin = %plugin_id, url = %url, %e, "trusted plugin proxy failed");
            error_body(
                StatusCode::BAD_GATEWAY,
                "plugin_unreachable",
                format!("plugin {plugin_id} unreachable: {e}"),
            )
        }
    }
}

/// `GET /v1/plugins` — 列出已安装 trusted plugin（id / version / host_api /
/// 启停状态）。状态仅反映 spawn 结果（children map），**不探活**（§6.6）。
pub async fn list_plugins(State(state): State<Arc<DaemonState>>) -> Response {
    let manifests = state.plugins.read().map(|g| g.clone()).unwrap_or_default();
    let children = state.plugin_children.lock().await;
    let items: Vec<Value> = manifests
        .iter()
        .map(|m| {
            let status = if children.contains_key(&m.id) {
                "running"
            } else {
                "stopped"
            };
            json!({
                "id": m.id,
                "version": m.version,
                "host_api": m.host_api,
                "status": status,
            })
        })
        .collect();
    Json(json!({ "plugins": items })).into_response()
}
