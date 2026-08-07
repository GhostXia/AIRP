//! Trusted Plugin HTTP 面（#498 §6.4 / §7.3）：反代路由与列表查询。
//!
//! - `GET/POST /api/plugins/:id/*path` → 反代到 `127.0.0.1:<port>/*path`
//!   （挂在鉴权层外：widget iframe 沙箱无 daemon token，且 §6.4 不做 caller
//!   限制——loopback 上任何进程都能调，trusted plugin 自己校验请求）。
//! - `GET /v1/plugins`（鉴权层内，webui 查询用）→ 已安装列表。
//!
//! 反代的安全与健壮性约束（审计 W2/W5/W7 + CodeRabbit）：
//! - loopback-only：`ConnectInfo` 校验 peer 为 loopback 地址（0.0.0.0 监听
//!   时远程请求被拒，不依赖路由层位置）。
//! - 保留编码：从原始 URI path 手工切出 wildcard 段（axum 的 `RawPathParams`
//!   与 `Path` 都会 percent-decode，`%2F` 解码后信息不可逆丢失），仅补码会
//!   被 URL 解析器误读的字符（空格 / `#` / 非 ASCII），`%XX` 与 RFC 3986
//!   字符原样。
//! - SSE 流式透传：`text/event-stream` 分块转发并监听 shutdown 广播提前
//!   断开（整体缓冲会在 30s 超时后断流，且阻塞 daemon 优雅退出）。
//! - 响应体上限：非流式响应累计 2MB，`Content-Length` 预检超限直接拒绝。
//! - 脱敏：日志与错误响应不携带目标 URL / query / 传输错误细节。

use std::sync::Arc;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Json, Response};
use futures_util::Stream;
use serde_json::{json, Value};

use crate::daemon::DaemonState;
use crate::plugins::TrustedPluginManifest;

/// 反代超时（复用 plugin_tool 的超时量级，#498 §6.4）。
const PROXY_TIMEOUT_SECS: u64 = 30;
/// 反代响应体上限（非流式；SSE 流式不在此限）。与请求体 2MB 对称。
const PROXY_RESPONSE_LIMIT: usize = 2 * 1024 * 1024;

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

/// 对反代目标 path / query 做最小重编码：保留已有 `%XX` 转义与 RFC 3986
/// 合法字符，仅编码会被 URL 解析器误读的字节（空格 / `#` / 控制字符 /
/// 非 ASCII UTF-8）。`allow_query` 时额外保留 `?` 与 `[]`（query 合法集）。
///
/// 幂等性：已编码输入（正常浏览器请求）原样保留；未编码输入（裸 curl）
/// 被补码，两种情况下插件收到的路径语义一致（审计 W5 / CodeRabbit）。
fn reencode_url_component(s: &str, allow_query: bool) -> String {
    let mut out = String::with_capacity(s.len());
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        if ch.is_ascii() {
            let b = ch as u8;
            let allowed = b.is_ascii_alphanumeric()
                || matches!(
                    b,
                    b'-' | b'.'
                        | b'_'
                        | b'~'
                        | b'!'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b'*'
                        | b'+'
                        | b','
                        | b';'
                        | b'='
                        | b':'
                        | b'@'
                        | b'/'
                        | b'%'
                )
                || (allow_query && matches!(b, b'?' | b'[' | b']'));
            if allowed {
                out.push(ch);
            } else {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            for &b in ch.encode_utf8(&mut buf).as_bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// 反代上游响应错误分类（决定 502 code 与日志级别）。
enum UpstreamBodyError {
    TooLarge,
    Read(std::io::Error),
}

/// 分块读取非流式响应体，累计上限 [`PROXY_RESPONSE_LIMIT`]；
/// `Content-Length` 声明超限在读前直接拒绝（CodeRabbit）。
async fn bounded_response_body(mut resp: reqwest::Response) -> Result<Vec<u8>, UpstreamBodyError> {
    if let Some(len) = resp.content_length() {
        if len > PROXY_RESPONSE_LIMIT as u64 {
            return Err(UpstreamBodyError::TooLarge);
        }
    }
    let mut out = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| UpstreamBodyError::Read(std::io::Error::other(e.to_string())))?
    {
        if out.len() + chunk.len() > PROXY_RESPONSE_LIMIT {
            return Err(UpstreamBodyError::TooLarge);
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// 流式响应（SSE）转 axum Body：分块转发 + 监听 shutdown 广播。
///
/// 审计 W1/W2：整体缓冲会在 30s 超时后断流（SSE 心跳周期可能超过超时）；
/// 且 daemon 优雅退出时 axum 会等待在飞请求完成，挂起的 SSE 会阻塞退出。
/// 此流在 shutdown 广播后立即返回 `None`（EOF），两端问题同时解决。
fn stream_or_shutdown(
    resp: reqwest::Response,
    shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> impl Stream<Item = Result<bytes::Bytes, std::io::Error>> {
    // state 携带所有权在 unfold 迭代间传递（闭包参数按值传入，
    // async move 块可安全捕获，避免借用逃逸 FnMut 闭包）。
    futures_util::stream::unfold(
        (resp, shutdown_rx),
        |(mut resp, mut shutdown_rx)| async move {
            tokio::select! {
                _ = shutdown_rx.changed() => None,
                chunk = resp.chunk() => match chunk {
                    Ok(Some(bytes)) => Some((Ok(bytes), (resp, shutdown_rx))),
                    Ok(None) => None,
                    Err(e) => Some((Err(std::io::Error::other(e.to_string())), (resp, shutdown_rx))),
                },
            }
        },
    )
}

/// `GET/POST /api/plugins/:plugin_id/*path` — 反向代理到插件子进程。
///
/// 只透传 method / path / query / body / Content-Type；**不透传** daemon 的
/// Authorization / Cookie / Origin 等头（daemon 凭据不泄漏给插件；插件应
/// 自行校验请求来源与 body schema，见 #498 §7.2）。
pub async fn proxy_plugin(
    Path((plugin_id, path)): Path<(String, String)>,
    State(state): State<Arc<DaemonState>>,
    connect_info: Option<ConnectInfo<std::net::SocketAddr>>,
    req: axum::extract::Request,
) -> Response {
    // 审计 W7 / B4 修复：反代挂在鉴权层外，0.0.0.0 监听时远程请求可直达
    // 插件。用 peer 地址强制 loopback-only。fail-closed：无 ConnectInfo
    // 时直接拒绝——生产 serve 必经 into_make_service_with_connect_info
    // 注入 peer 地址，缺失意味着 serve 拓扑异常（自定义 router 嵌入等），
    // 不应放行。测试通过 Extension(ConnectInfo(loopback)) 显式注入。
    let addr = match connect_info {
        Some(ConnectInfo(addr)) => addr,
        None => {
            tracing::warn!(
                "trusted plugin proxy request without ConnectInfo — rejecting (fail-closed)"
            );
            return error_body(
                StatusCode::FORBIDDEN,
                "plugin_remote_forbidden",
                "trusted plugin proxy is restricted to loopback clients",
            );
        }
    };
    if !addr.ip().is_loopback() {
        return error_body(
            StatusCode::FORBIDDEN,
            "plugin_remote_forbidden",
            "trusted plugin proxy is restricted to loopback clients",
        );
    }

    let Some(manifest) = find_manifest(&state, &plugin_id) else {
        return error_body(
            StatusCode::NOT_FOUND,
            "plugin_not_found",
            format!("trusted plugin {plugin_id} is not installed"),
        );
    };

    // 用原始 URI path 切出 wildcard 段：axum 的 `RawPathParams` / `Path`
    // 都会 percent-decode（`%2F` → `/`，信息不可逆丢失；审计 W5）。
    // `http::Uri` 的 path 部分保持原始字节，此处与路由
    // `/api/plugins/:plugin_id/*path` 的捕获范围完全一致。
    let raw_path = req
        .uri()
        .path()
        .strip_prefix("/api/plugins/")
        .and_then(|rest| rest.find('/').map(|i| &rest[i + 1..]))
        .unwrap_or(&path);
    let encoded_path = reencode_url_component(raw_path, false);
    let url = format!("http://127.0.0.1:{}/{}", manifest.port, encoded_path);
    // query 本身未解码；同样补码防 `#` 被当作 fragment 分隔符。
    let url = match req.uri().query() {
        Some(query) => format!("{url}?{}", reencode_url_component(query, true)),
        None => url,
    };

    let method = req.method().clone();
    let content_type = req
        .headers()
        .get(header::CONTENT_TYPE)
        .cloned()
        .unwrap_or_else(|| header::HeaderValue::from_static("application/octet-stream"));
    // 请求 body 受 axum 默认 2MB 上限约束（反代路由未放大）。
    let body = match axum::body::to_bytes(req.into_body(), 2 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            return error_body(
                StatusCode::BAD_REQUEST,
                "plugin_bad_request",
                "failed to read request body (limit 2MB)",
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
            // SSE → 流式透传（W2：不缓冲、不限长、shutdown 可中断）。
            if resp_content_type
                .to_str()
                .is_ok_and(|v| v.starts_with("text/event-stream"))
            {
                let body = axum::body::Body::from_stream(stream_or_shutdown(
                    resp,
                    state.shutdown.subscribe(),
                ));
                return (status, [(header::CONTENT_TYPE, resp_content_type)], body).into_response();
            }
            match bounded_response_body(resp).await {
                Ok(resp_body) => (
                    status,
                    [(header::CONTENT_TYPE, resp_content_type)],
                    resp_body,
                )
                    .into_response(),
                Err(UpstreamBodyError::TooLarge) => {
                    tracing::warn!(plugin = %plugin_id, "trusted plugin response exceeded 2MB limit");
                    error_body(
                        StatusCode::BAD_GATEWAY,
                        "plugin_response_too_large",
                        "trusted plugin response exceeded 2MB limit",
                    )
                }
                Err(UpstreamBodyError::Read(e)) => {
                    tracing::warn!(plugin = %plugin_id, %e, "trusted plugin response body read failed");
                    error_body(
                        StatusCode::BAD_GATEWAY,
                        "plugin_unreachable",
                        "failed to read trusted plugin response",
                    )
                }
            }
        }
        Err(e) => {
            // 连接失败 / 超时：插件未起、端口冲突或已崩溃（§6.5/§6.7）。
            // 脱敏：不记录完整 URL（query 可能含凭据），不向调用方暴露
            // 传输细节（CodeRabbit）。
            tracing::warn!(plugin = %plugin_id, %e, "trusted plugin proxy failed");
            error_body(
                StatusCode::BAD_GATEWAY,
                "plugin_unreachable",
                format!("trusted plugin {plugin_id} is unreachable"),
            )
        }
    }
}

/// `GET /v1/plugins` — 列出已安装 trusted plugin（id / version / host_api /
/// 启停状态）。状态仅反映 children map（spawn 结果，崩溃监控实时移除），
/// **不探活**（§6.6）。
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
