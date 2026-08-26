//! C-P2 扩展注册面（registry / catalog / digest-pinned 静态包 / intent 最小
//! 合同 / token 续期）行为测试。
//!
//! 覆盖 Task #8 验收点：
//! - catalog：engine 无安装扩展时下发内置默认计划（不硬失败）；
//! - install：digest-pinned、source 强制同源改写、sandbox/slot/摘要校验；
//! - 静态包服务：鉴权层外投放 + ACAO:* + immutable；篡改复检拒绝；未注册 404；
//! - intent 执行面：拒绝默认 403 + envelope 形状校验；
//! - token 续期：rotation（撤旧发新）、无效 401、无 key 403、access key 不得续期。

// token_test_lock（#485 E6）有意跨 await 持有：串行化全局 token store，
// 测试用 oneshot 请求无跨线程挂起风险，此处豁免 await_holding_lock。
#![allow(clippy::await_holding_lock)]

use super::*;
use crate::daemon::desktop_session::{
    clear_desktop_session_tokens_for_test, mint_desktop_session_token, token_test_lock,
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
        // C-P3：capability 权威授权面（grant / revoke / 查询）。
        .route(
            "/v1/extensions/:extension_id/grants",
            post(crate::extensions::api::grant_extension)
                .get(crate::extensions::api::get_extension_grants),
        )
        .route(
            "/v1/extensions/grants",
            get(crate::extensions::api::list_all_grants),
        )
        // C-P4 第二批（#484）：统一授权查询面。
        .route(
            "/v1/grants",
            get(crate::extensions::api::list_unified_grants),
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
    // C-P4 第二批（catalog 完整化）：engine 权威协商字段。
    assert_eq!(
        catalog["host_api_major"],
        serde_json::json!(crate::extensions::HOST_API_MAJOR),
        "catalog 顶层必须下发 engine 支持的 host_api major"
    );
    let caps: Vec<&str> = catalog["capabilities"]
        .as_array()
        .expect("capabilities 封闭集必须随 catalog 下发")
        .iter()
        .map(|c| c.as_str().unwrap())
        .collect();
    assert_eq!(
        caps,
        crate::extensions::KNOWN_CAPABILITIES.to_vec(),
        "catalog capabilities 与 engine policy 封闭集严格一致"
    );
    // 内置 manifests 均显式声明 host_api（与安装面缺省规则对齐，
    // 使下发形状与安装记录序列化形状一致）。
    for manifest in catalog["manifests"].as_array().unwrap() {
        assert_eq!(manifest["host_api"], "1", "内置 manifest 应声明 host_api");
    }
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
            Request::post(format!("/v1/extensions/{id}/disable"))
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
            Request::post(format!("/v1/extensions/{id}/enable"))
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
            Request::delete(format!("/v1/extensions/{id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let resp = router
        .clone()
        .oneshot(
            Request::delete(format!("/v1/extensions/{id}"))
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
    // #485 E5：错误响应不得携带 immutable 长缓存（仅成功响应可缓存）。
    assert!(
        !resp.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"),
        "404 响应不得 immutable 缓存"
    );

    // #485 E5 同规则：已注册 digest 但文件不在包清单 → 404 亦不得 immutable。
    let resp = router
        .clone()
        .oneshot(
            Request::get(format!("/extensions/{digest}/missing.js"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    assert!(
        !resp.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("immutable"),
        "包内缺失文件的 404 响应不得 immutable 缓存"
    );

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
    // #485 E6：token store 是进程级全局，持锁串行化防止并发测试互相清 token。
    let _token_lock = token_test_lock();
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

// ══════════════════════════════════════════════════════════════════════════
// C-P3：capability 权威授权 + intent 逐调用强制
// ══════════════════════════════════════════════════════════════════════════

/// 多 capability 安装体（用于 grant 子集 / 越界测试）。
fn install_body_multi_cap(
    widget_type: &str,
    slot: Option<&str>,
    capabilities: &[&str],
) -> serde_json::Value {
    let content = b"export default () => ({ mount() {} });";
    use base64::Engine;
    serde_json::json!({
        "manifest": {
            "type": widget_type,
            "version": "1.0.0",
            "capabilities": capabilities,
            "entry": {
                "kind": "esm",
                "source": "https://evil.example/w.js",
                "sandbox": true,
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

/// 安装一个扩展并返回其 id（默认 slot=chat.sidebar，capabilities=["read:state"]）。
async fn install_helper(router: &Router, widget_type: &str, capabilities: &[&str]) -> String {
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/extensions/install",
            &install_body_multi_cap(widget_type, Some("chat.sidebar"), capabilities),
        ))
        .await
        .unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "install must succeed for helper"
    );
    body_json(resp).await["id"].as_str().unwrap().to_string()
}

fn grant_request(action: &str, capabilities: Option<Vec<&str>>) -> serde_json::Value {
    let mut req = serde_json::json!({ "action": action });
    if let Some(caps) = capabilities {
        req["capabilities"] = serde_json::json!(caps);
    }
    req
}

fn intent_envelope(
    name: &str,
    widget_type: &str,
    instance_id: &str,
    capability: Option<&str>,
) -> serde_json::Value {
    let mut env = serde_json::json!({
        "name": name,
        "widget_type": widget_type,
        "instance_id": instance_id,
    });
    if let Some(cap) = capability {
        env["capability"] = serde_json::json!(cap);
    }
    env
}

#[tokio::test]
async fn cp3_grant_full_set_when_capabilities_omitted() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let id = install_helper(&router, "acme.grant-full", &["read:state", "write:state"]).await;

    // grant 不带 capabilities → 签发 manifest 全集。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["id"], id);
    assert_eq!(body["type"], "acme.grant-full");
    let mut granted: Vec<String> = body["granted_capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    granted.sort();
    assert_eq!(
        granted,
        vec!["read:state".to_string(), "write:state".to_string()]
    );
    assert!(
        body["granted_at"].as_u64().is_some(),
        "granted_at 必须是 UNIX 秒"
    );
}

#[tokio::test]
async fn cp3_grant_subset_only_grants_declared_capabilities() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let id = install_helper(&router, "acme.grant-sub", &["read:state", "write:state"]).await;

    // 子集授权：只 grant read:state。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("grant", Some(vec!["read:state"])),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let granted: Vec<String> = body["granted_capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(granted, vec!["read:state".to_string()]);

    // 越界 capability → 400 capability_not_declared。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("grant", Some(vec!["admin:system"])),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        body_json(resp).await["error"]["code"],
        "capability_not_declared"
    );
}

#[tokio::test]
async fn cp3_grant_revoke_clears_all_or_subset() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let id = install_helper(&router, "acme.revoke", &["read:state", "write:state"]).await;

    // 先 grant 全集。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 子集撤销：revoke read:state → 剩 write:state。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("revoke", Some(vec!["read:state"])),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let granted: Vec<String> = body["granted_capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert_eq!(granted, vec!["write:state".to_string()]);

    // 全集撤销（capabilities 缺省）→ granted_capabilities 空。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("revoke", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(
        body["granted_capabilities"].as_array().unwrap().is_empty(),
        "全集撤销后 granted_capabilities 必须为空"
    );
}

#[tokio::test]
async fn cp3_grant_invalid_action_and_unknown_extension() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);

    // 未知 extension_id → 404。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/extensions/no-such-id/grants",
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 非法 action → 400 invalid_action。
    let id = install_helper(&router, "acme.bad-action", &["read:state"]).await;
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &serde_json::json!({ "action": "rotate" }),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "invalid_action");
}

#[tokio::test]
async fn cp3_get_extension_grants_and_list_all_grants() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let id_a = install_helper(&router, "acme.list-a", &["read:state"]).await;
    let _id_b = install_helper(&router, "acme.list-b", &["read:state"]).await;

    // 仅 grant A；B 保持未授权。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id_a}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET 单扩展 grant。
    let resp = router
        .clone()
        .oneshot(
            Request::get(format!("/v1/extensions/{id_a}/grants"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["id"], id_a);
    assert_eq!(body["type"], "acme.list-a");
    assert_eq!(body["version"], "1.0.0");
    let source = body["source"].as_str().unwrap();
    assert!(source.starts_with("/extensions/") && source.ends_with("/index.js"));
    assert_eq!(body["granted_capabilities"].as_array().unwrap().len(), 1);

    // GET 未知 extension → 404。
    let resp = router
        .clone()
        .oneshot(
            Request::get("/v1/extensions/no-such/grants")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // GET 全部 grants → 含 A（已 grant）与 B（未 grant，空数组）。
    let resp = router
        .clone()
        .oneshot(
            Request::get("/v1/extensions/grants")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let grants = body["grants"].as_array().unwrap();
    assert_eq!(grants.len(), 2);
    let by_type: std::collections::HashMap<&str, &serde_json::Value> = grants
        .iter()
        .map(|g| (g["type"].as_str().unwrap(), g))
        .collect();
    assert_eq!(by_type["acme.list-a"]["digest"].as_str().unwrap().len(), 64);
    assert_eq!(by_type["acme.list-a"]["enabled"], true);
    assert_eq!(by_type["acme.list-b"]["version"], "1.0.0");
    assert_eq!(
        by_type["acme.list-a"]["granted_capabilities"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        by_type["acme.list-b"]["granted_capabilities"]
            .as_array()
            .unwrap()
            .is_empty(),
        "未 grant 的扩展 granted_capabilities 必须为空"
    );
}

#[tokio::test]
async fn cp4_unified_grants_endpoint_aggregates_with_kind() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let id_a = install_helper(&router, "acme.unified-a", &["read:state"]).await;
    let _id_b = install_helper(&router, "acme.unified-b", &["read:state"]).await;
    // 仅 grant A。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id_a}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // GET /v1/grants：统一面聚合全部授权主体，每条带 kind 判别字段。
    let resp = router
        .clone()
        .oneshot(Request::get("/v1/grants").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let grants = body["grants"].as_array().unwrap();
    assert_eq!(grants.len(), 2, "两个已安装扩展均入统一面");
    for grant in grants {
        assert_eq!(
            grant["kind"], "widget",
            "本阶段授权主体仅 widget 扩展，kind 必须为判别字段"
        );
        assert!(grant["id"].is_string());
        assert!(grant["type"].is_string());
        assert!(grant["granted_capabilities"].is_array());
    }
    let by_type: std::collections::HashMap<&str, &serde_json::Value> = grants
        .iter()
        .map(|g| (g["type"].as_str().unwrap(), g))
        .collect();
    assert_eq!(
        by_type["acme.unified-a"]["granted_capabilities"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        by_type["acme.unified-a"]["granted_at"].is_u64(),
        "已 grant 条目必须带签发时间戳"
    );
    assert!(
        by_type["acme.unified-b"]["granted_at"].is_null(),
        "未 grant 条目不得残留 granted_at"
    );
}

#[tokio::test]
async fn cp3_intent_without_capability_passes_through() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);

    // 无 capability 字段 → 放行 200（无需授权的 intent）。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &intent_envelope("ui.ping", "acme.uninstalled", "inst-1", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["ok"], true);
    assert_eq!(body["name"], "ui.ping");
    // 未授权 intent 不应返回 capability 字段。
    assert!(body.get("capability").is_none() || body["capability"].is_null());
}

#[tokio::test]
async fn cp3_intent_with_capability_denied_when_extension_missing() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);

    // 扩展未安装 → 403 intent_denied（不是 404，合同统一 deny 语义）。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &intent_envelope(
                "data.read",
                "acme.never-installed",
                "inst-1",
                Some("read:state"),
            ),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(resp).await["error"]["code"], "intent_denied");
}

#[tokio::test]
async fn cp3_intent_with_capability_denied_when_not_granted() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let _id = install_helper(&router, "acme.not-granted", &["read:state"]).await;

    // 已安装但未 grant → 403 intent_denied。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &intent_envelope(
                "data.read",
                "acme.not-granted",
                "inst-1",
                Some("read:state"),
            ),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "intent_denied");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("not granted"),
        "deny 消息必须明示 capability 未授权"
    );
}

#[tokio::test]
async fn cp3_intent_allowed_when_capability_granted() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state.clone());
    let id = install_helper(&router, "acme.allowed", &["read:state", "write:state"]).await;

    // grant 全集。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // intent with read:state（预置角色 state）→ 200 + 执行器结果。
    let state_path = crate::data_dir::char_state_dir(&state.data_root, "alice").join("live.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    std::fs::write(&state_path, r#"{"hp": 42}"#).unwrap();
    let mut envelope = intent_envelope("data.read", "acme.allowed", "inst-1", Some("read:state"));
    envelope["params"] = serde_json::json!({ "character_id": "alice" });
    let resp = router
        .clone()
        .oneshot(post_json("/v1/widget-intents", &envelope))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["ok"], true);
    assert_eq!(body["capability"], "read:state");
    assert_eq!(body["result"]["hp"], 42);

    // 子集授权后再调用未授权 capability → 403。
    // 先 revoke read:state → 仅剩 write:state。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("revoke", Some(vec!["read:state"])),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // read:state 已撤销 → 403。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &intent_envelope("data.read", "acme.allowed", "inst-1", Some("read:state")),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // write:state 仍授权 → 200（C-P4.2 执行器落地前保持 echo）。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &intent_envelope("data.write", "acme.allowed", "inst-1", Some("write:state")),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_json(resp).await.get("result").is_none());
}

#[tokio::test]
async fn cp4_1_read_intent_executors_return_data() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state.clone());
    let id = install_helper(
        &router,
        "acme.reader",
        &["read:memory", "read:state", "read:worldbook"],
    )
    .await;
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 预置角色数据：committed state + resident.md + world/lorebook.json。
    let root = state.data_root.clone();
    let state_path = crate::data_dir::char_state_dir(&root, "alice").join("live.json");
    let character = crate::types::CharacterId::new("alice").unwrap();
    let state_service = crate::domain::StateService::new(&root);
    state_service
        .write(
            &character,
            &serde_json::json!({"hp": 50, "location": "camp"}),
        )
        .unwrap();
    let stale_state = std::fs::read(&state_path).unwrap();
    state_service
        .write(
            &character,
            &serde_json::json!({"hp": 100, "location": "forest"}),
        )
        .unwrap();
    std::fs::write(&state_path, stale_state).unwrap();

    let memory_path = crate::data_dir::resolve_session_dir(&root, "alice", None)
        .unwrap()
        .join("resident.md");
    std::fs::create_dir_all(memory_path.parent().unwrap()).unwrap();
    let memory_text = "Alice remembers the blue door.";
    std::fs::write(&memory_path, memory_text).unwrap();

    let lorebook_path = crate::data_dir::char_world_lorebook_path(&root, "alice");
    std::fs::create_dir_all(lorebook_path.parent().unwrap()).unwrap();
    std::fs::write(
        &lorebook_path,
        r#"{"entries":[{"keys":["blue door"],"content":"A blue door in the forest","enabled":true,"priority":20}]}"#,
    )
    .unwrap();

    // read:state → 200 + committed revision 内容，并修复 stale live 投影。
    let mut envelope = intent_envelope("data.read", "acme.reader", "inst-1", Some("read:state"));
    envelope["params"] = serde_json::json!({ "character_id": "alice" });
    let resp = router
        .clone()
        .oneshot(post_json("/v1/widget-intents", &envelope))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["result"]["hp"], 100);
    assert_eq!(body["result"]["location"], "forest");

    // read:memory → 200 + resident memory 内容与字符统计。
    let mut envelope = intent_envelope("data.read", "acme.reader", "inst-1", Some("read:memory"));
    envelope["params"] = serde_json::json!({ "character_id": "alice" });
    let resp = router
        .clone()
        .oneshot(post_json("/v1/widget-intents", &envelope))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(body["result"]["content"], memory_text);
    assert_eq!(
        body["result"]["char_count"],
        memory_text.chars().count() as u64
    );
    // 合同字段 drift 防护（CodeRabbit #505）：capacity 来自默认配置，
    // 必须断言以防实现与 protocol/widget-intents.json 漂移。
    assert_eq!(
        body["result"]["capacity"],
        crate::memory::ResidentMemoryConfig::default().capacity_chars
    );

    // read:worldbook → 200 + lorebook 条目。
    let mut envelope =
        intent_envelope("data.read", "acme.reader", "inst-1", Some("read:worldbook"));
    envelope["params"] = serde_json::json!({ "character_id": "alice" });
    let resp = router
        .clone()
        .oneshot(post_json("/v1/widget-intents", &envelope))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert_eq!(
        body["result"]["entries"][0]["content"],
        "A blue door in the forest"
    );

    // 缺 character_id → 400 intent_bad_params。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &intent_envelope("data.read", "acme.reader", "inst-1", Some("read:state")),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "intent_bad_params");

    // 非法 character_id（路径遍历字符）→ 400 intent_bad_params。
    // 三条 read capability 走不同文件系统路径，逐一遍历防单路径缺口
    // （CodeRabbit #505）。
    for capability in ["read:state", "read:memory", "read:worldbook"] {
        let mut envelope = intent_envelope("data.read", "acme.reader", "inst-1", Some(capability));
        envelope["params"] = serde_json::json!({ "character_id": "../evil" });
        let resp = router
            .clone()
            .oneshot(post_json("/v1/widget-intents", &envelope))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{capability} traversal must be rejected"
        );
        assert_eq!(body_json(resp).await["error"]["code"], "intent_bad_params");
    }

    // 目标不存在 → 404 intent_target_missing（read:state / read:worldbook
    // 均有 NotFound 语义；read:memory 语义见下）。
    for capability in ["read:state", "read:worldbook"] {
        let mut envelope = intent_envelope("data.read", "acme.reader", "inst-1", Some(capability));
        envelope["params"] = serde_json::json!({ "character_id": "bob" });
        let resp = router
            .clone()
            .oneshot(post_json("/v1/widget-intents", &envelope))
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "{capability} unknown character must 404"
        );
        assert_eq!(
            body_json(resp).await["error"]["code"],
            "intent_target_missing"
        );
    }

    // read:memory 无记忆文件 → 200 空 content（无 404 语义）。
    let mut envelope = intent_envelope("data.read", "acme.reader", "inst-1", Some("read:memory"));
    envelope["params"] = serde_json::json!({ "character_id": "bob" });
    let resp = router
        .clone()
        .oneshot(post_json("/v1/widget-intents", &envelope))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(body_json(resp).await["result"]["content"], "");

    // session_id 提供但非字符串 → 400 intent_bad_params（不静默忽略）。
    let mut envelope = intent_envelope("data.read", "acme.reader", "inst-1", Some("read:memory"));
    envelope["params"] = serde_json::json!({ "character_id": "alice", "session_id": 42 });
    let resp = router
        .clone()
        .oneshot(post_json("/v1/widget-intents", &envelope))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"]["code"], "intent_bad_params");

    // 执行器内部错误（live.json 损坏）→ 500 intent_executor_error，
    // 且响应体不携带内部细节（脱敏合同，error.rs public_message）。
    let state_path = crate::data_dir::char_state_dir(&root, "carol").join("live.json");
    std::fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    std::fs::write(&state_path, r#"{"hp": "#).unwrap();
    let mut envelope = intent_envelope("data.read", "acme.reader", "inst-1", Some("read:state"));
    envelope["params"] = serde_json::json!({ "character_id": "carol" });
    let resp = router
        .clone()
        .oneshot(post_json("/v1/widget-intents", &envelope))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = body_json(resp).await;
    assert_eq!(body["error"]["code"], "intent_executor_error");
    assert_eq!(body["error"]["message"], "internal error");
}

#[tokio::test]
async fn cp3_intent_denied_when_extension_disabled() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let id = install_helper(&router, "acme.disabled", &["read:state"]).await;

    // grant 全集。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{id}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // disable → 出 catalog；intent 即使 grant 仍在也拒绝（find_enabled_by_type 排除停用）。
    let resp = router
        .clone()
        .oneshot(
            Request::post(format!("/v1/extensions/{id}/disable"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/widget-intents",
            &intent_envelope("data.read", "acme.disabled", "inst-1", Some("read:state")),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert_eq!(body_json(resp).await["error"]["code"], "intent_denied");
}

#[tokio::test]
async fn cp3_reinstall_same_type_clears_grant() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state);
    let _id_v1 = install_helper(&router, "acme.reinstall", &["read:state"]).await;

    // grant 全集。
    let resp = router
        .clone()
        .oneshot(post_json(
            &format!("/v1/extensions/{_id_v1}/grants"),
            &grant_request("grant", None),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 重装同 type（替换语义）→ 新记录 granted_capabilities 必须为空。
    let resp = router
        .clone()
        .oneshot(post_json(
            "/v1/extensions/install",
            &install_body_multi_cap("acme.reinstall", Some("chat.sidebar"), &["read:state"]),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let new_id = body_json(resp).await["id"].as_str().unwrap().to_string();
    assert_ne!(new_id, _id_v1, "重装必须生成新 id（旧记录被替换）");

    // GET 新记录 grant → 空。
    let resp = router
        .clone()
        .oneshot(
            Request::get(format!("/v1/extensions/{new_id}/grants"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    assert!(
        body["granted_capabilities"].as_array().unwrap().is_empty(),
        "重装必须清空 grant（新代码新身份）"
    );

    // 旧 id 已被替换 → GET 404。
    let resp = router
        .clone()
        .oneshot(
            Request::get(format!("/v1/extensions/{_id_v1}/grants"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

// C-P4-1：catalog 未知扩展点 fail-closed（defense in depth）。
//
// 安装面已校验 slot ∈ DEFAULT_SLOT_IDS（invalid_slot），但若 extensions.json
// 被直接篡改或未来安装面逻辑漂移，catalog 组装面必须再次 fail-closed：
// 未知 slot 的扩展不进入下发计划，并 log warn（不静默丢弃）。
#[tokio::test]
async fn cp4_catalog_skips_extension_with_unknown_slot() {
    let (state, _guard) = make_state_no_key();
    let router = ext_router(state.clone());

    // 正常安装一个合法扩展（slot=chat.sidebar）。
    let _ = install_helper(&router, "acme.known", &["read:state"]).await;

    // 直接写一个 extensions.json，含一条 slot 为未知值的 enabled 记录
    // （绕过安装面的 slot 校验，模拟篡改/漂移场景）。
    let data_root = state.data_root.clone();
    let tampered = serde_json::json!({
        "extensions": [{
            "id": "tampered-1",
            "type": "acme.unknown_slot",
            "digest": "0".repeat(64),
            "installed_at": 0u64,
            "enabled": true,
            "slot": "not.a.real.slot",
            "manifest": {
                "type": "acme.unknown_slot",
                "version": "1.0.0",
                "capabilities": ["read:state"],
                "entry": {
                    "kind": "esm",
                    "source": "/extensions/0000000000000000000000000000000000000000000000000000000000000000/index.js",
                    "sandbox": true
                }
            },
            "files": [],
            "granted_capabilities": []
        }]
    });
    std::fs::write(
        data_root.join("extensions.json"),
        serde_json::to_vec_pretty(&tampered).unwrap(),
    )
    .unwrap();

    // 重新加载 state 让篡改生效。
    let state2 = crate::daemon::tests::make_state_with_data_root(data_root.clone());
    let router2 = ext_router(state2);

    let resp = router2
        .oneshot(
            Request::get("/v1/extensions/catalog")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let catalog = body_json(resp).await;

    // 篡改记录的 widget_type 不应出现在 catalog manifests 中。
    let types: Vec<&str> = catalog["manifests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["type"].as_str().unwrap_or(""))
        .collect();
    assert!(
        !types.contains(&"acme.unknown_slot"),
        "未知 slot 的扩展必须 fail-closed，不进入 catalog 下发计划"
    );

    // 篡改记录不应出现在任何 slot 的 widgets 列表中。
    for slot in catalog["slots"].as_array().unwrap() {
        for widget in slot["widgets"].as_array().unwrap() {
            assert_ne!(
                widget["instance"]["type"].as_str().unwrap_or(""),
                "acme.unknown_slot",
                "未知 slot 的扩展不应被编入任何 slot"
            );
        }
    }
}
