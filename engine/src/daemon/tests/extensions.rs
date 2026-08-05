//! C-P2 扩展注册面（registry / catalog / digest-pinned 静态包 / intent 最小
//! 合同 / token 续期）行为测试。
//!
//! 覆盖 Task #8 验收点：
//! - catalog：engine 无安装扩展时下发内置默认计划（不硬失败）；
//! - install：digest-pinned、source 强制同源改写、sandbox/slot/摘要校验；
//! - 静态包服务：鉴权层外投放 + ACAO:* + immutable；篡改复检拒绝；未注册 404；
//! - intent 执行面：拒绝默认 403 + envelope 形状校验；
//! - token 续期：rotation（撤旧发新）、无效 401、无 key 403、access key 不得续期。

use super::*;
use crate::daemon::desktop_session::{
    clear_desktop_session_tokens_for_test, mint_desktop_session_token,
};
use axum::body::Body;
use axum::http::{header, Request, StatusCode};

fn sha256_of(content: &[u8]) -> String {
    crate::extensions::sha256_hex(content)
}

fn install_body(widget_type: &str, slot: Option<&str>, sandbox: bool) -> serde_json::Value {
    let content = b"export default () => ({ mount() {} });";
    use base64::Engine;
    serde_json::json!({
        "manifest": {
            "type": widget_type,
            "version": "1.0.0",
            "capabilities": ["read:state"],
            "entry": {
                "kind": "esm",
                "source": "https://evil.example/w.js",
                "sandbox": sandbox,
            }
        },
        "files": [{
            "path": "index.js",
            "content_base64": base64::engine::general_purpose::STANDARD.encode(content),
            "sha256": sha256_of(content),
        }],
        "slot": slot,
    })
}

/// 最小 v1 extensions 路由（带 auth 层），与 daemon/mod.rs 的接线同形状。
fn ext_router(state: Arc<DaemonState>) -> Router {
    let v1 = Router::new()
        .route(
            "/v1/extensions/install",
            post(crate::extensions::api::install_extension),
        )
        .route(
            "/v1/extensions",
            get(crate::extensions::api::list_extensions),
        )
        .route(
            "/v1/extensions/catalog",
            get(crate::extensions::api::get_catalog),
        )
        .route(
            "/v1/extensions/:extension_id/enable",
            post(crate::extensions::api::enable_extension),
        )
        .route(
            "/v1/extensions/:extension_id/disable",
            post(crate::extensions::api::disable_extension),
        )
        .route(
            "/v1/extensions/:extension_id",
            delete(crate::extensions::api::delete_extension),
        )
        .route(
            "/v1/widget-intents",
            post(crate::extensions::api::widget_intent),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ));
    Router::new().merge(v1).with_state(state)
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn post_json(uri: &str, value: &serde_json::Value) -> Request<Body> {
    Request::post(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap()
}

#[tokio::test]
async fn catalog_defaults_to_builtin_plan_when_no_extensions() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let resp = router
        .oneshot(
            Request::get("/v1/extensions/catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let catalog = body_json(resp).await;
    assert_eq!(catalog["version"], 1);
    assert_eq!(
        catalog["manifests"].as_array().unwrap().len(),
        3,
        "内置 3 manifests"
    );
    assert_eq!(
        catalog["slots"].as_array().unwrap().len(),
        5,
        "内置 5 slots"
    );
    let types: Vec<&str> = catalog["manifests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap())
        .collect();
    assert!(types.contains(&"airp.clock"));
}

#[tokio::test]
async fn extension_lifecycle_install_catalog_disable_enable_delete() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);

    // install：source 强制改写为同源 digest 路径（R0）。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/extensions/install",
            &install_body("acme.lifecycle", Some("chat.sidebar"), true),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let installed = body_json(resp).await;
    let id = installed["id"].as_str().unwrap().to_string();
    let digest = installed["digest"].as_str().unwrap().to_string();
    assert_eq!(installed["slot"], "chat.sidebar");
    assert!(installed["enabled"].as_bool().unwrap());

    // list 含该记录。
    let resp = router
        .clone()
        .oneshot(Request::get("/v1/extensions").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let list = body_json(resp).await;
    assert_eq!(list["extensions"].as_array().unwrap().len(), 1);

    // catalog：manifest upsert + 实例编入指定 slot（source 同源 digest 路径）。
    let resp = router
        .clone()
        .oneshot(
            Request::get("/v1/extensions/catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let catalog = body_json(resp).await;
    let manifest = catalog["manifests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["type"] == "acme.lifecycle")
        .expect("installed manifest in catalog")
        .clone();
    assert_eq!(
        manifest["entry"]["source"],
        format!("/extensions/{digest}/index.js")
    );
    let slot = catalog["slots"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["id"] == "chat.sidebar")
        .unwrap()
        .clone();
    let instances: Vec<&str> = slot["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["instance"]["type"].as_str().unwrap())
        .collect();
    assert!(instances.contains(&"acme.lifecycle"));

    // disable → 出 catalog；enable → 回 catalog。
    let resp = router
        .clone()
        .oneshot(
            Request::post(&format!("/v1/extensions/{id}/disable"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let resp = router
        .clone()
        .oneshot(
            Request::get("/v1/extensions/catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let catalog = body_json(resp).await;
    assert!(
        !catalog["manifests"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["type"] == "acme.lifecycle"),
        "disabled extension must leave the catalog"
    );
    let resp = router
        .clone()
        .oneshot(
            Request::post(&format!("/v1/extensions/{id}/enable"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // delete → 204；再次 delete → 404；enable 未知 id → 404。
    let resp = router
        .clone()
        .oneshot(
            Request::delete(&format!("/v1/extensions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let resp = router
        .clone()
        .oneshot(
            Request::delete(&format!("/v1/extensions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let resp = router
        .oneshot(
            Request::post("/v1/extensions/nope/enable")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn install_rejects_unsafe_manifests_over_http() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);

    // 第三方 esm 缺 sandbox → sandbox_required（BUG-6 第三道）。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/extensions/install",
            &install_body("acme.nosandbox", None, false),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "sandbox_required");

    // 未知 slot → invalid_slot。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/extensions/install",
            &install_body("acme.badslot", Some("not.a.slot"), true),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "invalid_slot");

    // 摘要不符 → digest_mismatch。
    let mut tampered = install_body("acme.badsha", None, true);
    tampered["files"][0]["sha256"] = serde_json::json!("0".repeat(64));
    let resp = router
        .clone()
        .oneshot(post_json("/v1/extensions/install", &tampered))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "digest_mismatch");
}

#[tokio::test]
async fn serve_extension_asset_is_digest_pinned_and_tamper_evident() {
    let (state, _guard) = make_state_no_key();
    let webui = tempfile::tempdir().unwrap();
    std::fs::write(webui.path().join("index.html"), "<h1>AIRP</h1>").unwrap();
    // 完整 local-webui router：含 v1 安装面 + 鉴权层外的 /extensions 投放 +
    // local_webui_security_headers（ACAO / Cache-Control 断言对象）。
    let router = create_local_webui_router(state.clone(), webui.path().to_path_buf());

    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/extensions/install",
            &install_body("acme.serve", None, true),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let installed = body_json(resp).await;
    let digest = installed["digest"].as_str().unwrap().to_string();

    // 已注册 digest → 200 + ACAO:*（opaque-origin 沙箱 CORS import 必需）
    // + immutable 长缓存；鉴权层外（无 bearer 也能加载）。
    let resp = router
        .clone()
        .oneshot(
            Request::get(format!("/extensions/{digest}/index.js"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(resp.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "*");
    assert!(resp.headers()[header::CACHE_CONTROL]
        .to_str()
        .unwrap()
        .contains("immutable"));
    assert_eq!(resp.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");

    // 未注册 digest → 404（不投放未经安装面登记的内容）。
    let unknown = "0".repeat(64);
    let resp = router
        .clone()
        .oneshot(
            Request::get(format!("/extensions/{unknown}/index.js"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 负例：非 /extensions/ 前缀不得被附 ACAO:*（豁免即配负例测试）。
    let resp = router
        .clone()
        .oneshot(
            Request::get("/runtime-config.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(resp
        .headers()
        .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
        .is_none());

    // 篡改包内文件 → 服务时摘要复检拒绝（500 digest_mismatch）。
    let package_file = state
        .data_root
        .join("extensions")
        .join(&digest)
        .join("index.js");
    std::fs::write(
        &package_file,
        b"export default () => ({ mount() { /* evil */ } });",
    )
    .unwrap();
    let resp = router
        .oneshot(
            Request::get(format!("/extensions/{digest}/index.js"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body_json(resp).await["error"]["code"], "digest_mismatch");
}

#[tokio::test]
async fn widget_intent_is_deny_by_default_and_validates_envelope() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);

    // 合法 envelope + capability 预留字段 → 拒绝默认 403 intent_denied。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &serde_json::json!({
                "name": "third-party.ping",
                "params": { "id": "demo" },
                "widget_type": "acme.demo",
                "instance_id": "ext-1",
                "capability": "read:state",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(resp).await["error"]["code"], "intent_denied");

    // 空 name → 400 intent_invalid。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &serde_json::json!({
                "name": "",
                "widget_type": "acme.demo",
                "instance_id": "ext-1",
            }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "intent_invalid");

    // 缺 widget_type / instance_id → 400 intent_invalid。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &serde_json::json!({ "name": "x.ping", "widget_type": "", "instance_id": "ext-1" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 缺必填字段 → 422/400（axum Json 提取器拒收，合同形状锁定）。
    let resp = router
        .oneshot(post_json(
            "/v1/widget-intents",
            &serde_json::json!({ "params": {} }),
        ))
        .await
        .unwrap();
    assert!(resp.status().is_client_error());
}

#[tokio::test]
async fn renew_rotates_token_and_rejects_stale_or_keyless() {
    clear_desktop_session_tokens_for_test();
    let (state, _guard) = make_state_with_key(Some("renew-key"));
    let router = Router::new()
        .route(
            "/v1/desktop-session/renew",
            post(crate::daemon::desktop_session::desktop_session_renew_endpoint),
        )
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .with_state(state);

    let (old_token, _) = mint_desktop_session_token();

    // rotation：旧 token 换新 token，旧 token 立即失效。
    let resp = router
        .clone()
        .oneshot(
            Request::post("/v1/desktop-session/renew")
                .header(header::AUTHORIZATION, format!("Bearer {old_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let new_token = body["token"].as_str().unwrap().to_string();
    assert_ne!(new_token, old_token);
    assert!(crate::daemon::desktop_session::validate_desktop_session_token(&new_token));
    assert!(
        !crate::daemon::desktop_session::validate_desktop_session_token(&old_token),
        "rotation 必须撤销旧 token"
    );

    // 旧 token 再次续期 → 401（auth 层即拒绝，已不在有效集合）。
    let resp = router
        .clone()
        .oneshot(
            Request::post("/v1/desktop-session/renew")
                .header(header::AUTHORIZATION, format!("Bearer {old_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // access key 不得被当作续期对象（全权凭据不参与 rotation）。
    let resp = router
        .clone()
        .oneshot(
            Request::post("/v1/desktop-session/renew")
                .header(header::AUTHORIZATION, "Bearer renew-key")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

    // 无 key（local-webui 便携模式）→ 403 fail-closed。
    let (state_no_key, _guard2) = make_state_no_key();
    let router_no_key = Router::new()
        .route(
            "/v1/desktop-session/renew",
            post(crate::daemon::desktop_session::desktop_session_renew_endpoint),
        )
        .route_layer(middleware::from_fn_with_state(
            state_no_key.clone(),
            auth_middleware,
        ))
        .with_state(state_no_key);
    let (token, _) = mint_desktop_session_token();
    let resp = router_no_key
        .oneshot(
            Request::post("/v1/desktop-session/renew")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}
