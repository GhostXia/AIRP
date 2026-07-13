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
use airp_core::domain::ChatService;
use airp_core::quota::QuotaConfig;
use airp_core::types::CharacterId;

fn inline_card() -> &'static str {
    r#"{"spec":"chara_card_v2","spec_version":"2.0","data":{"name":"TestChar","description":"A test character","personality":"","scenario":"","first_mes":"Hello!","mes_example":"","creator_notes":"","system_prompt":"","post_history_instructions":"","tags":[],"creator":"","character_version":"","alternate_greetings":[],"extensions":{}}}"#
}

fn build_sse_body(tokens: &[&str]) -> String {
    let mut out = String::new();
    for tk in tokens {
        out.push_str(&format!(
            "data: {{\"choices\":[{{\"delta\":{{\"content\":\"{}\"}}}}]}}\n\n",
            tk
        ));
    }
    out.push_str("data: [DONE]\n\n");
    out
}

#[derive(Clone, Copy)]
enum PlannerMode {
    ToolThenGenerate,
    AlwaysTool,
    GenerationFails,
}

impl Respond for PlannerMode {
    fn respond(&self, request: &WiremockRequest) -> ResponseTemplate {
        let body: serde_json::Value = request.body_json().unwrap();
        if body["stream"] == false {
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
                        "function": {"name": "echo", "arguments": "{\"probe\":\"structured\"}"}
                    }]
                })
            } else {
                serde_json::json!({"role": "assistant", "content": "No more tools needed."})
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
    (state, tmp)
}

/// POST /v1/agent/run 并把 SSE body 解析为 JSON 事件序列。
async fn run_agent_and_collect(state: Arc<DaemonState>, max_steps: u32) -> Vec<serde_json::Value> {
    let body = serde_json::json!({
        "message": "Hi!",
        "character_id": "testchar",
        "user_profile": { "name": "Tester", "variables": {} },
        "max_steps": max_steps,
        "capabilities": ["call:tool"],
        "allowed_tools": ["echo"]
    });
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
    let events = run_agent_and_collect(state.clone(), 3).await;

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
        events[p_tool_result]["output"]["probe"], "structured",
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
