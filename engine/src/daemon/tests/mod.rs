// Tests for daemon — declared as `#[cfg(test)] mod tests;` in `daemon/mod.rs`.
//
// This file is the `daemon::tests` child module. `use super::*;` imports all
// accessible items from `daemon` (including private functions like
// `allowed_cors_origins`, `constant_time_eq`, `auth_middleware`,
// `production_cache_policy`, `UserOrIpKeyExtractor`, `RATE_LIMIT_PERIOD`,
// `RATE_LIMIT_BURST`, since child modules can see parent private items in Rust).
//
// Sub-modules below do their own `use super::*;` to pull from THIS scope. Test
// fixtures (`make_state_with_key`, `make_router_for_test`, `make_state_no_key`)
// are intentionally `pub(super)` so they are visible to descendant test
// sub-modules but never leak into production code.

use super::*;
use axum::body::Body;
use tower::util::ServiceExt;

mod catalog;
mod chat;
mod conversations;
mod desktop_session;
mod extensions;
mod health_settings;
mod images;
mod local_webui;
mod memory;
mod persona;
mod plugins;
mod security;
mod session_recovery;
mod sessions;
mod state_scene;
mod style;
mod templates;

/// Build a `DaemonState` rooted at a fresh tempdir, optionally with an
/// `access_api_key`. The returned guard must stay alive for the test lifetime.
pub(super) fn make_state_with_key(key: Option<&str>) -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let state = Arc::new(DaemonState {
        data_root: tmp.path().to_path_buf(),
        http_client: reqwest::Client::new(),
        fts: Default::default(),
        settings_update: Default::default(),
        session_coordinators: Default::default(),
        provider_router: Default::default(),
        provider_routing_update: Default::default(),
        plugin_tools: Default::default(),
        plugin_tools_update: Default::default(),
        extensions: std::sync::OnceLock::new(),
        plugins: Default::default(),
        plugin_children: tokio::sync::Mutex::new(std::collections::HashMap::new()),
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
    });
    (state, tmp)
}

#[test]
fn state_fixture_keeps_data_root_alive_until_guard_drops() {
    let (state, tmp) = make_state_with_key(None);
    assert!(state.data_root.exists());
    drop(tmp);
    assert!(!state.data_root.exists());
}

/// Minimal router exercising only `auth_middleware` over `/v1/ping`. Used by
/// DX-2 tests that need to assert accept/reject without standing up the full
/// route table.
pub(super) fn make_router_for_test(state: Arc<DaemonState>) -> Router {
    let v1_ping = Router::new()
        .route("/v1/ping", get(|| async { "ok" }))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));
    Router::new().merge(v1_ping).with_state(state)
}

/// Build a `DaemonState` rooted at a fresh tempdir with no auth key, returning
/// the tempdir handle alongside the state so callers that need to assert
/// on-disk artifacts can keep the dir alive for the test's lifetime.
pub(super) fn make_state_no_key() -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let state = Arc::new(DaemonState {
        data_root: tmp.path().to_path_buf(),
        http_client: reqwest::Client::new(),
        fts: Default::default(),
        settings_update: Default::default(),
        session_coordinators: Default::default(),
        provider_router: Default::default(),
        provider_routing_update: Default::default(),
        plugin_tools: Default::default(),
        plugin_tools_update: Default::default(),
        extensions: std::sync::OnceLock::new(),
        plugins: Default::default(),
        plugin_children: tokio::sync::Mutex::new(std::collections::HashMap::new()),
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

/// C-P4-1：从已有 data_root 构造 state（extensions OnceLock 未初始化，
/// 首次访问 extensions 端点时从磁盘加载）。用于 catalog fail-closed 测试
/// 模拟 extensions.json 被篡改后重新加载的场景。
pub(super) fn make_state_with_data_root(data_root: std::path::PathBuf) -> Arc<DaemonState> {
    Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        fts: Default::default(),
        settings_update: Default::default(),
        session_coordinators: Default::default(),
        provider_router: Default::default(),
        provider_routing_update: Default::default(),
        plugin_tools: Default::default(),
        plugin_tools_update: Default::default(),
        extensions: std::sync::OnceLock::new(),
        plugins: Default::default(),
        plugin_children: tokio::sync::Mutex::new(std::collections::HashMap::new()),
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
    })
}
