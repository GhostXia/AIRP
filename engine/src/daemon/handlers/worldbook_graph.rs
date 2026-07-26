//! Phase 4.4: 世界书知识图谱 HTTP handler。
//!
//! 端点：`GET /v1/characters/:character_id/lorebook/graph`
//!
//! 读取角色 lorebook.json，调用 `worldbook_graph::build_graph` 生成
//! 节点/边/冲突警告，返回 JSON 供 WebUI 力导向图渲染。

use crate::daemon::DaemonState;
use crate::error::AirpError;
use crate::types::CharacterId;
use crate::worldbook_graph::{build_graph, GraphQuery};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use std::sync::Arc;

/// Phase 4.4: `GET /v1/characters/:character_id/lorebook/graph`
///
/// 查询参数（均可选，默认全 true）：
/// - `include_references=true/false` — 是否包含 content→key 引用边
/// - `include_key_overlap=true/false` — 是否包含 key 重叠边
/// - `detect_conflicts=true/false` — 是否检测设定冲突
/// - `min_weight=N` — 最小权重阈值（默认 1）
pub(in crate::daemon) async fn get_lorebook_graph_endpoint(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Query(query): Query<GraphQuery>,
) -> impl IntoResponse {
    // R1: 把同步 fs + 解析 + build_graph 放到 spawn_blocking，避免阻塞 axum
    // runtime 线程。lorebook.json 可能达到数百 KB（500 条 entry），读取与
    // 反序列化在繁忙 daemon 上会显著延迟其它请求。
    // R2: 直接把 WorldbookGraph 交给 Json（WorldbookGraph 已实现 Serialize），
    // 不再绕一层 serde_json::to_value → Value → Json，减少一次序列化拷贝。
    let state = state.clone();
    match tokio::task::spawn_blocking(move || run_get_graph_sync(&state, &character_id, query))
        .await
    {
        Ok(Ok(graph)) => (StatusCode::OK, Json(graph)).into_response(),
        Ok(Err(e)) => e.into_response(),
        Err(join_err) => {
            AirpError::Internal(format!("lorebook graph task join failed: {}", join_err))
                .into_response()
        }
    }
}

fn run_get_graph_sync(
    state: &DaemonState,
    character_id: &str,
    query: GraphQuery,
) -> Result<crate::worldbook_graph::WorldbookGraph, AirpError> {
    let cid = CharacterId::new(character_id)?;

    // 校验角色存在
    let exists = crate::data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }

    // 读取 lorebook.json
    let lorebook_path = crate::data_dir::char_world_lorebook_path(&state.data_root, cid.as_str());
    let lorebook_text = std::fs::read_to_string(&lorebook_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AirpError::NotFound(format!(
                "character {} has no lorebook (world/lorebook.json missing)",
                cid
            ))
        } else {
            AirpError::from(e)
        }
    })?;

    let lorebook: crate::orchestrator::lorebook::Lorebook = serde_json::from_str(&lorebook_text)
        .map_err(|e| AirpError::BadRequest(format!("lorebook.json 解析失败: {}", e)))?;

    build_graph(cid.as_str(), &lorebook, &query)
}
