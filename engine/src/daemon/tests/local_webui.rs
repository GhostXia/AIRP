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

#[tokio::test]
async fn local_webui_widget_assets_get_cors_for_opaque_sandbox_iframe() {
    // C-P1：沙箱 iframe 是 opaque origin，其内 import() widget module 属 CORS
    // 请求；只有 /assets/widgets/ 附 ACAO:*，其余静态资产不附。
    let (state, _data_guard) = make_state_no_key();
    let webui = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(webui.path().join("assets/widgets")).unwrap();
    std::fs::write(
        webui.path().join("assets/widgets/status.module.js"),
        "export default () => ({ mount() {} });",
    )
    .unwrap();
    std::fs::write(webui.path().join("app.js"), "window.loaded = true;").unwrap();
    let router = create_local_webui_router(state, webui.path().to_path_buf());

    let widget = router
        .clone()
        .oneshot(
            Request::get("/assets/widgets/status.module.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(widget.status(), StatusCode::OK);
    assert_eq!(widget.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");

    let ordinary = router
        .oneshot(Request::get("/app.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(ordinary.status(), StatusCode::OK);
    assert!(ordinary
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());
}

#[tokio::test]
async fn local_webui_sandbox_frame_is_embeddable_by_same_origin_host_only() {
    // C-P1：sandbox-frame.html 须被同源宿主页嵌入：不带 X-Frame-Options，
    // CSP frame-ancestors 放宽为 'self'（第三方仍不得嵌入）；其余页面维持
    // DENY + frame-ancestors 'none'。
    let (state, _data_guard) = make_state_no_key();
    let webui = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(webui.path().join("assets/widgets")).unwrap();
    std::fs::write(
        webui.path().join("assets/widgets/sandbox-frame.html"),
        "<html></html>",
    )
    .unwrap();
    std::fs::write(webui.path().join("index.html"), "<h1>AIRP local</h1>").unwrap();
    let router = create_local_webui_router(state, webui.path().to_path_buf());

    let frame = router
        .clone()
        .oneshot(
            Request::get("/assets/widgets/sandbox-frame.html?src=x")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(frame.status(), StatusCode::OK);
    assert!(frame.headers().get(header::X_FRAME_OPTIONS).is_none());
    assert!(frame.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'self'"));

    let page = router
        .oneshot(Request::get("/index.html").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(page.headers()[header::X_FRAME_OPTIONS], "DENY");
    assert!(page.headers()[header::CONTENT_SECURITY_POLICY]
        .to_str()
        .unwrap()
        .contains("frame-ancestors 'none'"));
}
