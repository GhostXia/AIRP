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

async fn submit_settings_patch_with_invalid_volume(
    state: Arc<DaemonState>,
) -> axum::response::Response {
    create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "endpoint": "https://api.openai.com",
                        "model": "gpt-4o-mini",
                        "volume": {
                            "soft_threshold_tokens": 4000,
                            "hard_threshold_tokens": 3500,
                            "seal_temperature": 0.3,
                            "maintenance_interval": 20
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap()
}

// #165 SET-01：同一 patch 同时携带有效 endpoint/model 和无效 volume（soft >= hard）
// 时，必须在拿写锁前拒绝整笔请求，不得留下部分内存更新或落盘。
#[tokio::test]
async fn settings_update_rejects_invalid_volume_without_partial_live_config_mutation() {
    let (state, _tmp) = make_state_no_key();
    let response = submit_settings_patch_with_invalid_volume(state.clone()).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    // 失败后 live config 的所有字段必须与请求前一致（make_state_no_key 初值：
    // endpoint="http://localhost"、model="gpt-4o"、volume=VolumeConfig::default()）。
    // VolumeConfig 未派生 PartialEq，用字段级断言精确比较。
    let after = state.config.read().unwrap();
    assert_eq!(after.endpoint, "http://localhost");
    assert_eq!(after.model, "gpt-4o");
    assert_eq!(after.volume_config.soft_threshold_tokens, 2500);
    assert_eq!(after.volume_config.hard_threshold_tokens, 3500);
    assert_eq!(after.volume_config.seal_temperature, 0.3);
    assert!(after.volume_config.seal_model.is_none());
    assert_eq!(after.volume_config.maintenance_interval, 20);
    drop(after);

    // 失败后 settings.json 不得被创建或修改。
    assert!(!state.data_root.join("settings.json").exists());
}

#[tokio::test]
async fn settings_update_rejects_invalid_volume_without_modifying_existing_settings_file() {
    let (state, _tmp) = make_state_no_key();
    let path = state.data_root.join("settings.json");
    let original =
        b"{\n  \"endpoint\": \"https://existing.example\",\n  \"model\": \"persisted\"\n}\n";
    std::fs::write(&path, original).unwrap();

    let response = submit_settings_patch_with_invalid_volume(state).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(std::fs::read(path).unwrap(), original);
}

// #165 SET-01 回归保护：valid volume 与 endpoint/model 同 patch 时仍能成功更新，
// 证明把 validate 提到写锁外没有破坏 volume 成功路径。
#[tokio::test]
async fn settings_update_applies_valid_volume_alongside_endpoint_and_model() {
    let (state, _tmp) = make_state_no_key();
    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "endpoint": "https://api.openai.com",
                        "model": "gpt-4o-mini",
                        "volume": {
                            "soft_threshold_tokens": 2000,
                            "hard_threshold_tokens": 3000,
                            "seal_temperature": 0.2,
                            "maintenance_interval": 15
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let after = state.config.read().unwrap();
    assert_eq!(after.endpoint, "https://api.openai.com");
    assert_eq!(after.model, "gpt-4o-mini");
    assert_eq!(after.volume_config.soft_threshold_tokens, 2000);
    assert_eq!(after.volume_config.hard_threshold_tokens, 3000);
    assert_eq!(after.volume_config.seal_temperature, 0.2);
    assert_eq!(after.volume_config.maintenance_interval, 15);
    drop(after);

    // 成功路径必须落盘，且 volume 字段同步写入。
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.data_root.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(persisted["endpoint"], "https://api.openai.com");
    assert_eq!(persisted["model"], "gpt-4o-mini");
    assert_eq!(persisted["volume"]["soft_threshold_tokens"], 2000);
    assert_eq!(persisted["volume"]["hard_threshold_tokens"], 3000);
}

// #187：写盘失败时请求返回 500，但 live config 必须保持请求前真值。
#[tokio::test]
async fn settings_update_write_failure_does_not_commit_live_config() {
    let (state, _tmp) = make_state_no_key();
    std::fs::create_dir(state.data_root.join("settings.json.tmp")).unwrap();

    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "endpoint": "https://must-not-commit.example",
                        "model": "must-not-commit"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let runtime = state.config.read().unwrap();
    assert_eq!(runtime.endpoint, "http://localhost");
    assert_eq!(runtime.model, "gpt-4o");
    drop(runtime);
    assert!(!state.data_root.join("settings.json").exists());
}

// #187：并发提交结束后，live config 与 settings.json 必须来自同一个提交。
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_settings_updates_leave_runtime_and_disk_on_same_commit() {
    let (state, _tmp) = make_state_no_key();
    let mut tasks = Vec::new();
    for index in 0..16 {
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            create_router(state)
                .oneshot(
                    axum::http::Request::builder()
                        .method("POST")
                        .uri("/v1/settings")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            serde_json::json!({
                                "endpoint": format!("https://commit-{index}.example"),
                                "model": format!("commit-{index}")
                            })
                            .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap()
        }));
    }
    for task in tasks {
        assert_eq!(task.await.unwrap().status(), StatusCode::OK);
    }

    let runtime = state.config.read().unwrap().clone();
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state.data_root.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(persisted["endpoint"], runtime.endpoint);
    assert_eq!(persisted["model"], runtime.model);
}
