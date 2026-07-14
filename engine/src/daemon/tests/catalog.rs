// `/v1/agent/tools` catalog tests.
//
// Moved verbatim from `daemon::tests`. Asserts the catalog returns the
// expected 19 builtin tool names in sorted order — the same contract that
// #155 PR 2/3 will rely on when splitting the agent tools module.

use super::*;

#[tokio::test]
async fn agent_tool_catalog_exposes_sorted_builtin_metadata() {
    let app = create_router(make_state_with_key(None));
    let response = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/agent/tools")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let tools: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let names: Vec<_> = tools
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(names.len(), 19);
    assert!(names.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(names.contains(&"export_context_bundle"));
    assert!(names.contains(&"seal_volume"));
}
