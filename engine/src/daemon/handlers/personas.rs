//! Persona HTTP handlers — legacy default + multi-persona CRUD + bindings.
//!
//! #155 PR4：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；Persona 解析、revision 冲突、binding 管理在
//! `PersonaService`。
//!
//! 端点：
//! - `GET    /v1/users/:user_id/persona` — 读默认 Persona（不存在返回兜底，不写盘）
//! - `PUT    /v1/users/:user_id/persona` — 原子写默认 Persona（revision 校验）
//! - `GET    /v1/users/:user_id/personas` — 列出该用户所有 Persona id
//! - `POST   /v1/users/:user_id/personas` — 创建新 Persona（非 default）
//! - `GET    /v1/users/:user_id/personas/:persona_id` — 读指定 Persona
//! - `PUT    /v1/users/:user_id/personas/:persona_id` — 更新指定 Persona
//! - `DELETE /v1/users/:user_id/personas/:persona_id` — 删除指定 Persona
//! - `POST   /v1/users/:user_id/personas/:persona_id/bindings` — 添加绑定（幂等）
//! - `DELETE /v1/users/:user_id/personas/:persona_id/bindings` — 移除绑定（幂等）
//! - `GET    /v1/users/:user_id/persona/effective` — 解析 binding→default，返回生效 Persona + 来源 + 两 scope owner

use crate::daemon::DaemonState;
use crate::domain::{Persona, PersonaBinding, PersonaService};
use crate::error::AirpError;
use crate::types::UserId;
use axum::{http::StatusCode, Json};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ── Persona handlers（#114，每用户一个默认 Persona）────────────────────────────
//
// WEBUI-MVP-PLAN §3.1：GET 读当前 Persona（不存在返回兜底，不写盘）；
// PUT 原子写并 revision bump；expected_revision 不匹配返回 400 + PersonaRevisionConflict JSON。
// `user_id` 走路径参数，经 UserId::new 校验（拒绝路径遍历）；`default_name` 走 query string。

/// GET /v1/users/:user_id/persona — 读当前 Persona；不存在返回兜底（revision=0）。
pub(in crate::daemon) async fn get_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let persona = PersonaService::new(&state.data_root).get_default(&uid, "User")?;
    Ok(Json(persona))
}

/// PUT /v1/users/:user_id/persona — 原子写入 Persona；revision 不匹配返回 400。
pub(in crate::daemon) async fn update_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(payload): Json<UpdatePersonaRequest>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let service = PersonaService::new(&state.data_root);
    let current = service.get_default(&uid, "User")?;
    let persona = Persona {
        schema: Persona::SCHEMA,
        revision: 0, // save 内会 bump；payload 的 revision 不信，用 expected_revision 校验
        updated_at: String::new(),
        name: payload.name,
        description: payload.description,
        variables: payload.variables,
        id: current.id,
        // The legacy endpoint does not own schema-v2 binding fields. Preserve
        // them so editing a name or description cannot silently unbind chats.
        bindings: current.bindings,
    };
    let saved = service.save_default(&uid, payload.expected_revision, persona)?;
    Ok(Json(saved))
}

/// PUT /v1/users/:user_id/persona 的请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct UpdatePersonaRequest {
    /// 客户端持有的当前 revision；不匹配服务端时返回 400 + PersonaRevisionConflict。
    expected_revision: u64,
    /// 用户显示名。
    name: String,
    /// 自由描述。
    #[serde(default)]
    description: String,
    /// 自定义变量表。
    #[serde(default)]
    variables: HashMap<String, String>,
}

// ── Multi-Persona handlers（#114 A1a，多 Persona CRUD + 绑定）──────────────────
//
// PersonaService（PR #127）已交付 list/get/save/delete/bind/unbind/find_for_character；
// 本组 handler 把多 Persona 闭环暴露到 HTTP plural 路径。chat_pipeline 消费
// find_for_character 自动激活留 A1b（独立 PR，会改 ChatCompletionRequest contract）。

/// GET /v1/users/:user_id/personas — 列出该用户所有 Persona id（含 "default"）。
pub(in crate::daemon) async fn list_personas_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<Vec<String>>, AirpError> {
    let uid = UserId::new(user_id)?;
    let ids = PersonaService::new(&state.data_root).list(&uid)?;
    Ok(Json(ids))
}

/// POST /v1/users/:user_id/personas — 创建新 Persona（非 default）。
/// "default" 走 legacy PUT /v1/users/:id/persona；重复 id 由 revision 冲突拒绝。
pub(in crate::daemon) async fn create_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(payload): Json<CreatePersonaRequest>,
) -> Result<Json<Persona>, AirpError> {
    if payload.persona_id.eq_ignore_ascii_case("default") {
        return Err(AirpError::BadRequest(
            "default persona 不能在此创建；使用 PUT /v1/users/:id/persona".to_string(),
        ));
    }
    let uid = UserId::new(user_id)?;
    let persona = Persona {
        schema: Persona::SCHEMA,
        revision: 0, // save 内 bump；payload 的 revision 不信
        updated_at: String::new(),
        name: payload.name,
        description: payload.description,
        variables: payload.variables,
        id: payload.persona_id.clone(),
        bindings: Vec::new(),
    };
    // expected_revision=0：不存在的文件 current_revision_at=0 匹配 → 创建；
    // 已存在则 current_revision≥1，0≠current → BadRequest(PersonaRevisionConflict)。
    let saved =
        PersonaService::new(&state.data_root).save(&uid, &payload.persona_id, 0, persona)?;
    Ok(Json(saved))
}

/// GET /v1/users/:user_id/personas/:persona_id — 读取指定 Persona。
/// default 不存在时返回 initial（不写盘）；非 default 不存在返回 404。
pub(in crate::daemon) async fn get_persona_multi_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let persona = PersonaService::new(&state.data_root).get(&uid, &persona_id, "User")?;
    Ok(Json(persona))
}

/// PUT /v1/users/:user_id/personas/:persona_id — 更新指定 Persona；保留 bindings。
pub(in crate::daemon) async fn update_persona_multi_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
    Json(payload): Json<UpdateMultiPersonaRequest>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let service = PersonaService::new(&state.data_root);
    let current = service.get(&uid, &persona_id, "User")?;
    let persona = Persona {
        schema: Persona::SCHEMA,
        revision: 0, // save 内 bump
        updated_at: String::new(),
        name: payload.name,
        description: payload.description,
        variables: payload.variables,
        id: current.id,
        // 编辑 name/description/variables 不能静默解绑；bindings 由 bind/unbind 专门管理。
        bindings: current.bindings,
    };
    let saved = service.save(&uid, &persona_id, payload.expected_revision, persona)?;
    Ok(Json(saved))
}

/// DELETE /v1/users/:user_id/personas/:persona_id — 删除指定 Persona（default 不可删）。
pub(in crate::daemon) async fn delete_persona_multi_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
) -> Result<StatusCode, AirpError> {
    let uid = UserId::new(user_id)?;
    PersonaService::new(&state.data_root).delete(&uid, &persona_id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/users/:user_id/personas/:persona_id/bindings — 添加绑定（幂等）。
pub(in crate::daemon) async fn bind_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
    Json(payload): Json<BindPersonaRequest>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let binding = PersonaBinding {
        character_id: payload.character_id,
        session_id: payload.session_id,
    };
    let updated = PersonaService::new(&state.data_root).bind(&uid, &persona_id, binding)?;
    Ok(Json(updated))
}

/// DELETE /v1/users/:user_id/personas/:persona_id/bindings — 移除绑定（幂等）。
/// character_id 必填（query），session_id 可选（query）。
pub(in crate::daemon) async fn unbind_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
    query: Result<
        axum::extract::Query<UnbindPersonaQuery>,
        axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<Persona>, AirpError> {
    let axum::extract::Query(query) =
        query.map_err(|error| AirpError::BadRequest(error.to_string()))?;
    let uid = UserId::new(user_id)?;
    let updated = PersonaService::new(&state.data_root).unbind(
        &uid,
        &persona_id,
        &query.character_id,
        query.session_id.as_deref(),
    )?;
    Ok(Json(updated))
}

/// POST /v1/users/:user_id/personas 的请求体（创建新 Persona，非 default）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct CreatePersonaRequest {
    /// 新 Persona 的 ID；走 `validate_id_segment` 校验，拒路径遍历。
    /// "default" 不允许在此创建（用 legacy PUT /v1/users/:id/persona）。
    persona_id: String,
    /// 用户显示名。
    name: String,
    /// 自由描述。
    #[serde(default)]
    description: String,
    /// 自定义变量表，键名对应 prompt 中 `{{key}}` 占位符。
    #[serde(default)]
    variables: HashMap<String, String>,
}

/// PUT /v1/users/:user_id/personas/:persona_id 的请求体（更新指定 Persona）。
/// 与 legacy `UpdatePersonaRequest` 同形状，但路径带 `:persona_id`。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct UpdateMultiPersonaRequest {
    /// 客户端持有的当前 revision；不匹配服务端时返回 400 + PersonaRevisionConflict。
    expected_revision: u64,
    /// 用户显示名。
    name: String,
    /// 自由描述。
    #[serde(default)]
    description: String,
    /// 自定义变量表。
    #[serde(default)]
    variables: HashMap<String, String>,
}

/// POST /v1/users/:user_id/personas/:persona_id/bindings 的请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct BindPersonaRequest {
    /// 绑定到的角色 ID；走 `CharacterId::new` 校验。
    character_id: String,
    /// 可选 session ID；`None` 表示该角色下所有会话通用。走 `SessionId::parse` 校验。
    #[serde(default)]
    session_id: Option<String>,
}

/// DELETE /v1/users/:user_id/personas/:persona_id/bindings 的 query 参数。
#[derive(Debug, Deserialize)]
pub(in crate::daemon) struct UnbindPersonaQuery {
    /// 必填：要移除绑定的角色 ID。
    character_id: String,
    /// 可选：要移除绑定的 session ID；省略表示移除该角色的全会话通用绑定。
    #[serde(default)]
    session_id: Option<String>,
}

// ── Effective Persona（#114 C-PR1，WebUI 闭环支持）──────────────────────────────
//
// 解析 binding→default 两层，返回生效 Persona + 来源 + 两个 scope 的 owner。
// explicit 层由 WebUI 本地根据下拉选择判定，不进端点。复用
// `PersonaService::resolve_effective_persona`，与 chat_pipeline 的 binding 层
// 使用同一真相。路径用 singular `/persona/effective`，避免把 `effective` 占作
// `/personas/:persona_id` 的保留 ID。

/// GET /v1/users/:user_id/persona/effective?character_id=X&session_id=Y —
/// 返回该角色/会话下生效的 Persona（binding 命中或 default）+ 来源 + 两 scope owner。
pub(in crate::daemon) async fn get_effective_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    query: Result<
        axum::extract::Query<EffectivePersonaQuery>,
        axum::extract::rejection::QueryRejection,
    >,
) -> Result<Json<EffectivePersonaResponse>, AirpError> {
    let axum::extract::Query(query) =
        query.map_err(|error| AirpError::BadRequest(error.to_string()))?;
    let uid = UserId::new(user_id)?;
    let service = PersonaService::new(&state.data_root);
    let resolution = service.resolve_effective_persona(
        &uid,
        &query.character_id,
        query.session_id.as_deref(),
    )?;
    let persona = match &resolution.effective_persona_id {
        Some(pid) => service.get(&uid, pid, "User")?,
        None => service.get_default(&uid, "User")?,
    };
    Ok(Json(EffectivePersonaResponse {
        persona,
        source: resolution.source,
        bindings: EffectivePersonaBindings {
            character_persona_id: resolution.character_persona_id,
            session_persona_id: resolution.session_persona_id,
        },
    }))
}

/// GET .../persona/effective 的 query 参数。
#[derive(Debug, Deserialize)]
pub(in crate::daemon) struct EffectivePersonaQuery {
    /// 必填：角色 ID。
    character_id: String,
    /// 可选：会话 ID；省略时只查角色级通用绑定。
    #[serde(default)]
    session_id: Option<String>,
}

/// GET .../persona/effective 的响应体。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct EffectivePersonaResponse {
    /// 生效的 Persona。
    persona: Persona,
    /// 来源：`session_binding` / `character_binding` / `default`。
    source: crate::domain::EffectivePersonaSource,
    /// 两个 scope 各自的 owner，供 UI 按钮分别决策。
    bindings: EffectivePersonaBindings,
}

/// Effective 端点响应中的 binding scope owner 集合。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct EffectivePersonaBindings {
    /// character scope 的 owner Persona id；无绑定时为 null。
    character_persona_id: Option<String>,
    /// session scope 的 owner Persona id；无 session_id 参数或无绑定时为 null。
    session_persona_id: Option<String>,
}
