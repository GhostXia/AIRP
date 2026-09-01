use super::*;
use axum::{body::to_bytes, http::StatusCode};
use sha2::{Digest, Sha256};

use crate::revision::{
    atomic::{commit_revision, CommitOptions, StagedRevision},
    manifest::{AssetKind, AssetSource},
};

const AUTH: &str = "Bearer workspace-secret";

fn blueprint_v1_source() -> serde_json::Value {
    serde_json::json!({
        "version": "legacy-http-test",
        "profile": "story",
        "theme": {
            "name": "legacy-theme",
            "tokens": { "accent": "kept-for-parsing-but-not-migrated" }
        },
        "layout": {
            "type": "tabs",
            "areas": [{
                "id": "main",
                "widgets": ["chat"],
                "props": { "arbitrary": [true, 7, { "nested": "value" }] }
            }]
        },
        "widgets": [{
            "id": "chat",
            "type": "core.chat",
            "props": { "runtime": "must-drop" },
            "state": "session",
            "capabilities": ["read:memory"]
        }]
    })
}

async fn request_json(
    app: Router,
    method: axum::http::Method,
    uri: &str,
    body: Option<serde_json::Value>,
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", AUTH);
    let body = match body {
        Some(value) => {
            builder = builder.header("content-type", "application/json");
            Body::from(value.to_string())
        }
        None => Body::empty(),
    };
    let response = app.oneshot(builder.body(body).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn workspace_migration_errors_are_no_store_for_every_exact_route() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state);
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/migrations/rollback")
                .header("authorization", AUTH)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": "0",
                        "backup_id": "missing-backup"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers()["cache-control"], "no-store");

    let (_, plan) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/dry-run",
        Some(serde_json::json!({ "source": blueprint_v1_source() })),
    )
    .await;
    let (status, _) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands",
        Some(serde_json::json!({
            "expected_revision": "0",
            "command": { "type": "reset_layout" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let response = app
        .oneshot(
            Request::post("/v1/ui/workspace/migrations/blueprint-v1/apply")
                .header("authorization", AUTH)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": "0",
                        "source": blueprint_v1_source(),
                        "planned_source_sha256": plan["source_sha256"],
                        "planned_candidate_sha256": plan["candidate_sha256"],
                        "planned_converter_version": plan["converter_version"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(response.headers()["cache-control"], "no-store");

    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state);
    let mut requests = tokio::task::JoinSet::new();
    for _ in 0..(RATE_LIMIT_BURST * 2) {
        let app = app.clone();
        requests.spawn(async move {
            app.oneshot(
                Request::post("/v1/ui/workspace/migrations/blueprint-v1/dry-run")
                    .header("authorization", AUTH)
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap()
        });
    }
    let mut rate_limited = None;
    while let Some(response) = requests.join_next().await {
        let response = response.unwrap();
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            rate_limited = Some(response);
        }
    }
    let response = rate_limited.expect("production governor must exhaust its burst budget");
    assert_eq!(response.headers()["cache-control"], "no-store");
}

#[tokio::test]
async fn workspace_requires_configured_and_valid_bearer() {
    let (unconfigured, _tmp) = make_state_with_key(None);
    let app = create_router(unconfigured);
    for request in [
        Request::get("/v1/ui/workspace")
            .body(Body::empty())
            .unwrap(),
        Request::get("/v1/ui/workspace/history")
            .body(Body::empty())
            .unwrap(),
        Request::get("/v1/ui/workspace/export")
            .body(Body::empty())
            .unwrap(),
        Request::post("/v1/ui/workspace/commands")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        Request::post("/v1/ui/workspace/rollback")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        Request::post("/v1/ui/workspace/migrations/blueprint-v1/dry-run")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        Request::post("/v1/ui/workspace/migrations/blueprint-v1/apply")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
        Request::post("/v1/ui/workspace/migrations/rollback")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap(),
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/migrations/blueprint-v1/dry-run")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["cache-control"], "no-store");

    let (configured, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(configured);
    let response = app
        .clone()
        .oneshot(
            Request::get("/v1/ui/workspace")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/migrations/blueprint-v1/dry-run")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "source": blueprint_v1_source() }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let response = app
        .oneshot(
            Request::get("/v1/ui/workspace")
                .header("authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn unauthorized_workspace_command_cannot_commit_before_later_authorized_request() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let service = crate::domain::WorkspaceService::new(&state.data_root);
    assert_eq!(service.read().unwrap().revision.value(), 0);
    let app = create_router(state.clone());
    let command = serde_json::json!({
        "expected_revision": "0",
        "command": { "type": "reset_layout" }
    });

    let unauthorized = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/commands")
                .header("authorization", "Bearer stale")
                .header("content-type", "application/json")
                .body(Body::from(command.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(service.read().unwrap().revision.value(), 0);

    let authorized = app
        .oneshot(
            Request::post("/v1/ui/workspace/commands")
                .header("authorization", AUTH)
                .header("content-type", "application/json")
                .body(Body::from(command.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authorized.status(), StatusCode::OK);
    assert_eq!(service.read().unwrap().revision.value(), 1);
    assert_eq!(service.history(256).unwrap().len(), 1);
}

#[tokio::test]
async fn workspace_command_rejects_non_string_revision_without_writing() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state);
    let (status, rejection) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands",
        Some(serde_json::json!({
            "expected_revision": 0,
            "command": {
                "type": "resize_split",
                "split_id": "workspace-root",
                "ratio_basis_points": 6000
            }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(rejection["error"]["code"], "bad_request");

    let (status, document) =
        request_json(app, axum::http::Method::GET, "/v1/ui/workspace", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(document["revision"], "0");
}

#[tokio::test]
async fn workspace_oversized_command_is_rejected_without_writing() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state);
    let oversized = serde_json::json!({
        "expected_revision": "0",
        "command": {
            "type": "resize_split",
            "split_id": "workspace-root",
            "ratio_basis_points": 6000,
            "padding": "x".repeat(super::super::handlers::WORKSPACE_HTTP_MAX_BODY_BYTES)
        }
    });
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/commands")
                .header("authorization", AUTH)
                .header("content-type", "application/json")
                .body(Body::from(oversized.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let (status, document) =
        request_json(app, axum::http::Method::GET, "/v1/ui/workspace", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(document["revision"], "0");
}

#[tokio::test]
async fn workspace_http_preserves_string_cas_user_scope_history_and_forward_rollback() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state);
    let (status, initial) = request_json(
        app.clone(),
        axum::http::Method::GET,
        "/v1/ui/workspace?user_id=alice",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(initial["revision"], "0");

    let resize = serde_json::json!({
        "expected_revision": "0",
        "command": {
            "type": "resize_split",
            "split_id": "workspace-root",
            "ratio_basis_points": 6000
        },
    });
    let (status, first) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands?user_id=alice",
        Some(resize.clone()),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["revision"], "1");

    let (status, stale) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands?user_id=alice",
        Some(resize),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(stale["error"]["code"], "workspace_revision_conflict");
    assert_eq!(stale["error"]["recovery"], "refresh_and_retry");
    assert_eq!(stale["error"]["current_revision"], "1");

    let (status, second) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands?user_id=alice",
        Some(serde_json::json!({
            "expected_revision": "1",
            "command": {
                "type": "resize_split",
                "split_id": "workspace-root",
                "ratio_basis_points": 5500
            },
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["revision"], "2");

    let (status, history) = request_json(
        app.clone(),
        axum::http::Method::GET,
        "/v1/ui/workspace/history?user_id=alice&limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(history["entries"][0]["revision"], "2");
    assert_eq!(history["entries"][1]["revision"], "1");

    let (status, invalid_limit) = request_json(
        app.clone(),
        axum::http::Method::GET,
        "/v1/ui/workspace/history?user_id=alice&limit=0",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_limit["error"]["code"], "bad_request");

    let (status, rolled) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/rollback?user_id=alice",
        Some(serde_json::json!({
            "expected_revision": "2",
            "target_revision": "1",
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rolled["revision"], "3");

    let (status, bob) = request_json(
        app,
        axum::http::Method::GET,
        "/v1/ui/workspace?user_id=bob",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bob["revision"], "0");
}

#[tokio::test]
async fn workspace_http_accepts_the_closed_layout_command_set() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state);
    let commands = [
        serde_json::json!({
            "type": "open_widget", "instance_id": "map", "widget_type": "core.map",
            "target_id": "workspace-context", "index": 1
        }),
        serde_json::json!({
            "type": "move_widget", "instance_id": "map",
            "target_id": "workspace-primary", "index": 1
        }),
        serde_json::json!({
            "type": "activate_tab", "tabs_id": "workspace-primary", "node_id": "map"
        }),
        serde_json::json!({"type": "close_widget", "instance_id": "map"}),
        serde_json::json!({"type": "reset_layout"}),
    ];
    for (revision, command) in commands.into_iter().enumerate() {
        let (status, document) = request_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/ui/workspace/commands",
            Some(serde_json::json!({
                "expected_revision": revision.to_string(),
                "command": command
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(document["revision"], (revision + 1).to_string());
    }
}

#[tokio::test]
async fn workspace_export_returns_exact_hashed_json() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state);
    let _ = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands",
        Some(serde_json::json!({
            "expected_revision": "0",
            "command": {
                "type": "resize_split",
                "split_id": "workspace-root",
                "ratio_basis_points": 6000
            },
        })),
    )
    .await;

    let response = app
        .oneshot(
            Request::get("/v1/ui/workspace/export")
                .header("authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/json; charset=utf-8"
    );
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"airp-workspace-default.json\""
    );
    let expected_hash = response.headers()["x-airp-workspace-sha256"]
        .to_str()
        .unwrap()
        .to_string();
    let bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    assert_eq!(format!("{:x}", Sha256::digest(&bytes)), expected_hash);
    let raw: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(raw["revision"], "1");
}

#[tokio::test]
async fn workspace_future_major_is_structured_and_only_raw_export_remains_available() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state.clone());
    let (_, mut future) = request_json(
        app.clone(),
        axum::http::Method::GET,
        "/v1/ui/workspace",
        None,
    )
    .await;
    future["schema"] = serde_json::json!(99);
    future["revision"] = serde_json::json!("1");
    let bytes = serde_json::to_vec_pretty(&future).unwrap();
    let asset_dir = state.data_root.join("ui/workspaces/default");
    commit_revision(
        &StagedRevision {
            content_revision: 1,
            asset_kind: AssetKind::Workspace,
            asset_id: "default".to_string(),
            created_at: "future-test".to_string(),
            source: AssetSource::default(),
            files: vec![("workspace.json".to_string(), bytes.clone())],
        },
        &CommitOptions::new(asset_dir),
    )
    .unwrap();

    let (status, rejection) = request_json(
        app.clone(),
        axum::http::Method::GET,
        "/v1/ui/workspace",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(rejection["error"]["code"], "workspace_unsupported_major");
    assert_eq!(rejection["error"]["actual_major"], 99);
    assert_eq!(rejection["error"]["supported_major"], 1);
    assert_eq!(rejection["error"]["recovery"], "export_or_upgrade");

    let response = app
        .oneshot(
            Request::get("/v1/ui/workspace/export")
                .header("authorization", AUTH)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX).await.unwrap(),
        bytes
    );
}

#[tokio::test]
async fn workspace_migration_dry_run_is_strict_bounded_no_store_and_write_free() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state.clone());
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/migrations/blueprint-v1/dry-run")
                .header("authorization", AUTH)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "source": blueprint_v1_source() }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    let plan: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(plan["source"], "blueprint-v1");
    assert_eq!(plan["writes_performed"], false);
    assert_eq!(plan["workspace"]["revision"], "0");
    assert_eq!(plan["recovery"], "review_and_apply");
    assert!(!plan["workspace"].to_string().contains("must-drop"));
    assert!(!state.data_root.join("ui/workspaces/default").exists());
    assert!(crate::backup::list_backups(&state.data_root)
        .unwrap()
        .is_empty());

    let mut unknown_sources = Vec::new();
    let mut top_level = blueprint_v1_source();
    top_level["unexpected"] = serde_json::json!(true);
    unknown_sources.push(top_level);
    let mut theme = blueprint_v1_source();
    theme["theme"]["unexpected"] = serde_json::json!(true);
    unknown_sources.push(theme);
    let mut layout = blueprint_v1_source();
    layout["layout"]["unexpected"] = serde_json::json!(true);
    unknown_sources.push(layout);
    let mut area = blueprint_v1_source();
    area["layout"]["areas"][0]["unexpected"] = serde_json::json!(true);
    unknown_sources.push(area);
    let mut widget = blueprint_v1_source();
    widget["widgets"][0]["unexpected"] = serde_json::json!(true);
    unknown_sources.push(widget);
    for source_with_unknown in unknown_sources {
        let response = app
            .clone()
            .oneshot(
                Request::post("/v1/ui/workspace/migrations/blueprint-v1/dry-run")
                    .header("authorization", AUTH)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "source": source_with_unknown }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }

    let oversized = serde_json::json!({
        "source": blueprint_v1_source(),
        "padding": "x".repeat(
            super::super::handlers::WORKSPACE_MIGRATION_HTTP_MAX_BODY_BYTES
        )
    });
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/migrations/blueprint-v1/dry-run")
                .header("authorization", AUTH)
                .header("content-type", "application/json")
                .body(Body::from(oversized.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(response.headers()["cache-control"], "no-store");

    for (path, payload) in [
        (
            "/v1/ui/workspace/migrations/blueprint-v1/apply",
            serde_json::json!({
                "expected_revision": "0",
                "source": blueprint_v1_source(),
                "planned_source_sha256": "unused",
                "planned_candidate_sha256": "unused",
                "planned_converter_version": "unused",
                "padding": "x".repeat(
                    super::super::handlers::WORKSPACE_MIGRATION_HTTP_MAX_BODY_BYTES
                )
            }),
        ),
        (
            "/v1/ui/workspace/migrations/rollback",
            serde_json::json!({
                "expected_revision": "0",
                "backup_id": "unused",
                "padding": "x".repeat(
                    super::super::handlers::WORKSPACE_MIGRATION_HTTP_MAX_BODY_BYTES
                )
            }),
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post(path)
                    .header("authorization", AUTH)
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
    assert!(!state.data_root.join("ui/workspaces/default").exists());
    assert!(crate::backup::list_backups(&state.data_root)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn workspace_migration_revision_zero_apply_creates_verified_backup_and_rolls_forward() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state.clone());
    let (status, plan) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/dry-run",
        Some(serde_json::json!({ "source": blueprint_v1_source() })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let (status, applied) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/apply",
        Some(serde_json::json!({
            "expected_revision": "0",
            "source": blueprint_v1_source(),
            "planned_source_sha256": plan["source_sha256"],
            "planned_candidate_sha256": plan["candidate_sha256"],
            "planned_converter_version": plan["converter_version"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(applied["workspace"]["revision"], "1");
    assert_eq!(applied["source_sha256"], plan["source_sha256"]);
    assert_eq!(applied["candidate_sha256"], plan["candidate_sha256"]);
    assert_eq!(applied["converter_version"], plan["converter_version"]);
    assert_eq!(applied["recovery"], "rollback_with_backup_id");
    let backup_id = applied["backup_id"].as_str().unwrap();
    assert!(crate::backup::verify_backup(&state.data_root, backup_id).is_ok());

    let (status, rolled) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/rollback",
        Some(serde_json::json!({
            "expected_revision": "1",
            "backup_id": backup_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(rolled["workspace"]["revision"], "2");
    assert_eq!(rolled["backup_id"], backup_id);
    assert_eq!(rolled["recovery"], "forward_rollback_completed");

    let (status, history) = request_json(
        app,
        axum::http::Method::GET,
        "/v1/ui/workspace/history?limit=10",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        history["entries"][0]["source_kind"],
        "workspace_backup_rollback"
    );
    assert_eq!(
        history["entries"][1]["source_kind"],
        "workspace_migration_blueprint_v1"
    );
}

#[tokio::test]
async fn workspace_migration_rejections_create_no_revision_or_backup() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state.clone());
    let (_, plan) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/dry-run",
        Some(serde_json::json!({ "source": blueprint_v1_source() })),
    )
    .await;
    let valid_apply = serde_json::json!({
        "expected_revision": "0",
        "source": blueprint_v1_source(),
        "planned_source_sha256": plan["source_sha256"],
        "planned_candidate_sha256": plan["candidate_sha256"],
        "planned_converter_version": plan["converter_version"]
    });

    let mut numeric_revision = valid_apply.clone();
    numeric_revision["expected_revision"] = serde_json::json!(0);
    let (status, _) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/apply",
        Some(numeric_revision),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut mismatched = valid_apply.clone();
    mismatched["planned_candidate_sha256"] = serde_json::json!("wrong");
    let (status, _) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/apply",
        Some(mismatched),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let (status, _) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands",
        Some(serde_json::json!({
            "expected_revision": "0",
            "command": { "type": "reset_layout" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, stale) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/apply",
        Some(valid_apply),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(stale["error"]["code"], "workspace_revision_conflict");
    assert_eq!(stale["error"]["current_revision"], "1");

    let (status, current) =
        request_json(app, axum::http::Method::GET, "/v1/ui/workspace", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current["revision"], "1");
    assert!(crate::backup::list_backups(&state.data_root)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn workspace_migration_backup_is_user_scoped_and_tamper_safe() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state.clone());
    let (_, plan) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/dry-run?user_id=alice",
        Some(serde_json::json!({ "source": blueprint_v1_source() })),
    )
    .await;
    let (status, applied) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/apply?user_id=alice",
        Some(serde_json::json!({
            "expected_revision": "0",
            "source": blueprint_v1_source(),
            "planned_source_sha256": plan["source_sha256"],
            "planned_candidate_sha256": plan["candidate_sha256"],
            "planned_converter_version": plan["converter_version"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let alice_backup_id = applied["backup_id"].as_str().unwrap();
    let (status, invalid_id) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/rollback?user_id=alice",
        Some(serde_json::json!({
            "expected_revision": "1",
            "backup_id": "../outside"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(invalid_id["error"]["code"], "bad_request");

    let (status, rejection) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/rollback?user_id=bob",
        Some(serde_json::json!({
            "expected_revision": "0",
            "backup_id": alice_backup_id
        })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(rejection["error"]["code"], "not_found");
    let (status, bob) = request_json(
        app.clone(),
        axum::http::Method::GET,
        "/v1/ui/workspace?user_id=bob",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bob["revision"], "0");

    let (status, _) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/commands",
        Some(serde_json::json!({
            "expected_revision": "0",
            "command": { "type": "reset_layout" }
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (_, root_plan) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/dry-run",
        Some(serde_json::json!({ "source": blueprint_v1_source() })),
    )
    .await;
    let (status, root_applied) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/apply",
        Some(serde_json::json!({
            "expected_revision": "1",
            "source": blueprint_v1_source(),
            "planned_source_sha256": root_plan["source_sha256"],
            "planned_candidate_sha256": root_plan["candidate_sha256"],
            "planned_converter_version": root_plan["converter_version"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let root_backup_id = root_applied["backup_id"].as_str().unwrap();
    let manifest = crate::backup::read_backup_manifest(&state.data_root, root_backup_id).unwrap();
    let approved = manifest
        .files
        .first()
        .expect("revision-one backup has a file");
    std::fs::write(
        state
            .data_root
            .join("backups")
            .join(root_backup_id)
            .join("files")
            .join(&approved.path),
        b"tampered",
    )
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/workspace/migrations/rollback")
                .header("authorization", AUTH)
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "expected_revision": "2",
                        "backup_id": root_backup_id
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.headers()["cache-control"], "no-store");
    let tampered: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap()).unwrap();
    assert_eq!(tampered["error"]["code"], "internal_error");
    assert_eq!(tampered["error"]["message"], "internal error");
    assert!(!tampered.to_string().contains("backups"));
    let (status, current) =
        request_json(app, axum::http::Method::GET, "/v1/ui/workspace", None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(current["revision"], "2");
}

#[tokio::test]
async fn workspace_migration_apply_fails_closed_for_future_major_without_backup() {
    let (state, _tmp) = make_state_with_key(Some("workspace-secret"));
    let app = create_router(state.clone());
    let (_, mut future) = request_json(
        app.clone(),
        axum::http::Method::GET,
        "/v1/ui/workspace",
        None,
    )
    .await;
    future["schema"] = serde_json::json!(99);
    future["revision"] = serde_json::json!("1");
    let bytes = serde_json::to_vec_pretty(&future).unwrap();
    commit_revision(
        &StagedRevision {
            content_revision: 1,
            asset_kind: AssetKind::Workspace,
            asset_id: "default".to_string(),
            created_at: "future-migration-test".to_string(),
            source: AssetSource::default(),
            files: vec![("workspace.json".to_string(), bytes)],
        },
        &CommitOptions::new(state.data_root.join("ui/workspaces/default")),
    )
    .unwrap();
    let (_, plan) = request_json(
        app.clone(),
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/dry-run",
        Some(serde_json::json!({ "source": blueprint_v1_source() })),
    )
    .await;
    let (status, rejection) = request_json(
        app,
        axum::http::Method::POST,
        "/v1/ui/workspace/migrations/blueprint-v1/apply",
        Some(serde_json::json!({
            "expected_revision": "1",
            "source": blueprint_v1_source(),
            "planned_source_sha256": plan["source_sha256"],
            "planned_candidate_sha256": plan["candidate_sha256"],
            "planned_converter_version": plan["converter_version"]
        })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(rejection["error"]["code"], "workspace_unsupported_major");
    assert!(crate::backup::list_backups(&state.data_root)
        .unwrap()
        .is_empty());
    assert_eq!(
        std::fs::read_to_string(
            state
                .data_root
                .join("ui/workspaces/default/current_revision")
        )
        .unwrap(),
        "1"
    );
}
