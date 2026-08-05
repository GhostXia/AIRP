//! C-P2 扩展注册面的 HTTP 面（全部 additive，bearer 鉴权由 v1 路由层承担）。
//!
//! 端点：
//! - `POST   /v1/extensions/install`              —— 安装/替换扩展包（digest-pinned）
//! - `GET    /v1/extensions`                      —— 列出已安装扩展记录
//! - `POST   /v1/extensions/:extension_id/enable` —— 启用（进 catalog）
//! - `POST   /v1/extensions/:extension_id/disable`—— 停用（出 catalog，包保留）
//! - `DELETE /v1/extensions/:extension_id`        —— 卸载（记录 + 孤儿包目录清理）
//! - `GET    /v1/extensions/catalog`              —— 机器可读下发：manifests + slot 计划
//! - `POST   /v1/widget-intents`                  —— intent 执行面最小合同（拒绝默认）
//!
//! 静态包服务：`GET /extensions/:digest/*file` 挂在**鉴权层外**（内容寻址
//! 不可变 + 仅 loopback 拓扑 + nosniff；opaque-origin 沙箱 iframe 的 module
//! import 属 CORS 请求，需要 ACAO:*，由 local_webui_security_headers 统一附）。
//! 服务时按记录中的文件摘要复检内容（防篡改），摘要不符即 500 拒绝投放。
//!
//! intent 执行面（本阶段最小合同）：
//! - 合同形状见 `protocol/widget-intents.json`（机器可读唯一事实源）；
//! - 拒绝默认：C-P2 无任何已注册执行器，一切 intent 返回 403 `intent_denied`；
//! - envelope 携带 `capability` 字段（可空）——C-P3 逐调用强制的预留面，
//!   本阶段只校验形状与留痕（tracing），不消费其值。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{sha256_hex, ExtensionStore};
use crate::daemon::DaemonState;

/// 内置默认 catalog（webui 无 engine / engine 无安装扩展时的权威默认计划，
/// 与 webui/assets/widgets/slots.json 内容一致，source 用绝对路径形态）。
/// 「engine 无配置时用内置默认计划，不得硬失败」的 engine 侧半边。
const DEFAULT_CATALOG_JSON: &str = r#"{
  "version": 1,
  "manifests": [
    {
      "type": "airp.clock",
      "version": "1.0.0",
      "title": "时钟",
      "entry": { "kind": "builtin" }
    },
    {
      "type": "airp.status-pill",
      "version": "1.0.0",
      "title": "状态胶囊",
      "capabilities": ["read:state"],
      "entry": { "kind": "esm", "source": "/assets/widgets/status.module.js", "sandbox": true }
    },
    {
      "type": "acme.third-party-example",
      "version": "0.1.0",
      "title": "第三方示范 widget",
      "author": "AIRP C-P1 demo",
      "capabilities": ["read:state"],
      "entry": { "kind": "esm", "source": "/assets/widgets/third-party-example.js", "sandbox": true }
    }
  ],
  "slots": [
    {
      "id": "chat.sidebar",
      "screen": "chat",
      "region": "sidebar",
      "description": "对话空间左侧栏底部（会话列表之下）",
      "widgets": [
        { "instance": { "id": "clock-chat", "type": "airp.clock" }, "state": { "label": "chat" } }
      ]
    },
    {
      "id": "chat.panel-right",
      "screen": "chat",
      "region": "panel-right",
      "description": "对话空间右侧面板顶部（事件日志之上）",
      "widgets": [
        { "instance": { "id": "status-chat", "type": "airp.status-pill" }, "state": { "label": "Engine", "on": false } }
      ]
    },
    {
      "id": "settings.context",
      "screen": "settings",
      "region": "context",
      "description": "设置屏右侧上下文栏底部",
      "widgets": [
        { "instance": { "id": "status-settings", "type": "airp.status-pill" }, "state": { "label": "设置页", "on": true } }
      ]
    },
    {
      "id": "diagnostics.context",
      "screen": "diagnostics",
      "region": "context",
      "description": "诊断屏右侧上下文栏底部",
      "widgets": [
        { "instance": { "id": "third-party-diagnostics", "type": "acme.third-party-example" }, "state": { "label": "诊断页" } }
      ]
    },
    {
      "id": "workbench.grid",
      "screen": "workbench",
      "region": "grid",
      "description": "工作台主视图末尾（widget 组合示范区）",
      "widgets": [
        { "instance": { "id": "clock-workbench", "type": "airp.clock" }, "state": { "label": "workbench" } },
        { "instance": { "id": "status-workbench", "type": "airp.status-pill" }, "state": { "label": "Engine", "on": true } }
      ]
    }
  ]
}"#;

fn error_body(code: &str, message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": { "code": code, "message": message.into() } })),
    )
        .into_response()
}

fn not_found(message: &str) -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({ "error": { "code": "not_found", "message": message } })),
    )
        .into_response()
}

fn store(state: &DaemonState) -> Arc<ExtensionStore> {
    state.extensions().clone()
}

/// `POST /v1/extensions/install`：安装（或按 type 替换）一个扩展包。
pub async fn install_extension(
    State(state): State<Arc<DaemonState>>,
    Json(request): Json<super::InstallRequest>,
) -> Response {
    let store = store(&state);
    match store.install(request) {
        Ok(record) => (
            StatusCode::OK,
            Json(json!({
                "id": record.id,
                "type": record.widget_type,
                "digest": record.digest,
                "slot": record.slot,
                "enabled": record.enabled,
            })),
        )
            .into_response(),
        Err(error) => {
            let status = if error.code == "storage_error" {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::BAD_REQUEST
            };
            (
                status,
                Json(json!({ "error": { "code": error.code, "message": error.message } })),
            )
                .into_response()
        }
    }
}

/// `GET /v1/extensions`：列出全部已安装扩展记录（含停用项）。
pub async fn list_extensions(State(state): State<Arc<DaemonState>>) -> Json<Value> {
    let store = store(&state);
    Json(json!({ "extensions": store.list() }))
}

async fn set_enabled(state: Arc<DaemonState>, id: String, enabled: bool) -> Response {
    let store = store(&state);
    match store.set_enabled(&id, enabled) {
        Some(record) => (StatusCode::OK, Json(json!(record))).into_response(),
        None => not_found("extension not found"),
    }
}

/// `POST /v1/extensions/:extension_id/enable`。
pub async fn enable_extension(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Response {
    set_enabled(state, id, true).await
}

/// `POST /v1/extensions/:extension_id/disable`。
pub async fn disable_extension(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Response {
    set_enabled(state, id, false).await
}

/// `DELETE /v1/extensions/:extension_id`：卸载（记录 + 孤儿包目录清理）。
pub async fn delete_extension(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Response {
    let store = store(&state);
    match store.remove(&id) {
        Some(_) => StatusCode::NO_CONTENT.into_response(),
        None => not_found("extension not found"),
    }
}

/// `GET /v1/extensions/catalog`：机器可读下发（manifests + slot 计划）。
///
/// 组装规则：内置默认计划打底；已启用扩展的 manifest 按 type upsert
/// （第三方版本替换同名首方示范），并编入其安装时指定的 slot。
pub async fn get_catalog(State(state): State<Arc<DaemonState>>) -> Response {
    let mut catalog: Value = match serde_json::from_str(DEFAULT_CATALOG_JSON) {
        Ok(value) => value,
        Err(error) => {
            // 内置计划是编译期常量，解析失败属实现缺陷：fail-loud 但 5xx，
            // webui 侧会降级到本地 slots.json，用户面不硬失败。
            tracing::error!(%error, "default catalog 解析失败（实现缺陷）");
            return (StatusCode::INTERNAL_SERVER_ERROR, "catalog unavailable").into_response();
        }
    };
    let store = store(&state);
    for record in store.enabled() {
        let manifest = serde_json::to_value(&record.manifest).unwrap_or_default();
        let manifests = catalog.get_mut("manifests").and_then(Value::as_array_mut);
        if let Some(manifests) = manifests {
            if let Some(existing) = manifests.iter_mut().find(|m| {
                m.get("type").and_then(Value::as_str) == Some(record.widget_type.as_str())
            }) {
                *existing = manifest;
            } else {
                manifests.push(manifest);
            }
        }
        let widget = json!({
            "instance": { "id": format!("ext-{}", record.id), "type": record.widget_type },
            "state": {},
        });
        let slots = catalog.get_mut("slots").and_then(Value::as_array_mut);
        if let Some(slots) = slots {
            if let Some(slot) = slots
                .iter_mut()
                .find(|s| s.get("id").and_then(Value::as_str) == Some(record.slot.as_str()))
            {
                if let Some(widgets) = slot.get_mut("widgets").and_then(Value::as_array_mut) {
                    // 同 type 实例不重复编入（重装替换语义）。
                    widgets.retain(|w| {
                        w.get("instance")
                            .and_then(|i| i.get("type"))
                            .and_then(Value::as_str)
                            != Some(record.widget_type.as_str())
                    });
                    widgets.push(widget);
                }
            }
        }
    }
    Json(catalog).into_response()
}

/// intent envelope（合同权威：protocol/widget-intents.json）。
///
/// C-P3 预留：`capability` 已是合同一等字段；逐调用强制在 C-P3 落 engine 侧
/// 授权检查（manifest ∩ 用户同意 ∩ engine policy），本阶段拒绝默认。
#[derive(Debug, Deserialize)]
pub struct WidgetIntentEnvelope {
    pub name: String,
    #[serde(default)]
    pub params: Value,
    pub widget_type: String,
    pub instance_id: String,
    /// 该 intent 所需的 capability（如 `read:state`）；无需求时可省。
    #[serde(default)]
    pub capability: Option<String>,
}

/// `POST /v1/widget-intents`：intent 执行面最小合同——**拒绝默认**。
///
/// C-P2 无任何已注册执行器：envelope 校验通过后一律 403 `intent_denied`。
/// 这保证 C-P3 接入执行器前没有任何「假交互」路径，且合同形状已锁定。
pub async fn widget_intent(
    State(_state): State<Arc<DaemonState>>,
    Json(envelope): Json<WidgetIntentEnvelope>,
) -> Response {
    if envelope.name.is_empty() || envelope.name.len() > 128 {
        return error_body("intent_invalid", "intent name must be 1..=128 chars");
    }
    if envelope.widget_type.is_empty() || envelope.instance_id.is_empty() {
        return error_body("intent_invalid", "widget_type and instance_id are required");
    }
    // 留痕：拒绝也要可观察（C-P3 审计日志的前身）。
    tracing::info!(
        intent = %envelope.name,
        widget_type = %envelope.widget_type,
        instance_id = %envelope.instance_id,
        capability = envelope.capability.as_deref().unwrap_or("-"),
        "widget intent denied (C-P2 default-deny; executors arrive in C-P3)"
    );
    (
        StatusCode::FORBIDDEN,
        Json(json!({
            "error": {
                "code": "intent_denied",
                "message": "no executor is registered for widget intents (C-P2 default-deny); per-call capability enforcement arrives in C-P3",
            }
        })),
    )
        .into_response()
}

/// `GET /extensions/:digest/*file`：digest-pinned 静态包服务（鉴权层外）。
///
/// 安全模型：内容寻址不可变 + 仅 loopback 拓扑 + nosniff；服务时按记录中
/// 的文件摘要复检（防篡改）。未注册的 digest 一律 404——即使目录存在
/// （不投放任何未经安装面登记的内容）。
pub async fn serve_extension_asset(
    State(state): State<Arc<DaemonState>>,
    Path((digest, file)): Path<(String, String)>,
) -> Response {
    let store = store(&state);
    let Some(record) = store.find_by_digest(&digest) else {
        return not_found("unknown extension digest");
    };
    let Some(expected) = record.files.iter().find(|f| f.path == file) else {
        return not_found("file not in package manifest");
    };
    let extensions_root = state.data_root.join("extensions");
    let Some(path) = super::resolve_package_file(&extensions_root, &digest, &file) else {
        return not_found("extension asset missing");
    };
    let Ok(bytes) = std::fs::read(&path) else {
        return not_found("extension asset missing");
    };
    // 加载时校验：内容与安装时登记的摘要不符即拒绝投放（tamper-evident）。
    if sha256_hex(&bytes) != expected.sha256 {
        tracing::error!(digest = %digest, file = %file, "extension asset digest mismatch; refusing to serve");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": { "code": "digest_mismatch", "message": "extension asset failed integrity check" } })),
        )
            .into_response();
    }
    let content_type = match file.rsplit_once('.') {
        Some((_, "js")) => "application/javascript; charset=utf-8",
        Some((_, "css")) => "text/css; charset=utf-8",
        Some((_, "json")) => "application/json; charset=utf-8",
        Some((_, "svg")) => "image/svg+xml",
        Some((_, "png")) => "image/png",
        _ => "application/octet-stream",
    };
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type.to_string()),
            (
                header::CACHE_CONTROL,
                "public, max-age=31536000, immutable".to_string(),
            ),
        ],
        bytes,
    )
        .into_response()
}
