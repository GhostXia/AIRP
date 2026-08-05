//! C-P0: 桌面壳 bearer 注入通道（desktop session token）。
//!
//! 背景：Tauri 桌面壳以进程级随机 access key 启动内置 engine sidecar
//! （`ui/src-tauri/src/main.rs`），但 engine 同源承载的 webui 运行在 WebView2
//! 里，无法直接拿到该 key（也绝不应拿到——access key 是全权凭据）。
//!
//! 通道合同（获批计划 C-P0 / 集成架构研究 §2.5）：
//! 1. 壳持 access key 调 `POST /v1/desktop-session`（loopback 进程互信，经
//!    `auth_middleware` 全量鉴权）换取**短时效** UI token；
//! 2. 壳把 token 以 URL fragment（`#airp-token=...`）传入 webview 首屏——
//!    fragment 不发送到服务端、不进访问日志、不进 Referer；
//! 3. webui 首屏引导脚本（`webui/assets/entry.js`）写入
//!    `sessionStorage.airp_bearer` 后清理 URL，承接 `api-client.js` 既有的
//!    同源 bearer 泄漏防护；
//! 4. `auth_middleware` 同时接受 access key 与有效 desktop token。
//!
//! 安全性质：
//! - token 与 access key 不同源：泄露 token 只暴露一个 8 小时时效的会话凭据，
//!   重启 engine 即全部失效（存储仅在内存，绝不落盘）；
//! - 端点仅在 daemon 已配置 access_api_key 时可用（进程互信前提成立）；
//!   local-webui 便携模式（无 key、无鉴权）下返回 403 fail-closed——该模式
//!   本就无需 bearer；
//! - 该端点同时被 GovernorLayer 限流与 auth_middleware 覆盖（挂在 v1 路由内）。
//!
//! LOCK-ORDER：`DESKTOP_SESSION_TOKENS` 是全局 utility 叶锁（§1.5 / R4），
//! 临界区内不得获取任何其他锁。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use super::DaemonState;

/// 桌面会话 token 时效：8 小时（一个工作日量级；engine 重启即失效）。
pub const DESKTOP_SESSION_TTL_SECS: u64 = 8 * 60 * 60;

/// 生效 TTL：`AIRP_DESKTOP_SESSION_TTL_SECS` 可覆盖（clamp 10s..=8h）。
/// 冒烟/测试用短 TTL 验证续期链路；生产不设即默认 8h。
fn desktop_session_ttl_secs() -> u64 {
    std::env::var("AIRP_DESKTOP_SESSION_TTL_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(10, DESKTOP_SESSION_TTL_SECS))
        .unwrap_or(DESKTOP_SESSION_TTL_SECS)
}

/// 进程级 token 存储：token -> 过期时刻（unix 秒）。仅内存，绝不落盘。
/// daemon 单进程模型下全局唯一；测试并行时 token 为随机值，互不冲突。
static DESKTOP_SESSION_TOKENS: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();

fn token_store() -> &'static Mutex<HashMap<String, u64>> {
    DESKTOP_SESSION_TOKENS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 签发一个新的桌面会话 token，返回 `(token, expires_in_secs)`。
/// 顺带惰性清理过期条目，防止长期运行累积。
pub fn mint_desktop_session_token() -> (String, u64) {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let ttl = desktop_session_ttl_secs();
    let now = now_unix_secs();
    let mut store = token_store().lock().unwrap_or_else(|e| e.into_inner());
    store.retain(|_, expires_at| *expires_at > now);
    store.insert(token.clone(), now + ttl);
    (token, ttl)
}

/// C-P2 token 续期：**rotation** 语义——旧 token 有效则撤销并签发新 token；
/// 旧 token 无效/过期返回 `None`。单锁内原子完成，杜绝「旧新并存」窗口。
/// rotation 而非延期：泄露的旧 token 立即失效，续期本身成为一次凭据刷新。
pub fn renew_desktop_session_token(old_token: &str) -> Option<(String, u64)> {
    if old_token.is_empty() {
        return None;
    }
    let ttl = desktop_session_ttl_secs();
    let now = now_unix_secs();
    let mut store = token_store().lock().unwrap_or_else(|e| e.into_inner());
    store.retain(|_, expires_at| *expires_at > now);
    store.remove(old_token).as_ref()?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    store.insert(token.clone(), now + ttl);
    Some((token, ttl))
}

/// 校验桌面会话 token 是否有效（存在且未过期）。过期条目惰性移除。
pub fn validate_desktop_session_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let now = now_unix_secs();
    let mut store = token_store().lock().unwrap_or_else(|e| e.into_inner());
    let mut valid = false;
    // 统一惰性清理全部过期条目；被校验 token 已过期时同样被移除。
    store.retain(|candidate, expires_at| {
        let alive = *expires_at > now;
        if candidate == token {
            valid = alive;
        }
        alive
    });
    valid
}

/// 测试专用：清空全局 token 存储，保证用例间隔离。
#[cfg(test)]
pub(crate) fn clear_desktop_session_tokens_for_test() {
    token_store()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
}

/// 测试专用串行锁（#485 E6）：token store 是进程级全局，`cargo test`
/// 并行线程里一个用例的 clear 会扫掉另一用例刚 mint 的 token，产生
/// 随机竞态失败。所有触碰全局 token store 的测试（clear / mint /
/// validate / 续期）必须在开头持有本锁至用例结束，把这类测试串行化。
#[cfg(test)]
static TOKEN_TEST_LOCK: Mutex<()> = Mutex::new(());

/// 获取 token store 测试串行锁（poison 时 recover 而非 panic）。
#[cfg(test)]
#[must_use = "锁守卫必须持有至测试结束，否则串行化失效"]
pub(crate) fn token_test_lock() -> std::sync::MutexGuard<'static, ()> {
    TOKEN_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `POST /v1/desktop-session`：进程互信换短时效 UI token。
///
/// 请求无 body；鉴权由 v1 路由层的 `auth_middleware`（access key）完成。
/// fail-closed：daemon 未配置 access_api_key 时返回 403——没有进程互信
/// 前提（local-webui 便携模式无鉴权，本就不需要 bearer 注入）。
pub async fn desktop_session_endpoint(
    State(state): State<std::sync::Arc<DaemonState>>,
) -> impl IntoResponse {
    let key_configured = {
        let cfg = state.read_config();
        cfg.access_api_key.as_deref().is_some_and(|k| !k.is_empty())
    };
    if !key_configured {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": {
                    "code": "desktop_session_unavailable",
                    "message": "desktop session exchange requires daemon access key authentication",
                }
            })),
        )
            .into_response();
    }
    let (token, expires_in) = mint_desktop_session_token();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "token": token,
            "token_type": "Bearer",
            "expires_in": expires_in,
        })),
    )
        .into_response()
}

/// `POST /v1/desktop-session/renew`：C-P2 token 续期（rotation）。
///
/// 壳/webui 在 TTL 过半时主动续期，避免 8h 边界会话锁死（对应 C-P1
/// 交接的 token 续期项）。请求以 `Authorization: Bearer <旧token>` 携带
/// 待续期 token（鉴权层已接受它）；成功返回新 token 并**立即撤销旧 token**。
/// fail-closed 与 exchange 端点一致：无 access key 配置时 403；旧 token
/// 无效/过期时 401（壳收到 401 应回到 exchange 重交换路径）。
pub async fn desktop_session_renew_endpoint(
    State(state): State<std::sync::Arc<DaemonState>>,
    headers: axum::http::HeaderMap,
) -> axum::response::Response {
    let key_configured = {
        let cfg = state.read_config();
        cfg.access_api_key.as_deref().is_some_and(|k| !k.is_empty())
    };
    if !key_configured {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": {
                    "code": "desktop_session_unavailable",
                    "message": "desktop session renewal requires daemon access key authentication",
                }
            })),
        )
            .into_response();
    }
    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let Some(old_token) = provided else {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    };
    // access key 本身也可通过鉴权层；但续期只接受 desktop token——
    // 对 access key 做 rotation 无意义且会把全权凭据误当会话 token 撤销对象。
    let is_access_key = {
        let cfg = state.read_config();
        cfg.access_api_key.as_deref() == Some(old_token)
    };
    if is_access_key {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": {
                    "code": "renew_requires_desktop_token",
                    "message": "renewal expects a desktop session token, not the daemon access key",
                }
            })),
        )
            .into_response();
    }
    match renew_desktop_session_token(old_token) {
        Some((token, expires_in)) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "token": token,
                "token_type": "Bearer",
                "expires_in": expires_in,
            })),
        )
            .into_response(),
        None => (StatusCode::UNAUTHORIZED, "Unauthorized").into_response(),
    }
}
