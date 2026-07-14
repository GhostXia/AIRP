# Persona 多 Persona HTTP API 闭环 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把已在 engine service 层交付的多 Persona 能力（list/get/save/delete/bind/unbind）暴露为 HTTP API，补齐 HTTP 集成测试，打通 engine 侧多 Persona CRUD + 绑定闭环。

**Architecture:** `PersonaService`（[engine/src/domain.rs:745-984](file:///D:/AIRP-Dev/engine/src/domain.rs#L745-L984)）已在 PR #127 全量交付 list/get/save/delete/bind/unbind/find_for_character，但 [handlers.rs:205-237](file:///D:/AIRP-Dev/engine/src/daemon/handlers.rs#L205-L237) 只暴露了 default 路径（GET/PUT `/v1/users/:id/persona`）。本计划新增 7 个 plural 路径 endpoint（`/v1/users/:id/personas/...`），handler 全部委托到现有 service，零新业务逻辑，复用 `validate_id_segment` / `CharacterId::new` / `SessionId::parse` 做输入校验，复用 `PersonaRevisionConflict` 做冲突信号。

**Tech Stack:** Rust + axum + serde + tempfile（测试）。无新依赖。

---

## 范围说明（重要）

本计划是 **A1a**（HTTP API 闭环），**不含** `chat_pipeline` 消费 persona 的变更。拆分理由：

- **A1a（本计划）**：纯 HTTP 层增项，handler 委托已有 service，零 contract 变更，向后兼容（legacy singular `/persona` endpoint 保留）。审计面收敛。
- **A1b（下一个 PR）**：`chat_pipeline` 新增 `persona_id` 解析 + `find_for_character` 自动激活 + 优先级合同（request > session binding > character binding > default > inline user_profile）。这会改 `ChatCompletionRequest` contract，需独立审计。
- **A2（再下一个 PR）**：WebUI 多 Persona 列表/切换/绑定 UI，依赖 A1a endpoint。

A1a 独立可合并、可审计、可测，是 A1b/A2 的地基。

## 不在本计划范围

- `chat_pipeline` 消费 `persona_id` / `find_for_character`（→ A1b）
- WebUI 多 Persona UI（→ A2）
- lock/unlock、duplicate、rollback、history、export endpoint（#114 P2/P3）
- `PUT /v1/sessions/:character_id/:session_id/rp-profile`（会话级 persona 选择，→ A1b/A2）
- `PersonaService` 自身改动（service 层已完整，本计划只接线）

## File Structure

| 文件 | 改动 | 责任 |
|---|---|---|
| [engine/src/daemon/handlers.rs](file:///D:/AIRP-Dev/engine/src/daemon/handlers.rs) | 修改 | 新增 7 个 handler + 4 个 request/query struct；domain import 加 `PersonaBinding` |
| [engine/src/daemon/mod.rs](file:///D:/AIRP-Dev/engine/src/daemon/mod.rs) | 修改 | `use handlers::{...}` 加 7 个新 handler；`v1_routes` 加 3 条新 route |
| [docs/CURRENT-BASELINE.md](file:///D:/AIRP-Dev/docs/CURRENT-BASELINE.md) | 修改 | §1/§4 反映 A1a 交付与 A1b/A2 仍开放 |

无新文件。

## API 合同

| Method | Path | Handler | Body/Query | 成功响应 |
|---|---|---|---|---|
| GET | `/v1/users/:user_id/personas` | `list_personas_endpoint` | — | `200` `Vec<String>`（含 "default"） |
| POST | `/v1/users/:user_id/personas` | `create_persona_endpoint` | `{persona_id, name, description?, variables?}` | `200` `Persona`（revision=1） |
| GET | `/v1/users/:user_id/personas/:persona_id` | `get_persona_multi_endpoint` | — | `200` `Persona` |
| PUT | `/v1/users/:user_id/personas/:persona_id` | `update_persona_multi_endpoint` | `{expected_revision, name, description?, variables?}` | `200` `Persona`（revision bump） |
| DELETE | `/v1/users/:user_id/personas/:persona_id` | `delete_persona_multi_endpoint` | — | `204` |
| POST | `/v1/users/:user_id/personas/:persona_id/bindings` | `bind_persona_endpoint` | `{character_id, session_id?}` | `200` `Persona` |
| DELETE | `/v1/users/:user_id/personas/:persona_id/bindings` | `unbind_persona_endpoint` | query `?character_id=&session_id=` | `200` `Persona` |

**错误语义**（复用 `AirpError` 现有映射，无新变体）：

- persona_id 路径遍历 / 非法字符 → `400 bad_request`（`validate_id_segment`）
- POST 创建 `persona_id == "default"` → `400 bad_request`（handler 显式拒）
- POST 创建已存在 id → `400 bad_request`（`save` 的 `expected_revision=0` 与已存在 revision≥1 冲突，message 携带 `PersonaRevisionConflict` JSON）
- GET/PUT 非 default 且不存在 → `404 not_found`（`service.get`）
- PUT `expected_revision` 不匹配 → `400 bad_request`（`PersonaRevisionConflict`）
- DELETE `default` → `400 bad_request`（`service.delete`）
- bind/unbind 非法 `character_id` / `session_id` → `400 bad_request`（`CharacterId::new` / `SessionId::parse`）
- bind/unbind 目标 persona 不存在（非 default）→ `404 not_found`（`service.get`）

**向后兼容**：legacy singular `/v1/users/:id/persona` (GET/PUT) 保留不动，default persona 仍走老路径。plural `personas` 与 singular `persona` 是不同路径，路由无冲突。

---

### Task 1: 在 handlers.rs 新增 request structs + 7 个 handler

**Files:**
- Modify: `engine/src/daemon/handlers.rs`（domain import 行 5；新增 struct 与 handler 插在 `update_persona_endpoint` 之后、`UpdatePersonaRequest` 之后，约行 253 处）

- [ ] **Step 1: 修改 domain import，加入 `PersonaBinding`**

把 [handlers.rs:5](file:///D:/AIRP-Dev/engine/src/daemon/handlers.rs#L5) 的：

```rust
use crate::domain::{ChatService, LorebookService, Persona, PersonaService};
```

改为：

```rust
use crate::domain::{ChatService, LorebookService, Persona, PersonaBinding, PersonaService};
```

- [ ] **Step 2: 在 `UpdatePersonaRequest` 定义之后（约 [handlers.rs:253](file:///D:/AIRP-Dev/engine/src/daemon/handlers.rs#L253) 之后）新增 4 个 request/query struct**

```rust
/// POST /v1/users/:user_id/personas 的请求体（创建新 Persona，非 default）。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreatePersonaRequest {
    /// 新 Persona 的 ID；走 `validate_id_segment` 校验，拒路径遍历。
    /// "default" 不允许在此创建（用 legacy PUT /v1/users/:id/persona）。
    pub persona_id: String,
    /// 用户显示名。
    pub name: String,
    /// 自由描述。
    #[serde(default)]
    pub description: String,
    /// 自定义变量表，键名对应 prompt 中 `{{key}}` 占位符。
    #[serde(default)]
    pub variables: HashMap<String, String>,
}

/// PUT /v1/users/:user_id/personas/:persona_id 的请求体（更新指定 Persona）。
/// 与 legacy `UpdatePersonaRequest` 同形状，但路径带 `:persona_id`。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct UpdateMultiPersonaRequest {
    /// 客户端持有的当前 revision；不匹配服务端时返回 400 + PersonaRevisionConflict。
    pub expected_revision: u64,
    /// 用户显示名。
    pub name: String,
    /// 自由描述。
    #[serde(default)]
    pub description: String,
    /// 自定义变量表。
    #[serde(default)]
    pub variables: HashMap<String, String>,
}

/// POST /v1/users/:user_id/personas/:persona_id/bindings 的请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BindPersonaRequest {
    /// 绑定到的角色 ID；走 `CharacterId::new` 校验。
    pub character_id: String,
    /// 可选 session ID；`None` 表示该角色下所有会话通用。走 `SessionId::parse` 校验。
    #[serde(default)]
    pub session_id: Option<String>,
}

/// DELETE /v1/users/:user_id/personas/:persona_id/bindings 的 query 参数。
#[derive(Debug, Deserialize)]
pub(super) struct UnbindPersonaQuery {
    /// 必填：要移除绑定的角色 ID。
    pub character_id: String,
    /// 可选：要移除绑定的 session ID；省略表示移除该角色的全会话通用绑定。
    #[serde(default)]
    pub session_id: Option<String>,
}
```

- [ ] **Step 3: 在 `update_persona_endpoint` 之后（约 [handlers.rs:237](file:///D:/AIRP-Dev/engine/src/daemon/handlers.rs#L237) 之后，`UpdatePersonaRequest` 之前或之后均可，建议紧接 handler 组）新增 7 个 handler**

```rust
// ── Multi-Persona handlers（#114 A1a，多 Persona CRUD + 绑定）──────────────────
//
// PersonaService（PR #127）已交付 list/get/save/delete/bind/unbind/find_for_character；
// 本组 handler 把多 Persona 闭环暴露到 HTTP plural 路径。chat_pipeline 消费
// find_for_character 自动激活留 A1b（独立 PR，会改 ChatCompletionRequest contract）。

/// GET /v1/users/:user_id/personas — 列出该用户所有 Persona id（含 "default"）。
pub(super) async fn list_personas_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> Result<Json<Vec<String>>, AirpError> {
    let uid = UserId::new(user_id)?;
    let ids = PersonaService::new(&state.data_root).list(&uid)?;
    Ok(Json(ids))
}

/// POST /v1/users/:user_id/personas — 创建新 Persona（非 default）。
/// "default" 走 legacy PUT /v1/users/:id/persona；重复 id 由 revision 冲突拒绝。
pub(super) async fn create_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    Json(payload): Json<CreatePersonaRequest>,
) -> Result<Json<Persona>, AirpError> {
    if payload.persona_id == "default" {
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
    let saved = PersonaService::new(&state.data_root)
        .save(&uid, &payload.persona_id, 0, persona)?;
    Ok(Json(saved))
}

/// GET /v1/users/:user_id/personas/:persona_id — 读取指定 Persona。
/// default 不存在时返回 initial（不写盘）；非 default 不存在返回 404。
pub(super) async fn get_persona_multi_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let persona = PersonaService::new(&state.data_root).get(&uid, &persona_id, "User")?;
    Ok(Json(persona))
}

/// PUT /v1/users/:user_id/personas/:persona_id — 更新指定 Persona；保留 bindings。
pub(super) async fn update_persona_multi_endpoint(
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
pub(super) async fn delete_persona_multi_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
) -> Result<StatusCode, AirpError> {
    let uid = UserId::new(user_id)?;
    PersonaService::new(&state.data_root).delete(&uid, &persona_id)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /v1/users/:user_id/personas/:persona_id/bindings — 添加绑定（幂等）。
pub(super) async fn bind_persona_endpoint(
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
pub(super) async fn unbind_persona_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path((user_id, persona_id)): axum::extract::Path<(String, String)>,
    axum::extract::Query(query): axum::extract::Query<UnbindPersonaQuery>,
) -> Result<Json<Persona>, AirpError> {
    let uid = UserId::new(user_id)?;
    let updated = PersonaService::new(&state.data_root).unbind(
        &uid,
        &persona_id,
        &query.character_id,
        query.session_id.as_deref(),
    )?;
    Ok(Json(updated))
}
```

- [ ] **Step 4: 编译检查（handler 签名 + struct 正确，但路由未接，不会有未使用警告因为下一步接线）**

Run: `cargo check -p airp-core --lib`（在 PowerShell 设置 D 盘工具链环境变量后）
Expected: 编译通过（可能因 handler 未被 `use` 而产生 dead_code 警告，Task 2 接线后消失；若编译报错则修正签名）

环境变量前置（PowerShell）：
```powershell
$env:RUSTUP_HOME = "D:\.rustup"
$env:CARGO_HOME = "D:\.cargo"
$env:PATH = "D:\.cargo\bin;D:\msys64\mingw64\bin;D:\nodejs;" + $env:PATH
```

---

### Task 2: 在 daemon/mod.rs 接线路由 + import

**Files:**
- Modify: `engine/src/daemon/mod.rs`（`use handlers::{...}` 块 行 34-44；`v1_routes` 行 312-315 之后）

- [ ] **Step 1: 在 `use handlers::{...}` 块加入 7 个新 handler**

把 [mod.rs:34-44](file:///D:/AIRP-Dev/engine/src/daemon/mod.rs#L34-L44) 的 import 列表扩展。在 `get_persona_endpoint,` 之后追加新 handler，并保持字母序（与现有风格一致）。最终块为：

```rust
use handlers::{
    add_scene_character_endpoint, agent_run, bind_persona_endpoint, chat_completion,
    create_persona_endpoint, create_scene_endpoint, create_session_endpoint,
    delete_character_endpoint, delete_persona_multi_endpoint, delete_session_endpoint,
    get_character_avatar, get_character_card, get_character_lorebook, get_character_state,
    get_character_state_history, get_character_state_schema, get_chat_history,
    get_persona_endpoint, get_persona_multi_endpoint, get_preset_endpoint, get_scene_endpoint,
    get_settings, import_character, import_preset_endpoint, list_agent_tools, list_characters,
    list_models, list_personas_endpoint, list_presets_endpoint, list_scenes_endpoint,
    list_sessions_endpoint, reextract_character_assets, regen_chat, rollback_chat,
    unbind_persona_endpoint, update_character_card, update_character_lorebook,
    update_persona_endpoint, update_persona_multi_endpoint, update_settings,
};
```

- [ ] **Step 2: 在 `v1_routes` 中 legacy persona route 之后（[mod.rs:312-315](file:///D:/AIRP-Dev/engine/src/daemon/mod.rs#L312-L315) 之后）插入 3 条新 route**

在现有：
```rust
.route(
    "/v1/users/:user_id/persona",
    get(get_persona_endpoint).put(update_persona_endpoint),
)
```
之后追加：

```rust
.route(
    "/v1/users/:user_id/personas",
    get(list_personas_endpoint).post(create_persona_endpoint),
)
.route(
    "/v1/users/:user_id/personas/:persona_id",
    get(get_persona_multi_endpoint)
        .put(update_persona_multi_endpoint)
        .delete(delete_persona_multi_endpoint),
)
.route(
    "/v1/users/:user_id/personas/:persona_id/bindings",
    post(bind_persona_endpoint).delete(unbind_persona_endpoint),
)
```

- [ ] **Step 3: 编译通过**

Run: `cargo check -p airp-core --lib`
Expected: 编译通过，无 dead_code 警告（所有 handler 都被路由引用）。

---

### Task 3: 在 daemon/mod.rs tests 模块新增 HTTP 集成测试

**Files:**
- Modify: `engine/src/daemon/mod.rs`（`#[cfg(test)] mod tests` 内，紧接现有 `legacy_persona_update_preserves_schema_v2_bindings` 测试之后，约 [mod.rs:579](file:///D:/AIRP-Dev/engine/src/daemon/mod.rs#L579) 之后）

测试沿用现有 `make_state_with_key(None)` + `create_router(state).oneshot(...)` 模式（见 [mod.rs:527-579](file:///D:/AIRP-Dev/engine/src/daemon/mod.rs#L527-L579)）。`access_api_key = None` 时 auth_middleware 放行无 Authorization 请求，已由现有测试证明。

- [ ] **Step 1: 新增 list 与 create + get 测试**

```rust
#[tokio::test]
async fn list_personas_returns_default_only_for_fresh_user() {
    let state = make_state_with_key(None);
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/users/bob/personas")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let ids: Vec<String> = serde_json::from_slice(&body).unwrap();
    assert_eq!(ids, vec!["default".to_string()]);
}

#[tokio::test]
async fn create_persona_then_get_returns_it() {
    let state = make_state_with_key(None);
    let create_body = serde_json::json!({
        "persona_id": "alice-alt",
        "name": "Alice Alt",
        "description": "alt persona",
        "variables": {"mood": "happy"}
    })
    .to_string();
    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/alice/personas")
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let created: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(created.id, "alice-alt");
    assert_eq!(created.name, "Alice Alt");
    assert_eq!(created.revision, 1);

    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/users/alice/personas/alice-alt")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let fetched: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(fetched.name, "Alice Alt");
    assert_eq!(fetched.variables.get("mood").unwrap(), "happy");
}
```

- [ ] **Step 2: 新增 create 错误用例测试（default / 重复 / 路径遍历）**

```rust
#[tokio::test]
async fn create_persona_rejects_default_id() {
    let state = make_state_with_key(None);
    let body = serde_json::json!({"persona_id":"default","name":"D","description":"","variables":{}}).to_string();
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_persona_rejects_duplicate() {
    let state = make_state_with_key(None);
    let body = serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}}).to_string();
    let first = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let second = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn create_persona_rejects_path_traversal() {
    let state = make_state_with_key(None);
    let body = serde_json::json!({"persona_id":"../etc","name":"X","description":"","variables":{}}).to_string();
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 3: 新增 update + delete 测试（含保留 bindings / 404 / default 不可删）**

```rust
#[tokio::test]
async fn update_persona_bumps_revision_and_preserves_bindings() {
    let state = make_state_with_key(None);
    let create_body = serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}}).to_string();
    let _ = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let bind_body = serde_json::json!({"character_id":"char-a"}).to_string();
    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas/p1/bindings")
                .header("content-type", "application/json")
                .body(Body::from(bind_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let after_bind: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(after_bind.bindings.len(), 1);
    let rev = after_bind.revision;

    let update_body = serde_json::json!({"expected_revision":rev,"name":"P1-renamed","description":"d","variables":{}}).to_string();
    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/users/u/personas/p1")
                .header("content-type", "application/json")
                .body(Body::from(update_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let updated: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(updated.name, "P1-renamed");
    assert_eq!(updated.revision, rev + 1);
    assert_eq!(updated.bindings.len(), 1);
    assert_eq!(updated.bindings[0].character_id, "char-a");
}

#[tokio::test]
async fn update_persona_rejects_wrong_expected_revision() {
    let state = make_state_with_key(None);
    let create_body = serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}}).to_string();
    let _ = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let update_body = serde_json::json!({"expected_revision":99,"name":"X","description":"","variables":{}}).to_string();
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/users/u/personas/p1")
                .header("content-type", "application/json")
                .body(Body::from(update_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_nonexistent_non_default_returns_404() {
    let state = make_state_with_key(None);
    let body = serde_json::json!({"expected_revision":0,"name":"X","description":"","variables":{}}).to_string();
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("PUT")
                .uri("/v1/users/u/personas/ghost")
                .header("content-type", "application/json")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_persona_removes_it_and_default_rejected() {
    let state = make_state_with_key(None);
    let create_body = serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}}).to_string();
    let _ = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/v1/users/u/personas/p1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/users/u/personas/p1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/v1/users/u/personas/default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 4: 新增 bind/unbind 测试（含幂等 + 非法 character_id）**

```rust
#[tokio::test]
async fn bind_persona_is_idempotent_and_unbind_removes_it() {
    let state = make_state_with_key(None);
    let create_body = serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}}).to_string();
    let _ = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let bind_body = serde_json::json!({"character_id":"char-a"}).to_string();
    let r1 = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas/p1/bindings")
                .header("content-type", "application/json")
                .body(Body::from(bind_body.clone()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r1.status(), StatusCode::OK);
    let body = axum::body::to_bytes(r1.into_body(), 4096).await.unwrap();
    let after_first: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(after_first.bindings.len(), 1);
    let rev_after_first = after_first.revision;

    // 幂等：第二次 bind 同一目标不 bump revision。
    let r2 = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas/p1/bindings")
                .header("content-type", "application/json")
                .body(Body::from(bind_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r2.status(), StatusCode::OK);
    let body = axum::body::to_bytes(r2.into_body(), 4096).await.unwrap();
    let after_second: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(after_second.bindings.len(), 1);
    assert_eq!(after_second.revision, rev_after_first);

    let response = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/v1/users/u/personas/p1/bindings?character_id=char-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let after_unbind: crate::domain::Persona = serde_json::from_slice(&body).unwrap();
    assert_eq!(after_unbind.bindings.len(), 0);

    // 幂等：再 unbind 同一目标不报错、不 bump revision。
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("DELETE")
                .uri("/v1/users/u/personas/p1/bindings?character_id=char-a")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn bind_rejects_invalid_character_id() {
    let state = make_state_with_key(None);
    let create_body = serde_json::json!({"persona_id":"p1","name":"P1","description":"","variables":{}}).to_string();
    let _ = create_router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas")
                .header("content-type", "application/json")
                .body(Body::from(create_body))
                .unwrap(),
        )
        .await
        .unwrap();

    let bind_body = serde_json::json!({"character_id":"bad/path"}).to_string();
    let response = create_router(state)
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/users/u/personas/p1/bindings")
                .header("content-type", "application/json")
                .body(Body::from(bind_body))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
```

- [ ] **Step 5: 运行新测试，全部通过**

Run: `cargo test -p airp-core --lib daemon::tests`
Expected: 所有新测试 PASS，原有 `legacy_persona_update_preserves_schema_v2_bindings` 继续通过。

---

### Task 4: 全量验证（test + clippy + fmt + 神圣不变式）

**Files:** 无改动，仅运行。

- [ ] **Step 1: workspace 全量测试**

Run: `cargo test --workspace`
Expected: 全绿（含 engine lib / integration / protocol / UI suites）。新增 8 个测试通过。

- [ ] **Step 2: clippy 零警告**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: 无警告。

- [ ] **Step 3: fmt 检查**

Run: `cargo fmt --all -- --check`
Expected: 无 diff。若有 diff，运行 `cargo fmt --all` 后重新检查。

- [ ] **Step 4: 神圣不变式单独精确运行**

Run: `cargo test -p airp-core --lib agent::tests::subagent_context_has_no_orchestrator_noise`
Expected: 1 passed。

- [ ] **Step 5: 若 clippy/fmt 改动了格式，回到 Task 1/2 微调后重跑 Step 1-4 直到全绿**

---

### Task 5: 更新 CURRENT-BASELINE.md + 提交

**Files:**
- Modify: `docs/CURRENT-BASELINE.md`

- [ ] **Step 1: 更新 §1「当前可用能力」**

在 [CURRENT-BASELINE.md:19](file:///D:/AIRP-Dev/docs/CURRENT-BASELINE.md#L19)（PR #127 多 Persona 存储那条）之后追加一条：

```markdown
- 多 Persona HTTP API（A1a）已交付：`GET/POST /v1/users/:id/personas`、`GET/PUT/DELETE /v1/users/:id/personas/:persona_id`、`POST/DELETE /v1/users/:id/personas/:persona_id/bindings`。handler 全部委托到 `PersonaService`（PR #127），legacy singular `/persona` endpoint 保留。`chat_pipeline` 消费 `persona_id`/`find_for_character` 自动激活仍开放（A1b）。
```

- [ ] **Step 2: 更新 §4「当前开放风险/issue 分组」RP Profile 行**

把 [CURRENT-BASELINE.md:45](file:///D:/AIRP-Dev/docs/CURRENT-BASELINE.md#L45) 的 RP Profile 行调整为：

```markdown
- RP Profile/诊断：#114、#115、#116、#117、#126；#114 已有基础 Preset 与多 Persona 存储地基，且多 Persona HTTP CRUD+绑定闭环（A1a）已交付，但 `chat_pipeline` 按 `persona_id`/`find_for_character` 自动激活（A1b）、WebUI 多 Persona UI（A2）、完整绑定闭环与 worldbook shared normalization（constant 已交付，仍开放的语义包括但不限于 selective/secondary_keys/position/depth）仍未交付。
```

- [ ] **Step 3: 提交**

```bash
git add engine/src/daemon/handlers.rs engine/src/daemon/mod.rs docs/CURRENT-BASELINE.md docs/PERSONA-HTTP-API-PLAN.md
git commit -m "$(cat <<'EOF'
feat(engine): expose multi-persona HTTP API (list/get/save/delete/bind/unbind)

PersonaService (PR #127) already implemented list/get/save/delete/bind/unbind/
find_for_character, but HTTP only exposed the default path. Add 7 plural-path
endpoints under /v1/users/:id/personas that delegate to the existing service,
plus 8 HTTP integration tests covering CRUD, revision conflict, path traversal,
binding idempotency and invalid character_id. Legacy singular /persona endpoint
is preserved. chat_pipeline persona_id resolution (A1b) and WebUI UI (A2) remain
follow-up PRs.
EOF
)"
```

---

## Self-Review

**1. Spec coverage（对照 #114 A1a 范围）**

- 多 Persona HTTP endpoint（list/get/save/delete/bind/unbind）→ Task 1+2 全部覆盖（7 endpoint）。
- HTTP 集成测试覆盖 CRUD、revision conflict、path traversal、绑定幂等、非法输入 → Task 3 覆盖（8 测试）。
- `chat_pipeline` 消费 `find_for_character` → **明确不在本计划范围**（A1b），范围说明已声明。
- WebUI UI → **不在范围**（A2），范围说明已声明。
- `PersonaService` 复用、不重复 handler 私有转换 → handler 全部委托 service，零新业务逻辑，满足 #114 "shared service → adapter → UI 边界"。
- 路径遍历防护 → 复用 `validate_id_segment`（persona_id）+ `CharacterId::new`/`SessionId::parse`（binding），测试 `create_persona_rejects_path_traversal` + `bind_rejects_invalid_character_id` 覆盖。
- 神圣不变式 → Task 4 Step 4 单独验证。

**2. Placeholder scan**

- 无 TBD/TODO/"implement later"。
- 每个 handler 与测试均给出完整代码。
- 错误处理复用 `AirpError` 现有变体，无 "add appropriate error handling" 含糊语。
- 命令均带 expected output。

**3. Type consistency**

- `Persona` 字段使用与 [domain.rs:676-697](file:///D:/AIRP-Dev/engine/src/domain.rs#L676-L697) 一致：`schema/revision/updated_at/name/description/variables/id/bindings`。
- `PersonaBinding { character_id, session_id: Option<String> }` 与 [domain.rs:723-729](file:///D:/AIRP-Dev/engine/src/domain.rs#L723-L729) 一致。
- `PersonaService::save(user_id, persona_id, expected_revision, persona)` 签名与 [domain.rs:840-846](file:///D:/AIRP-Dev/engine/src/domain.rs#L840-L846) 一致。
- `PersonaService::unbind(user_id, persona_id, character_id, Option<&str>)` 签名与 [domain.rs:925-931](file:///D:/AIRP-Dev/engine/src/domain.rs#L925-L931) 一致。
- `AirpError::BadRequest`/`NotFound` 与 [error.rs:40,44](file:///D:/AIRP-Dev/engine/src/error.rs#L40) 一致，状态映射 400/404。
- handler 命名 `list_personas_endpoint`/`create_persona_endpoint`/`get_persona_multi_endpoint`/`update_persona_multi_endpoint`/`delete_persona_multi_endpoint`/`bind_persona_endpoint`/`unbind_persona_endpoint` 在 Task 1（定义）、Task 2（import + route）、Task 3（测试隐式经路由调用）中一致。

## 执行交接

计划已保存到 `docs/PERSONA-HTTP-API-PLAN.md`。两种执行方式：

1. **Inline Execution（推荐，本 session 直接执行）** — 使用 executing-plans skill，按 Task 1→5 顺序执行，每个 Task 完成后 checkpoint。
2. **Subagent-Driven** — 每个 Task 派一个 fresh subagent，task 间 review。

本计划范围收敛（2 个源文件 + 1 个 docs 文件），建议 Inline Execution。
