// Legacy + multi-persona CRUD/binding tests.
//
// Originally inlined in `daemon::tests`; moved verbatim. `use super::*;` pulls
// in both `daemon` items (via `tests/mod.rs`'s `use super::*`) and the shared
// fixtures (`make_state_with_key`) declared `pub(super)` there.

use super::*;

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

    let body = serde_json::json!({
        "persona_id": "Default",
        "name": "D",
        "description": "",
        "variables": {}
    })
    .to_string();
    let response = create_router(make_state_with_key(None))
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
    let body = serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}})
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

    let response = create_router(make_state_with_key(None))
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/v1/users/u/personas/Default")
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
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let repeated: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(repeated.revision, after_unbind.revision);
}

#[tokio::test]
async fn unbind_missing_query_uses_airp_error_envelope() {
    let response = create_router(make_state_with_key(None))
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/v1/users/u/personas/p1/bindings")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json"
    );
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(error["error"]["code"], "bad_request");
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
