// Desktop-token registry tests must serialize global token mutation across
// their oneshot request; the test request has no cross-thread lock dependency.
#![allow(clippy::await_holding_lock)]

use super::*;
use axum::{body::Body, http::Request};
use futures_util::StreamExt;
use tower::ServiceExt;

fn surface_uri(character_id: &str, session_id: &crate::types::SessionId) -> String {
    format!("/v1/ui/surfaces/session/{session_id}?character_id={character_id}")
}

#[tokio::test]
async fn surface_snapshot_requires_configured_and_valid_bearer() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    let app = create_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::get(surface_uri("alice", &session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

    let response = app
        .oneshot(
            Request::get(surface_uri("alice", &session_id))
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(json["cursor"].as_str().unwrap().starts_with("surface-v1."));
    assert_eq!(
        json["snapshot"]["surfaceId"],
        format!("session:{session_id}")
    );
    assert_eq!(
        json["snapshot"]["blueprint"]["widgets"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test]
async fn chat_intent_resolves_scope_from_the_accepted_surface() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    let app = create_router(state);

    let snapshot = app
        .clone()
        .oneshot(
            Request::get(surface_uri("alice", &session_id))
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.status(), axum::http::StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::post("/v1/ui/intents")
                .header("authorization", "Bearer surface-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "surface_id": format!("session:{session_id}"),
                        "instance_id": "chat",
                        "name": "chat.loadMore",
                        "params": {"limit": 20}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);

    let rejected = app
        .oneshot(
            Request::post("/v1/ui/intents")
                .header("authorization", "Bearer surface-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "surface_id": format!("session:{session_id}"),
                        "instance_id": "memory",
                        "name": "chat.loadMore"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn chat_send_intent_reaches_the_existing_sse_pipeline() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    std::fs::write(
        state.data_root.join("characters/alice/card.json"),
        r#"{"name":"Alice","description":"","personality":"","scenario":"","first_mes":"","mes_example":""}"#,
    )
    .unwrap();
    let app = create_router(state);
    let snapshot = app
        .clone()
        .oneshot(
            Request::get(surface_uri("alice", &session_id))
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.status(), axum::http::StatusCode::OK);

    let response = app
        .oneshot(
            Request::post("/v1/ui/intents")
                .header("authorization", "Bearer surface-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "surface_id": format!("session:{session_id}"),
                        "instance_id": "chat",
                        "name": "chat.send",
                        "params": {"text": "hello"}
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response.headers()["content-type"]
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
}

#[tokio::test]
async fn chat_intent_rejects_an_unregistered_surface() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let response = create_router(state)
        .oneshot(
            Request::post("/v1/ui/intents")
                .header("authorization", "Bearer surface-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"surface_id":"session:forged","instance_id":"chat","name":"chat.loadMore"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn stale_registered_surface_cannot_resurrect_a_deleted_session() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    let app = create_router(state.clone());
    let snapshot = app
        .clone()
        .oneshot(
            Request::get(surface_uri("alice", &session_id))
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(snapshot.status(), axum::http::StatusCode::OK);
    let session_dir = state
        .data_root
        .join("characters/alice/sessions")
        .join(session_id.to_string());
    std::fs::remove_dir_all(&session_dir).unwrap();

    let response = app
        .oneshot(
            Request::post("/v1/ui/intents")
                .header("authorization", "Bearer surface-secret")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "surface_id": format!("session:{session_id}"),
                        "instance_id": "chat",
                        "name": "chat.loadMore"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(!session_dir.exists());
}

#[tokio::test]
async fn chat_intent_fails_closed_when_daemon_auth_is_disabled() {
    let (state, _tmp) = make_state_with_key(None);
    let response = create_router(state)
        .oneshot(
            Request::post("/v1/ui/intents")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"surface_id":"session:forged","instance_id":"chat","name":"chat.loadMore"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn surface_api_fails_closed_when_daemon_auth_is_disabled() {
    let (state, _tmp) = make_state_with_key(None);
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();

    let response = create_router(state)
        .oneshot(
            Request::get(surface_uri("alice", &session_id))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn missing_surface_is_read_only_and_returns_not_found() {
    let (state, tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    let response = create_router(state)
        .oneshot(
            Request::get(surface_uri("alice", &session_id))
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(!tmp.path().join("characters").exists());
}

#[tokio::test]
async fn missing_user_surface_does_not_create_a_user_root() {
    let (state, tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    let uri =
        format!("/v1/ui/surfaces/session/{session_id}?character_id=alice&user_id=missing-user");

    let response = create_router(state)
        .oneshot(
            Request::get(uri)
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
    assert!(!tmp.path().join("users").exists());
}

#[tokio::test]
async fn equal_character_and_session_ids_do_not_alias_across_user_roots() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    let character_id = crate::types::CharacterId::new("shared-character").unwrap();
    for (user, mood) in [("tenant-a", "calm"), ("tenant-b", "focused")] {
        let root = crate::data_dir::resolve_effective_root(&state.data_root, Some(user)).unwrap();
        crate::data_dir::create_session_with_id(&root, character_id.as_str(), &session_id).unwrap();
        crate::domain::StateService::new(&root)
            .write(&character_id, &serde_json::json!({"mood": mood}))
            .unwrap();
    }
    let app = create_router(state);

    let first =
        user_surface_snapshot_json(app.clone(), "tenant-a", &character_id, &session_id).await;
    let second = user_surface_snapshot_json(app, "tenant-b", &character_id, &session_id).await;

    assert_eq!(widget_props(&first, "character-state")["mood"], "calm");
    assert_eq!(widget_props(&second, "character-state")["mood"], "focused");
}

#[tokio::test]
async fn surface_context_uses_session_persona_binding_before_character_binding() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let user_id = crate::types::UserId::new("tenant-a").unwrap();
    let character_id = crate::types::CharacterId::new("alice").unwrap();
    let session_id = crate::types::SessionId::new();
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, Some(user_id.as_str())).unwrap();
    crate::data_dir::create_session_with_id(&effective_root, character_id.as_str(), &session_id)
        .unwrap();
    create_surface_persona(&state, &user_id, "character-persona");
    create_surface_persona(&state, &user_id, "session-persona");
    let service = crate::domain::PersonaService::new(&state.data_root);
    service
        .bind(
            &user_id,
            "character-persona",
            crate::domain::PersonaBinding {
                character_id: character_id.to_string(),
                session_id: None,
            },
        )
        .unwrap();
    service
        .bind(
            &user_id,
            "session-persona",
            crate::domain::PersonaBinding {
                character_id: character_id.to_string(),
                session_id: Some(session_id.to_string()),
            },
        )
        .unwrap();

    let snapshot = user_surface_snapshot_json(
        create_router(state),
        user_id.as_str(),
        &character_id,
        &session_id,
    )
    .await;
    let context = chat_context(&snapshot);
    assert_eq!(context["character_id"], character_id.as_str());
    assert_eq!(context["session_id"], session_id.to_string());
    assert_eq!(context["persona_id"], "session-persona");
    assert_eq!(context["persona_source"], "session_binding");
    assert!(context["scene_id"].is_null());
}

#[tokio::test]
async fn surface_context_projects_default_persona_for_user_without_binding() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let user_id = crate::types::UserId::new("tenant-a").unwrap();
    let character_id = crate::types::CharacterId::new("alice").unwrap();
    let session_id = crate::types::SessionId::new();
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, Some(user_id.as_str())).unwrap();
    crate::data_dir::create_session_with_id(&effective_root, character_id.as_str(), &session_id)
        .unwrap();

    let snapshot = user_surface_snapshot_json(
        create_router(state),
        user_id.as_str(),
        &character_id,
        &session_id,
    )
    .await;
    let context = chat_context(&snapshot);
    assert_eq!(context["persona_id"], "default");
    assert_eq!(context["persona_source"], "default");
}

#[tokio::test]
async fn surface_context_has_no_persona_without_user_scope() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();

    let snapshot = surface_snapshot_json(create_router(state), "alice", &session_id).await;
    let context = chat_context(&snapshot);
    assert!(context["persona_id"].is_null());
    assert!(context["persona_source"].is_null());
}

#[tokio::test]
async fn surface_context_projects_only_existing_canonical_character_worldbook() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let present_session = crate::types::SessionId::new();
    let missing_session = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "with-lore", &present_session)
        .unwrap();
    crate::data_dir::create_session_with_id(&state.data_root, "without-lore", &missing_session)
        .unwrap();
    let lorebook = crate::data_dir::char_world_lorebook_path(&state.data_root, "with-lore");
    std::fs::create_dir_all(lorebook.parent().unwrap()).unwrap();
    std::fs::write(&lorebook, r#"{"entries":[]}"#).unwrap();
    let app = create_router(state);

    let present = surface_snapshot_json(app.clone(), "with-lore", &present_session).await;
    let missing = surface_snapshot_json(app, "without-lore", &missing_session).await;
    assert_eq!(
        chat_context(&present)["worldbook_source_ids"],
        serde_json::json!(["character:with-lore"])
    );
    assert_eq!(
        chat_context(&missing)["worldbook_source_ids"],
        serde_json::json!([])
    );
}

#[tokio::test]
async fn surface_context_fails_closed_on_ambiguous_persona_binding() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let user_id = crate::types::UserId::new("tenant-a").unwrap();
    let character_id = crate::types::CharacterId::new("alice").unwrap();
    let session_id = crate::types::SessionId::new();
    let effective_root =
        crate::data_dir::resolve_effective_root(&state.data_root, Some(user_id.as_str())).unwrap();
    crate::data_dir::create_session_with_id(&effective_root, character_id.as_str(), &session_id)
        .unwrap();
    let service = crate::domain::PersonaService::new(&state.data_root);
    for persona_id in ["first", "second"] {
        create_surface_persona(&state, &user_id, persona_id);
        let mut persona = service.get(&user_id, persona_id, "User").unwrap();
        persona.bindings.push(crate::domain::PersonaBinding {
            character_id: character_id.to_string(),
            session_id: Some(session_id.to_string()),
        });
        let path = crate::data_dir::user_persona_multi_path(&state.data_root, &user_id, persona_id)
            .unwrap();
        std::fs::write(path, serde_json::to_vec_pretty(&persona).unwrap()).unwrap();
    }
    let uri = format!(
        "/v1/ui/surfaces/session/{session_id}?character_id={character_id}&user_id={user_id}"
    );

    let response = create_router(state)
        .oneshot(
            Request::get(uri)
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(body.contains("ambiguous session-scoped persona binding"));
}

#[tokio::test]
async fn character_discovery_respects_the_surface_user_scope() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    for (user, character) in [("tenant-a", "alice"), ("tenant-b", "bob")] {
        std::fs::create_dir_all(
            state
                .data_root
                .join("users")
                .join(user)
                .join("characters")
                .join(character),
        )
        .unwrap();
    }
    let app = create_router(state);
    for (user, expected) in [("tenant-a", "alice"), ("tenant-b", "bob")] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/v1/characters?user_id={user}"))
                    .header("authorization", "Bearer surface-secret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let characters: Vec<String> = serde_json::from_slice(
            &axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(characters, vec![expected]);
    }
}

#[tokio::test]
async fn activity_failure_survives_surface_registry_restart() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    let session_dir =
        crate::data_dir::resolve_session_dir(&state.data_root, "alice", Some(&session_id)).unwrap();
    crate::ui_activity::record_failure(
        &session_dir,
        crate::ui_activity::ActivitySource::Chat,
        crate::ui_activity::ActivityFailureCode::UpstreamError,
        Some("generation-safe-id"),
    )
    .unwrap();
    let app = create_router(state.clone());

    let first = surface_snapshot_json(app.clone(), "alice", &session_id).await;
    assert_eq!(activity_failure_code(&first), "upstream_error");

    *state.ui_surfaces.lock().unwrap() = crate::ui_surface::SurfaceRegistry::new();
    let reloaded = surface_snapshot_json(app, "alice", &session_id).await;
    assert_eq!(activity_failure_code(&reloaded), "upstream_error");
    let activity = activity_props(&reloaded);
    let serialized = serde_json::to_string(activity).unwrap();
    assert!(!serialized.contains("prompt"));
    assert!(!serialized.contains("message"));
    assert!(!serialized.contains("params"));
    assert!(!serialized.contains("output"));
}

#[tokio::test]
async fn surface_events_use_sse_and_accept_desktop_session_bearer() {
    let _guard = crate::daemon::desktop_session::token_test_lock();
    crate::daemon::desktop_session::clear_desktop_session_tokens_for_test();
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    let (token, _) = crate::daemon::desktop_session::mint_desktop_session_token();
    let uri = format!("/v1/ui/surfaces/session/{session_id}/events?character_id=alice");

    let response = create_router(state)
        .oneshot(
            Request::get(uri)
                .header("authorization", format!("Bearer {token}"))
                .header("last-event-id", "foreign.cursor")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert!(response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("text/event-stream"));
    crate::daemon::desktop_session::clear_desktop_session_tokens_for_test();
}

#[tokio::test]
async fn surface_sse_wire_matches_machine_contract() {
    let contract: serde_json::Value =
        serde_json::from_str(include_str!("../../../../protocol/surface-sse-events.json")).unwrap();
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    let uri = format!("/v1/ui/surfaces/session/{session_id}/events?character_id=alice");
    let app = create_router(state.clone());

    let response = app
        .clone()
        .oneshot(
            Request::get(&uri)
                .header("authorization", "Bearer surface-secret")
                .header(
                    contract["transport"]["resumeRequestHeader"]
                        .as_str()
                        .unwrap(),
                    "foreign.cursor",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        contract["transport"]["responseContentType"]
            .as_str()
            .unwrap()
    );

    let mut stream = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("Surface snapshot event should arrive promptly")
        .expect("Surface SSE stream should not be empty")
        .expect("Surface SSE stream should not fail");
    let first = String::from_utf8(first.to_vec()).unwrap();
    let (cursor, event_name, payload) = parse_surface_sse_frame(&first);
    assert_eq!(event_name, "snapshot");
    assert_eq!(payload["kind"], contract["events"][event_name]["dataKind"]);
    assert_surface_sse_fields(&contract, event_name, &payload);
    drop(stream);

    crate::domain::StateService::new(&state.data_root)
        .write(
            &crate::types::CharacterId::new("alice").unwrap(),
            &serde_json::json!({"mood": "focused"}),
        )
        .unwrap();
    let response = app
        .oneshot(
            Request::get(&uri)
                .header("authorization", "Bearer surface-secret")
                .header(
                    contract["transport"]["resumeRequestHeader"]
                        .as_str()
                        .unwrap(),
                    &cursor,
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut stream = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("Surface patch event should arrive promptly")
        .expect("resumed Surface SSE stream should not be empty")
        .expect("resumed Surface SSE stream should not fail");
    let first = String::from_utf8(first.to_vec()).unwrap();
    let (next_cursor, event_name, payload) = parse_surface_sse_frame(&first);
    assert_eq!(event_name, "patch");
    assert_ne!(next_cursor, cursor);
    assert_eq!(payload["kind"], contract["events"][event_name]["dataKind"]);
    assert_surface_sse_fields(&contract, event_name, &payload);
}

#[tokio::test]
async fn surface_sse_stops_on_daemon_shutdown() {
    let (state, _tmp) = make_state_with_key(Some("surface-secret"));
    let session_id = crate::types::SessionId::new();
    crate::data_dir::create_session_with_id(&state.data_root, "alice", &session_id).unwrap();
    let uri = format!("/v1/ui/surfaces/session/{session_id}/events?character_id=alice");
    let response = create_router(state.clone())
        .oneshot(
            Request::get(uri)
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let mut stream = response.into_body().into_data_stream();
    tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("initial Surface snapshot should arrive")
        .expect("Surface stream should contain a snapshot")
        .expect("Surface snapshot should not fail");

    state.shutdown.send(true).unwrap();
    let ended = tokio::time::timeout(std::time::Duration::from_secs(5), stream.next())
        .await
        .expect("Surface stream should observe daemon shutdown");
    assert!(ended.is_none(), "Surface stream must end during shutdown");
}

fn parse_surface_sse_frame(wire: &str) -> (String, &str, serde_json::Value) {
    let cursor = wire
        .lines()
        .find_map(|line| line.strip_prefix("id:").map(str::trim))
        .expect("Surface SSE event must carry an id cursor")
        .to_string();
    let event_name = wire
        .lines()
        .find_map(|line| line.strip_prefix("event:").map(str::trim))
        .expect("Surface SSE event must carry an event name");
    let data = wire
        .lines()
        .filter_map(|line| line.strip_prefix("data:").map(str::trim))
        .collect::<Vec<_>>()
        .join("\n");
    (cursor, event_name, serde_json::from_str(&data).unwrap())
}

fn assert_surface_sse_fields(
    contract: &serde_json::Value,
    event_name: &str,
    payload: &serde_json::Value,
) {
    assert!(contract["eventNames"]
        .as_array()
        .unwrap()
        .iter()
        .any(|name| name == event_name));
    for field in contract["events"][event_name]["requiredDataFields"]
        .as_array()
        .unwrap()
    {
        assert!(
            payload.get(field.as_str().unwrap()).is_some(),
            "{event_name} SSE data is missing {field}"
        );
    }
}

async fn surface_snapshot_json(
    app: axum::Router,
    character_id: &str,
    session_id: &crate::types::SessionId,
) -> serde_json::Value {
    let response = app
        .oneshot(
            Request::get(surface_uri(character_id, session_id))
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn activity_failure_code(snapshot: &serde_json::Value) -> &str {
    activity_props(snapshot)["recent_failures"]["items"]
        .as_array()
        .and_then(|items| items.last())
        .and_then(|item| item["code"].as_str())
        .expect("Activity widget should include the durable failure receipt")
}

fn activity_props(snapshot: &serde_json::Value) -> &serde_json::Value {
    widget_props(snapshot, "activity")
}

fn widget_props<'a>(snapshot: &'a serde_json::Value, widget_id: &str) -> &'a serde_json::Value {
    snapshot["snapshot"]["blueprint"]["widgets"]
        .as_array()
        .and_then(|widgets| widgets.iter().find(|widget| widget["id"] == widget_id))
        .map(|widget| &widget["props"])
        .expect("Surface snapshot should contain the requested widget")
}

fn chat_context(snapshot: &serde_json::Value) -> &serde_json::Value {
    &widget_props(snapshot, "chat")["context"]
}

fn create_surface_persona(
    state: &std::sync::Arc<crate::daemon::DaemonState>,
    user_id: &crate::types::UserId,
    persona_id: &str,
) {
    crate::domain::PersonaService::new(&state.data_root)
        .save(
            user_id,
            persona_id,
            0,
            crate::domain::Persona {
                schema: crate::domain::Persona::SCHEMA,
                revision: 0,
                updated_at: String::new(),
                name: persona_id.to_string(),
                description: String::new(),
                variables: std::collections::HashMap::new(),
                id: persona_id.to_string(),
                bindings: Vec::new(),
            },
        )
        .unwrap();
}

async fn user_surface_snapshot_json(
    app: axum::Router,
    user_id: &str,
    character_id: &crate::types::CharacterId,
    session_id: &crate::types::SessionId,
) -> serde_json::Value {
    let uri = format!(
        "/v1/ui/surfaces/session/{session_id}?character_id={character_id}&user_id={user_id}"
    );
    let response = app
        .oneshot(
            Request::get(uri)
                .header("authorization", "Bearer surface-secret")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}
