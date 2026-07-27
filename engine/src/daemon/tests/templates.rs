use super::*;

#[tokio::test]
async fn instantiate_validates_character_id_before_building_overridden_card() {
    let (state, _tmp) = make_state_with_key(None);
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/v1/character-templates/fantasy-knight/instantiate")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"character_id":"../invalid","name_override":"   "}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error["error"]["code"], "bad_request");
    assert!(!error["error"]["message"]
        .as_str()
        .unwrap()
        .contains("name_override"));
}
