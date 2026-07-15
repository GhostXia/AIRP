//! Preset HTTP handlers — list / get / import.
//!
//! #155 PR5：从 `handlers.rs` 原样迁移，零行为变更。handler 只做 HTTP extraction
//! 与 preset orchestration；`TavernPreset` 校验和 `replace_file` 原子写在 `data_dir` 模块。
//!
//! 端点：
//! - `GET  /v1/presets` — 列出所有 preset 文件名
//! - `GET  /v1/presets/:preset_id` — 返回 preset 的 prompts 数组
//! - `POST /v1/presets/import` — 校验 + 落盘一份 preset JSON

use crate::daemon::DaemonState;
use crate::data_dir;
use crate::error::AirpError;
use crate::types::PresetId;
use axum::Json;
use serde::{Deserialize, Serialize};
use std::fs;
use std::sync::{Arc, Mutex, OnceLock};

// Preset imports are infrequent administrative writes. A single process-wide
// lock keeps the fixed temporary/backup names safe without retaining an
// unbounded per-preset lock map.
static PRESET_IMPORT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

/// GET /v1/presets — list all available preset file names under data/presets/
pub(in crate::daemon) async fn list_presets_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
) -> Result<Json<Vec<String>>, AirpError> {
    let presets = data_dir::list_presets(&state.data_root)?;
    Ok(Json(presets))
}

/// GET /v1/presets/:preset_id — get all prompts of a preset
pub(in crate::daemon) async fn get_preset_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    axum::extract::Path(preset_id): axum::extract::Path<String>,
) -> Result<Json<Vec<crate::orchestrator::TavernPrompt>>, AirpError> {
    let preset_id = PresetId::new(preset_id)?;
    let normalized_path = data_dir::preset_json_path(&state.data_root, preset_id.as_str());
    let legacy_path = data_dir::legacy_preset_json_path(&state.data_root, preset_id.as_str());
    let preset_path = if normalized_path.exists() {
        normalized_path
    } else {
        legacy_path
    };
    if !preset_path.exists() {
        return Err(AirpError::NotFound(format!(
            "Preset {} not found",
            preset_id
        )));
    }
    let json_str = fs::read_to_string(&preset_path)?;
    let preset: crate::orchestrator::TavernPreset = serde_json::from_str(&json_str)
        .map_err(|e| AirpError::BadRequest(format!("Invalid preset JSON: {}", e)))?;

    Ok(Json(preset.prompts.unwrap_or_default()))
}

// ── Preset import（#114，WEBUI-MVP-PLAN §3.1：最小 JSON 导入）────────────────────
//
// 接收 `{preset_id, preset_json}`，校验 TavernPreset schema 后原子写盘到
// `presets/{id}/preset.json`。保留 raw sidecar（原样写盘，不解释 prompt 内容），
// 拒绝脚本执行和路径输入（preset_id 走 PresetId::new 校验，preset_json 走 serde
// 反序列化 TavernPreset）。rename/duplicate/export、字段级迁移报告、PromptAssemblyTrace
// 留 #115。

/// POST /v1/presets/import — 校验 + 落盘一份 preset JSON。
pub(in crate::daemon) async fn import_preset_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<ImportPresetRequest>,
) -> Result<Json<ImportPresetResponse>, AirpError> {
    let preset_id = PresetId::new(req.preset_id)?;
    // 校验 JSON 形状：必须是 TavernPreset（顶层 prompts[] + 模型参数）。
    // 反序列化失败 → BadRequest，不落盘，避免脏文件残留。
    let cleaned = data_dir::strip_utf8_bom(&req.preset_json).to_owned();
    let parsed: crate::orchestrator::TavernPreset =
        serde_json::from_str(&cleaned).map_err(|e| {
            AirpError::BadRequest(format!("preset_json 不是有效 TavernPreset JSON: {}", e))
        })?;
    // 再序列化为规范 pretty JSON 写盘（保留 raw sidecar，原样不解释 prompt）。
    let bytes = serde_json::to_vec_pretty(&parsed)
        .map_err(|e| AirpError::Internal(format!("preset 序列化失败: {}", e)))?;

    let _guard = PRESET_IMPORT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("preset import lock poisoned");
    let dir = state.data_root.join("presets").join(preset_id.as_str());
    let final_path = dir.join("preset.json");
    let legacy_path = data_dir::legacy_preset_json_path(&state.data_root, preset_id.as_str());
    if final_path.exists() || legacy_path.exists() {
        return Err(AirpError::BadRequest(format!(
            "preset {} already exists; explicit overwrite is not supported",
            preset_id.as_str()
        )));
    }
    fs::create_dir_all(&dir)?;
    data_dir::replace_file(&final_path, &bytes)?;

    Ok(Json(ImportPresetResponse {
        preset_id: preset_id.to_string(),
        prompts_count: parsed.prompts.map(|p| p.len()).unwrap_or(0),
    }))
}

/// POST /v1/presets/import 的请求体。
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::daemon) struct ImportPresetRequest {
    /// 目标 preset ID；走 PresetId::new 校验，拒路径遍历。
    preset_id: String,
    /// TavernPreset 规范的 JSON 文本（原样 sidecar，不解释）。
    preset_json: String,
}

/// POST /v1/presets/import 的响应体。
#[derive(Debug, Serialize)]
pub(in crate::daemon) struct ImportPresetResponse {
    preset_id: String,
    prompts_count: usize,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    // #155 PR5：从 handlers.rs inline tests 原样迁移，测试名和断言不变。
    // helper 在本模块私有复制，不提升为 crate/test 公共 abstraction。
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

    /// #114：合法 TavernPreset 导入应写盘到 presets/{id}/preset.json，且 prompts_count 正确。
    #[tokio::test]
    async fn import_preset_writes_preset_json_and_returns_count() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "preset_id": "myrp",
            "preset_json": r#"{"prompts":[{"identifier":"main","name":"Main","prompt":"hi","enabled":true}]}"#
        });
        let resp = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["preset_id"], "myrp");
        assert_eq!(v["prompts_count"], 1);

        // 写盘：presets/myrp/preset.json 存在且可回解析
        let written = std::fs::read_to_string(
            state
                .data_root
                .join("presets")
                .join("myrp")
                .join("preset.json"),
        )
        .unwrap();
        let back: crate::orchestrator::TavernPreset = serde_json::from_str(&written).unwrap();
        assert_eq!(back.prompts.unwrap().len(), 1);

        let get = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/v1/presets/myrp")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get.status(), axum::http::StatusCode::OK);
        let bytes = axum::body::to_bytes(get.into_body(), usize::MAX)
            .await
            .unwrap();
        let prompts: Vec<crate::orchestrator::TavernPrompt> =
            serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            prompts.len(),
            1,
            "an imported preset must be immediately readable"
        );
    }

    #[tokio::test]
    async fn concurrent_imports_do_not_overwrite_the_same_preset() {
        use axum::body::Body;
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());
        let request = |prompt: &'static str| {
            let body = serde_json::json!({
                "preset_id": "same-id",
                "preset_json": format!(r#"{{"prompts":[{{"identifier":"main","name":"Main","prompt":"{prompt}","enabled":true}}]}}"#)
            });
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/presets/import")
                .header("Content-Type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        };

        let (first, second) = tokio::join!(
            app.clone().oneshot(request("first")),
            app.oneshot(request("second"))
        );
        let statuses = [first.unwrap().status(), second.unwrap().status()];
        assert_eq!(
            statuses.iter().filter(|status| status.is_success()).count(),
            1
        );
        assert_eq!(
            statuses
                .iter()
                .filter(|status| **status == axum::http::StatusCode::BAD_REQUEST)
                .count(),
            1
        );

        let dir = state.data_root.join("presets/same-id");
        let written = std::fs::read_to_string(dir.join("preset.json")).unwrap();
        serde_json::from_str::<crate::orchestrator::TavernPreset>(&written).unwrap();
        assert!(!dir.join("preset.json.tmp").exists());
        assert!(!dir.join("preset.json.bak").exists());
    }

    #[tokio::test]
    async fn import_preset_rejects_legacy_duplicate_without_creating_directory() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let presets = state.data_root.join("presets");
        std::fs::create_dir_all(&presets).unwrap();
        std::fs::write(presets.join("legacy.json"), r#"{"prompts":[]}"#).unwrap();
        let app = crate::daemon::create_router(state.clone());
        let body = serde_json::json!({
            "preset_id": "legacy",
            "preset_json": r#"{"prompts":[]}"#
        });

        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(!presets.join("legacy").exists());
    }

    /// #114：preset_id 路径遍历 → BadRequest，不写盘。
    #[tokio::test]
    async fn import_preset_rejects_traversal_preset_id() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "preset_id": "../evil",
            "preset_json": r#"{"prompts":[]}"#
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        // 关键：无 preset.json 落盘（路径遍历被 PresetId::new 拒，未进写盘分支）
        assert!(
            !state.data_root.join("presets").join("evil").exists(),
            "traversal preset_id must not write any preset file"
        );
    }

    /// #114：preset_json 非 TavernPreset 形状 → BadRequest，不写盘。
    #[tokio::test]
    async fn import_preset_rejects_invalid_preset_json() {
        use tower::util::ServiceExt;

        let (state, _tmp) = make_state_for_http_test();
        let app = crate::daemon::create_router(state.clone());

        let body = serde_json::json!({
            "preset_id": "bad",
            "preset_json": "not json at all"
        });
        let resp = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri("/v1/presets/import")
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(!state.data_root.join("presets").join("bad").exists());
    }
}
