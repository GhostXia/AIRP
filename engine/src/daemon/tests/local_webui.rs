use super::*;
use axum::http::{header, Request, StatusCode};

#[tokio::test]
async fn local_webui_serves_assets_runtime_mode_and_preserves_not_found() {
    let (state, _data_guard) = make_state_no_key();
    let webui = tempfile::tempdir().unwrap();
    std::fs::write(webui.path().join("index.html"), "<h1>AIRP local</h1>").unwrap();
    std::fs::write(webui.path().join("app.js"), "window.loaded = true;").unwrap();
    let router = create_local_webui_router(state, webui.path().to_path_buf());

    let runtime = router
        .clone()
        .oneshot(
            Request::get("/runtime-config.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(runtime.status(), StatusCode::OK);
    assert_eq!(runtime.headers()[header::CACHE_CONTROL], "no-store");
    assert!(runtime.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains("script-src 'self'"));
    let runtime_body = axum::body::to_bytes(runtime.into_body(), usize::MAX)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&runtime_body).contains("mode: 'local'"));

    let asset = router
        .clone()
        .oneshot(Request::get("/app.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(asset.status(), StatusCode::OK);
    assert!(asset.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .contains("javascript"));

    let missing = router
        .oneshot(
            Request::get("/v1/not-a-real-endpoint")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
