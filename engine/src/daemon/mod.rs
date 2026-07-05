//! Daemon state, config, auth middleware, and axum router factory.

pub(crate) mod handlers;
pub mod types;

pub use types::{
    ChatCompletionRequest, ChatResponseChunk, HistoryQuery, RegenRequest, RollbackRequest,
    UserProfile,
};

use crate::adapter::{BackendEngine, Provider};
use crate::config::VolumeConfig;
use axum::{
    extract::{ConnectInfo, DefaultBodyLimit},
    http::{header, Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tower_governor::{governor::GovernorConfigBuilder, GovernorLayer};
use tower_http::cors::{Any, CorsLayer};

use handlers::{
    add_scene_character_endpoint, agent_run, chat_completion, create_scene_endpoint,
    create_session_endpoint, get_character_avatar, get_character_state,
    get_character_state_history, get_character_state_schema, get_chat_history,
    get_preset_endpoint, get_scene_endpoint, get_settings, import_character, list_characters,
    list_models, list_presets_endpoint, list_scenes_endpoint, list_sessions_endpoint,
    reextract_character_assets, regen_chat, rollback_chat, update_settings,
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
}

/// `GET /v1/settings` 返回值：api_key 脱敏为 `Some("***")` / `None`。
#[derive(Debug, Serialize)]
pub struct SettingsView {
    pub provider: Provider,
    pub endpoint: String,
    pub api_key_set: bool,
    pub model: String,
    pub volume_config: VolumeConfig,
    pub engine: BackendEngine,
    pub quota: crate::quota::QuotaConfig,
}

impl SettingsView {
    pub(crate) fn from_config(cfg: &MutableConfig) -> Self {
        Self {
            provider: cfg.provider.clone(),
            endpoint: cfg.endpoint.clone(),
            api_key_set: cfg.api_key.as_deref().is_some_and(|s| !s.is_empty()),
            model: cfg.model.clone(),
            volume_config: cfg.volume_config.clone(),
            engine: cfg.engine.clone(),
            quota: cfg.quota.clone(),
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

/// 构造 axum Router：注册所有 `/v1/*` 端点、CORS 中间件、限流中间件。
pub fn create_router(state: Arc<DaemonState>) -> Router {
    let cors = CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(Any);

    // A2-7: rate limiting previously protected only /v1/chat/completions,
    // leaving import / sync / scene / mcp endpoints unthrottled. Build ONE
    // shared config (per-IP token bucket) and apply it as a router-wide
    // `.layer()` over both v1 and mcp routes so every request path shares
    // the same budget. 10 req/s sustained, burst 20 per IP.
    let governor_conf = Arc::new({
        let mut b = GovernorConfigBuilder::default();
        b.per_second(10).burst_size(20);
        b.key_extractor(UserOrIpKeyExtractor)
            .finish()
            .expect("GovernorConfigBuilder 配置有效")
    });

    let v1_routes = Router::new()
        .route("/v1/chat/completions", post(chat_completion))
        .route("/v1/agent/run", post(agent_run))
        .route("/v1/chat/history", post(get_chat_history))
        .route("/v1/chat/rollback", post(rollback_chat))
        .route("/v1/chat/regen", post(regen_chat))
        .route("/v1/characters", get(list_characters))
        .route(
            "/v1/characters/import",
            post(import_character).layer(DefaultBodyLimit::max(10 * 1024 * 1024)),
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
            "/v1/sessions/:character_id",
            get(list_sessions_endpoint).post(create_session_endpoint),
        )
        .route("/v1/settings", get(get_settings).post(update_settings))
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
        .merge(v1_routes)
        .layer(cors)
        .with_state(state)
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
        assert!(v["version"].as_str().unwrap().len() > 0);
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
}

// ── DX-4 tests: UserOrIpKeyExtractor ─────────────────────────────────────────

#[cfg(test)]
mod tests_dx4 {
    use super::UserOrIpKeyExtractor;
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
        let key = ext.extract(&req).expect("must not panic on multibyte token");
        assert_eq!(key, "ip:unknown", "multibyte token rejected by to_str(); expected IP fallback");
    }
}
