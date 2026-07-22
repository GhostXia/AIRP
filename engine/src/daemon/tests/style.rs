use super::*;

#[tokio::test]
async fn drift_put_get_and_rollback_expose_revision_contract() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);

    for content in ["first", "second"] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/characters/hero/drift")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({ "content": content }).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let rollback = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/characters/hero/drift/rollback")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({ "revision": 1 }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rollback.status(), StatusCode::OK);
    let rollback_body = axum::body::to_bytes(rollback.into_body(), 4096)
        .await
        .unwrap();
    let rollback_json: serde_json::Value = serde_json::from_slice(&rollback_body).unwrap();
    assert_eq!(rollback_json["revision"], 3);

    let get = app
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/characters/hero/drift")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let get_body = axum::body::to_bytes(get.into_body(), 4096).await.unwrap();
    let get_json: serde_json::Value = serde_json::from_slice(&get_body).unwrap();
    assert_eq!(get_json["content"], "first");
    assert_eq!(get_json["revision"], 3);
}

#[tokio::test]
async fn drift_rollback_rejects_unknown_revision_without_mutation() {
    let (state, _tmp) = make_state_with_key(None);
    crate::style::write_soul_drift(&state.data_root, "hero", "first").unwrap();
    let app = create_router(state.clone());

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/characters/hero/drift/rollback")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({ "revision": 99 }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        crate::style::read_soul_drift(&state.data_root, "hero").unwrap(),
        "first"
    );
}
