//! Preset HTTP handlers — list / get / import.
//!
//! #155 PR5：从 `handlers.rs` 原样迁移。#115 P1 第二阶段：`import_preset_endpoint`
//! 改为走 `normalize_preset` 归一化 + 诊断，落盘 canonical `preset.json` 与原始
//! `raw.json` sidecar，响应体扩展含 `PresetImportReport`。
//!
//! handler 只做 HTTP extraction 与 preset orchestration；`normalize_preset` 在
//! `orchestrator::preset` 模块，`replace_file` 原子写在 `data_dir` 模块。
//!
//! 端点：
//! - `GET  /v1/presets` — 列出所有 preset 文件名
//! - `GET  /v1/presets/:preset_id` — 返回 preset 的 prompts 数组
//! - `POST /v1/presets/import` — 归一化 + 诊断 + 落盘 canonical + raw sidecar

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

// ── Preset import（#114 + #115 P1 第二阶段）──────────────────────────────────
//
// 接收 `{preset_id, preset_json}`，走 `normalize_preset` 归一化 + 诊断，原子写盘到
// `presets/{id}/preset.json`（canonical）与 `presets/{id}/raw.json`（原始 sidecar，
// 无损保留 ST-only 字段如 `prompt_order` / `injection_position` / `probability`）。
// 拒绝脚本执行和路径输入（preset_id 走 PresetId::new 校验，preset_json 走 serde
// 反序列化 serde_json::Value + normalize_preset 守门）。rename/duplicate/export 与
// 跨资产完整 revision/provenance 仍属后续；PromptAssemblyTrace/preview 已由 chat
// pipeline 统一提供，不在本 handler 重复实现。

/// POST /v1/presets/import — 校验 + 落盘一份 preset JSON。
pub(in crate::daemon) async fn import_preset_endpoint(
    axum::extract::State(state): axum::extract::State<Arc<DaemonState>>,
    Json(req): Json<ImportPresetRequest>,
) -> Result<Json<ImportPresetResponse>, AirpError> {
    let preset_id = PresetId::new(req.preset_id)?;
    // 校验 JSON 形状：先解析为宽松 Value，再交给 normalize_preset 做归一化 + 诊断。
    // serde_json::Value 接受任意 JSON；顶层非对象 / prompts 非数组等形状错误由
    // normalize_preset 的 source_error / replacement_error 报告，避免脏文件残留。
    let cleaned = data_dir::strip_utf8_bom(&req.preset_json);
    let source: serde_json::Value = serde_json::from_str(cleaned)
        .map_err(|e| AirpError::BadRequest(format!("preset_json 不是有效 JSON: {}", e)))?;
    let (canonical, report) = crate::orchestrator::normalize_preset(&source);
    if let Some(reason) = report.replacement_error() {
        return Err(AirpError::BadRequest(format!(
            "preset_json 无法作为 TavernPreset 导入: {reason}"
        )));
    }
    // canonical pretty JSON 写盘（runtime 唯一消费形态）。
    let canonical_bytes = serde_json::to_vec_pretty(&canonical)
        .map_err(|e| AirpError::Internal(format!("preset 序列化失败: {}", e)))?;
    // raw sidecar：保存去 BOM 后的原始文本，不丢失格式或重复键。
    let raw_bytes = cleaned.as_bytes();

    let _guard = PRESET_IMPORT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("preset import lock poisoned");
    let dir = state.data_root.join("presets").join(preset_id.as_str());
    let final_path = dir.join("preset.json");
    let raw_path = dir.join("raw.json");
    let legacy_path = data_dir::legacy_preset_json_path(&state.data_root, preset_id.as_str());
    if final_path.exists() || legacy_path.exists() {
        return Err(AirpError::BadRequest(format!(
            "preset {} already exists; explicit overwrite is not supported",
            preset_id.as_str()
        )));
    }
    fs::create_dir_all(&dir)?;
    // canonical preset.json 是发布提交点：raw sidecar 失败时不安装 canonical。
    data_dir::replace_file(&raw_path, raw_bytes)?;
    data_dir::replace_file(&final_path, &canonical_bytes)?;

    Ok(Json(ImportPresetResponse {
        preset_id: preset_id.to_string(),
        prompts_count: report.converted,
        import_report: report,
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
    /// #115 P1：归一化诊断报告（format_version / source_hash / converted /
    /// invalid / needs_review / advisory_preserved 等）。前端可据此展示迁移摘要。
    import_report: crate::orchestrator::PresetImportReport,
}

#[cfg(test)]
mod tests {
    use crate::daemon::tests::make_state_no_key as make_state_for_http_test;

    // #155 PR5：从 handlers.rs inline tests 原样迁移，测试名和断言不变。
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

    // ── #115 P1 第二阶段：normalize_preset 接入后的端到端测试 ──────────────────

    /// 辅助：POST /v1/presets/import 并返回 (status, response_json, data_root, _tmp)。
    /// `_tmp` 必须由调用方持有直到断言完成，否则 `tempfile::TempDir` 会被 drop
    /// 并删除测试数据目录（参考 memory: tempdir early 回收坑）。
    async fn do_import(
        preset_id: &str,
        preset_json: &str,
    ) -> (
        axum::http::StatusCode,
        serde_json::Value,
        std::path::PathBuf,
        tempfile::TempDir,
    ) {
        use tower::util::ServiceExt;

        let (state, tmp) = make_state_for_http_test();
        let data_root = state.data_root.clone();
        let app = crate::daemon::create_router(state.clone());
        let body = serde_json::json!({
            "preset_id": preset_id,
            "preset_json": preset_json,
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
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, v, data_root, tmp)
    }

    /// v2 canonical 源（带 `prompt_order`）应被识别为 v2_canonical，advisory_preserved
    /// ≥ 1，响应含完整 import_report，且 raw sidecar 落盘保留 prompt_order。
    #[tokio::test]
    async fn import_preset_normalizes_v2_canonical_source_and_returns_import_report() {
        let preset_json = serde_json::json!({
            "prompts": [
                {"identifier": "main", "name": "Main", "role": "system", "content": "hi", "enabled": true}
            ],
            "prompt_order": [{"character_id": "main", "order": ["main"]}],
            "temperature": 0.7
        })
        .to_string();
        let (status, v, data_root, _tmp) = do_import("v2src", &preset_json).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(v["preset_id"], "v2src");
        assert_eq!(v["prompts_count"], 1);
        let report = &v["import_report"];
        assert_eq!(report["format_version"], "v2_canonical");
        assert_eq!(report["converted"], 1);
        assert_eq!(report["total_input"], 1);
        assert!(report["invalid"].as_array().unwrap().is_empty());
        assert!(report["needs_review"].as_array().unwrap().is_empty());
        // prompt_order 是 ST-only 顶层字段 → advisory_preserved ≥ 1
        assert!(
            report["advisory_preserved"].as_u64().unwrap() >= 1,
            "advisory_preserved should be >= 1 for v2_canonical source"
        );
        assert_eq!(
            report["top_level_params"],
            serde_json::json!(["temperature"])
        );
        assert_eq!(report["converter_version"], "airp-v1");
        assert_eq!(report["source_hash"].as_str().unwrap().len(), 12);

        // raw sidecar 落盘：raw.json 含原始 prompt_order
        let raw_path = data_root.join("presets").join("v2src").join("raw.json");
        let raw_str = std::fs::read_to_string(&raw_path).unwrap();
        let raw: serde_json::Value = serde_json::from_str(&raw_str).unwrap();
        assert!(
            raw.get("prompt_order").is_some(),
            "raw sidecar must preserve ST-only prompt_order field"
        );
        assert_eq!(raw["temperature"], 0.7);

        // canonical preset.json 不含 prompt_order（runtime 只读 canonical）
        let canon_str =
            std::fs::read_to_string(data_root.join("presets").join("v2src").join("preset.json"))
                .unwrap();
        let canon: serde_json::Value = serde_json::from_str(&canon_str).unwrap();
        assert!(
            canon.get("prompt_order").is_none(),
            "canonical preset.json must not contain ST-only prompt_order"
        );
        assert_eq!(canon["temperature"], 0.7);
    }

    /// raw sidecar 保留去 BOM 后的原始文本，包括空白和重复键。
    #[tokio::test]
    async fn import_preset_preserves_raw_source_text() {
        let preset_json =
            "\u{feff}{\n  \"prompts\": [],\n  \"note\": \"first\",\n  \"note\": \"second\"\n}";
        let (status, _v, data_root, _tmp) = do_import("rawtext", preset_json).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        let raw =
            std::fs::read_to_string(data_root.join("presets").join("rawtext").join("raw.json"))
                .unwrap();
        assert_eq!(raw, crate::data_dir::strip_utf8_bom(preset_json));
    }

    /// 顶层 SillyTavern 别名 `openai_max_tokens` / `openai_model` 应被 serde alias
    /// 归一化到 canonical `max_tokens` / `model`，并报告 aliases_normalized ≥ 1。
    #[tokio::test]
    async fn import_preset_normalizes_top_level_aliases() {
        let preset_json = serde_json::json!({
            "prompts": [
                {"identifier": "p1", "name": "P1", "role": "system", "content": "x", "enabled": true}
            ],
            "openai_max_tokens": 4096,
            "openai_model": "gpt-4o"
        })
        .to_string();
        let (status, v, data_root, _tmp) = do_import("aliases", &preset_json).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        let report = &v["import_report"];
        assert!(
            report["aliases_normalized"].as_u64().unwrap() >= 1,
            "aliases_normalized should be >= 1 for top-level ST aliases"
        );
        assert_eq!(
            report["top_level_params"],
            serde_json::json!(["max_tokens", "model"])
        );

        // canonical preset.json 的 max_tokens / model 字段已被归一化
        let canon_str = std::fs::read_to_string(
            data_root
                .join("presets")
                .join("aliases")
                .join("preset.json"),
        )
        .unwrap();
        let canon: crate::orchestrator::TavernPreset = serde_json::from_str(&canon_str).unwrap();
        assert_eq!(canon.max_tokens, Some(4096));
        assert_eq!(canon.model.as_deref(), Some("gpt-4o"));
    }

    /// 缺 identifier / name 的 prompt 应被跳过并记录到 invalid，不再拒绝整个 preset。
    #[tokio::test]
    async fn import_preset_skips_invalid_prompts_and_reports_them() {
        let preset_json = serde_json::json!({
            "prompts": [
                {"name": "NoId", "role": "system", "content": "x"},
                {"identifier": "ok", "name": "Ok", "role": "system", "content": "y", "enabled": true}
            ]
        })
        .to_string();
        let (status, v, data_root, _tmp) = do_import("mixed", &preset_json).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(v["prompts_count"], 1, "only the valid prompt is converted");
        let report = &v["import_report"];
        assert_eq!(report["total_input"], 2);
        assert_eq!(report["converted"], 1);
        let invalid = report["invalid"].as_array().unwrap();
        assert_eq!(invalid.len(), 1);
        assert_eq!(invalid[0]["index"], 0);
        assert_eq!(invalid[0]["name"], "NoId");
        assert!(
            invalid[0]["reason"]
                .as_str()
                .unwrap()
                .contains("identifier"),
            "invalid reason should mention identifier"
        );

        // canonical preset.json 只含 1 条有效 prompt
        let canon_str =
            std::fs::read_to_string(data_root.join("presets").join("mixed").join("preset.json"))
                .unwrap();
        let canon: crate::orchestrator::TavernPreset = serde_json::from_str(&canon_str).unwrap();
        let prompts = canon.prompts.unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].identifier, "ok");
    }

    /// enabled=false / 空 content 的 prompt 不阻塞写入，但被标记为 needs_review。
    #[tokio::test]
    async fn import_preset_flags_needs_review_without_blocking_write() {
        let preset_json = serde_json::json!({
            "prompts": [
                {"identifier": "disabled", "name": "Disabled", "enabled": false, "role": "system", "content": "x"},
                {"identifier": "empty", "name": "Empty", "enabled": true, "role": "system", "content": "   "}
            ]
        })
        .to_string();
        let (status, v, _data_root, _tmp) = do_import("review", &preset_json).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(v["prompts_count"], 2, "both prompts are converted");
        let report = &v["import_report"];
        let needs_review = report["needs_review"].as_array().unwrap();
        assert_eq!(needs_review.len(), 2);
        // 第一条：enabled=false
        assert_eq!(needs_review[0]["identifier"], "disabled");
        assert!(needs_review[0]["reason"]
            .as_str()
            .unwrap()
            .contains("enabled=false"));
        // 第二条：content 空
        assert_eq!(needs_review[1]["identifier"], "empty");
        assert!(needs_review[1]["reason"]
            .as_str()
            .unwrap()
            .contains("content"));
    }

    /// 顶层非对象（数组 / 数字 / 字符串）→ BadRequest，不写盘。
    #[tokio::test]
    async fn import_preset_rejects_non_object_top_level() {
        let (status, v, data_root, _tmp) = do_import("nonobj", r#"["not", "an", "object"]"#).await;

        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert!(
            v["error"]["message"]
                .as_str()
                .unwrap()
                .contains("无法作为 TavernPreset 导入"),
            "error message should mention rejection, got: {}",
            v["error"]["message"]
        );
        assert!(!data_root.join("presets").join("nonobj").exists());
    }

    /// prompts 字段非数组（null）→ BadRequest（replacement_error），不写盘。
    #[tokio::test]
    async fn import_preset_rejects_non_array_prompts_as_source_error() {
        let (status, _v, data_root, _tmp) = do_import("badprompts", r#"{"prompts": null}"#).await;

        assert_eq!(status, axum::http::StatusCode::BAD_REQUEST);
        assert!(!data_root.join("presets").join("badprompts").exists());
    }

    /// 合法空 prompts 数组：视为显式清空，不拒绝写入。
    #[tokio::test]
    async fn import_preset_accepts_empty_prompts_as_explicit_clear() {
        let (status, v, data_root, _tmp) = do_import("empty", r#"{"prompts": []}"#).await;

        assert_eq!(status, axum::http::StatusCode::OK);
        assert_eq!(v["prompts_count"], 0);
        let report = &v["import_report"];
        assert_eq!(report["converted"], 0);
        assert!(report["source_error"].is_null());
        // preset.json + raw.json 都落盘
        assert!(data_root
            .join("presets")
            .join("empty")
            .join("preset.json")
            .exists());
        assert!(data_root
            .join("presets")
            .join("empty")
            .join("raw.json")
            .exists());
    }
}
