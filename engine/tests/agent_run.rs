//! #30: `/v1/agent/run` 集成覆盖——真实 AgentLoop::run 路径的事件顺序 + 有界闸。
//!
//! 工具实现各自正确 ≠ 公共 agent 端点没回归。本文件从 HTTP 面消费 SSE 事件流，
//! 锁死 structured tool-call 路径的：
//!   1. 事件顺序：plan(call_tool) → tool_call → tool_result → plan(generate)
//!      → delta+ → plan(finish) → done(converged)
//!   2. registry 接线：模型原生 tool_call 经 engine gate 调用 echo 并回传 typed observation
//!   3. step cap 闸：max_steps=2 时以 done(step_cap) 收敛
//!
//!   4. 收敛后共用 chat finalizer，且只持久化一次。

use std::path::PathBuf;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use std::net::SocketAddr;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request as WiremockRequest, Respond, ResponseTemplate};

use airp_core::adapter::{BackendEngine, Provider};
use airp_core::config::VolumeConfig;
use airp_core::daemon::{create_router, DaemonState, MutableConfig};
use airp_core::domain::{ChatService, StateService};
use airp_core::quota::QuotaConfig;
use airp_core::types::CharacterId;

fn inline_card() -> &'static str {
    r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"TestChar","description":"A test character","personality":"","scenario":"","first_mes":"Hello!","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#
}

fn build_sse_body(tokens: &[&str]) -> String {
    let mut out = String::new();
    for tk in tokens {
        let event = serde_json::json!({"choices": [{"delta": {"content": tk}}]});
        out.push_str(&format!("data: {event}\n\n"));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

fn build_anthropic_sse_body(tokens: &[&str]) -> String {
    tokens
        .iter()
        .map(|token| {
            format!(
                "event: content_block_delta\ndata: {}\n\n",
                serde_json::json!({"delta": {"type": "text_delta", "text": token}})
            )
        })
        .collect()
}

#[derive(Clone)]
struct StateCasConflictMode {
    data_root: PathBuf,
}

impl Respond for StateCasConflictMode {
    fn respond(&self, request: &WiremockRequest) -> ResponseTemplate {
        let body: serde_json::Value = request.body_json().unwrap();
        assert_eq!(body["stream"], true);
        StateService::new(&self.data_root)
            .replace_if_revision(
                &CharacterId::new("testchar").unwrap(),
                1,
                &serde_json::json!({"hp": 80}),
            )
            .unwrap();
        ResponseTemplate::new(200)
            .insert_header("content-type", "text/event-stream")
            .set_body_string(build_sse_body(&[
                "A durable reply.",
                "<state>{\"hp\": 70}</state>",
            ]))
    }
}

#[derive(Clone, Copy)]
enum PlannerMode {
    ToolThenGenerate,
    AlwaysTool,
    GenerationFails,
    TextAndTool,
}

impl Respond for PlannerMode {
    fn respond(&self, request: &WiremockRequest) -> ResponseTemplate {
        let body: serde_json::Value = request.body_json().unwrap();
        if body["stream"] == false {
            if matches!(self, PlannerMode::TextAndTool) {
                return ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "choices": [{"message": {
                        "content": "leaked planner transcript",
                        "tool_calls": [{"function": {
                            "name": "echo",
                            "arguments": "{}"
                        }}]
                    }}]
                }));
            }
            let user = body["messages"][1]["content"].as_str().unwrap_or_default();
            let call_tool = matches!(self, PlannerMode::AlwaysTool)
                || (matches!(
                    self,
                    PlannerMode::ToolThenGenerate | PlannerMode::GenerationFails
                ) && user.contains("\"observations\":[]"));
            let message = if call_tool {
                serde_json::json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {"name": "echo", "arguments": "{\"raw_param\":\"RAW_PARAM_SENTINEL\",\"answer\":\"UNIQUE_ANSWER_SENTINEL\",\"unselected\":\"UNSELECTED_RESULT_SENTINEL\"}"}
                    }]
                })
            } else {
                let planner_input: serde_json::Value = serde_json::from_str(user).unwrap();
                let evidence_id = planner_input["observations"][0]["evidence_candidates"][0]["id"]
                    .as_str()
                    .unwrap();
                serde_json::json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": "generate-1",
                        "type": "function",
                        "function": {
                            "name": "__airp_generate_with_evidence_v1",
                            "arguments": serde_json::to_string(&serde_json::json!({
                                "selected_evidence": [evidence_id]
                            })).unwrap()
                        }
                    }]
                })
            };
            return ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"choices": [{"message": message}]}));
        }
        if matches!(self, PlannerMode::GenerationFails) {
            ResponseTemplate::new(502).set_body_string("Bad Gateway")
        } else {
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_sse_body(&["Hello", " world"]))
        }
    }
}

#[derive(Clone, Copy)]
struct AnthropicPlannerMode;

impl Respond for AnthropicPlannerMode {
    fn respond(&self, request: &WiremockRequest) -> ResponseTemplate {
        let body: serde_json::Value = request.body_json().unwrap();
        if body["stream"] == true {
            return ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_anthropic_sse_body(&["Hello", " world"]));
        }
        let planner_input: serde_json::Value =
            serde_json::from_str(body["messages"][0]["content"].as_str().unwrap()).unwrap();
        let block = if planner_input["observations"].as_array().unwrap().is_empty() {
            serde_json::json!({
                "type": "tool_use",
                "id": "call-anthropic",
                "name": "echo",
                "input": {
                    "raw_param": "RAW_PARAM_SENTINEL",
                    "answer": "UNIQUE_ANSWER_SENTINEL",
                    "unselected": "UNSELECTED_RESULT_SENTINEL"
                }
            })
        } else {
            let evidence_id = planner_input["observations"][0]["evidence_candidates"][0]["id"]
                .as_str()
                .unwrap();
            serde_json::json!({
                "type": "tool_use",
                "id": "generate-anthropic",
                "name": "__airp_generate_with_evidence_v1",
                "input": {"selected_evidence": [evidence_id]}
            })
        };
        ResponseTemplate::new(200).set_body_json(serde_json::json!({"content": [block]}))
    }
}

async fn setup(upstream_url: &str) -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().to_path_buf();
    for d in ["characters", "presets", "sessions"] {
        std::fs::create_dir_all(data_root.join(d)).unwrap();
    }
    let card_dir = data_root.join("characters/testchar/card");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(card_dir.join("raw.json"), inline_card()).unwrap();
    let state = Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        fts: Default::default(),
        settings_update: Default::default(),
        session_coordinators: Default::default(),
        provider_router: Default::default(),
        plugin_tools: Default::default(),
        provider_routing_update: Default::default(),
        plugin_tools_update: Default::default(),
        extensions: std::sync::OnceLock::new(),
        ui_surfaces: Default::default(),
        plugins: Default::default(),
        plugin_children: std::sync::Arc::new(tokio::sync::Mutex::new(
            std::collections::HashMap::new(),
        )),
        shutdown: tokio::sync::watch::channel(false).0,
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint: format!("{}/v1/chat/completions", upstream_url),
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: Some("test-access-key".to_string()),
            engine: BackendEngine::default(),
            quota: QuotaConfig::default(),
            deployment_mode: Default::default(),
            public_origin: None,
        }),
    });
    let preset_dir = state.data_root.join("presets/agent-preset");
    std::fs::create_dir_all(preset_dir.join("versions/agent-fixture")).unwrap();
    std::fs::write(preset_dir.join("current"), "agent-fixture").unwrap();
    let preset_json = serde_json::json!({
        "schema_version": 1,
        "name": "agent-preset",
        "prompt_order": [],
        "prompts": [{
            "identifier": "agent-main",
            "name": "Agent main",
            "role": "system",
                "content": "PRESET_SENTINEL {{persona_fact}}",
            "enabled": true
        }],
        "parameters": {}
    })
    .to_string();
    std::fs::write(
        preset_dir.join("versions/agent-fixture/preset.json"),
        &preset_json,
    )
    .unwrap();
    std::fs::write(preset_dir.join("preset.json"), preset_json).unwrap();
    (state, tmp)
}

/// POST /v1/agent/run 并把 SSE body 解析为 JSON 事件序列。
async fn run_agent_and_collect(state: Arc<DaemonState>, max_steps: u32) -> Vec<serde_json::Value> {
    run_agent_and_collect_with_assignment(state, max_steps, true).await
}

async fn run_agent_and_collect_with_assignment(
    state: Arc<DaemonState>,
    max_steps: u32,
    include_assignment: bool,
) -> Vec<serde_json::Value> {
    let mut body = serde_json::json!({
        "message": "Hi!",
        "character_id": "testchar",
        "character_card_id": inline_card(),
        "lorebook_path": "{\"entries\":[{\"keys\":[\"lore\"],\"content\":\"WORLDBOOK_SENTINEL\",\"enabled\":true,\"constant\":true}]}",
        "user_profile": { "name": "Tester", "variables": {} },
        "messages_history": [{"role": "assistant", "content": "HISTORY_SENTINEL"}],
        "preset_id": "agent-preset",
        "max_steps": max_steps,
        "capabilities": ["call:tool"],
        "allowed_tools": ["echo"]
    });
    if include_assignment {
        body["assignment"] = serde_json::json!({
            "objective": "ASSIGNMENT_SENTINEL: ground the reply in selected facts",
            "role": "scene narrator",
            "output_contract": "roleplay prose"
        });
    }
    run_agent_body_and_collect(state, body).await
}

async fn run_agent_body_and_collect(
    state: Arc<DaemonState>,
    body: serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/v1/agent/run")
        .header("content-type", "application/json")
        .header("authorization", "Bearer test-access-key")
        .body(Body::from(serde_json::to_string(&body).unwrap()))
        .unwrap();
    req.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9999u16))));

    let resp = create_router(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "/v1/agent/run should be 200");

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let text = std::str::from_utf8(&bytes).unwrap();
    text.lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str::<serde_json::Value>(l).expect("SSE data should be JSON"))
        .collect()
}

#[tokio::test]
async fn ordinary_generate_payload_contains_rp_context_without_control_plane_data() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(PlannerMode::ToolThenGenerate)
        .mount(&server)
        .await;
    let (state, _tmp) = setup(&server.uri()).await;
    let user_root = state.data_root.join("users/tester");
    std::fs::create_dir_all(user_root.join("characters/testchar/card")).unwrap();
    std::fs::create_dir_all(user_root.join("presets/agent-preset")).unwrap();
    std::fs::create_dir_all(user_root.join("personas")).unwrap();
    std::fs::write(
        user_root.join("characters/testchar/card/raw.json"),
        inline_card(),
    )
    .unwrap();
    std::fs::copy(
        state.data_root.join("presets/agent-preset/preset.json"),
        user_root.join("presets/agent-preset/preset.json"),
    )
    .unwrap();
    std::fs::write(
        user_root.join("personas/default.json"),
        serde_json::json!({
            "schema": 2,
            "id": "default",
            "revision": 1,
            "updated_at": "2026-08-30T00:00:00Z",
            "name": "PERSONA_SENTINEL",
            "description": "persona decision-input fixture",
            "variables": {"persona_fact": "PERSONA_VARIABLE_SENTINEL"},
            "bindings": []
        })
        .to_string(),
    )
    .unwrap();
    StateService::new(&user_root)
        .write(
            &CharacterId::new("testchar").unwrap(),
            &serde_json::json!({"grounding_fact": "STATE_SENTINEL"}),
        )
        .unwrap();
    let body = serde_json::json!({
        "message": "Hi!",
        "character_id": "testchar",
        "character_card_id": inline_card(),
        "lorebook_path": "{\"entries\":[{\"keys\":[\"lore\"],\"content\":\"WORLDBOOK_SENTINEL\",\"enabled\":true,\"constant\":true}]}",
        "user_profile": {"name": "Tester", "variables": {}},
        "messages_history": [{"role": "assistant", "content": "HISTORY_SENTINEL"}],
        "preset_id": "agent-preset",
        "user_id": "tester",
        "persona_id": "default",
        "max_steps": 1
    });
    let events = run_agent_body_and_collect(state, body).await;
    assert_eq!(events.last().unwrap()["stop_reason"], "converged");
    let requests = server.received_requests().await.unwrap();
    let wire = requests
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body))
        .find(|wire| wire.contains("\"stream\":true") && wire.contains("Hi!"))
        .expect("ordinary RP generation request");
    assert!(wire.contains("test character"), "wire: {wire}");
    assert!(wire.contains("WORLDBOOK_SENTINEL"));
    assert!(wire.contains("PRESET_SENTINEL"));
    assert!(wire.contains("STATE_SENTINEL"));
    assert!(wire.contains("HISTORY_SENTINEL"));
    assert!(wire.contains("PERSONA_VARIABLE_SENTINEL"));
    assert!(wire.contains("Hi!"));
    assert!(!wire.contains("airp.selected-evidence.v1"));
    assert!(!wire.contains("ASSIGNMENT_SENTINEL"));
    assert!(!wire.contains("observations"));
    assert!(!wire.contains("steps_taken"));
}

#[tokio::test]
async fn decision_input_budget_failure_happens_before_prepare_side_effects() {
    let server = MockServer::start().await;
    let (state, _tmp) = setup(&server.uri()).await;
    let body = serde_json::json!({
        "message": "must not persist",
        "character_id": "testchar",
        "character_card_id": inline_card(),
        "user_profile": {"name": "Tester", "variables": {}},
        "max_steps": 1,
        "token_budget": 1,
        "assignment": {"objective": "far beyond the one-token remaining budget"}
    });
    let events = run_agent_body_and_collect(state.clone(), body).await;
    assert_eq!(events.last().unwrap()["stop_reason"], "token_budget");
    assert!(server.received_requests().await.unwrap().is_empty());
    let history = ChatService::new(&state.data_root)
        .history(&CharacterId::new("testchar").unwrap(), None)
        .unwrap();
    assert!(history.messages.is_empty());
}

/// #30 主断言：骨架路径事件顺序 + registry 真实接线 + converged 收敛。
#[tokio::test]
async fn agent_run_structured_tool_event_ordering() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(PlannerMode::ToolThenGenerate)
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    StateService::new(&state.data_root)
        .write(
            &CharacterId::new("testchar").unwrap(),
            &serde_json::json!({"grounding_fact": "STATE_SENTINEL"}),
        )
        .unwrap();
    let events = run_agent_and_collect(state.clone(), 3).await;
    let requests = server.received_requests().await.unwrap();
    let generation: serde_json::Value = requests
        .iter()
        .map(|request| request.body_json::<serde_json::Value>().unwrap())
        .find(|body| body["stream"] == true)
        .expect("final generation request");
    let generation_wire = serde_json::to_string(&generation).unwrap();
    assert!(
        generation_wire.contains("test character"),
        "wire: {generation_wire}"
    );
    assert!(generation_wire.contains("WORLDBOOK_SENTINEL"));
    assert!(generation_wire.contains("PRESET_SENTINEL"));
    assert!(generation_wire.contains("STATE_SENTINEL"));
    assert!(generation_wire.contains("HISTORY_SENTINEL"));
    assert!(generation_wire.contains("Hi!"));
    assert!(generation_wire.contains("ASSIGNMENT_SENTINEL"));
    assert!(generation_wire.contains("airp.selected-evidence.v1"));
    assert!(generation_wire.contains("UNIQUE_ANSWER_SENTINEL"));
    assert!(!generation_wire.contains("RAW_PARAM_SENTINEL"));
    assert!(!generation_wire.contains("UNSELECTED_RESULT_SENTINEL"));
    assert!(!generation_wire.contains("raw_param"));
    assert!(!generation_wire.contains("observations"));
    assert!(!generation_wire.contains("steps_taken"));
    assert!(!generation_wire.contains("dry_run"));
    assert!(!generation_wire.contains("__airp_generate_with_evidence_v1"));

    let types: Vec<&str> = events
        .iter()
        .map(|e| e["type"].as_str().unwrap_or("?"))
        .collect();

    // 1) 首事件是 plan(call_tool echo)，末事件是 done。
    assert_eq!(types.first(), Some(&"plan"), "events: {types:?}");
    assert_eq!(
        events[0]["action"]["call_tool"]["tool"], "echo",
        "structured planner should select echo"
    );
    assert_eq!(types.last(), Some(&"done"), "events: {types:?}");

    // 2) 相对顺序：tool_call < tool_result < plan(generate) < delta < done。
    let pos = |pred: &dyn Fn(&serde_json::Value) -> bool| {
        types.iter().zip(events.iter()).position(|(_, e)| pred(e))
    };
    let p_tool_call = pos(&|e| e["type"] == "tool_call").expect("tool_call event");
    let p_tool_result = pos(&|e| e["type"] == "tool_result").expect("tool_result event");
    let p_plan_generate =
        pos(&|e| e["type"] == "plan" && e["action"] == "generate").expect("plan generate");
    let p_delta = pos(&|e| e["type"] == "delta").expect("delta event");
    let p_done = pos(&|e| e["type"] == "done").expect("done event");
    assert!(
        p_tool_call < p_tool_result
            && p_tool_result < p_plan_generate
            && p_plan_generate < p_delta
            && p_delta < p_done,
        "event order broken: {types:?}"
    );

    // 3) registry 真实接线：echo 工具 output 回传探针参数。
    assert_eq!(events[p_tool_call]["tool"], "echo");
    assert_eq!(events[p_tool_result]["tool"], "echo");
    assert_eq!(
        events[p_tool_result]["output"]["answer"], "UNIQUE_ANSWER_SENTINEL",
        "echo output should round-trip probe param"
    );

    // 4) 收敛：converged，steps_taken=2（structured tool + clean generation）。
    let done = &events[p_done];
    assert_eq!(done["stop_reason"], "converged");
    assert_eq!(done["steps_taken"], 2);
    let history = ChatService::new(&state.data_root)
        .history(&CharacterId::new("testchar").unwrap(), None)
        .unwrap();
    assert_eq!(
        history.messages.len(),
        2,
        "converged run must finalize once"
    );
    assert!(history.messages[1].content.contains("Hello world"));
}

#[tokio::test]
async fn anthropic_agent_generation_uses_top_level_grounding_blocks() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(AnthropicPlannerMode)
        .mount(&server)
        .await;
    let (state, _tmp) = setup(&server.uri()).await;
    state.config.write().unwrap().engine = BackendEngine::AnthropicMessages;
    let events = run_agent_and_collect(state, 3).await;
    assert_eq!(events.last().unwrap()["stop_reason"], "converged");

    let requests = server.received_requests().await.unwrap();
    let generation_request = requests
        .iter()
        .find(|request| {
            request
                .body_json::<serde_json::Value>()
                .is_ok_and(|body| body["stream"] == true)
        })
        .expect("Anthropic generation request");
    assert_eq!(
        generation_request
            .headers
            .get("anthropic-version")
            .and_then(|value| value.to_str().ok()),
        Some("2023-06-01")
    );
    let body: serde_json::Value = generation_request.body_json().unwrap();
    assert!(body["system"].is_array());
    let wire = serde_json::to_string(&body).unwrap();
    assert!(wire.contains("ASSIGNMENT_SENTINEL"));
    assert!(wire.contains("UNIQUE_ANSWER_SENTINEL"));
    assert!(wire.contains("airp.selected-evidence.v1"));
    assert!(!wire.contains("RAW_PARAM_SENTINEL"));
    assert!(!wire.contains("UNSELECTED_RESULT_SENTINEL"));
    assert!(!wire.contains("observations"));
    assert!(body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["role"] != "system"));
}

/// #30 有界闸：max_steps=2 → 第三步（finish 前）触 step cap。
#[tokio::test]
async fn agent_run_step_cap_bounds_loop() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(PlannerMode::AlwaysTool)
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let events = run_agent_and_collect(state, 2).await;

    let done = events.last().expect("at least done event");
    assert_eq!(done["type"], "done");
    assert_eq!(done["stop_reason"], "step_cap");
    assert_eq!(done["steps_taken"], 2);
}

/// #30 上游失败路径：subagent 生成上游 5xx → done(upstream_error)，事件流仍正常收口。
#[tokio::test]
async fn agent_run_upstream_error_terminates_with_typed_done() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(PlannerMode::GenerationFails)
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let events = run_agent_and_collect(state, 3).await;

    let done = events.last().expect("at least done event");
    assert_eq!(done["type"], "done");
    assert_eq!(done["stop_reason"], "upstream_error");
    // 工具步先于生成步完成，因此 tool_result 仍应存在。
    assert!(
        events.iter().any(|e| e["type"] == "tool_result"),
        "tool step should have completed before upstream failure"
    );
}

#[tokio::test]
async fn planner_text_plus_tool_call_fails_closed_before_generation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(PlannerMode::TextAndTool)
        .expect(1)
        .mount(&server)
        .await;
    let (state, _tmp) = setup(&server.uri()).await;
    let events = run_agent_and_collect(state, 3).await;
    assert_eq!(events.last().unwrap()["stop_reason"], "upstream_error");
    assert!(events.iter().all(|event| event["type"] != "tool_call"));
    assert!(events.iter().all(|event| event["type"] != "delta"));
}

#[tokio::test]
async fn agent_run_state_cas_conflict_reports_finalization_error_with_recovery_evidence() {
    let server = MockServer::start().await;
    let (state, _tmp) = setup(&server.uri()).await;
    let character = CharacterId::new("testchar").unwrap();
    StateService::new(&state.data_root)
        .write(&character, &serde_json::json!({"hp": 90}))
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(StateCasConflictMode {
            data_root: state.data_root.clone(),
        })
        .expect(1)
        .mount(&server)
        .await;

    let events = run_agent_and_collect(state.clone(), 1).await;
    let done_events: Vec<_> = events
        .iter()
        .filter(|event| event["type"] == "done")
        .collect();
    assert_eq!(done_events.len(), 1);
    assert!(events
        .iter()
        .all(|event| event["stop_reason"] != "converged"));
    let done = events.last().expect("at least done event");
    assert_eq!(done["type"], "done");
    assert_eq!(done["stop_reason"], "finalization_error");

    let history = ChatService::new(&state.data_root)
        .history(&character, None)
        .unwrap();
    assert_eq!(history.messages.len(), 2);
    assert_eq!(
        history.messages[1].role,
        airp_core::adapter::MessageRole::Assistant
    );
    assert_eq!(history.messages[1].content, "A durable reply.");
    assert!(!history.messages[1].content.contains("<state>"));

    let (revision, _, current) = StateService::new(&state.data_root)
        .read_surface_state(&character)
        .unwrap();
    assert_eq!(revision, 2);
    assert_eq!(current, serde_json::json!({"hp": 80}));

    let marker_path = state
        .data_root
        .join("characters/testchar/history/turn_commit.json");
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(marker_path).unwrap()).unwrap();
    assert_eq!(marker["schema_version"], 2);
    assert_eq!(marker["phase"], "message_committed");
    assert_eq!(marker["message_expected"], true);
    assert_eq!(marker["state_expected"], true);
    assert_eq!(marker["volume_expected"], true);
    let generation_id = marker["generation_id"].as_str().unwrap();
    assert!(!generation_id.is_empty());

    let activity_path = state
        .data_root
        .join("characters/testchar/memory/ui-activity.json");
    let activity: serde_json::Value =
        serde_json::from_slice(&std::fs::read(activity_path).unwrap()).unwrap();
    let receipt = activity["items"].as_array().unwrap().last().unwrap();
    assert_eq!(receipt["source"], "agent");
    assert_eq!(receipt["kind"], "failed");
    assert_eq!(receipt["severity"], "error");
    assert_eq!(receipt["code"], "finalization_failed");
    assert_eq!(receipt["generation_id"], generation_id);
}

#[tokio::test]
async fn agent_tools_are_disabled_without_daemon_bearer_authority() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(PlannerMode::ToolThenGenerate)
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    state.config.write().unwrap().access_api_key = None;
    let events = run_agent_and_collect(state, 3).await;
    assert!(events.iter().all(|event| event["type"] != "tool_call"));
    assert!(events.iter().any(|event| event["type"] == "delta"));
    assert_eq!(events.last().unwrap()["stop_reason"], "converged");
}
