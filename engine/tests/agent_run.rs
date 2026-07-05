//! #30: `/v1/agent/run` 集成覆盖——真实 AgentLoop::run 路径的事件顺序 + 有界闸。
//!
//! 工具实现各自正确 ≠ 公共 agent 端点没回归。本文件从 HTTP 面消费 SSE 事件流，
//! 锁死 M_AGENT-1 骨架路径的：
//!   1. 事件顺序：plan(call_tool) → tool_call → tool_result → plan(generate)
//!      → delta+ → plan(finish) → done(converged)
//!   2. registry 接线：echo 工具经注册表真实调用并回传 output
//!   3. step cap 闸：max_steps=2 时以 done(step_cap) 收敛
//!
//! 未来 M_AGENT-2/4 引入 ReAct 规划时，本测试的固定序列断言需要同步改写——
//! 那是预期中的红，不是过拟合。

use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Method, Request, StatusCode};
use std::net::SocketAddr;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use airp_core::adapter::{BackendEngine, Provider};
use airp_core::config::VolumeConfig;
use airp_core::daemon::{create_router, DaemonState, MutableConfig};
use airp_core::quota::QuotaConfig;

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

async fn setup(upstream_url: &str) -> (Arc<DaemonState>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let data_root = tmp.path().to_path_buf();
    for d in ["characters", "presets", "sessions"] {
        std::fs::create_dir_all(data_root.join(d)).unwrap();
    }
    let state = Arc::new(DaemonState {
        data_root,
        http_client: reqwest::Client::new(),
        config: std::sync::RwLock::new(MutableConfig {
            provider: Provider::OpenAI,
            endpoint: format!("{}/v1/chat/completions", upstream_url),
            api_key: Some("test-key".to_string()),
            model: "test-model".to_string(),
            volume_config: VolumeConfig::default(),
            access_api_key: None,
            engine: BackendEngine::default(),
            quota: QuotaConfig::default(),
        }),
    });
    (state, tmp)
}

/// POST /v1/agent/run 并把 SSE body 解析为 JSON 事件序列。
async fn run_agent_and_collect(
    state: Arc<DaemonState>,
    max_steps: u32,
) -> Vec<serde_json::Value> {
    let body = serde_json::json!({
        "message": "Hi!",
        "character_card_id": inline_card(),
        "user_profile": { "name": "Tester", "variables": {} },
        "max_steps": max_steps
    });
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/v1/agent/run")
        .header("content-type", "application/json")
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
async fn agent_run_skeleton_event_ordering() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_sse_body(&["Hello", " world"])),
        )
        .mount(&server)
        .await;

    let (state, _tmp) = setup(&server.uri()).await;
    let events = run_agent_and_collect(state, 3).await;

    let types: Vec<&str> = events
        .iter()
        .map(|e| e["type"].as_str().unwrap_or("?"))
        .collect();

    // 1) 首事件是 plan(call_tool echo)，末事件是 done。
    assert_eq!(types.first(), Some(&"plan"), "events: {types:?}");
    assert_eq!(
        events[0]["action"]["call_tool"]["tool"], "echo",
        "skeleton plan step 1 should call echo"
    );
    assert_eq!(types.last(), Some(&"done"), "events: {types:?}");

    // 2) 相对顺序：tool_call < tool_result < plan(generate) < delta < done。
    let pos = |pred: &dyn Fn(&serde_json::Value) -> bool| types
        .iter()
        .zip(events.iter())
        .position(|(_, e)| pred(e));
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
        events[p_tool_result]["output"]["probe"], "loop-skeleton",
        "echo output should round-trip probe param"
    );

    // 4) 收敛：converged，steps_taken=3（call_tool + generate + finish）。
    let done = &events[p_done];
    assert_eq!(done["stop_reason"], "converged");
    assert_eq!(done["steps_taken"], 3);
}

/// #30 有界闸：max_steps=2 → 第三步（finish 前）触 step cap。
#[tokio::test]
async fn agent_run_step_cap_bounds_loop() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(build_sse_body(&["ok"])),
        )
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
        .respond_with(ResponseTemplate::new(502).set_body_string("Bad Gateway"))
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
