//! Daemon state, config, auth middleware, and axum router factory.

pub(crate) mod decompose_handlers;
pub(crate) mod handlers;
pub mod types;

pub use types::{
    ChatCompletionRequest, ChatResponseChunk, HistoryQuery, RegenRequest, RollbackRequest,
    UserProfile,
};

use crate::adapter::{BackendEngine, Provider};
use crate::config::{DeploymentMode, VolumeConfig};
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit},
    handler::Handler,
    http::{header, HeaderValue, Method, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;

use decompose_handlers::{
    decompose_character, decompose_preset, enhance_or_apply_character_analysis,
    get_character_analysis_file, list_character_analysis,
};
use handlers::{
    add_scene_character_endpoint, agent_run, bind_persona_endpoint, chat_completion, chat_search,
    continue_chat, create_persona_endpoint, create_scene_endpoint, create_session_endpoint,
    delete_character_endpoint, delete_message, delete_persona_multi_endpoint,
    delete_session_endpoint, edit_message, get_character_avatar, get_character_card,
    get_character_lorebook, get_character_state, get_character_state_history,
    get_character_state_schema, get_chat_history, get_drift, get_effective_persona_endpoint,
    get_persona_endpoint, get_persona_multi_endpoint, get_preset_endpoint, get_resident_memory,
    get_scene_endpoint, get_settings, get_user_model, import_character, import_preset_endpoint,
    list_agent_tools, list_characters, list_models, list_personas_endpoint, list_presets_endpoint,
    list_scenes_endpoint, list_sessions_endpoint, preview_chat_assembly,
    reextract_character_assets, regen_chat, rollback_chat, style_review, swipe_chat,
    switch_branch, unbind_persona_endpoint, update_character_card, update_character_lorebook,
    update_drift, update_persona_endpoint, update_persona_multi_endpoint, update_resident_memory,
    update_settings, update_user_model,
};

/// daemon 进程全局共享状态。通过 axum `State<Arc<DaemonState>>` 注入到所有 handler。
pub struct DaemonState {
    /// 用户数据根目录（默认 `./data/`，可由 `AIRP_DATA_DIR` 覆盖）。
    pub data_root: PathBuf,
    /// M0 F-01：共享 HTTP 客户端（内部 `Arc<ConnectionPool>`，clone 廉价）。
    pub http_client: reqwest::Client,
    /// M4.4：热重载窗口。`GET /v1/settings` 读、`POST /v1/settings` 写。
    pub config: std::sync::RwLock<MutableConfig>,
    /// 串行 settings 候选构造、持久化与 live config 提交，不阻塞其他 config readers。
    pub settings_update: SettingsUpdateCoordinator,
}

/// `/v1/settings` 的异步事务协调器。
pub struct SettingsUpdateCoordinator {
    pub(crate) transaction: tokio::sync::Mutex<()>,
    #[cfg(test)]
    persistence_override: std::sync::Mutex<Option<SettingsPersistenceOverride>>,
}

#[cfg(test)]
type SettingsPersistenceOverride =
    Arc<dyn Fn(&std::path::Path, &[u8]) -> Result<(), crate::error::AirpError> + Send + Sync>;

impl Default for SettingsUpdateCoordinator {
    fn default() -> Self {
        Self {
            transaction: tokio::sync::Mutex::new(()),
            #[cfg(test)]
            persistence_override: std::sync::Mutex::new(None),
        }
    }
}

#[cfg(test)]
impl SettingsUpdateCoordinator {
    pub(crate) fn set_persistence_override(&self, hook: Option<SettingsPersistenceOverride>) {
        *self
            .persistence_override
            .lock()
            .expect("settings persistence override lock poisoned") = hook;
    }

    pub(crate) fn run_persistence_override(
        &self,
        path: &std::path::Path,
        bytes: &[u8],
    ) -> Option<Result<(), crate::error::AirpError>> {
        let hook = self
            .persistence_override
            .lock()
            .expect("settings persistence override lock poisoned")
            .clone();
        hook.map(|hook| hook(path, bytes))
    }
}

/// M4.4：可在运行时热重载的配置子集。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutableConfig {
    pub provider: Provider,
    pub endpoint: String,
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default)]
    pub volume_config: VolumeConfig,
    /// DX-2：daemon 访问鉴权 key。None = 不启用鉴权。
    #[serde(default)]
    pub access_api_key: Option<String>,
    /// DX-6：后端引擎选择。
    #[serde(default)]
    pub engine: BackendEngine,
    /// DX-3：每日配额限制。
    #[serde(default)]
    pub quota: crate::quota::QuotaConfig,
    /// Process-level policy; immutable through /v1/settings.
    #[serde(default)]
    pub deployment_mode: DeploymentMode,
    /// Production public origin; immutable through /v1/settings.
    #[serde(default)]
    pub public_origin: Option<String>,
}

/// `GET /v1/settings` 返回值：api_key 脱敏为 `Some("***")` / `None`。
///
/// 审计 PR #100 (gemini/CodeRabbit)：暴露 `data_root` 让调用方能从引擎本身稳定得知
/// 产物落盘根，不再硬编 `target/...` 路径——治"复现命令路径基不统"的根因，提可观察性。
#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub provider: Provider,
    pub endpoint: String,
    pub api_key_set: bool,
    pub model: String,
    pub volume_config: VolumeConfig,
    pub engine: BackendEngine,
    pub quota: crate::quota::QuotaConfig,
    /// A5：daemon 自鉴权 access_api_key 是否已设置（脱敏 bool，不返回 key 本体）。
    /// 与 `api_key_set`（上游 provider key）区分：前者保护本 engine，后者用于调用上游 LLM。
    pub access_api_key_set: bool,
    /// 数据落盘根目录真值（`AIRP_DATA_DIR` 或默认 `./data/`，由 `resolve_data_root` 定）。
    /// 审计 PR #100：暴露让调用方稳定寻产物，逼外部硬编路径是可观察性缺口。
    pub data_root: PathBuf,
    pub deployment_mode: DeploymentMode,
    pub public_origin: Option<String>,
}

impl SettingsView {
    pub(crate) fn from_config(cfg: &MutableConfig, data_root: &std::path::Path) -> Self {
        Self {
            provider: cfg.provider.clone(),
            endpoint: cfg.endpoint.clone(),
            api_key_set: cfg.api_key.as_deref().is_some_and(|s| !s.is_empty()),
            model: cfg.model.clone(),
            volume_config: cfg.volume_config.clone(),
            engine: cfg.engine.clone(),
            quota: cfg.quota.clone(),
            access_api_key_set: cfg.access_api_key.as_deref().is_some_and(|s| !s.is_empty()),
            data_root: data_root.to_path_buf(),
            deployment_mode: cfg.deployment_mode,
            public_origin: cfg.public_origin.clone(),
        }
    }
}

/// A2-5: constant-time byte comparison for the access key.
///
/// Plain `==` on `&str` short-circuits at the first differing byte, leaking
/// a timing oracle that lets an attacker recover the key byte-by-byte. This
/// compares all bytes with an XOR accumulator so the time depends only on
/// length (key length is not a meaningful secret here).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// DX-2: 可选 API key 鉴权中间件。
pub async fn auth_middleware(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let required_key = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        cfg.access_api_key.clone()
    };
    if let Some(key) = required_key {
        if !key.is_empty() {
            let provided = request
                .headers()
                .get(header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.strip_prefix("Bearer "));
            let ok = provided
                .map(|k| constant_time_eq(k.as_bytes(), key.as_bytes()))
                .unwrap_or(false);
            if !ok {
                return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
            }
        }
    }
    next.run(request).await
}

/// Production responses are never stored by browser/proxy caches. SSE retains
/// `no-cache` so intermediaries do not buffer or reuse a stream; all other
/// engine responses use `no-store`.
async fn production_cache_policy(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let production = {
        let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
        cfg.deployment_mode == DeploymentMode::Production
    };
    let mut response = next.run(request).await;
    if production {
        let is_event_stream = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/event-stream"));
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(if is_event_stream {
                "no-cache"
            } else {
                "no-store"
            }),
        );
    }
    response
}

/// DX-4: Rate-limit key extractor — Bearer token for authenticated requests, peer IP otherwise.
#[derive(Debug, Clone, Copy)]
struct UserOrIpKeyExtractor;

impl tower_governor::key_extractor::KeyExtractor for UserOrIpKeyExtractor {
    type Key = String;

    fn extract<T>(&self, req: &Request<T>) -> Result<Self::Key, tower_governor::GovernorError> {
        if let Some(auth) = req.headers().get(header::AUTHORIZATION) {
            if let Ok(s) = auth.to_str() {
                if let Some(token) = s.strip_prefix("Bearer ") {
                    // A4: 用 chars().take(32) 而非 &token[..32] —— 后者是字节切片，
                    // 32 不在 char boundary 上时会 panic（热路径 5xx）。take(32) 永远安全。
                    let key: String = token.chars().take(32).collect();
                    return Ok(format!("k:{}", key));
                }
            }
        }
        // A2-7: fall back to a fixed key instead of erroring when ConnectInfo
        // is absent. Production always injects it (serve_with_connect_info);
        // the fallback only matters for in-process tests (oneshot) and shared
        // a single bucket there — never returns UnableToExtractKey, which would
        // turn every request into an error once the governor covers all routes.
        let key = req
            .extensions()
            .get::<ConnectInfo<std::net::SocketAddr>>()
            .map(|ci| format!("ip:{}", ci.0.ip()))
            .unwrap_or_else(|| "ip:unknown".to_string());
        Ok(key)
    }
}

const RATE_LIMIT_PERIOD: Duration = Duration::from_millis(100);
const RATE_LIMIT_BURST: u32 = 20;

/// 构造 axum Router：注册所有 `/v1/*` 端点、CORS 中间件、限流中间件。
pub fn create_router(state: Arc<DaemonState>) -> Router {
    let cors = cors_layer(&state);

    // A2-7: rate limiting previously protected only /v1/chat/completions,
    // leaving import / sync / scene / mcp endpoints unthrottled. Build ONE
    // shared config (per-IP token bucket) and apply it as a router-wide
    // `.layer()` over both v1 and mcp routes so every request path shares
    // the same budget. 10 req/s sustained, burst 20 per IP.
    let governor_conf = Arc::new({
        let mut b = GovernorConfigBuilder::default();
        // tower_governor's `per_second(n)` means one token every n seconds,
        // not n tokens per second. A 100ms period is the intended 10 req/s.
        b.period(RATE_LIMIT_PERIOD).burst_size(RATE_LIMIT_BURST);
        b.key_extractor(UserOrIpKeyExtractor)
            .finish()
            .expect("GovernorConfigBuilder 配置有效")
    });

    let v1_routes = Router::new()
        .route("/v1/chat/completions", post(chat_completion))
        .route("/v1/chat/preview", post(preview_chat_assembly))
        .route("/v1/agent/run", post(agent_run))
        .route("/v1/agent/tools", get(list_agent_tools))
        .route("/v1/chat/history", post(get_chat_history))
        .route("/v1/chat/rollback", post(rollback_chat))
        .route("/v1/chat/regen", post(regen_chat))
        .route("/v1/chat/continue", post(continue_chat))
        .route("/v1/chat/delete", post(delete_message))
        .route(
            "/v1/chat/message",
            put(edit_message.layer(DefaultBodyLimit::max(2 * 1024 * 1024))),
        )
        .route("/v1/chat/swipe", post(swipe_chat))
        .route("/v1/chat/branch/switch", post(switch_branch))
        .route("/v1/characters", get(list_characters))
        .route(
            "/v1/characters/import",
            post(import_character).layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
        )
        .route(
            "/v1/characters/:character_id",
            get(get_character_card)
                .put(update_character_card.layer(DefaultBodyLimit::max(2 * 1024 * 1024)))
                .delete(delete_character_endpoint),
        )
        .route(
            "/v1/characters/:character_id/reextract",
            post(reextract_character_assets),
        )
        .route(
            "/v1/characters/:character_id/avatar",
            get(get_character_avatar),
        )
        .route(
            "/v1/characters/:character_id/lorebook",
            get(get_character_lorebook)
                .put(update_character_lorebook.layer(DefaultBodyLimit::max(2 * 1024 * 1024))),
        )
        .route(
            "/v1/characters/:character_id/state",
            get(get_character_state),
        )
        .route(
            "/v1/characters/:character_id/state/history",
            get(get_character_state_history),
        )
        .route(
            "/v1/characters/:character_id/state/schema",
            get(get_character_state_schema),
        )
        .route(
            "/v1/scenes",
            get(list_scenes_endpoint).post(create_scene_endpoint),
        )
        .route("/v1/scenes/:scene_id", get(get_scene_endpoint))
        .route(
            "/v1/scenes/:scene_id/characters",
            post(add_scene_character_endpoint),
        )
        .route("/v1/models", get(list_models))
        .route("/v1/presets", get(list_presets_endpoint))
        .route("/v1/presets/:preset_id", get(get_preset_endpoint))
        .route(
            "/v1/presets/import",
            post(import_preset_endpoint).layer(DefaultBodyLimit::max(2 * 1024 * 1024)),
        )
        .route(
            "/v1/users/:user_id/persona",
            get(get_persona_endpoint).put(update_persona_endpoint),
        )
        .route(
            "/v1/users/:user_id/persona/effective",
            get(get_effective_persona_endpoint),
        )
        .route(
            "/v1/users/:user_id/personas",
            get(list_personas_endpoint).post(create_persona_endpoint),
        )
        .route(
            "/v1/users/:user_id/personas/:persona_id",
            get(get_persona_multi_endpoint)
                .put(update_persona_multi_endpoint)
                .delete(delete_persona_multi_endpoint),
        )
        .route(
            "/v1/users/:user_id/personas/:persona_id/bindings",
            post(bind_persona_endpoint).delete(unbind_persona_endpoint),
        )
        .route(
            "/v1/sessions/:character_id",
            get(list_sessions_endpoint).post(create_session_endpoint),
        )
        .route(
            "/v1/sessions/:character_id/:session_id",
            axum::routing::delete(delete_session_endpoint),
        )
        .route("/v1/settings", get(get_settings).post(update_settings))
        // ── Style API（4.1/4.2 风格系统） ──────────────────────────────────
        .route("/v1/style/review", post(style_review))
        .route(
            "/v1/characters/:character_id/drift",
            get(get_drift).put(update_drift),
        )
        // ── Search API（4.3 FTS5 历史检索） ─────────────────────────────────
        .route("/v1/chat/search", post(chat_search))
        // ── Memory API（2.4 记忆可见性） ────────────────────────────────────
        // 审计 B2 修复：PUT 路由配置 2MB body limit，与项目硬约束
        // "PUT endpoints must have body limit configured (2MB) to prevent DoS attacks" 对齐。
        .route(
            "/v1/memory/resident",
            get(get_resident_memory)
                .put(update_resident_memory.layer(DefaultBodyLimit::max(2 * 1024 * 1024))),
        )
        .route(
            "/v1/memory/user-model",
            get(get_user_model)
                .put(update_user_model.layer(DefaultBodyLimit::max(2 * 1024 * 1024))),
        )
        // ── Decompose Agent Flow（Task 7） ──────────────────────────────────
        .route(
            "/v1/characters/:character_id/decompose",
            post(decompose_character).layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/v1/presets/:preset_id/decompose",
            post(decompose_preset).layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/v1/characters/:character_id/analysis",
            get(list_character_analysis),
        )
        .route(
            "/v1/characters/:character_id/analysis/*filename",
            get(get_character_analysis_file)
                .post(enhance_or_apply_character_analysis)
                .layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        // A2-7: governor over all /v1/* (not just chat).
        .layer(GovernorLayer {
            config: governor_conf.clone(),
        });

    Router::new()
        .route("/version", get(version_handler))
        .route("/health", get(health_handler))
        .merge(v1_routes)
        .layer(middleware::from_fn_with_state(
            state.clone(),
            production_cache_policy,
        ))
        .layer(cors)
        .with_state(state)
}

/// Add the browser UI to the daemon for the loopback-only Windows package.
///
/// This remains opt-in so API-only and Tauri callers retain their existing
/// router, while the local WebUI package gets a single same-origin process.
pub fn create_local_webui_router(state: Arc<DaemonState>, webui_dir: PathBuf) -> Router {
    create_router(state)
        .route(
            "/runtime-config.js",
            get(|| async {
                (
                    [(
                        header::CONTENT_TYPE,
                        "application/javascript; charset=utf-8",
                    )],
                    "window.AIRP_WEBUI_CONFIG = Object.freeze({ mode: 'local' });\n",
                )
            }),
        )
        .fallback_service(ServeDir::new(webui_dir))
        .layer(middleware::from_fn(local_webui_security_headers))
}

async fn local_webui_security_headers(request: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    let is_event_stream = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if is_event_stream {
            "no-cache"
        } else {
            "no-store"
        }),
    );
    response
}

fn cors_layer(state: &DaemonState) -> CorsLayer {
    let configured = std::env::var("AIRP_CORS_ORIGINS").ok();
    let cfg = state.config.read().unwrap_or_else(|e| e.into_inner());
    let origins = allowed_cors_origins(
        cfg.deployment_mode,
        cfg.public_origin.as_deref(),
        configured.as_deref(),
    );

    CorsLayer::new()
        .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
        .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE])
        .allow_origin(AllowOrigin::list(origins))
}

fn allowed_cors_origins(
    deployment_mode: DeploymentMode,
    public_origin: Option<&str>,
    configured: Option<&str>,
) -> Vec<HeaderValue> {
    if deployment_mode == DeploymentMode::Production {
        return match public_origin.map(|origin| origin.parse::<HeaderValue>()) {
            Some(Ok(value)) => vec![value],
            Some(Err(error)) => {
                tracing::warn!(%error, "invalid AIRP_PUBLIC_ORIGIN; CORS will reject all origins");
                Vec::new()
            }
            None => {
                tracing::warn!(
                    "AIRP_PUBLIC_ORIGIN is unset in production; CORS will reject all origins"
                );
                Vec::new()
            }
        };
    }

    const DEFAULT_ORIGINS: &[&str] = &[
        "http://127.0.0.1:9001",
        "http://localhost:9001",
        "tauri://localhost",
        "http://tauri.localhost",
        "https://tauri.localhost",
    ];

    let mut origins: Vec<HeaderValue> = DEFAULT_ORIGINS
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    let additions: Vec<HeaderValue> = configured
        .into_iter()
        .flat_map(|value| value.split(',').map(str::trim))
        .filter(|origin| !origin.is_empty())
        .filter_map(|origin| match origin.parse() {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::warn!(origin, %error, "ignoring invalid AIRP_CORS_ORIGINS entry");
                None
            }
        })
        .collect();
    if configured.is_some() && additions.is_empty() {
        tracing::warn!(
            "AIRP_CORS_ORIGINS resolved to zero valid origins; retaining built-in defaults"
        );
    }
    for origin in additions {
        if !origins.contains(&origin) {
            origins.push(origin);
        }
    }

    origins
}

/// AUDIT-10: Diagnostic version endpoint for harness / monitoring tools.
///
/// Returns crate name and version. Unauthenticated by design — safe to expose
/// since contents are static build metadata.
async fn version_handler() -> axum::Json<VersionInfo> {
    axum::Json(VersionInfo {
        name: env!("CARGO_PKG_NAME"),
        version: env!("CARGO_PKG_VERSION"),
    })
}

#[derive(serde::Serialize)]
struct VersionInfo {
    name: &'static str,
    version: &'static str,
}

/// WEBUI-BACKEND-PLAN §4.2：健康就绪探针。
///
/// 返回 engine 状态、provider 是否已配置、data_root 是否可写。
/// 不鉴权（与 `/version` 同级），因为只暴露就绪状态，不泄露敏感信息。
async fn health_handler(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> axum::Json<HealthInfo> {
    let cfg = state.config.read().unwrap();
    let provider_configured =
        cfg.api_key.as_deref().is_some_and(|s| !s.is_empty()) && !cfg.endpoint.is_empty();
    drop(cfg);

    // data_root 可写检查：尝试写一个临时文件
    let data_root_writable = std::fs::File::create(state.data_root.join(".health_probe")).is_ok();
    if data_root_writable {
        let _ = std::fs::remove_file(state.data_root.join(".health_probe"));
    }

    axum::Json(HealthInfo {
        engine: "ok",
        provider_configured,
        data_root_writable,
    })
}

#[derive(serde::Serialize)]
struct HealthInfo {
    engine: &'static str,
    provider_configured: bool,
    data_root_writable: bool,
}

#[cfg(test)]
mod tests;
