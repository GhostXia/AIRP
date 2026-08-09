//! C-P2/C-P3 扩展注册面 + capability 权威授权的 HTTP 面（全部 additive，
//! bearer 鉴权由 v1 路由层承担）。
//!
//! 端点：
//! - `POST   /v1/extensions/install`              —— 安装/替换扩展包（digest-pinned）
//! - `GET    /v1/extensions`                      —— 列出已安装扩展记录
//! - `POST   /v1/extensions/:extension_id/enable` —— 启用（进 catalog）
//! - `POST   /v1/extensions/:extension_id/disable`—— 停用（出 catalog，包保留）
//! - `DELETE /v1/extensions/:extension_id`        —— 卸载（记录 + 孤儿包目录清理）
//! - `GET    /v1/extensions/catalog`              —— 机器可读下发：manifests + slot 计划
//! - `POST   /v1/extensions/:extension_id/grants` —— C-P3：签发/撤销 capability grant
//! - `GET    /v1/extensions/:extension_id/grants` —— C-P3：查询单扩展 grant 状态
//! - `GET    /v1/extensions/grants`               —— C-P3：列出全部 grant（consent 初始化用）
//! - `POST   /v1/widget-intents`                  —— intent 执行面（C-P3 逐调用强制）
//!
//! 静态包服务：`GET /extensions/:digest/*file` 挂在**鉴权层外**（内容寻址
//! 不可变 + 仅 loopback 拓扑 + nosniff；opaque-origin 沙箱 iframe 的 module
//! import 属 CORS 请求，需要 ACAO:*，由 local_webui_security_headers 统一附）。
//! 服务时按记录中的文件摘要复检内容（防篡改），摘要不符即 500 拒绝投放。
//!
//! intent 执行面（C-P3 逐调用强制 + C-P4.1 read 执行器）：
//! - 合同形状见 `protocol/widget-intents.json`（机器可读唯一事实源）；
//! - envelope `capability` 字段缺省 → 视为无需授权的 intent，放行 200；
//! - envelope `capability` 存在 → engine 逐调用强制：
//!   1. 按 `widget_type` 找已启用扩展记录（未安装/停用 → 403 `intent_denied`）；
//!   2. 校验 `capability ∈ record.granted_capabilities`（未授权 → 403 `intent_denied`）；
//!   3. 授权通过 → C-P4.1 read 三件套（read:memory/read:state/read:worldbook）派发真实
//!      执行器；其余 capability 保持 200 echo（C-P3 兼容，执行器 C-P4.2 再补）。
//! - 授权决策（grant/revoke/intent allow/deny）全部 tracing::info! 留痕（审计日志）。

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use super::{sha256_hex, ExtensionStore, DEFAULT_SLOT_IDS, HOST_API_MAJOR, KNOWN_CAPABILITIES};
use crate::daemon::DaemonState;
use crate::domain::LorebookService;
use crate::error::AirpError;
use crate::types::{CharacterId, SessionId};

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
      "host_api": "1",
      "entry": { "kind": "builtin" }
    },
    {
      "type": "airp.status-pill",
      "version": "1.0.0",
      "title": "状态胶囊",
      "capabilities": ["read:state"],
      "host_api": "1",
      "entry": { "kind": "esm", "source": "/assets/widgets/status.module.js", "sandbox": true }
    },
    {
      "type": "acme.third-party-example",
      "version": "0.1.0",
      "title": "第三方示范 widget",
      "author": "AIRP C-P1 demo",
      "capabilities": ["read:state"],
      "host_api": "1",
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

/// #485 E1：变更类操作失败的统一映射——NotFound → 404；Storage → 500
/// storage_error（此前 Option::None 双义被一律映射 404，掩盖持久化失败）。
fn mutation_error_response(error: super::MutationError) -> Response {
    match error {
        super::MutationError::NotFound => not_found("extension not found"),
        super::MutationError::Storage(message) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": { "code": "storage_error", "message": message } })),
        )
            .into_response(),
    }
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
        Ok(record) => (StatusCode::OK, Json(json!(record))).into_response(),
        Err(error) => mutation_error_response(error),
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
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => mutation_error_response(error),
    }
}

/// C-P3 grant 请求体。
#[derive(Debug, Deserialize)]
pub struct GrantRequest {
    /// `"grant"` 签发 / `"revoke"` 撤销。
    pub action: String,
    /// 缺省：grant = manifest 全集 / revoke = 全部撤销。
    /// 指定：grant = 子集授权（须 ∈ manifest） / revoke = 子集撤销。
    #[serde(default)]
    pub capabilities: Option<Vec<String>>,
}

/// grant 视图：暴露授权与稳定 widget 身份字段，不含 manifest/files 等安装细节。
/// consent.js 用此形状初始化权威 grant 缓存，并以 version/source/digest 校验 manifest
/// 身份，避免同 type 的旧/伪造 manifest 继承授权。
fn grant_view(record: &super::ExtensionRecord) -> Value {
    json!({
        "id": record.id,
        "type": record.widget_type,
        "version": record.manifest.version,
        "source": record.manifest.entry.source,
        "digest": record.digest,
        "enabled": record.enabled,
        "granted_capabilities": record.granted_capabilities,
        "granted_at": record.granted_at,
    })
}

/// `POST /v1/extensions/:extension_id/grants`：C-P3 签发/撤销 capability grant。
///
/// 权威语义见 [`super::ExtensionStore::grant`] / [`super::ExtensionStore::revoke_grant`]。
pub async fn grant_extension(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
    Json(request): Json<GrantRequest>,
) -> Response {
    let store = store(&state);
    let result = match request.action.as_str() {
        "grant" => store.grant(&id, request.capabilities),
        "revoke" => store.revoke_grant(&id, request.capabilities),
        _ => {
            return error_body("invalid_action", "action must be 'grant' or 'revoke'");
        }
    };
    match result {
        Ok(Some(record)) => (StatusCode::OK, Json(grant_view(&record))).into_response(),
        Ok(None) => not_found("extension not found"),
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

/// `GET /v1/extensions/:extension_id/grants`：查询单扩展 grant 状态。
pub async fn get_extension_grants(
    State(state): State<Arc<DaemonState>>,
    Path(id): Path<String>,
) -> Response {
    let store = store(&state);
    match store.get(&id) {
        Some(record) => (StatusCode::OK, Json(grant_view(&record))).into_response(),
        None => not_found("extension not found"),
    }
}

/// `GET /v1/extensions/grants`：列出全部 grant（webui consent.js 初始化用）。
///
/// 返回所有已安装扩展的 grant 状态（含未 grant 的，granted_capabilities 为空）。
/// consent.js 据此建立唯一 type → grant 映射，与本地 catalog 交叉判定 canMount；
/// engine 不可达或快照为空时，消费端必须 fail-closed，不得回退本地授权。
pub async fn list_all_grants(State(state): State<Arc<DaemonState>>) -> Json<Value> {
    let store = store(&state);
    let grants: Vec<Value> = store.list().iter().map(grant_view).collect();
    Json(json!({ "grants": grants }))
}

/// `GET /v1/grants`：C-P4 第二批（#484）统一授权查询面。
///
/// 跨类型授权聚合的唯一权威入口：每个 grant 条目带 `kind` 判别字段，
/// 本阶段仅有 `kind: "widget"`（扩展 grant）；后续 MCP/plugin 等授权
/// 主体接入时 additive 追加新 kind，消费方按 kind 分支。
/// `/v1/extensions/grants` 保留不动（consent.js 初始化面兼容）；本端点
/// 面向授权总览/审计类消费（console-runtime 扩展管理页授权总览卡）。
pub async fn list_unified_grants(State(state): State<Arc<DaemonState>>) -> Json<Value> {
    let store = store(&state);
    let grants: Vec<Value> = store
        .list()
        .iter()
        .map(|record| {
            let mut view = grant_view(record);
            if let Some(object) = view.as_object_mut() {
                object.insert("kind".to_string(), json!("widget"));
            }
            view
        })
        .collect();
    Json(json!({ "grants": grants }))
}

/// `GET /v1/extensions/catalog`：机器可读下发（manifests + slot 计划）。
///
/// 组装规则：内置默认计划打底；已启用扩展的 manifest 按 type upsert
/// （第三方版本替换同名首方示范），并编入其安装时指定的 slot。
///
/// C-P4 第二批（catalog 完整化，#484）：顶层另下发两个 engine 权威
/// 协商字段——`host_api_major`（engine 当前支持的宿主合同 major，webui
/// 可据此预判兼容性）与 `capabilities`（engine policy capability 封闭集，
/// 授权 UI 的全集清单权威来源）。二者皆 additive 字段，旧消费方不受影响。
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
    // C-P4 第二批：engine 权威协商字段（additive）。
    if let Some(object) = catalog.as_object_mut() {
        object.insert("host_api_major".to_string(), Value::from(HOST_API_MAJOR));
        object.insert(
            "capabilities".to_string(),
            Value::from(KNOWN_CAPABILITIES.to_vec()),
        );
    }
    let store = store(&state);
    for record in store.enabled() {
        // C-P4-1 fail-closed：安装面已校验 slot ∈ DEFAULT_SLOT_IDS，catalog 组装面
        // 再校验一次（defense in depth）。未知 slot 不编入下发计划，并 log warn——
        // 静默丢弃会让 webui 看到残缺计划却无法定位原因。
        if !DEFAULT_SLOT_IDS.contains(&record.slot.as_str()) {
            tracing::warn!(
                extension_id = %record.id,
                widget_type = %record.widget_type,
                slot = %record.slot,
                "enabled extension references unknown slot; skipping from catalog (install-time validation should have caught this)"
            );
            continue;
        }
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
/// C-P3：`capability` 是逐调用强制的输入字段；engine 按 widget_type 查找
/// 已启用扩展记录，校验 `capability ∈ record.granted_capabilities`。
#[derive(Debug, Deserialize)]
pub struct WidgetIntentEnvelope {
    pub name: String,
    #[serde(default)]
    pub params: Value,
    pub widget_type: String,
    pub instance_id: String,
    /// 该 intent 所需的 capability（如 `read:state`）；无需求时可省。
    /// 缺省 → 无需授权校验，放行 200（合同 required: false）。
    #[serde(default)]
    pub capability: Option<String>,
}

/// `POST /v1/widget-intents`：intent 执行面——C-P3 逐调用强制。
///
/// 授权决策流（逐调用，无缓存）：
/// 1. envelope 形状校验（name/widget_type/instance_id）；
/// 2. `capability` 缺省 → 放行 200（无需授权的 intent）；
/// 3. `capability` 存在 → 按 `widget_type` 找已启用扩展记录：
///    - 未安装/停用 → 403 `intent_denied`；
///    - `capability ∉ granted_capabilities` → 403 `intent_denied`（越权/未授权）；
///    - 授权通过 → C-P4.1 read 三件套派发真实执行器；其余 capability 保持
///      200 echo（C-P3 兼容，执行器 C-P4.2 再补，YAGNI）。
///
/// 审计日志：每次决策（allow/deny）均 tracing::info! 留痕（intent/widget_type/
/// instance_id/capability/extension_id），供运维与未来授权审计面消费。
pub async fn widget_intent(
    State(state): State<Arc<DaemonState>>,
    Json(envelope): Json<WidgetIntentEnvelope>,
) -> Response {
    if envelope.name.is_empty() || envelope.name.len() > 128 {
        return error_body("intent_invalid", "intent name must be 1..=128 chars");
    }
    if envelope.widget_type.is_empty() || envelope.instance_id.is_empty() {
        return error_body("intent_invalid", "widget_type and instance_id are required");
    }

    // C-P3 逐调用强制：无 capability 字段 → 放行（合同 required: false）。
    let Some(capability) = envelope.capability.as_deref() else {
        tracing::info!(
            intent = %envelope.name,
            widget_type = %envelope.widget_type,
            instance_id = %envelope.instance_id,
            "widget intent allowed (no capability required)"
        );
        return (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "name": envelope.name,
                "widget_type": envelope.widget_type,
                "instance_id": envelope.instance_id,
            })),
        )
            .into_response();
    };

    // 有 capability → engine 权威逐调用强制。
    let store = store(&state);
    let Some(record) = store.find_enabled_by_type(&envelope.widget_type) else {
        tracing::info!(
            intent = %envelope.name,
            widget_type = %envelope.widget_type,
            instance_id = %envelope.instance_id,
            capability = %capability,
            "widget intent denied: extension not installed or not enabled"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": {
                    "code": "intent_denied",
                    "message": "extension not installed or not enabled for this widget_type",
                }
            })),
        )
            .into_response();
    };

    if !record.granted_capabilities.iter().any(|c| c == capability) {
        tracing::info!(
            intent = %envelope.name,
            widget_type = %envelope.widget_type,
            instance_id = %envelope.instance_id,
            capability = %capability,
            extension_id = %record.id,
            "widget intent denied: capability not granted"
        );
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": {
                    "code": "intent_denied",
                    "message": format!(
                        "capability {capability} not granted to {}",
                        envelope.widget_type
                    ),
                }
            })),
        )
            .into_response();
    }

    // 授权通过 → C-P4.1 派发真实执行器（read 三件套）。未实现执行器的
    // capability（write:*/call:tool）保持 C-P3 echo 语义（C-P4.2，YAGNI）。
    match capability {
        "read:memory" | "read:state" | "read:worldbook" => {
            // 审计日志：executor 路径提前 return，必须在此留痕（与 allow/deny
            // 分支同字段集），否则授权数据读取无审计记录。
            tracing::info!(
                intent = %envelope.name,
                widget_type = %envelope.widget_type,
                instance_id = %envelope.instance_id,
                capability = %capability,
                extension_id = %record.id,
                "widget intent dispatched to read executor"
            );
            return exec_intent_read(&state, capability, &envelope).await;
        }
        _ => {}
    }

    // C-P3 兼容：授权通过即视为 intent 被接受并留痕。
    tracing::info!(
        intent = %envelope.name,
        widget_type = %envelope.widget_type,
        instance_id = %envelope.instance_id,
        capability = %capability,
        extension_id = %record.id,
        "widget intent allowed (capability granted; executor not implemented for this capability)"
    );
    (
        StatusCode::OK,
        Json(json!({
            "ok": true,
            "name": envelope.name,
            "widget_type": envelope.widget_type,
            "instance_id": envelope.instance_id,
            "capability": capability,
        })),
    )
        .into_response()
}

/// C-P4.1 执行器（read 三件套）：从 envelope.params 取参，在 `spawn_blocking`
/// 中执行同步文件 IO（#433：不占用 tokio worker），结果映射回 intent 合同。
///
/// 语义与既有 handler 一致：`read:state` ≈ GET /v1/characters/:id/state、
/// `read:worldbook` ≈ GET /v1/characters/:id/lorebook（目标不存在 → 404
/// `intent_target_missing`）；`read:memory` ≈ GET /v1/memory/resident
/// （会话无记忆文件时返回空 content 的 200，不产生 404）。
async fn exec_intent_read(
    state: &DaemonState,
    capability: &str,
    envelope: &WidgetIntentEnvelope,
) -> Response {
    let Some(cid_str) = envelope
        .params
        .get("character_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    else {
        return error_body(
            "intent_bad_params",
            "params.character_id (string) is required for read capabilities",
        );
    };
    // session_id 可选；提供时必须是字符串（非字符串 → 400，与 character_id
    // 的严格校验一致，不静默忽略类型错误）。
    let sid_str = match envelope.params.get("session_id") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => {
            return error_body(
                "intent_bad_params",
                "params.session_id (string) is optional for read capabilities",
            );
        }
        None => None,
    };
    let data_root = state.data_root.clone();
    let cid_owned = cid_str.to_string();
    let capability_owned = capability.to_string();
    let loaded = tokio::task::spawn_blocking(move || -> Result<Value, AirpError> {
        let cid = CharacterId::new(&cid_owned)?;
        match capability_owned.as_str() {
            "read:memory" => {
                let sid = sid_str.as_deref().map(SessionId::parse).transpose()?;
                let session_dir =
                    crate::data_dir::resolve_session_dir(&data_root, cid.as_str(), sid.as_ref())?;
                let content = crate::memory::read_resident_memory(&session_dir)?;
                Ok(json!({
                    "content": content,
                    "char_count": content.chars().count(),
                    "capacity": crate::memory::ResidentMemoryConfig::default().capacity_chars,
                }))
            }
            "read:state" => {
                let live =
                    crate::data_dir::char_state_dir(&data_root, cid.as_str()).join("live.json");
                match std::fs::read_to_string(&live) {
                    Ok(text) => serde_json::from_str(&text).map_err(AirpError::from),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(AirpError::NotFound(
                        format!("state for character {cid} not found"),
                    )),
                    Err(e) => Err(AirpError::from(e)),
                }
            }
            "read:worldbook" => LorebookService::new(&data_root)
                .read(&cid)
                .and_then(|l| serde_json::to_value(l).map_err(AirpError::from)),
            // 调用面已按 capability 白名单过滤，此处仅防御。
            other => Err(AirpError::BadRequest(format!(
                "capability {other} has no read executor"
            ))),
        }
    })
    .await;

    match loaded {
        Ok(Ok(result)) => (
            StatusCode::OK,
            Json(json!({
                "ok": true,
                "name": envelope.name,
                "widget_type": envelope.widget_type,
                "instance_id": envelope.instance_id,
                "capability": capability,
                "result": result,
            })),
        )
            .into_response(),
        Ok(Err(error)) => intent_read_error(error),
        Err(error) => {
            tracing::error!(%error, "widget intent read executor join failed");
            intent_read_error(AirpError::Internal(format!(
                "intent read task join failed: {error}"
            )))
        }
    }
}

/// 执行器错误 → intent 合同错误码（404 目标缺失 / 400 参数非法 / 500 其余）。
fn intent_read_error(error: AirpError) -> Response {
    match error {
        AirpError::NotFound(msg) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": { "code": "intent_target_missing", "message": msg } })),
        )
            .into_response(),
        AirpError::BadRequest(msg) => error_body("intent_bad_params", msg),
        other => {
            // 脱敏合同（error.rs public_message）：500 细节只进 tracing，
            // 响应体不携带内部/IO 路径细节。
            tracing::error!(%other, "widget intent executor failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": { "code": "intent_executor_error", "message": "internal error" }
                })),
            )
                .into_response()
        }
    }
}

/// `GET /extensions/:digest/*file`：digest-pinned 静态包服务（鉴权层外）。
///
/// 安全模型：内容寻址不可变 + 仅 loopback 拓扑 + nosniff；服务时按记录中
/// 的文件摘要复检（防篡改）。未注册的 digest 一律 404——即使目录存在
/// （不投放任何未经安装面登记的内容）。
///
/// #485 E2：磁盘读 + 全量 SHA-256 复检（≤1MB）移入 `spawn_blocking`，
/// 不占用 tokio worker 线程；digest 复检语义不变。
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
    let expected_sha = expected.sha256.clone();
    let extensions_root = state.data_root.join("extensions");
    // 留副本供响应段（content-type / 日志）使用；digest/file 被阻塞闭包 move。
    let digest_log = digest.clone();
    let file_log = file.clone();
    /// 阻塞段结果：保留原 404/500 语义划分（#485 E2 仅移出线程，不改行为）。
    enum AssetLoad {
        Bytes(Vec<u8>),
        Missing,
        Mismatch,
    }
    // 阻塞 I/O + 摘要复检移出 async worker（内容不可变，读与复检幂等）。
    let loaded = tokio::task::spawn_blocking(move || {
        let Some(path) = super::resolve_package_file(&extensions_root, &digest, &file) else {
            return AssetLoad::Missing;
        };
        let Ok(bytes) = std::fs::read(&path) else {
            return AssetLoad::Missing;
        };
        // 加载时校验：内容与安装时登记的摘要不符即拒绝投放（tamper-evident）。
        if sha256_hex(&bytes) != expected_sha {
            return AssetLoad::Mismatch;
        }
        AssetLoad::Bytes(bytes)
    })
    .await;
    let bytes = match loaded {
        Ok(AssetLoad::Bytes(bytes)) => bytes,
        Ok(AssetLoad::Missing) => return not_found("extension asset missing"),
        Ok(AssetLoad::Mismatch) => {
            tracing::error!(digest = %digest_log, file = %file_log, "extension asset digest mismatch; refusing to serve");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "code": "digest_mismatch", "message": "extension asset failed integrity check" } })),
            )
                .into_response();
        }
        Err(error) => {
            // join 失败（阻塞任务被取消）：不投放无法自证完整的资产。
            tracing::error!(%error, "extension asset blocking task join failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": { "code": "digest_mismatch", "message": "extension asset failed integrity check" } })),
            )
                .into_response();
        }
    };
    let content_type = match file_log.rsplit_once('.') {
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
