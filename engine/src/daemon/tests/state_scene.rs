// Character state, state schema, state history (M_LS-3 / LS-5 / LS-7) and
// scene management (M_MS-3) endpoint tests.
//
// Moved verbatim from `daemon::tests`. All cases keep the shared fixture's
// tempdir guard alive while exercising the router and on-disk artifacts.

use super::*;

// M_LS-3 tests

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
