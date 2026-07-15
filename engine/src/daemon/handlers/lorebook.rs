//! Character lorebook HTTP handlers — read + update canonical lorebook JSON.
//!
//! #155 PR6：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 service orchestration；lorebook 归一化由 `normalize_worldbook` 完成，
//! 读写落盘在 `LorebookService`。
//!
//! 端点：
//! - `GET /v1/characters/:character_id/lorebook` — 返回角色级世界书 JSON（不存在 → 404）
//! - `PUT /v1/characters/:character_id/lorebook` — 整体替换世界书（接受三种 body 形式）

use crate::daemon::DaemonState;
use crate::data_dir;
use crate::domain::LorebookService;
use crate::error::AirpError;
use crate::types::CharacterId;
use axum::Json;
use std::sync::Arc;

/// GET /v1/characters/:character_id/lorebook — 返回角色级世界书 JSON。
/// 不存在 → 404（与空对象 {} 区分）。
pub(in crate::daemon) async fn get_character_lorebook(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
) -> Result<Json<serde_json::Value>, AirpError> {
    // #67 #5 fix: 改用 Result<Json<Value>, AirpError> 统一错误格式。
    // 之前返回 Response + 裸 StatusCode::BAD_REQUEST，客户端 formatError 拿不到结构化 error body。
    let char_id = CharacterId::new(character_id)?;
    let lorebook = LorebookService::new(&state.data_root).read(&char_id)?;
    Ok(Json(serde_json::to_value(lorebook)?))
}

/// PUT /v1/characters/:character_id/lorebook — 更新角色级世界书（整体替换）。
///
/// body 接受三种形式（由 [`normalize_worldbook`] 统一归一化）：
/// - AIRP canonical Lorebook JSON（幂等）
/// - SillyTavern lorebook / character_book entries（含 `disable`/`order`/
///   `keysecondary`/`caseSensitive` 等别名）
/// - 裸 entry 数组
///
/// 返回写入的 canonical Lorebook 条目数 + 归一化诊断报告。
/// 角色不存在 → 404。
pub(in crate::daemon) async fn update_character_lorebook(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(character_id): axum::extract::Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    // 校验角色已存在
    let exists = data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }
    let (lorebook, report) = crate::orchestrator::normalize_worldbook(&body);
    if let Some(reason) = report.replacement_error() {
        return Err(AirpError::BadRequest(format!("invalid lorebook: {reason}")));
    }
    LorebookService::new(&state.data_root).write(&cid, &lorebook)?;
    Ok(Json(serde_json::json!({
        "character_id": cid.as_str(),
        "entries_count": lorebook.entries.len(),
        "import_report": report,
        "status": "ok"
    })))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    // ── PR #74 W-01: get_character_lorebook HTTP-level 回归测试 ─────────────
    //
    // 守 #67 #5 修复：handler 改为 `Result<Json<Value>, AirpError>` 后，错误响应
    // 必须是 JSON envelope（`{"error":{"code","message"}}`），不能是裸 StatusCode。
    // 复用 make_state_for_http_test，3 个 case 覆盖主要分支。

    fn make_state_for_http_test() -> (Arc<crate::daemon::DaemonState>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let state = Arc::new(crate::daemon::DaemonState {
            data_root: tmp.path().to_path_buf(),
            http_client: reqwest::Client::new(),
            config: std::sync::RwLock::new(crate::daemon::MutableConfig {
                provider: crate::adapter::Provider::OpenAI,
                endpoint: "http://localhost".to_string(),
                api_key: None,
                model: "gpt-4o".to_string(),
                volume_config: crate::config::VolumeConfig::default(),
                access_api_key: None,
                engine: crate::adapter::BackendEngine::default(),
                quota: crate::quota::QuotaConfig::default(),
                deployment_mode: Default::default(),
                public_origin: None,
            }),
        });
        (state, tmp)
    }

    #[tokio::test]
    async fn pr74_lorebook_not_found_returns_json_envelope() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/does_not_exist/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::NOT_FOUND);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/json"
        );
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["error"]["code"], "not_found");
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("does_not_exist"),
            "错误 message 应含 character_id，got: {}",
            v["error"]["message"]
        );
    }

    #[tokio::test]
    async fn pr74_lorebook_invalid_character_id_returns_400_envelope() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state);
        // 含路径遍历字符 → CharacterId::new 校验失败 → BadRequest
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/..%2Fetc/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["error"]["code"], "bad_request");
    }

    #[tokio::test]
    async fn pr74_lorebook_happy_path_returns_json_value() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, tmp) = make_state_for_http_test();
        // 在 data_root 下放一个合法 lorebook 文件
        let char_dir = tmp.path().join("characters").join("test_char");
        std::fs::create_dir_all(char_dir.join("world")).unwrap();
        std::fs::write(
            char_dir.join("world").join("lorebook.json"),
            r#"{"entries":[{"keys":["hi"],"content":"hello"}]}"#,
        )
        .unwrap();

        let app = crate::daemon::create_router(state);
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("GET")
                    .uri("/v1/characters/test_char/lorebook")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let body_bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(v["entries"][0]["keys"][0], "hi");
        assert_eq!(v["entries"][0]["content"], "hello");
    }

    #[tokio::test]
    async fn lorebook_put_rejects_invalid_replacement_without_overwrite() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, tmp) = make_state_for_http_test();
        let world_dir = tmp
            .path()
            .join("characters")
            .join("test_char")
            .join("world");
        std::fs::create_dir_all(&world_dir).unwrap();
        let lorebook_path = world_dir.join("lorebook.json");
        let original = r#"{"entries":[{"keys":["safe"],"content":"keep me"}]}"#;
        std::fs::write(&lorebook_path, original).unwrap();

        let response = crate::daemon::create_router(state)
            .oneshot(
                axum::http::Request::builder()
                    .method("PUT")
                    .uri("/v1/characters/test_char/lorebook")
                    .header("Content-Type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "entries": [{"keys": ["bad"], "content": 42}]
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(std::fs::read_to_string(lorebook_path).unwrap(), original);
    }
}
