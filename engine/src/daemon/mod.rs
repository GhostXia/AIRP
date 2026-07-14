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
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::cors::{AllowOrigin, CorsLayer};

use decompose_handlers::{
    decompose_character, decompose_preset, enhance_or_apply_character_analysis,
    get_character_analysis_file, list_character_analysis,
};
use handlers::{
    add_scene_character_endpoint, agent_run, bind_persona_endpoint, chat_completion,
    create_persona_endpoint, create_scene_endpoint, create_session_endpoint,
    delete_character_endpoint, delete_persona_multi_endpoint, delete_session_endpoint,
    get_character_avatar, get_character_card, get_character_lorebook, get_character_state,
    get_character_state_history, get_character_state_schema, get_chat_history,
    get_persona_endpoint, get_persona_multi_endpoint, get_preset_endpoint, get_scene_endpoint,
    get_settings, import_character, import_preset_endpoint, list_agent_tools, list_characters,
    list_models, list_personas_endpoint, list_presets_endpoint, list_scenes_endpoint,
    list_sessions_endpoint, reextract_character_assets, regen_chat, rollback_chat,
    unbind_persona_endpoint, update_character_card, update_character_lorebook,
    update_persona_endpoint, update_persona_multi_endpoint, update_settings,
};

/// daemon 进程全局共享状态。通过 axum `State<Arc<DaemonState>>` 注入到所有 handler。
pub struct DaemonState {
    /// 用户数据根目录（默认 `./data/`，可由 `AIRP_DATA_DIR` 覆盖）。
    pub data_root: PathBuf,
    /// M0 F-01：共享 HTTP 客户端（内部 Arc<ConnectionPool>，clone 廉价）。
    pub http_client: reqwest::Client,
    /// M4.4：热重载窗口。`GET /v1/settings` 读、`POST /v1/settings` 写。
    pub config: std::sync::RwLock<MutableConfig>,
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
        .route("/v1/agent/run", post(agent_run))
        .route("/v1/agent/tools", get(list_agent_tools))
        .route("/v1/chat/history", post(get_chat_history))
        .route("/v1/chat/rollback", post(rollback_chat))
        .route("/v1/chat/regen", post(regen_chat))
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::util::ServiceExt;

    fn make_state_with_key(key: Option<&str>) -> Arc<DaemonState> {
        let tmp = tempfile::tempdir().unwrap();
        Arc::new(DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "http://localhost".to_string(),
                api_key: None,
                model: "gpt-4o".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: key.map(|s| s.to_string()),
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        })
    }

    fn make_router_for_test(state: Arc<DaemonState>) -> Router {
        let v1_ping = Router::new()
            .route("/v1/ping", get(|| async { "ok" }))
            .route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth_middleware,
            ));
        Router::new().merge(v1_ping).with_state(state)
    }

    #[tokio::test]
    async fn legacy_persona_update_preserves_schema_v2_bindings() {
        let state = make_state_with_key(None);
        let uid = crate::types::UserId::new("alice").unwrap();
        let service = crate::domain::PersonaService::new(&state.data_root);
        let saved = service
            .save_default(
                &uid,
                0,
                crate::domain::Persona {
                    schema: crate::domain::Persona::SCHEMA,
                    revision: 0,
                    updated_at: String::new(),
                    name: "Old".to_string(),
                    description: String::new(),
                    variables: std::collections::HashMap::new(),
                    id: "default".to_string(),
                    bindings: vec![crate::domain::PersonaBinding {
                        character_id: "char-a".to_string(),
                        session_id: None,
                    }],
                },
            )
            .unwrap();

        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/users/alice/persona")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "expected_revision": saved.revision,
                            "name": "New",
                            "description": "updated",
                            "variables": {}
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let updated: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(updated.name, "New");
        assert_eq!(updated.bindings.len(), 1);
        assert_eq!(updated.bindings[0].character_id, "char-a");
    }

    #[tokio::test]
    async fn list_personas_returns_default_only_for_fresh_user() {
        let state = make_state_with_key(None);
        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/users/bob/personas")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let ids: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert_eq!(ids, vec!["default".to_string()]);
    }

    #[tokio::test]
    async fn create_persona_then_get_returns_it() {
        let state = make_state_with_key(None);
        let create_body = serde_json::json!({
            "persona_id": "alice-alt",
            "name": "Alice Alt",
            "description": "alt persona",
            "variables": {"mood": "happy"}
        })
        .to_string();
        let response = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/alice/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let created: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(created.id, "alice-alt");
        assert_eq!(created.name, "Alice Alt");
        assert_eq!(created.revision, 1);

        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/users/alice/personas/alice-alt")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let fetched: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(fetched.name, "Alice Alt");
        assert_eq!(fetched.variables.get("mood").unwrap(), "happy");
    }

    #[tokio::test]
    async fn create_persona_rejects_default_id() {
        let state = make_state_with_key(None);
        let body =
            serde_json::json!({"persona_id":"default","name":"D","description":"","variables":{}})
                .to_string();
        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_persona_rejects_duplicate() {
        let state = make_state_with_key(None);
        let body =
            serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}})
                .to_string();
        let first = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);
        let second = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn create_persona_rejects_path_traversal() {
        let state = make_state_with_key(None);
        let body =
            serde_json::json!({"persona_id":"../etc","name":"X","description":"","variables":{}})
                .to_string();
        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_persona_bumps_revision_and_preserves_bindings() {
        let state = make_state_with_key(None);
        let create_body =
            serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}})
                .to_string();
        let _ = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let bind_body = serde_json::json!({"character_id":"char-a"}).to_string();
        let response = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas/p1/bindings")
                    .header("content-type", "application/json")
                    .body(Body::from(bind_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let after_bind: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(after_bind.bindings.len(), 1);
        let rev = after_bind.revision;

        let update_body = serde_json::json!({"expected_revision":rev,"name":"P1-renamed","description":"d","variables":{}}).to_string();
        let response = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/users/u/personas/p1")
                    .header("content-type", "application/json")
                    .body(Body::from(update_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let updated: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(updated.name, "P1-renamed");
        assert_eq!(updated.revision, rev + 1);
        assert_eq!(updated.bindings.len(), 1);
        assert_eq!(updated.bindings[0].character_id, "char-a");
    }

    #[tokio::test]
    async fn update_persona_rejects_wrong_expected_revision() {
        let state = make_state_with_key(None);
        let create_body =
            serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}})
                .to_string();
        let _ = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let update_body =
            serde_json::json!({"expected_revision":99,"name":"X","description":"","variables":{}})
                .to_string();
        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/users/u/personas/p1")
                    .header("content-type", "application/json")
                    .body(Body::from(update_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn update_nonexistent_non_default_returns_404() {
        let state = make_state_with_key(None);
        let body =
            serde_json::json!({"expected_revision":0,"name":"X","description":"","variables":{}})
                .to_string();
        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/users/u/personas/ghost")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn delete_persona_removes_it_and_default_rejected() {
        let state = make_state_with_key(None);
        let create_body =
            serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}})
                .to_string();
        let _ = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let response = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/v1/users/u/personas/p1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/users/u/personas/p1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/v1/users/u/personas/default")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn bind_persona_is_idempotent_and_unbind_removes_it() {
        let state = make_state_with_key(None);
        let create_body =
            serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}})
                .to_string();
        let _ = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let bind_body = serde_json::json!({"character_id":"char-a"}).to_string();
        let r1 = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas/p1/bindings")
                    .header("content-type", "application/json")
                    .body(Body::from(bind_body.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r1.status(), StatusCode::OK);
        let body = axum::body::to_bytes(r1.into_body(), 4096).await.unwrap();
        let after_first: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(after_first.bindings.len(), 1);
        let rev_after_first = after_first.revision;

        // 幂等：第二次 bind 同一目标不 bump revision。
        let r2 = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas/p1/bindings")
                    .header("content-type", "application/json")
                    .body(Body::from(bind_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(r2.status(), StatusCode::OK);
        let body = axum::body::to_bytes(r2.into_body(), 4096).await.unwrap();
        let after_second: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(after_second.bindings.len(), 1);
        assert_eq!(after_second.revision, rev_after_first);

        let response = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/v1/users/u/personas/p1/bindings?character_id=char-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let after_unbind: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
        assert_eq!(after_unbind.bindings.len(), 0);

        // 幂等：再 unbind 同一目标不报错、不 bump revision。
        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("DELETE")
                    .uri("/v1/users/u/personas/p1/bindings?character_id=char-a")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn bind_rejects_invalid_character_id() {
        let state = make_state_with_key(None);
        let create_body =
            serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}})
                .to_string();
        let _ = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas")
                    .header("content-type", "application/json")
                    .body(Body::from(create_body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let bind_body = serde_json::json!({"character_id":"bad/path"}).to_string();
        let response = create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/users/u/personas/p1/bindings")
                    .header("content-type", "application/json")
                    .body(Body::from(bind_body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cors_allows_bundled_webui_origin() {
        let app = create_router(make_state_with_key(None));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .header(header::ORIGIN, "http://127.0.0.1:9001")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("http://127.0.0.1:9001"))
        );
    }

    #[tokio::test]
    async fn production_cache_policy_keeps_streams_unbuffered_and_other_responses_unstored() {
        let state = make_state_with_key(None);
        {
            let mut cfg = state.config.write().unwrap();
            cfg.deployment_mode = DeploymentMode::Production;
            cfg.public_origin = Some("https://airp.example.com".to_string());
        }
        let app = Router::new()
            .route(
                "/json",
                get(|| async { axum::Json(serde_json::json!({"ok": true})) }),
            )
            .route(
                "/stream",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        "data: ok\n\n",
                    )
                }),
            )
            .layer(middleware::from_fn_with_state(
                state.clone(),
                production_cache_policy,
            ))
            .with_state(state);

        let json = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            json.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-store"))
        );

        let stream = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/stream")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            stream.headers().get(header::CACHE_CONTROL),
            Some(&HeaderValue::from_static("no-cache"))
        );
    }

    #[tokio::test]
    async fn cors_rejects_unlisted_origin() {
        let app = create_router(make_state_with_key(None));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .header(header::ORIGIN, "https://attacker.example")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none());
    }

    #[tokio::test]
    async fn agent_tool_catalog_exposes_sorted_builtin_metadata() {
        let app = create_router(make_state_with_key(None));
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/agent/tools")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let tools: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let names: Vec<_> = tools
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names.len(), 19);
        assert!(names.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(names.contains(&"export_context_bundle"));
        assert!(names.contains(&"seal_volume"));
    }

    #[test]
    fn configured_cors_origins_extend_built_in_defaults() {
        let origins = super::allowed_cors_origins(
            crate::config::DeploymentMode::Development,
            None,
            Some("https://example.test, https://example.test, invalid origin"),
        );
        assert!(origins.contains(&HeaderValue::from_static("http://127.0.0.1:9001")));
        assert!(origins.contains(&HeaderValue::from_static("https://example.test")));
        assert_eq!(
            origins
                .iter()
                .filter(|origin| *origin == "https://example.test")
                .count(),
            1
        );
    }

    #[test]
    fn production_cors_uses_only_the_public_origin() {
        let origins = super::allowed_cors_origins(
            crate::config::DeploymentMode::Production,
            Some("https://airp.example.com"),
            Some("https://operator-added.example,http://127.0.0.1:9001"),
        );
        assert_eq!(
            origins,
            vec![HeaderValue::from_static("https://airp.example.com")]
        );
    }

    #[test]
    fn test_a2_5_constant_time_eq() {
        assert!(super::constant_time_eq(b"secret", b"secret"));
        assert!(!super::constant_time_eq(b"secret", b"secreT"));
        assert!(!super::constant_time_eq(b"secret", b"secre")); // length differs
        assert!(super::constant_time_eq(b"", b""));
    }

    #[tokio::test]
    async fn test_dx2_no_key_all_pass() {
        let state = make_state_with_key(None);
        let app = make_router_for_test(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dx2_correct_key_passes() {
        let state = make_state_with_key(Some("secret-key"));
        let app = make_router_for_test(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/ping")
                    .header("Authorization", "Bearer secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_dx2_wrong_key_returns_401() {
        let state = make_state_with_key(Some("secret-key"));
        let app = make_router_for_test(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/ping")
                    .header("Authorization", "Bearer wrong-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_dx2_missing_header_returns_401() {
        let state = make_state_with_key(Some("secret-key"));
        let app = make_router_for_test(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/ping")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_dx2_bearer_prefix_required() {
        let state = make_state_with_key(Some("secret-key"));
        let app = make_router_for_test(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/ping")
                    .header("Authorization", "secret-key")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // M_LS-3 tests

    fn make_state_no_key() -> (Arc<DaemonState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "http://localhost".to_string(),
                api_key: None,
                model: "gpt-4o".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: None,
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        });
        (state, tmp)
    }

    #[tokio::test]
    async fn test_mls3_state_404_when_no_live_json() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/alice/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_mls3_state_returns_live_json() {
        let (state, _tmp) = make_state_no_key();
        let state_dir = state
            .data_root
            .join("characters")
            .join("alice")
            .join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::write(
            state_dir.join("live.json"),
            r#"{"hp":100,"location":"base"}"#,
        )
        .unwrap();

        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/alice/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["hp"], 100);
        assert_eq!(v["location"], "base");
    }

    #[tokio::test]
    async fn test_mls3_state_bad_char_id_returns_400() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/../evil/state")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(resp.status() == StatusCode::BAD_REQUEST || resp.status() == StatusCode::NOT_FOUND);
    }

    // M_MS MS-3 tests

    #[tokio::test]
    async fn test_ms3_list_scenes_empty() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/scenes")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: Vec<String> = serde_json::from_slice(&body).unwrap();
        assert!(v.is_empty());
    }

    #[tokio::test]
    async fn test_ms3_create_and_get_scene() {
        let (state, _tmp) = make_state_no_key();
        let scene = crate::scene::SceneConfig {
            scene_id: crate::types::SceneId::new("tavern").unwrap(),
            description: "Tea house".to_string(),
            characters: vec![],
            narrator_style: String::new(),
            lorebook_merge: crate::scene::LorebookMerge::Union,
            format_hint: String::new(),
        };

        let app = create_router(state.clone());
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/scenes")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&scene).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/scenes/tavern")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["scene_id"], "tavern");
    }

    #[tokio::test]
    async fn test_ms3_get_scene_404() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/scenes/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_ms3_add_character_to_scene() {
        let (state, _tmp) = make_state_no_key();
        let scene = crate::scene::SceneConfig {
            scene_id: crate::types::SceneId::new("forest").unwrap(),
            description: "Forest".to_string(),
            characters: vec![],
            narrator_style: String::new(),
            lorebook_merge: crate::scene::LorebookMerge::Union,
            format_hint: String::new(),
        };
        scene.save(&state.data_root).unwrap();

        let app = create_router(state);
        let body = serde_json::json!({"character_id": "ranger", "intro": "Forest ranger"});
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/scenes/forest/characters")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let resp_body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(v["character_count"], 1);
    }

    // M_LS LS-7: schema endpoint

    #[tokio::test]
    async fn test_ls7_schema_404_when_no_file() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/alice/state/schema")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_ls7_schema_returns_json() {
        let (state, _tmp) = make_state_no_key();
        let state_dir = crate::data_dir::char_state_dir(&state.data_root, "alice");
        std::fs::create_dir_all(&state_dir).unwrap();
        let schema = serde_json::json!({
            "fields": [
                {"key": "hp", "type": "number", "min": 0, "max": 100, "label": "生命值"}
            ]
        });
        std::fs::write(
            state_dir.join("schema.json"),
            serde_json::to_string(&schema).unwrap(),
        )
        .unwrap();

        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/alice/state/schema")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["fields"][0]["key"], "hp");
        assert_eq!(v["fields"][0]["max"], 100);
    }

    // M_LS LS-5 tests

    #[tokio::test]
    async fn test_ls5_history_404_when_no_file() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/alice/state/history")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn test_ls5_history_returns_reversed_entries() {
        let (state, _tmp) = make_state_no_key();
        let state_dir = crate::data_dir::char_state_dir(&state.data_root, "alice");
        std::fs::create_dir_all(&state_dir).unwrap();
        let history_path = crate::data_dir::char_state_history_path(&state.data_root, "alice");
        let mut content = String::new();
        for i in 1u32..=3 {
            let line = serde_json::json!({
                "timestamp": format!("2026-01-0{}T00:00:00Z", i),
                "state": { "turn": i }
            });
            content.push_str(&serde_json::to_string(&line).unwrap());
            content.push('\n');
        }
        std::fs::write(&history_path, content).unwrap();

        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/alice/state/history")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["state"]["turn"], 3);
        assert_eq!(entries[2]["state"]["turn"], 1);
    }

    #[tokio::test]
    async fn test_ls5_history_limit_param() {
        let (state, _tmp) = make_state_no_key();
        let state_dir = crate::data_dir::char_state_dir(&state.data_root, "bob");
        std::fs::create_dir_all(&state_dir).unwrap();
        let history_path = crate::data_dir::char_state_history_path(&state.data_root, "bob");
        let mut content = String::new();
        for i in 1u32..=10 {
            let line = serde_json::json!({"timestamp": "2026-01-01T00:00:00Z", "state": {"n": i}});
            content.push_str(&serde_json::to_string(&line).unwrap());
            content.push('\n');
        }
        std::fs::write(&history_path, content).unwrap();

        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/characters/bob/state/history?limit=3")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let entries: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["state"]["n"], 10);
    }

    // AUDIT-10: /version diagnostic endpoint
    #[tokio::test]
    async fn test_audit_10_version_endpoint_returns_metadata() {
        let state = make_state_with_key(None);
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["name"], "airp-core");
        assert!(!v["version"].as_str().unwrap().is_empty());
    }

    // AUDIT-10: /version requires no auth even when access_api_key is set
    #[tokio::test]
    async fn test_audit_10_version_unauthenticated_with_key_set() {
        let state = make_state_with_key(Some("secret-key"));
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/version")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // WEBUI-BACKEND-PLAN §4.2: /health 就绪探针
    #[tokio::test]
    async fn test_health_endpoint_returns_status() {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "http://localhost".to_string(),
                api_key: None,
                model: "gpt-4o".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: None,
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        });
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["engine"], "ok");
        assert_eq!(v["provider_configured"].as_bool(), Some(false)); // api_key=None
        assert_eq!(v["data_root_writable"].as_bool(), Some(true)); // tempdir 可写
    }

    // /health 不鉴权（与 /version 同级）
    #[tokio::test]
    async fn test_health_unauthenticated_with_key_set() {
        let state = make_state_with_key(Some("secret-key"));
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // /health provider_configured=true when api_key + endpoint 都有值
    #[tokio::test]
    async fn test_health_provider_configured_when_api_key_and_endpoint_set() {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "https://api.openai.com".to_string(),
                api_key: Some("sk-test".to_string()),
                model: "gpt-4o".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: None,
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        });
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["engine"], "ok");
        assert_eq!(v["provider_configured"], true);
        assert_eq!(v["data_root_writable"], true);
    }

    // ── A5: SettingsView.access_api_key_set ────────────────────────────────

    #[tokio::test]
    async fn test_a5_settings_exposes_access_api_key_set_false_when_none() {
        // 无 access_api_key 时，SettingsView.access_api_key_set 必须为 false
        let state = make_state_with_key(None);
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["access_api_key_set"].as_bool(), Some(false));
    }

    #[tokio::test]
    async fn test_a5_settings_exposes_access_api_key_set_true_when_set() {
        // 有 access_api_key 时，SettingsView.access_api_key_set 必须为 true
        let state = make_state_with_key(Some("secret-key"));
        let app = create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .header("Authorization", "Bearer secret-key")
                    .uri("/v1/settings")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(v["access_api_key_set"].as_bool(), Some(true));
    }

    #[tokio::test]
    async fn settings_update_keeps_secrets_runtime_only() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state.clone());
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/settings")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "api_key": "sk-runtime",
                            "access_api_key": "daemon-runtime",
                            "model": "runtime-model"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let runtime = state.config.read().unwrap();
        assert_eq!(runtime.api_key.as_deref(), Some("sk-runtime"));
        assert_eq!(runtime.access_api_key.as_deref(), Some("daemon-runtime"));
        drop(runtime);
        let persisted: serde_json::Value =
            serde_json::from_slice(&std::fs::read(state.data_root.join("settings.json")).unwrap())
                .unwrap();
        assert!(persisted.get("api_key").is_none());
        assert!(persisted.get("access_api_key").is_none());
        assert_eq!(persisted["model"], "runtime-model");
    }

    #[tokio::test]
    async fn production_settings_rejects_access_key_replacement_without_partial_update() {
        let state = make_state_with_key(Some("old-production-key"));
        {
            let mut cfg = state.config.write().unwrap();
            cfg.deployment_mode = crate::config::DeploymentMode::Production;
            cfg.public_origin = Some("https://airp.example.com".to_string());
        }
        let response = create_router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/settings")
                    .header(header::AUTHORIZATION, "Bearer old-production-key")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "access_api_key": "replacement-key",
                            "model": "must-not-change"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let runtime = state.config.read().unwrap();
        assert_eq!(
            runtime.access_api_key.as_deref(),
            Some("old-production-key")
        );
        assert_ne!(runtime.model, "must-not-change");
        assert!(!state.data_root.join("settings.json").exists());
    }

    // ── A6: chat/history 支持 session_id 字段 ──────────────────────────────

    #[tokio::test]
    async fn test_a6_chat_history_accepts_session_id_field() {
        // A6 修复前：deny_unknown_fields 拒绝 session_id → 422
        // A6 修复后：session_id 被接受，返回 200 + ChatLog
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let body = serde_json::json!({"character_id": "alice", "session_id": "550e8400-e29b-41d4-a716-446655440000"});
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // 200 即证明 session_id 被接受（修复前是 422 unknown field）
    }

    #[tokio::test]
    async fn test_a6_chat_history_session_scoped_vs_legacy_diverge() {
        // A6 核心验证：同一 character_id 下，
        //   1. 不传 session_id → legacy per-character log
        //   2. 传 session_id → session-scoped log
        // 两个 log 写到不同路径（legacy 在 characters/{id}/history/，
        // scoped 在 characters/{id}/sessions/{sid}/history/）
        let (state, tmp) = make_state_no_key();
        let app = create_router(state.clone());

        // (1) 不传 session_id → legacy log
        let body1 = serde_json::json!({"character_id": "alice"});
        let resp1 = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body1).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        // (2) 传 session_id → session-scoped log
        let scoped_sid = "550e8400-e29b-41d4-a716-446655440000";
        let body2 = serde_json::json!({"character_id": "alice", "session_id": scoped_sid});
        let resp2 = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body2).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);

        // 核心断言：两个 log 写到不同路径
        // legacy: characters/alice/history/chat_log.jsonl
        // scoped: characters/alice/sessions/{sid}/history/chat_log.jsonl
        let legacy_jsonl = tmp
            .path()
            .join("characters")
            .join("alice")
            .join("history")
            .join("chat_log.jsonl");
        let scoped_jsonl = tmp
            .path()
            .join("characters")
            .join("alice")
            .join("sessions")
            .join(scoped_sid)
            .join("history")
            .join("chat_log.jsonl");
        assert!(
            legacy_jsonl.exists(),
            "legacy log 必须存在: {:?}",
            legacy_jsonl
        );
        assert!(
            scoped_jsonl.exists(),
            "session-scoped log 必须存在: {:?}",
            scoped_jsonl
        );
        assert_ne!(legacy_jsonl, scoped_jsonl, "A6 核心断言：两个路径必须不同");
    }

    #[tokio::test]
    async fn test_a6_chat_rollback_accepts_session_id() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        // 直接 rollback 一个空 session（messages 为空，rollback_to(0) 应安全）
        let body = serde_json::json!({
            "character_id": "alice",
            "message_index": 0,
            "session_id": "550e8400-e29b-41d4-a716-446655440000"
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/rollback")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        // 期望 200（A6 修复前会 422 unknown field session_id）
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_a6_chat_regen_accepts_session_id() {
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let body = serde_json::json!({
            "character_id": "alice",
            "session_id": "550e8400-e29b-41d4-a716-446655440000"
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/regen")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        // 期望 200（A6 修复前会 422 unknown field session_id）
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_a6_chat_history_without_session_id_still_works() {
        // 回退兼容：不传 session_id 时仍然走 legacy 路径，不能因 A6 改动破坏旧客户端
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let body = serde_json::json!({"character_id": "alice"});
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ── O1 (#86): ChatLog.scope_session_id 在 HTTP 响应中暴露 ──────────────

    #[tokio::test]
    async fn test_o1_session_scoped_history_exposes_scope_session_id() {
        // 传 session_id 调 history → 响应应包含 scope_session_id 字段，值与传入一致
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let scope_id = "550e8400-e29b-41d4-a716-446655440000";
        let body = serde_json::json!({"character_id": "alice", "session_id": scope_id});
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            v["scope_session_id"].as_str(),
            Some(scope_id),
            "session-scoped 响应必须暴露 scope_session_id 且与传入值一致"
        );
    }

    #[tokio::test]
    async fn test_o1_legacy_history_omits_scope_session_id() {
        // 不传 session_id 调 history → 响应不应包含 scope_session_id 字段（None skip）
        let (state, _tmp) = make_state_no_key();
        let app = create_router(state);
        let body = serde_json::json!({"character_id": "alice"});
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/chat/history")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            v.get("scope_session_id").is_none() || v["scope_session_id"].is_null(),
            "legacy 响应不应包含 scope_session_id 字段"
        );
    }
}

// ── DX-4 tests: UserOrIpKeyExtractor ─────────────────────────────────────────

#[cfg(test)]
mod tests_dx4 {
    use super::{UserOrIpKeyExtractor, RATE_LIMIT_BURST, RATE_LIMIT_PERIOD};
    use axum::extract::ConnectInfo;
    use axum::http::Request;
    use std::net::SocketAddr;
    use tower_governor::key_extractor::KeyExtractor;

    fn req_no_auth() -> Request<()> {
        Request::builder().body(()).unwrap()
    }

    fn req_with_auth(token: &str) -> Request<()> {
        Request::builder()
            .header("Authorization", format!("Bearer {}", token))
            .body(())
            .unwrap()
    }

    fn req_with_connect_info(ip: &str) -> Request<()> {
        let addr: SocketAddr = format!("{}:12345", ip).parse().unwrap();
        let mut req = Request::builder().body(()).unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        req
    }

    #[test]
    fn rate_limit_matches_ten_requests_per_second_with_twenty_burst() {
        assert_eq!(RATE_LIMIT_PERIOD, std::time::Duration::from_millis(100));
        assert_eq!(RATE_LIMIT_BURST, 20);
    }

    #[test]
    fn test_dx4_bearer_token_used_as_key() {
        let ext = UserOrIpKeyExtractor;
        let req = req_with_auth("my-secret-token");
        let key = ext.extract(&req).unwrap();
        assert!(key.starts_with("k:"), "expected k: prefix, got: {}", key);
        assert!(key.contains("my-secret-token"));
    }

    #[test]
    fn test_dx4_ip_used_when_no_auth_header() {
        let ext = UserOrIpKeyExtractor;
        let req = req_with_connect_info("10.0.0.1");
        let key = ext.extract(&req).unwrap();
        assert!(key.starts_with("ip:"), "expected ip: prefix, got: {}", key);
        assert!(key.contains("10.0.0.1"));
    }

    #[test]
    fn test_a2_7_falls_back_to_fixed_key_without_auth_or_connect_info() {
        // A2-7: previously this errored (UnableToExtractKey). Now that the
        // governor covers every route, erroring would turn ConnectInfo-less
        // requests (in-process tests) into hard failures. The extractor falls
        // back to a fixed "ip:unknown" key instead — never errors.
        let ext = UserOrIpKeyExtractor;
        let req = req_no_auth();
        let key = ext.extract(&req).expect("must not error");
        assert_eq!(key, "ip:unknown");
    }

    #[test]
    fn test_dx4_long_token_truncated_to_32_chars() {
        let ext = UserOrIpKeyExtractor;
        let long_token = "x".repeat(64);
        let req = req_with_auth(&long_token);
        let key = ext.extract(&req).unwrap();
        assert_eq!(key.len(), 34, "k: prefix (2) + 32 chars = 34; got: {}", key);
    }

    #[test]
    fn test_a4_multibyte_token_does_not_panic() {
        // A4: 审计假设多字节 token 能到达 `&token[..32]` 切片路径，从而触发
        // "byte index 32 is not a char boundary" panic。实测：HTTP HeaderValue
        // 的 `to_str()` 在任意字节 ≥ 0x80 时返回 Err（HTTP/1.1 头值规范只允许
        // visible ASCII），因此非 ASCII token 会落回 IP key，永远到不了切片那行。
        // 结论：A4 描述的 panic 在当前代码路径下不可达；但 `chars().take(32)`
        // 仍是更安全的 Rust 写法（defense-in-depth），保留。
        // 本测试固化"多字节 token 落回 ip:unknown、不 panic"的现状，防止未来
        // 有人放松 `to_str()` 检查时悄悄把 panic 引回来。
        let ext = UserOrIpKeyExtractor;
        let multibyte_token = "🛡".repeat(40); // U+1F6E1，单码点 4 字节，40 个 = 160 字节
        let req = req_with_auth(&multibyte_token);
        let key = ext
            .extract(&req)
            .expect("must not panic on multibyte token");
        assert_eq!(
            key, "ip:unknown",
            "multibyte token rejected by to_str(); expected IP fallback"
        );
    }
}
