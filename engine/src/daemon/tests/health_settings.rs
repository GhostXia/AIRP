// `/version`, `/health` readiness probe, and `/v1/settings` (A5 / runtime
// secret-keeping / production partial-update gate) tests.
//
// Moved verbatim from `daemon::tests`. Some health/settings tests build a
// one-off `DaemonState` inline rather than calling `make_state_with_key`,
// because they need to vary `api_key` / `endpoint` independently of
// `access_api_key`; their local `TempDir` guards keep the paths alive for the
// test lifetime and clean them up afterward.

use super::*;

// AUDIT-10: /version diagnostic endpoint
#[tokio::test]
async fn test_audit_10_version_endpoint_returns_metadata() {
    let (state, _tmp) = make_state_with_key(None);
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
    let (state, _tmp) = make_state_with_key(Some("secret-key"));
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
    let (state, _tmp) = make_state_with_key(Some("secret-key"));
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
    let (state, _tmp) = make_state_with_key(None);
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
    let (state, _tmp) = make_state_with_key(Some("secret-key"));
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
    let (state, _tmp) = make_state_with_key(Some("old-production-key"));
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
