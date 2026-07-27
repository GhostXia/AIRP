use super::*;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn generate_image_persists_b64_json_when_url_is_unavailable() {
    let server = MockServer::start().await;
    let png = b"\x89PNG\r\n\x1a\npayload";
    Mock::given(method("POST"))
        .and(path("/v1/images/generations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": [{
                "b64_json": STANDARD.encode(png),
                "revised_prompt": "revised"
            }]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let (state, _tmp) = make_state_with_key(None);
    state.config.write().unwrap().endpoint = format!("{}/v1/chat/completions", server.uri());
    let data_root = state.data_root.clone();
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method(axum::http::Method::POST)
                .uri("/v1/image/generate")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"character_id":"hero","prompt":"a scene","download":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["success"], true);
    assert!(body["image_url"].is_null());
    assert_eq!(body["revised_prompt"], "revised");
    assert_eq!(body["meta"]["size"], format!("{} bytes", png.len()));

    let relative = body["image_path"].as_str().unwrap();
    assert_eq!(std::fs::read(data_root.join(relative)).unwrap(), png);
}

#[tokio::test]
async fn list_images_applies_bounded_limit_and_offset() {
    let (state, _tmp) = make_state_with_key(None);
    let index_path = crate::image_gen::images_index_path(&state.data_root, "hero", None);
    std::fs::create_dir_all(index_path.parent().unwrap()).unwrap();
    let images: Vec<_> = (0..55)
        .map(|index| {
            serde_json::json!({
                "filename": format!("{index}.png"),
                "prompt": format!("prompt {index}"),
                "timestamp": index,
                "size": "8 bytes"
            })
        })
        .collect();
    std::fs::write(&index_path, serde_json::to_vec(&images).unwrap()).unwrap();
    let app = create_router(state);

    let page = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/characters/hero/images?limit=2&offset=3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(page.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(page.into_body(), 4096).await.unwrap();
    let page: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(page.as_array().unwrap().len(), 2);
    assert_eq!(page[0]["filename"], "3.png");
    assert_eq!(page[1]["filename"], "4.png");

    let default_page = app
        .clone()
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/characters/hero/images")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = axum::body::to_bytes(default_page.into_body(), usize::MAX)
        .await
        .unwrap();
    let default_page: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(default_page.as_array().unwrap().len(), 50);

    let invalid = app
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/characters/hero/images?limit=201")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
}
