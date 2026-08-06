//! HTTP contract tests for `POST /v1/chat/session-recover` (BUG-2 mitigation
//! slice): quarantine the pending TurnCommit marker, never delete it, and
//! refuse recovery when there is nothing to recover or the session is busy.

use super::*;
use axum::body::Body;
use tower::util::ServiceExt;

fn recover_request_body(character: &str) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "character_id": character })).unwrap()
}

async fn post_session_recover(app: &Router, body: Vec<u8>) -> axum::response::Response {
    app.clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/chat/session-recover")
                .header("Content-Type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn session_recover_quarantines_pending_marker_and_unblocks_the_session() {
    let (state, _tmp) = make_state_no_key();
    let character = crate::types::CharacterId::new("recover-unblock").unwrap();
    let mut commit = crate::turn_commit::TurnCommit::begin(
        &state.data_root,
        &character,
        None,
        "interrupted-unblock".to_string(),
        true,
        true,
        false,
    )
    .unwrap();
    commit.mark_message_committed().unwrap();
    // Simulate a crash: the non-terminal marker survives.
    std::mem::forget(commit);

    let app = create_router(state.clone());
    let response = post_session_recover(&app, recover_request_body("recover-unblock")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["status"], "recovered");
    assert_eq!(value["character_id"], "recover-unblock");
    assert_eq!(value["generation_id"], "interrupted-unblock");
    assert_eq!(value["phase"], "message_committed");
    let quarantined = value["quarantined_marker"].as_str().unwrap();
    assert!(
        quarantined.contains("quarantine") && quarantined.contains("turn_commit.quarantined."),
        "response must point at the quarantine archive, got: {quarantined}"
    );
    // The marker bytes survive in the archive; the live marker path is empty.
    assert!(std::path::Path::new(quarantined).exists());
    let archived = std::fs::read(quarantined).unwrap();
    let marker: serde_json::Value = serde_json::from_slice(&archived).unwrap();
    assert_eq!(marker["generation_id"], "interrupted-unblock");
    assert!(crate::turn_commit::pending_turn(&state.data_root, &character, None).is_none());

    // The Coordinator reports idle again and admits the next mutation.
    let status = state
        .session_coordinators
        .status(&state.data_root, &character, None);
    assert_eq!(status.phase, crate::session_coordinator::SessionPhase::Idle);
    let lease = state
        .session_coordinators
        .try_submit(
            &state.data_root,
            &character,
            None,
            crate::session_coordinator::SessionCommand::Completion,
        )
        .expect("recovered session must accept a new commit");
    drop(lease);
}

#[tokio::test]
async fn session_recover_rejects_a_clean_session() {
    let (state, _tmp) = make_state_no_key();
    let app = create_router(state.clone());
    let response = post_session_recover(&app, recover_request_body("recover-clean")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(
        text.contains("no pending turn commit marker"),
        "expected an explicit no-marker error, got: {text}"
    );
    // Nothing may have been archived for a clean session.
    assert!(!state.data_root.join("quarantine").exists());
}

#[tokio::test]
async fn session_recover_rejects_a_busy_session() {
    let (state, _tmp) = make_state_no_key();
    let character = crate::types::CharacterId::new("recover-busy").unwrap();
    let _lease = state
        .session_coordinators
        .try_submit(
            &state.data_root,
            &character,
            None,
            crate::session_coordinator::SessionCommand::Completion,
        )
        .unwrap();

    let app = create_router(state);
    let response = post_session_recover(&app, recover_request_body("recover-busy")).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(text.contains("session_busy"), "got: {text}");
}

#[tokio::test]
async fn session_recover_quarantines_an_unreadable_marker() {
    let (state, _tmp) = make_state_no_key();
    let character = crate::types::CharacterId::new("recover-corrupt").unwrap();
    let history_dir = state
        .data_root
        .join("characters")
        .join("recover-corrupt")
        .join("history");
    std::fs::create_dir_all(&history_dir).unwrap();
    std::fs::write(history_dir.join("turn_commit.json"), b"{torn").unwrap();

    let app = create_router(state.clone());
    let response = post_session_recover(&app, recover_request_body("recover-corrupt")).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["status"], "recovered");
    // Unreadable markers carry no generation id; the bytes are still kept.
    assert_eq!(value["generation_id"], "");
    let archived = std::fs::read(value["quarantined_marker"].as_str().unwrap()).unwrap();
    assert_eq!(archived, b"{torn");
    assert!(crate::turn_commit::pending_turn(&state.data_root, &character, None).is_none());
}

#[tokio::test]
async fn session_recover_respects_the_user_scope_root() {
    let (state, _tmp) = make_state_no_key();
    let character = crate::types::CharacterId::new("recover-user-scope").unwrap();
    let user_root =
        crate::data_dir::resolve_effective_root(&state.data_root, Some("alice")).unwrap();
    let commit = crate::turn_commit::TurnCommit::begin(
        &user_root,
        &character,
        None,
        "interrupted-user-scope".to_string(),
        true,
        false,
        false,
    )
    .unwrap();
    std::mem::forget(commit);

    let app = create_router(state.clone());
    // Daemon root has no marker: recovering without the user scope must fail.
    let response = post_session_recover(&app, recover_request_body("recover-user-scope")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let scoped_body = serde_json::to_vec(&serde_json::json!({
        "character_id": "recover-user-scope",
        "user_id": "alice",
    }))
    .unwrap();
    let response = post_session_recover(&app, scoped_body).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(crate::turn_commit::pending_turn(&user_root, &character, None).is_none());
}
