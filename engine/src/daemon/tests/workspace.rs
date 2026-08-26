use super::*;
use axum::{body::to_bytes, http::StatusCode};
use sha2::{Digest, Sha256};

use crate::revision::{
    atomic::{commit_revision, CommitOptions, StagedRevision},
    manifest::{AssetKind, AssetSource},
};

const AUTH: &str = "Bearer workspace-secret";

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
    ] {
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

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
