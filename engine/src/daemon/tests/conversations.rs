use super::*;
use async_trait::async_trait;
use wiremock::matchers::method;
use wiremock::{Mock, MockServer, ResponseTemplate};

struct ParallelTestPolicy;

#[async_trait]
impl crate::conversation_policy::ConversationPolicy for ParallelTestPolicy {
    fn descriptor(&self) -> crate::conversation_policy::ConversationPolicyDescriptor {
        crate::conversation_policy::ConversationPolicyDescriptor {
            schema_version:
                crate::conversation_policy::CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION,
            policy_id: "test.parallel.v1".to_string(),
            policy_version: "1.0.0".to_string(),
            description: "Test-only parallel policy".to_string(),
            provenance: crate::conversation_policy::ConversationPolicyProvenance {
                source: crate::conversation_policy::ConversationPolicySource::External,
                provider: "AIRP daemon test".to_string(),
                implementation: "daemon::tests::conversations::ParallelTestPolicy".to_string(),
            },
            capabilities: crate::conversation_policy::ConversationPolicyCapabilities {
                execution_modes: vec![
                    crate::conversation_policy::ConversationExecutionMode::Parallel,
                ],
                supports_speaker_limit: false,
            },
            resource_limits: crate::conversation_policy::ConversationPolicyResourceLimits {
                max_speakers_per_turn: 4,
                max_parallelism: 2,
                max_config_bytes: 1024,
                planning_timeout_ms: 100,
            },
            lifecycle_state: crate::conversation_policy::ConversationPolicyLifecycleState::Active,
            config_schema: serde_json::json!({
                "type": "object",
                "maxProperties": 0,
                "additionalProperties": false
            }),
        }
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), crate::error::AirpError> {
        if !config.as_object().is_some_and(serde_json::Map::is_empty) {
            return Err(crate::error::AirpError::BadRequest(
                "parallel test policy requires empty config".to_string(),
            ));
        }
        Ok(())
    }

    async fn plan_turn(
        &self,
        manifest: &crate::conversation::ConversationManifest,
        _policy: &crate::conversation::ConversationPolicyRef,
        _user_actor_id: &str,
    ) -> Result<crate::conversation_policy::ConversationTurnPlan, crate::error::AirpError> {
        let speakers = manifest
            .participants
            .iter()
            .filter_map(|participant| {
                let resource = participant.resource.as_ref()?;
                (participant.kind == "character").then(|| {
                    crate::conversation_policy::ConversationSpeaker {
                        participant_id: participant.participant_id.clone(),
                        resource_id: resource.id.clone(),
                    }
                })
            })
            .collect();
        Ok(crate::conversation_policy::ConversationTurnPlan {
            scene_id: "tavern".to_string(),
            speakers,
            execution_mode: crate::conversation_policy::ConversationExecutionMode::Parallel,
            stop_after_speakers: None,
        })
    }
}

fn create_body() -> serde_json::Value {
    serde_json::json!({
        "title": "Extensible RP",
        "participants": [
            {"participant_id": "user", "kind": "user"},
            {
                "participant_id": "alice",
                "kind": "character",
                "resource": {"kind": "character", "id": "alice", "revision": "7"}
            }
        ],
        "resources": [{"kind": "scene", "id": "tavern"}],
        "orchestration": {
            "policy_id": "airp.round_robin.v1",
            "config": {"include_user": false}
        },
        "extensions": {"example.future": {"enabled": true}}
    })
}

async fn create_conversation(app: &Router, body: serde_json::Value) -> String {
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/conversations")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    manifest["conversation_id"].as_str().unwrap().to_string()
}

fn write_character_card(data_root: &std::path::Path, character_id: &str) {
    let directory = data_root.join("characters").join(character_id);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        directory.join("card.json"),
        format!(
            r#"{{"name":"{character_id}","description":"","personality":"","scenario":"","first_mes":"","mes_example":""}}"#
        ),
    )
    .unwrap();
}

async fn create_solo_scene_conversation(app: &Router) -> String {
    let scene = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "scene_id": "solo",
                        "characters": [{"character_id": "alice", "role": "primary"}]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(scene.status(), StatusCode::CREATED);
    let conversation = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes/solo/conversations")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "additional_participants": [
                            {"participant_id": "human:gm", "kind": "human"}
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conversation.status(), StatusCode::OK);
    let body = axum::body::to_bytes(conversation.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    manifest["conversation_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn create_list_and_get_preserve_open_contract() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let conversation_id = create_conversation(&app, create_body()).await;

    let get = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversations/{conversation_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest["schema_version"], 1);
    assert_eq!(manifest["extensions"]["example.future"]["enabled"], true);
    assert_eq!(
        manifest["orchestration"]["policy_id"],
        "airp.round_robin.v1"
    );

    let list = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/conversations")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let body = axum::body::to_bytes(list.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifests: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0]["conversation_id"], conversation_id);
}

#[tokio::test]
async fn policy_catalog_exposes_versioned_config_contract() {
    let (state, _tmp) = make_state_with_key(None);
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/conversation-policies")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let policies: Vec<serde_json::Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(policies.len(), 1);
    assert_eq!(
        policies[0]["schema_version"],
        crate::conversation_policy::CONVERSATION_POLICY_DESCRIPTOR_SCHEMA_VERSION
    );
    assert_eq!(
        policies[0]["policy_id"],
        crate::conversation_policy::SCENE_ROUND_ROBIN_V1
    );
    assert_eq!(
        policies[0]["config_schema"]["oneOf"][0]["additionalProperties"],
        false
    );
    assert_eq!(policies[0]["policy_version"], "1.0.0");
    assert_eq!(policies[0]["provenance"]["source"], "built_in");
    assert_eq!(policies[0]["lifecycle_state"], "active");
}

#[tokio::test]
async fn capability_discovery_exposes_global_limits_versions_and_redaction_contract() {
    let (state, _tmp) = make_state_with_key(None);
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/conversation-capabilities")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 32 * 1024)
        .await
        .unwrap();
    let capabilities: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(capabilities["schema_version"], 1);
    assert_eq!(capabilities["contract_version"], "airp.conversation.v1");
    assert_eq!(capabilities["schemas"]["http_error"], 1);
    assert_eq!(capabilities["schemas"]["migration_report"], 1);
    assert_eq!(
        capabilities["adapter_versions"]["legacy_migration"],
        crate::conversation_compat::CONVERSATION_COMPAT_ADAPTER_VERSION
    );
    assert_eq!(
        capabilities["execution_limits"]["max_speakers_per_turn"],
        crate::conversation_policy::MAX_CONVERSATION_SPEAKERS_PER_TURN
    );
    assert_eq!(
        capabilities["execution_limits"]["turn_timeout_secs"],
        crate::conversation_observability::CONVERSATION_TURN_TIMEOUT_SECS
    );
    assert!(capabilities["observability"]["excluded_data"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("provider_error_body")));
    assert!(capabilities["turn_errors"]
        .as_array()
        .unwrap()
        .iter()
        .any(|error| {
            error["code"] == "unknown_commit" && error["recovery"] == "manual_retry_or_continue"
        }));
}

#[tokio::test]
async fn append_and_cursor_history_enforce_sequence() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let conversation_id = create_conversation(&app, create_body()).await;

    let mut event_ids = Vec::new();
    for sequence in 0..3 {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/v1/conversations/{conversation_id}/events"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": "message.created",
                            "actor_id": if sequence % 2 == 0 { "user" } else { "alice" },
                            "payload": {"content": format!("message-{sequence}")},
                            "expected_next_sequence": sequence
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
        let event: serde_json::Value = serde_json::from_slice(&body).unwrap();
        event_ids.push(event["event_id"].as_str().unwrap().to_string());
    }

    let page = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/events?limit=1&before={}",
                    event_ids[2]
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let body = axum::body::to_bytes(page.into_body(), 16 * 1024)
        .await
        .unwrap();
    let page: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(page["events"][0]["sequence"], 1);
    assert_eq!(page["has_more"], true);
    assert_eq!(page["total"], 3);
    assert_eq!(page["next_sequence"], 3);

    let conflict = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/events"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "kind": "message.created",
                        "actor_id": "user",
                        "expected_next_sequence": 2
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn per_user_conversation_roots_are_not_cross_visible() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let mut body = create_body();
    body["user_id"] = serde_json::json!("alice");
    let conversation_id = create_conversation(&app, body).await;

    let global = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversations/{conversation_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(global.status(), StatusCode::NOT_FOUND);

    let alice = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversations/{conversation_id}?user_id=alice"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(alice.status(), StatusCode::OK);
}

#[tokio::test]
async fn scene_adapter_snapshots_scene_participants_into_generic_conversation() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let create_scene = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "scene_id": "tavern",
                        "description": "Night shift",
                        "characters": [
                            {"character_id": "alice", "role": "primary", "intro": "Innkeeper"},
                            {"character_id": "bob", "role": "npc", "intro": "Traveler"}
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_scene.status(), StatusCode::CREATED);

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes/tavern/conversations")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "additional_participants": [
                            {"participant_id": "human:gm", "kind": "human", "display_name": "GM"}
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest["resources"][0]["kind"], "scene");
    assert_eq!(manifest["resources"][0]["id"], "tavern");
    assert_eq!(
        manifest["participants"][0]["participant_id"],
        "character:alice"
    );
    assert_eq!(
        manifest["participants"][1]["participant_id"],
        "character:bob"
    );
    assert_eq!(manifest["participants"][2]["participant_id"], "human:gm");
    assert_eq!(
        manifest["orchestration"]["policy_id"],
        "airp.scene.round_robin.v1"
    );
}

#[tokio::test]
async fn scene_conversation_creation_records_active_conversation_id_on_scene_manifest() {
    // #343: POST /v1/scenes/:scene_id/conversations 创建 Conversation 后，
    // scene manifest 必须记录 active_conversation_id，供 WebUI 刷新恢复。
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state.clone());

    // 1. 创建 scene
    let create_scene = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "scene_id": "tavern",
                        "description": "Night shift",
                        "characters": [
                            {"character_id": "alice", "role": "primary"},
                            {"character_id": "bob", "role": "npc"}
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create_scene.status(), StatusCode::CREATED);

    // 2. 创建 Conversation
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes/tavern/conversations")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let conversation_id = manifest["conversation_id"].as_str().unwrap().to_string();

    // 3. GET /v1/scenes/tavern 必须返回 active_conversation_id
    let get_scene = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/scenes/tavern")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(get_scene.status(), StatusCode::OK);
    let body = axum::body::to_bytes(get_scene.into_body(), 16 * 1024)
        .await
        .unwrap();
    let scene_json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(scene_json["active_conversation_id"], conversation_id);

    // 4. 模拟 Engine 重启：从磁盘重新 load SceneConfig
    let reloaded = crate::scene::SceneConfig::load(
        &state.data_root,
        &crate::types::SceneId::new("tavern").unwrap(),
    )
    .unwrap();
    assert_eq!(
        reloaded.active_conversation_id.map(|id| id.to_string()),
        Some(conversation_id.clone())
    );

    // 5. 创建第二个 Conversation，active_conversation_id 必须更新为新值
    let response2 = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes/tavern/conversations")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::json!({}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response2.status(), StatusCode::OK);
    let body2 = axum::body::to_bytes(response2.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifest2: serde_json::Value = serde_json::from_slice(&body2).unwrap();
    let conversation_id2 = manifest2["conversation_id"].as_str().unwrap().to_string();
    assert_ne!(
        conversation_id2, conversation_id,
        "new conversation must have different id"
    );

    let get_scene2 = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/scenes/tavern")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(get_scene2.into_body(), 16 * 1024)
        .await
        .unwrap();
    let scene_json2: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(scene_json2["active_conversation_id"], conversation_id2);
}

#[tokio::test]
async fn engine_executes_scene_turn_with_ordered_attributed_messages() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n\
                     data: [DONE]\n\n",
                    "text/event-stream",
                ),
        )
        .expect(2)
        .mount(&server)
        .await;

    let (state, _tmp) = make_state_with_key(None);
    state.config.write().unwrap().endpoint = server.uri();
    for character_id in ["alice", "bob"] {
        let directory = state.data_root.join("characters").join(character_id);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("card.json"),
            format!(
                r#"{{"name":"{character_id}","description":"","personality":"","scenario":"","first_mes":"","mes_example":""}}"#
            ),
        )
        .unwrap();
    }
    let app = create_router(state);
    let scene = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "scene_id": "tavern",
                        "characters": [
                            {"character_id": "alice", "role": "primary"},
                            {"character_id": "bob", "role": "npc"}
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(scene.status(), StatusCode::CREATED);
    let conversation = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes/tavern/conversations")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "additional_participants": [
                            {"participant_id": "human:gm", "kind": "human"}
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(conversation.into_body(), 16 * 1024)
        .await
        .unwrap();
    let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let conversation_id = manifest["conversation_id"].as_str().unwrap();

    let turn_id = crate::ulid::new_id();
    let turn_body = serde_json::json!({
        "turn_id": turn_id.clone(),
        "user_actor_id": "human:gm",
        "expected_next_sequence": 0,
        "base": {
            "user_profile": {"name": "GM", "variables": {}},
            "message": "Welcome"
        }
    });
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(turn_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(outcome["status"], "completed");
    assert_eq!(outcome["lifecycle_state"], "completed");
    assert_eq!(outcome["events"][0]["kind"], "turn.accepted");
    assert_eq!(outcome["events"][1]["kind"], "turn.started");
    assert_eq!(outcome["events"][2]["actor_id"], "human:gm");
    assert_eq!(outcome["events"][3]["actor_id"], "character:alice");
    assert_eq!(outcome["events"][4]["actor_id"], "character:bob");
    assert_eq!(outcome["events"][5]["kind"], "turn.completed");
    assert_eq!(outcome["events"][5]["payload"]["message_count"], 3);
    assert_eq!(outcome["next_sequence"], 6);

    let mut replay_body = turn_body.clone();
    replay_body["base"]["endpoint"] = serde_json::json!("https://rotated.invalid/v1");
    replay_body["base"]["api_key"] = serde_json::json!("rotated-secret");
    let replay = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(replay_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(replay.status(), StatusCode::OK);
    let replay = axum::body::to_bytes(replay.into_body(), 64 * 1024)
        .await
        .unwrap();
    let replay: serde_json::Value = serde_json::from_slice(&replay).unwrap();
    assert_eq!(
        replay, outcome,
        "credentials and transport location must not change semantic idempotency"
    );

    let mut conflicting_body = turn_body.clone();
    conflicting_body["base"]["message"] = serde_json::json!("Different");
    let conflict = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(conflicting_body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);

    let status = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status = axum::body::to_bytes(status.into_body(), 64 * 1024)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status).unwrap();
    assert_eq!(status["lifecycle_state"], "completed");
    assert_eq!(status["events"], outcome["events"]);

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let second: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    let messages = second["messages"].as_array().unwrap();
    assert_eq!(
        messages
            .iter()
            .find(|message| message["role"] == "user")
            .unwrap()["content"],
        "Welcome"
    );
    assert_eq!(
        messages
            .iter()
            .find(|message| message["role"] == "assistant")
            .unwrap()["content"],
        "[character:alice] Hello"
    );

    let observability = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}/observability"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(observability.status(), StatusCode::OK);
    let observability = axum::body::to_bytes(observability.into_body(), 32 * 1024)
        .await
        .unwrap();
    let observation: serde_json::Value = serde_json::from_slice(&observability).unwrap();
    assert_eq!(
        observation["policy"]["policy_id"],
        crate::conversation_policy::SCENE_ROUND_ROBIN_V1
    );
    assert_eq!(observation["quota_reserved_requests"], 2);
    assert_eq!(observation["speakers"][0]["outcome"], "committed");
    assert_eq!(observation["speakers"][1]["outcome"], "committed");
    assert!(observation["recorded_output_tokens"].as_u64().unwrap() > 0);
    assert!(observation["latency_ms"].is_number());
    let observation_text = String::from_utf8_lossy(&observability);
    assert!(!observation_text.contains("Welcome"));
    assert!(!observation_text.contains("rotated-secret"));

    let journal = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/events?limit=10"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(journal.into_body(), 64 * 1024)
        .await
        .unwrap();
    let journal: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(journal["events"], outcome["events"]);
}

#[tokio::test]
async fn injected_parallel_policy_generates_from_one_snapshot_and_commits_in_plan_order() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"Parallel\"}}]}\n\n\
                     data: [DONE]\n\n",
                    "text/event-stream",
                ),
        )
        .expect(2)
        .mount(&server)
        .await;

    let (state, _tmp) = make_state_with_key(None);
    state.config.write().unwrap().endpoint = server.uri();
    for character_id in ["alice", "bob"] {
        write_character_card(&state.data_root, character_id);
    }
    let registry = crate::conversation_policy::ConversationPolicyRegistry::with_builtins();
    let app = create_router_with_conversation_policy_registry(state, registry.clone());
    registry
        .register_external(Arc::new(ParallelTestPolicy))
        .unwrap();
    let scene = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/scenes")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "scene_id": "tavern",
                        "characters": [
                            {"character_id": "alice", "role": "primary"},
                            {"character_id": "bob", "role": "npc"}
                        ]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(scene.status(), StatusCode::CREATED);
    let conversation_id = create_conversation(
        &app,
        serde_json::json!({
            "participants": [
                {"participant_id": "human:gm", "kind": "human"},
                {
                    "participant_id": "character:alice",
                    "kind": "character",
                    "resource": {"kind": "character", "id": "alice"}
                },
                {
                    "participant_id": "character:bob",
                    "kind": "character",
                    "resource": {"kind": "character", "id": "bob"}
                }
            ],
            "resources": [{"kind": "scene", "id": "tavern"}],
            "orchestration": {"policy_id": "test.parallel.v1", "config": {}}
        }),
    )
    .await;

    let response = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_actor_id": "human:gm",
                        "expected_next_sequence": 0,
                        "base": {
                            "user_profile": {"name": "GM", "variables": {}},
                            "message": "Start"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(outcome["status"], "completed");
    assert_eq!(
        outcome["events"][0]["payload"]["policy"]["policy_id"],
        "test.parallel.v1"
    );
    assert_eq!(
        outcome["events"][0]["payload"]["policy"]["execution_mode"],
        "parallel"
    );
    assert_eq!(
        outcome["events"][0]["payload"]["policy"]["max_parallelism"],
        2
    );
    assert_eq!(
        outcome["events"][0]["payload"]["policy"]["speakers"][0]["participant_id"],
        "character:alice"
    );
    assert_eq!(
        outcome["events"][0]["payload"]["policy"]["speakers"][1]["participant_id"],
        "character:bob"
    );
    assert_eq!(outcome["events"][3]["actor_id"], "character:alice");
    assert_eq!(outcome["events"][4]["actor_id"], "character:bob");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        let request: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert!(
            request["messages"]
                .as_array()
                .unwrap()
                .iter()
                .all(|message| message["role"] != "assistant"),
            "parallel speakers must generate from the same pre-batch history snapshot"
        );
    }
}

#[tokio::test]
async fn turn_rejects_client_owned_history_before_committing() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let conversation_id = create_conversation(&app, create_body()).await;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_actor_id": "user",
                        "expected_next_sequence": 0,
                        "base": {
                            "user_profile": {"name": "User", "variables": {}},
                            "message": "hi",
                            "messages_history": []
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let journal = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversations/{conversation_id}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(journal.into_body(), 4096)
        .await
        .unwrap();
    let journal: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(journal["total"], 0);
}

#[tokio::test]
async fn turn_rejects_unregistered_policy_before_committing() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let conversation_id = create_conversation(&app, create_body()).await;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_actor_id": "user",
                        "expected_next_sequence": 0,
                        "base": {
                            "user_profile": {"name": "User", "variables": {}},
                            "message": "hi"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let journal = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversations/{conversation_id}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(journal.into_body(), 4096)
        .await
        .unwrap();
    let journal: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(journal["total"], 0);
}

#[tokio::test]
async fn turn_rejects_an_oversized_speaker_plan_before_committing() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let mut participants = vec![serde_json::json!({
        "participant_id": "human:gm",
        "kind": "human"
    })];
    participants.extend(
        (0..=crate::conversation_policy::MAX_CONVERSATION_SPEAKERS_PER_TURN).map(|index| {
            let character_id = format!("character-{index}");
            serde_json::json!({
                "participant_id": format!("character:{character_id}"),
                "kind": "character",
                "resource": {"kind": "character", "id": character_id}
            })
        }),
    );
    let conversation_id = create_conversation(
        &app,
        serde_json::json!({
            "participants": participants,
            "resources": [{"kind": "scene", "id": "oversized"}],
            "orchestration": {
                "policy_id": crate::conversation_policy::SCENE_ROUND_ROBIN_V1,
                "config": {}
            }
        }),
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_actor_id": "human:gm",
                        "expected_next_sequence": 0,
                        "base": {
                            "user_profile": {"name": "GM", "variables": {}},
                            "message": "hi"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let journal = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversations/{conversation_id}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(journal.into_body(), 4096)
        .await
        .unwrap();
    let journal: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(journal["total"], 0);
}

#[tokio::test]
async fn turn_reserves_request_quota_for_every_planned_provider_call() {
    let (state, _tmp) = make_state_with_key(None);
    state.config.write().unwrap().quota.max_requests_per_day = 1;
    let data_root = state.data_root.clone();
    let app = create_router(state);
    let conversation_id = create_conversation(
        &app,
        serde_json::json!({
            "participants": [
                {"participant_id": "human:gm", "kind": "human"},
                {
                    "participant_id": "character:alice",
                    "kind": "character",
                    "resource": {"kind": "character", "id": "alice"}
                },
                {
                    "participant_id": "character:bob",
                    "kind": "character",
                    "resource": {"kind": "character", "id": "bob"}
                }
            ],
            "resources": [{"kind": "scene", "id": "quota-scene"}],
            "orchestration": {
                "policy_id": crate::conversation_policy::SCENE_ROUND_ROBIN_V1,
                "config": {}
            }
        }),
    )
    .await;

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_actor_id": "human:gm",
                        "expected_next_sequence": 0,
                        "base": {
                            "user_profile": {"name": "GM", "variables": {}},
                            "message": "hi"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let quota = crate::quota::QuotaState::load(&crate::quota::quota_file_path(&data_root));
    assert_eq!(quota.requests_today, 0);

    let journal = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversations/{conversation_id}/events"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(journal.into_body(), 4096)
        .await
        .unwrap();
    let journal: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(journal["total"], 0);
}

#[tokio::test]
async fn provider_failure_is_reported_as_partially_committed_turn() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_string("private upstream detail"))
        .expect(1)
        .mount(&server)
        .await;
    let (state, _tmp) = make_state_with_key(None);
    state.config.write().unwrap().endpoint = server.uri();
    write_character_card(&state.data_root, "alice");
    let app = create_router(state);
    let conversation_id = create_solo_scene_conversation(&app).await;
    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "user_actor_id": "human:gm",
                        "expected_next_sequence": 0,
                        "base": {
                            "user_profile": {"name": "GM", "variables": {}},
                            "message": "Hello"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 32 * 1024)
        .await
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(outcome["status"], "partially_committed");
    assert_eq!(outcome["lifecycle_state"], "failed");
    assert_eq!(outcome["failure"]["schema_version"], 1);
    assert_eq!(outcome["failure"]["code"], "generation_failed");
    assert_eq!(outcome["failure"]["recovery"], "inspect_and_continue");
    assert_eq!(outcome["events"][0]["kind"], "turn.accepted");
    assert_eq!(outcome["events"][1]["kind"], "turn.started");
    assert_eq!(outcome["events"][2]["kind"], "message.created");
    assert_eq!(outcome["events"][3]["kind"], "turn.failed");
    assert_eq!(
        outcome["events"][3]["payload"]["commit_state"],
        "partially_committed"
    );
    assert!(
        !String::from_utf8_lossy(&body).contains("private upstream detail"),
        "upstream details must not cross the Engine API boundary"
    );

    let turn_id = outcome["turn_id"].as_str().unwrap();
    let observation = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}/observability"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(observation.status(), StatusCode::OK);
    let body = axum::body::to_bytes(observation.into_body(), 32 * 1024)
        .await
        .unwrap();
    let observation: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(observation["schema_version"], 1);
    assert_eq!(observation["trace_id"], turn_id);
    assert_eq!(observation["lifecycle_state"], "failed");
    assert_eq!(observation["quota_reserved_requests"], 1);
    assert_eq!(observation["partial_commit"], true);
    assert_eq!(
        observation["speakers"][0]["participant_id"],
        "character:alice"
    );
    assert_eq!(observation["speakers"][0]["outcome"], "failed");
    assert!(
        !String::from_utf8_lossy(&body).contains("private upstream detail"),
        "observability must not copy provider-private details"
    );
}

#[tokio::test]
async fn dangling_turn_is_reconciled_to_unknown_commit_without_deleting_events() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let conversation_id = create_conversation(&app, create_body()).await;
    let turn_id = crate::ulid::new_id();

    for (sequence, kind) in [(0, "turn.accepted"), (1, "turn.started")] {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!("/v1/conversations/{conversation_id}/events"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "kind": kind,
                            "correlation_id": turn_id.clone(),
                            "payload": if sequence == 0 {
                                serde_json::json!({"request_fingerprint": "stale"})
                            } else {
                                serde_json::json!({})
                            },
                            "expected_next_sequence": sequence
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let response = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let snapshot: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(snapshot["lifecycle_state"], "unknown_commit");
    assert_eq!(snapshot["events"].as_array().unwrap().len(), 3);
    assert_eq!(snapshot["events"][2]["kind"], "turn.unknown_commit");
    assert_eq!(snapshot["next_sequence"], 3);

    let second = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let second = axum::body::to_bytes(second.into_body(), 16 * 1024)
        .await
        .unwrap();
    let second: serde_json::Value = serde_json::from_slice(&second).unwrap();
    assert_eq!(second, snapshot, "recovery must itself be idempotent");
}

#[tokio::test]
async fn legacy_accepted_turn_without_fingerprint_is_not_reexecuted() {
    let (state, _tmp) = make_state_with_key(None);
    write_character_card(&state.data_root, "alice");
    let app = create_router(state);
    let conversation_id = create_solo_scene_conversation(&app).await;
    let turn_id = crate::ulid::new_id();
    let accepted = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/events"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "kind": "turn.accepted",
                        "correlation_id": turn_id.clone(),
                        "payload": {},
                        "expected_next_sequence": 0
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.status(), StatusCode::OK);

    let retry = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!("/v1/conversations/{conversation_id}/turns"))
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "turn_id": turn_id,
                        "user_actor_id": "human:gm",
                        "expected_next_sequence": 1,
                        "base": {
                            "user_profile": {"name": "GM", "variables": {}},
                            "message": "Do not execute"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retry.status(), StatusCode::CONFLICT);
}

#[tokio::test]
async fn turn_endpoints_reject_malformed_durable_ids() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
    let conversation_id = create_conversation(&app, create_body()).await;
    let cases = [
        (
            "POST",
            format!("/v1/conversations/{conversation_id}/turns"),
            Body::from(
                serde_json::json!({
                    "turn_id": "not-a-durable-id",
                    "user_actor_id": "user",
                    "expected_next_sequence": 0,
                    "base": {
                        "user_profile": {"name": "User", "variables": {}},
                        "message": "hi"
                    }
                })
                .to_string(),
            ),
        ),
        (
            "GET",
            format!("/v1/conversations/{conversation_id}/turns/not-a-durable-id"),
            Body::empty(),
        ),
        (
            "POST",
            format!("/v1/conversations/{conversation_id}/turns/not-a-durable-id/cancel"),
            Body::empty(),
        ),
    ];

    for (method, uri, body) in cases {
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn explicit_cancel_interrupts_generation_and_commits_cancelled_state() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_delay(std::time::Duration::from_secs(5))
                .set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n\
                     data: [DONE]\n\n",
                    "text/event-stream",
                ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let (state, _tmp) = make_state_with_key(None);
    state.config.write().unwrap().endpoint = server.uri();
    write_character_card(&state.data_root, "alice");
    let app = create_router(state);
    let conversation_id = create_solo_scene_conversation(&app).await;
    let turn_id = crate::ulid::new_id();
    let turn_app = app.clone();
    let turn_uri = format!("/v1/conversations/{conversation_id}/turns");
    let turn_body = serde_json::json!({
        "turn_id": turn_id.clone(),
        "user_actor_id": "human:gm",
        "expected_next_sequence": 0,
        "base": {
            "user_profile": {"name": "GM", "variables": {}},
            "message": "Stop this"
        }
    });
    let turn_task = tokio::spawn(async move {
        turn_app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(turn_uri)
                    .header("content-type", "application/json")
                    .body(Body::from(turn_body.to_string()))
                    .unwrap(),
            )
            .await
    });

    let mut provider_started = false;
    for _ in 0..500 {
        if !server.received_requests().await.unwrap().is_empty() {
            provider_started = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        provider_started,
        "provider request did not start within the 5 second test deadline"
    );

    let running = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(running.status(), StatusCode::OK);
    let running = axum::body::to_bytes(running.into_body(), 32 * 1024)
        .await
        .unwrap();
    let running: serde_json::Value = serde_json::from_slice(&running).unwrap();
    assert_eq!(running["lifecycle_state"], "running");

    let cancel = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}/cancel"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(cancel.status(), StatusCode::OK);
    let cancel = axum::body::to_bytes(cancel.into_body(), 4096)
        .await
        .unwrap();
    let cancel: serde_json::Value = serde_json::from_slice(&cancel).unwrap();
    assert_eq!(cancel["cancel_requested"], true);

    let response = turn_task.await.unwrap().unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 32 * 1024)
        .await
        .unwrap();
    let outcome: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(outcome["status"], "partially_committed");
    assert_eq!(outcome["lifecycle_state"], "cancelled");
    assert_eq!(outcome["failure"]["recovery"], "resubmit_new_turn");
    assert_eq!(
        outcome["events"].as_array().unwrap().last().unwrap()["kind"],
        "turn.cancelled"
    );

    let status = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = axum::body::to_bytes(status.into_body(), 32 * 1024)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status).unwrap();
    assert_eq!(status["lifecycle_state"], "cancelled");

    let observation = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}/observability"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let observation = axum::body::to_bytes(observation.into_body(), 32 * 1024)
        .await
        .unwrap();
    let observation: serde_json::Value = serde_json::from_slice(&observation).unwrap();
    assert_eq!(observation["cancelled"], true);
    assert_eq!(observation["failure"]["recovery"], "resubmit_new_turn");
}

#[tokio::test]
async fn client_disconnect_preserves_partial_journal_for_unknown_commit_recovery() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_delay(std::time::Duration::from_secs(5))
                .set_body_raw(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"late\"}}]}\n\n\
                     data: [DONE]\n\n",
                    "text/event-stream",
                ),
        )
        .expect(1)
        .mount(&server)
        .await;

    let (state, _tmp) = make_state_with_key(None);
    state.config.write().unwrap().endpoint = server.uri();
    write_character_card(&state.data_root, "alice");
    let app = create_router(state);
    let conversation_id = create_solo_scene_conversation(&app).await;
    let turn_id = crate::ulid::new_id();
    let turn_app = app.clone();
    let turn_uri = format!("/v1/conversations/{conversation_id}/turns");
    let turn_body = serde_json::json!({
        "turn_id": turn_id.clone(),
        "user_actor_id": "human:gm",
        "expected_next_sequence": 0,
        "base": {
            "user_profile": {"name": "GM", "variables": {}},
            "message": "Keep the durable part"
        }
    });
    let turn_task = tokio::spawn(async move {
        turn_app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(turn_uri)
                    .header("content-type", "application/json")
                    .body(Body::from(turn_body.to_string()))
                    .unwrap(),
            )
            .await
    });

    let mut provider_started = false;
    for _ in 0..500 {
        if !server.received_requests().await.unwrap().is_empty() {
            provider_started = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        provider_started,
        "provider request did not start within the 5 second test deadline"
    );
    turn_task.abort();
    assert!(turn_task.await.unwrap_err().is_cancelled());

    let status = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/turns/{turn_id}"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(status.status(), StatusCode::OK);
    let status = axum::body::to_bytes(status.into_body(), 32 * 1024)
        .await
        .unwrap();
    let status: serde_json::Value = serde_json::from_slice(&status).unwrap();
    assert_eq!(status["lifecycle_state"], "unknown_commit");
    let kinds: Vec<_> = status["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "turn.accepted",
            "turn.started",
            "message.created",
            "turn.unknown_commit"
        ]
    );
    assert_eq!(
        status["events"][2]["payload"]["content"],
        "Keep the durable part"
    );

    let journal = app
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/v1/conversations/{conversation_id}/events?limit=10"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let journal = axum::body::to_bytes(journal.into_body(), 32 * 1024)
        .await
        .unwrap();
    let journal: serde_json::Value = serde_json::from_slice(&journal).unwrap();
    assert_eq!(journal["events"], status["events"]);
}

#[tokio::test]
async fn compatibility_migration_keeps_legacy_http_shape_and_supports_export_rollback() {
    let (state, _tmp) = make_state_with_key(None);
    let root = state.data_root.clone();
    let character = crate::types::CharacterId::new("alice").unwrap();
    let mut log = crate::chat_store::ChatLog::new(character.as_str());
    log.append(
        &root,
        crate::adapter::ChatMessage {
            role: crate::adapter::MessageRole::User,
            content: "hello".to_string(),
        },
    )
    .unwrap();
    log.append(
        &root,
        crate::adapter::ChatMessage {
            role: crate::adapter::MessageRole::Assistant,
            content: "hi".to_string(),
        },
    )
    .unwrap();
    let app = create_router(state);
    let history_request = || {
        axum::http::Request::builder()
            .method("POST")
            .uri("/v1/chat/history")
            .header("content-type", "application/json")
            .body(Body::from(r#"{"character_id":"alice"}"#))
            .unwrap()
    };
    let before = app.clone().oneshot(history_request()).await.unwrap();
    assert_eq!(before.status(), StatusCode::OK);
    let before = axum::body::to_bytes(before.into_body(), 32 * 1024)
        .await
        .unwrap();

    let source = serde_json::json!({
        "kind": "character_chat",
        "character_id": "alice"
    });
    let plan = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/conversation-migrations/plan")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({"source": source}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(plan.status(), StatusCode::OK);
    let plan = axum::body::to_bytes(plan.into_body(), 32 * 1024)
        .await
        .unwrap();
    let plan: serde_json::Value = serde_json::from_slice(&plan).unwrap();
    assert_eq!(plan["status"], "ready");
    let migration_id = crate::ulid::new_id();
    let execute = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/conversation-migrations")
                .header("content-type", "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "migration_id": migration_id,
                        "source": {
                            "kind": "character_chat",
                            "character_id": "alice"
                        },
                        "expected_source_sha256": plan["source_sha256"],
                        "confirm": true
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(execute.status(), StatusCode::OK);
    let execute = axum::body::to_bytes(execute.into_body(), 32 * 1024)
        .await
        .unwrap();
    let execute: serde_json::Value = serde_json::from_slice(&execute).unwrap();
    assert_eq!(execute["status"], "completed");

    let export = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/v1/conversation-migrations/{migration_id}/export"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(export.status(), StatusCode::OK);
    let export = axum::body::to_bytes(export.into_body(), 32 * 1024)
        .await
        .unwrap();
    let export: serde_json::Value = serde_json::from_slice(&export).unwrap();
    assert_eq!(export["records"].as_array().unwrap().len(), 2);

    let after = app.clone().oneshot(history_request()).await.unwrap();
    assert_eq!(after.status(), StatusCode::OK);
    let after = axum::body::to_bytes(after.into_body(), 32 * 1024)
        .await
        .unwrap();
    assert_eq!(after, before, "legacy response shape and content changed");

    let rollback = app
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/conversation-migrations/{migration_id}/rollback"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rollback.status(), StatusCode::OK);
    let rollback = axum::body::to_bytes(rollback.into_body(), 32 * 1024)
        .await
        .unwrap();
    let rollback: serde_json::Value = serde_json::from_slice(&rollback).unwrap();
    assert_eq!(rollback["status"], "rolled_back");
}
