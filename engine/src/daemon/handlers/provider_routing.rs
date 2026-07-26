//! Phase 5.1: Multi-Provider Routing HTTP handlers.
//!
//! 端点：
//! - `GET  /v1/providers` — 列出所有 provider 条目（`api_key` 脱敏为 `api_key_set: bool`）
//! - `POST /v1/providers` — 用整段 `ProvidersUpdate` 替换 providers + routing 配置
//!   （body limit 2MB；`api_key` 字段写入 `data/provider_keys.json`，不写入 `data/providers.json`）
//! - `GET  /v1/provider-routing` — 返回当前路由策略表
//! - `PUT  /v1/provider-routing` — 仅替换路由策略表（不动 providers 数组）
//!
//! 所有写操作都是 atomic：先校验 → 持久化到磁盘 → 提交到内存 `RwLock<ProviderRouter>`。
//! 持久化失败时不会污染内存状态。

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::provider_routing::{
    load_provider_keys, save_provider_keys, save_provider_routing, validate_provider_config,
    ProviderEntry, ProviderRouter, ProviderRouting,
};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

// ── 视图模型（脱敏） ─────────────────────────────────────────────────────────

/// `GET /v1/providers` 返回的单条 provider 条目（`api_key` 脱敏为 `api_key_set`）。
#[derive(Debug, Serialize)]
pub struct ProviderEntryView {
    pub name: String,
    pub endpoint: String,
    pub model: String,
    pub engine: crate::adapter::BackendEngine,
    pub is_default: bool,
    /// 是否已设置 api_key（不返回 key 本体）。
    pub api_key_set: bool,
}

impl ProviderEntryView {
    fn from_entry(entry: &ProviderEntry) -> Self {
        Self {
            name: entry.name.clone(),
            endpoint: entry.endpoint.clone(),
            model: entry.model.clone(),
            engine: entry.engine.clone(),
            is_default: entry.is_default,
            api_key_set: entry.api_key.as_deref().is_some_and(|s| !s.is_empty()),
        }
    }
}

/// `GET /v1/providers` 返回体。
#[derive(Debug, Serialize)]
pub struct ProvidersResponse {
    pub entries: Vec<ProviderEntryView>,
    pub routing: ProviderRouting,
    /// 多 provider 路由是否启用（entries 非空）。
    pub enabled: bool,
}

// ── 请求模型 ─────────────────────────────────────────────────────────────────

/// `POST /v1/providers` 请求体。
///
/// `api_key` 字段可选；为空字符串或 None 时视为未设置（不会写入 `provider_keys.json`）。
/// `api_key` 不会持久化到 `data/providers.json`（`ProviderEntry.api_key` 已 `#[serde(skip)]`）。
#[derive(Debug, Deserialize)]
pub struct ProvidersUpdate {
    pub entries: Vec<ProviderEntry>,
    #[serde(default)]
    pub routing: ProviderRouting,
}

/// `PUT /v1/provider-routing` 请求体。
#[derive(Debug, Deserialize)]
pub struct RoutingUpdate {
    pub routing: ProviderRouting,
}

// ── handlers ────────────────────────────────────────────────────────────────

/// `GET /v1/providers` — 列出当前所有 provider 条目 + 路由策略（api_key 脱敏）。
pub(in crate::daemon) async fn list_providers_endpoint(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<ProvidersResponse>, AirpError> {
    let router = state
        .provider_router
        .read()
        .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
    let entries: Vec<ProviderEntryView> = router
        .entries()
        .iter()
        .map(ProviderEntryView::from_entry)
        .collect();
    let routing = router.routing().clone();
    let enabled = !router.is_empty();
    Ok(Json(ProvidersResponse {
        entries,
        routing,
        enabled,
    }))
}

/// `POST /v1/providers` — 替换整段 providers + routing 配置。
///
/// 流程：
/// 1. 反序列化 + 校验（`validate_provider_config`）。
/// 2. 提取 `api_key` 到独立的 HashMap（不入 `data/providers.json`）。
///    **Critical4, 2026-07-26**：`api_key` 字段语义：
///    - `None`（JSON 中省略）→ 保留服务端已有 key（preserve-on-edit）
///    - `Some("")` → 清空 key
///    - `Some("xyz")` → 更新 key
/// 3. 持久化 `data/providers.json` + `data/provider_keys.json`（atomic via `replace_file`）。
/// 4. 提交到 `RwLock<ProviderRouter>` 内存状态。
///
/// 持久化失败时不会污染内存（先写盘后改内存）。
///
/// **Major1, 2026-07-26**：通过 `provider_routing_update` 协调器串行化
/// read-persist-commit，避免并发 POST/PUT 造成盘-内存不一致。
pub(in crate::daemon) async fn update_providers_endpoint(
    State(state): State<Arc<DaemonState>>,
    Json(update): Json<ProvidersUpdate>,
) -> Result<(StatusCode, Json<ProvidersResponse>), AirpError> {
    // 1) 校验
    validate_provider_config(&update.entries, &update.routing)
        .map_err(|e| AirpError::BadRequest(format!("providers 配置不合法: {e}")))?;

    // Major1: 串行化 read-persist-commit。
    let _lock = state.provider_routing_update.lock().await;

    // 2) 提取 api_keys（Critical4: 保留未显式清空的 key）
    // 先加载服务端已有 keys，对每个 entry：
    //   - api_key = None → 保留已有 key（如有）
    //   - api_key = Some("") → 清空
    //   - api_key = Some("xyz") → 更新
    let existing_keys = load_provider_keys(&state.data_root)?;
    let mut keys: HashMap<String, String> = HashMap::new();
    for entry in &update.entries {
        match &entry.api_key {
            None => {
                // 保留服务端已有 key
                if let Some(k) = existing_keys.get(&entry.name) {
                    keys.insert(entry.name.clone(), k.clone());
                }
            }
            Some(new_key) if !new_key.is_empty() => {
                // 更新 key
                keys.insert(entry.name.clone(), new_key.clone());
            }
            Some(_) => {
                // 空字符串 → 清空（不插入 map，save_provider_keys 会写入过滤后的 map）
            }
        }
    }

    // 3) 持久化（spawn_blocking 避免 tokio I/O 阻塞）
    let data_root = state.data_root.clone();
    let entries_clone = update.entries.clone();
    let routing_clone = update.routing.clone();
    let keys_clone = keys.clone();
    tokio::task::spawn_blocking(move || {
        save_provider_routing(&data_root, &entries_clone, &routing_clone)?;
        save_provider_keys(&data_root, &keys_clone)?;
        Ok::<(), AirpError>(())
    })
    .await
    .map_err(|e| AirpError::Internal(format!("provider 持久化任务失败: {e}")))??;

    // 4) 提交到内存（注入 api_key 供 router 内部使用）
    let mut entries_for_memory = update.entries;
    for entry in entries_for_memory.iter_mut() {
        if let Some(k) = keys.get(&entry.name) {
            entry.api_key = Some(k.clone());
        } else {
            entry.api_key = None;
        }
    }
    let new_router = ProviderRouter::new(entries_for_memory, update.routing);
    let view_entries: Vec<ProviderEntryView> = new_router
        .entries()
        .iter()
        .map(ProviderEntryView::from_entry)
        .collect();
    let view_routing = new_router.routing().clone();
    let enabled = !new_router.is_empty();

    let mut router_lock = state
        .provider_router
        .write()
        .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
    *router_lock = new_router;

    Ok((
        StatusCode::OK,
        Json(ProvidersResponse {
            entries: view_entries,
            routing: view_routing,
            enabled,
        }),
    ))
}

/// `GET /v1/provider-routing` — 返回当前路由策略表。
pub(in crate::daemon) async fn get_routing_endpoint(
    State(state): State<Arc<DaemonState>>,
) -> Result<Json<ProviderRouting>, AirpError> {
    let router = state
        .provider_router
        .read()
        .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
    Ok(Json(router.routing().clone()))
}

/// `PUT /v1/provider-routing` — 仅替换路由策略表（不动 providers 数组）。
///
/// 校验：新 routing 必须指向已存在的 provider name。
///
/// **Major1, 2026-07-26**：与 `update_providers_endpoint` 共享
/// `provider_routing_update` 协调器，串行化 read-persist-commit。
pub(in crate::daemon) async fn update_routing_endpoint(
    State(state): State<Arc<DaemonState>>,
    Json(update): Json<RoutingUpdate>,
) -> Result<Json<ProviderRouting>, AirpError> {
    // Major1: 串行化 read-persist-commit。
    let _lock = state.provider_routing_update.lock().await;

    // 1) 读当前 entries，与新 routing 一起校验
    let (entries, current_routing) = {
        let router = state
            .provider_router
            .read()
            .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
        (router.entries().to_vec(), router.routing().clone())
    };
    if entries.is_empty() {
        return Err(AirpError::BadRequest(
            "providers 数组为空，无法更新 routing（请先 POST /v1/providers 添加 provider"
                .to_string(),
        ));
    }
    validate_provider_config(&entries, &update.routing)
        .map_err(|e| AirpError::BadRequest(format!("routing 配置不合法: {e}")))?;

    // 2) 持久化
    let data_root = state.data_root.clone();
    let entries_clone = entries.clone();
    let routing_clone = update.routing.clone();
    tokio::task::spawn_blocking(move || {
        save_provider_routing(&data_root, &entries_clone, &routing_clone)
    })
    .await
    .map_err(|e| AirpError::Internal(format!("routing 持久化任务失败: {e}")))??;

    // 3) 提交到内存
    let new_router = ProviderRouter::new(entries, update.routing.clone());
    let view_routing = new_router.routing().clone();
    let mut router_lock = state
        .provider_router
        .write()
        .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
    *router_lock = new_router;

    // 抑制未使用变量警告（current_routing 保留以便未来审计日志记录差异）
    let _ = current_routing;

    Ok(Json(view_routing))
}

/// `GET /v1/providers/resolve?character_id=...&scene_role=...&task_kind=...`
///
/// 调试用端点：根据 RouteContext 解析命中的 provider（脱敏）。
/// 任一查询参数缺省时视为 None。
#[derive(Debug, Deserialize)]
pub struct ResolveQuery {
    pub character_id: Option<String>,
    pub scene_role: Option<String>,
    pub task_kind: Option<String>,
}

/// `GET /v1/providers/resolve` 返回体。
#[derive(Debug, Serialize)]
pub struct ResolveResponse {
    pub matched: bool,
    pub entry: Option<ProviderEntryView>,
    pub matched_rule: Option<&'static str>,
}

pub(in crate::daemon) async fn resolve_provider_endpoint(
    State(state): State<Arc<DaemonState>>,
    axum::extract::Query(query): axum::extract::Query<ResolveQuery>,
) -> Result<Json<ResolveResponse>, AirpError> {
    let router = state
        .provider_router
        .read()
        .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
    let ctx = crate::provider_routing::RouteContext {
        character_id: query.character_id,
        scene_role: query.scene_role,
        task_kind: query.task_kind,
    };
    let resolved = router.resolve(&ctx);
    let (matched, entry, matched_rule) = match resolved {
        Some(r) => (
            true,
            Some(ProviderEntryView::from_entry(r.entry)),
            Some(match r.matched_rule {
                crate::provider_routing::MatchedRule::Character => "character",
                crate::provider_routing::MatchedRule::SceneRole => "scene_role",
                crate::provider_routing::MatchedRule::TaskKind => "task_kind",
                crate::provider_routing::MatchedRule::Default => "default",
                crate::provider_routing::MatchedRule::FirstDefault => "first_default",
            }),
        ),
        None => (false, None, None),
    };
    Ok(Json(ResolveResponse {
        matched,
        entry,
        matched_rule,
    }))
}

// ── 共享内部辅助 ─────────────────────────────────────────────────────────────

/// 重新从磁盘加载 providers + keys，更新内存 router。
///
/// 当前未在 handler 中直接调用（POST/PUT 路径已 inline 完成等价工作），
/// 保留为未来 admin / CLI 工具刷新内存状态的入口。
#[allow(dead_code)]
pub(in crate::daemon) fn reload_provider_router_from_disk(
    state: &DaemonState,
) -> Result<(), AirpError> {
    let mut entries: Vec<ProviderEntry> = {
        let router = state
            .provider_router
            .read()
            .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
        router.entries().to_vec()
    };
    let routing = {
        let router = state
            .provider_router
            .read()
            .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
        router.routing().clone()
    };
    let keys = load_provider_keys(&state.data_root)?;
    for entry in entries.iter_mut() {
        if let Some(key) = keys.get(&entry.name) {
            entry.api_key = Some(key.clone());
        }
    }
    validate_provider_config(&entries, &routing)
        .map_err(|e| AirpError::BadRequest(format!("重载后校验失败: {e}")))?;
    let new_router = ProviderRouter::new(entries, routing);
    let mut router_lock = state
        .provider_router
        .write()
        .map_err(|_| AirpError::Internal("provider_router lock poisoned".to_string()))?;
    *router_lock = new_router;
    Ok(())
}

// 处理 IntoResponse trait 兜底（保留 AirpError 自动转换路径，无需手动实现）。
#[allow(dead_code)]
fn _assert_into_response() -> impl IntoResponse {
    AirpError::Internal("unreachable".to_string())
}
