// `/v1/agent/tools` catalog tests.
//
// Moved from `daemon::tests`. Asserts the catalog remains sorted with 28
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
    assert_eq!(names.len(), 28);
    assert!(names.windows(2).all(|pair| pair[0] <= pair[1]));
    // Pre-existing canonical tools (regression guard).
    assert!(names.contains(&"export_context_bundle"));
    assert!(names.contains(&"seal_volume"));
    assert!(names.contains(&"get_preset"));
    assert!(names.contains(&"update_preset"));
    // PR #272 阶段三：Agent RP 差异化新增 6 个工具。显式断言名称存在，
    // 防止 snapshot count 测试在新增/重命名时被静默绕过（CodeRabbit nit）。
    assert!(names.contains(&"advance_plot"));
    assert!(names.contains(&"get_plot_status"));
    assert!(names.contains(&"list_world_events"));
    assert!(names.contains(&"npc_action"));
    assert!(names.contains(&"trigger_world_event"));
    assert!(names.contains(&"update_relationship"));
}
