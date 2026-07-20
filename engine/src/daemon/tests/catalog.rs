// `/v1/agent/tools` catalog tests.
//
// Moved from `daemon::tests`. Asserts the catalog remains sorted with 27
// entries and retains the context-export and volume-sealing tools.

use super::*;

#[tokio::test]
async fn agent_tool_catalog_exposes_sorted_builtin_metadata() {
    let (state, _tmp) = make_state_with_key(None);
    let app = create_router(state);
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
    assert_eq!(names.len(), 27);
    assert!(names.windows(2).all(|pair| pair[0] <= pair[1]));
    assert!(names.contains(&"export_context_bundle"));
    assert!(names.contains(&"seal_volume"));
    assert!(names.contains(&"get_preset"));
    assert!(names.contains(&"update_preset"));
}
