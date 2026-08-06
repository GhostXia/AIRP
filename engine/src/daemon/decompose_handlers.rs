//! Decompose Agent Flow HTTP handlers.
//!
//! 对应计划 `docs/superpowers/plans/2026-07-07-decompose-agent-flow.md` Task 5/6/7。
//!
//! 端点清单：
//! - `POST /v1/characters/:id/decompose` — 拆解角色卡为 MD 骨架
//! - `POST /v1/presets/:id/decompose` — 拆解预设为 MD 骨架
//! - `GET  /v1/characters/:id/analysis` — 列出 analysis 目录的 MD 文件
//! - `GET  /v1/characters/:id/analysis/*filename` — 读单个 analysis MD 文件
//! - `POST /v1/characters/:id/analysis/*filename` — enhance / apply 二合一端点
//!   （axum 0.7 通配符贪婪匹配，无法与 `/*filename/enhance` 子路径共存；
//!   用 body 中的 `action` 字段区分。A1：enhance 只读返回 diff 预览，不写盘。）

use crate::data_dir;
use crate::decompose::{CharacterDecomposer, DecomposeResult, PresetDecomposer};
use crate::domain::AnalysisService;
use crate::error::AirpError;
use crate::orchestrator::card::{TavernCardV2, TavernPreset};
use crate::orchestrator::lorebook::Lorebook;
use crate::types::{CharacterId, PresetId};
use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use super::DaemonState;

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct DecomposeResponse {
    /// B2 修复：对齐 DecomposeResult，用 asset_id + asset_type
    pub asset_id: String,
    pub asset_type: String,
    pub files_written: Vec<String>,
    pub target_dir: String,
    pub lorebook_decomposed: bool,
}

impl From<DecomposeResult> for DecomposeResponse {
    fn from(r: DecomposeResult) -> Self {
        Self {
            asset_id: r.asset_id,
            asset_type: r.asset_type,
            files_written: r.files_written,
            target_dir: r.target_dir,
            lorebook_decomposed: r.lorebook_decomposed,
        }
    }
}

#[derive(Serialize)]
pub struct AnalysisFileList {
    pub asset_id: String,
    pub files: Vec<AnalysisFileEntry>,
}

#[derive(Serialize)]
pub struct AnalysisFileEntry {
    pub filename: String,
    pub size: u64,
}

#[derive(Serialize)]
pub struct AnalysisFileContent {
    pub filename: String,
    pub content: String,
}

/// A1 修复：enhance 端点返回 diff 预览，不写盘
/// #432：新增 `original_md_hash` 供 apply 阶段做 CAS 校验
#[derive(Serialize)]
pub struct EnhancePreview {
    pub filename: String,
    pub original_md: String,
    pub original_md_hash: String,
    pub enhanced_md: String,
    pub has_changes: bool,
}

/// A1 修复：合并 enhance/apply 的请求体
///
/// axum 0.7 通配符贪婪匹配，`/*filename/enhance` 子路径与 `/*filename` 冲突，
/// 因此合并为单 POST 端点，用 `action` 字段区分：
/// - `action: "enhance"` — 只读返回 diff 预览（enhanced_md 由 LLM 真实生成，见 enhance_md_via_llm；issue #92 L3 修复）
/// - `action: "apply"` — 写入 `enhanced_md` 到文件（需用户提供 enhanced_md 字段）
///
/// #432：`expected_hash` 可选，来自 enhance 返回的 `original_md_hash`。
/// 不传则跳过 CAS 校验（向后兼容）。
#[derive(Deserialize)]
pub struct EnhanceApplyRequest {
    pub action: String,
    pub enhanced_md: Option<String>,
    pub expected_hash: Option<String>,
}

// ── Query params ─────────────────────────────────────────────────────────────

/// B4 修复：decompose 支持 ?force=true 覆盖已有 analysis
#[derive(Deserialize)]
pub struct DecomposeQuery {
    pub force: Option<bool>,
}

// ── Handlers ─────────────────────────────────────────────────────────────────

/// `POST /v1/characters/:character_id/decompose`
///
/// 拆解角色卡为 MD 骨架文件。B4 修复：检测已有非空 analysis 目录时支持 ?force=true。
pub(super) async fn decompose_character(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
    Query(query): Query<DecomposeQuery>,
) -> Result<Json<DecomposeResponse>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let exists = data_dir::list_characters(&state.data_root)?
        .into_iter()
        .any(|c| c == cid.as_str());
    if !exists {
        return Err(AirpError::NotFound(format!(
            "character {} does not exist",
            cid
        )));
    }

    let analysis_dir_path = data_dir::char_analysis_dir_path(&state.data_root, cid.as_str())?;

    // B4 修复：检测已有非空 analysis 目录
    let has_existing = analysis_dir_path.exists()
        && std::fs::read_dir(&analysis_dir_path)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
    if has_existing && !query.force.unwrap_or(false) {
        return Err(AirpError::BadRequest(
            "analysis directory already exists and is non-empty; pass ?force=true to overwrite"
                .into(),
        ));
    }

    // 读角色卡
    let json_str = data_dir::read_character_card_text(&state.data_root, &cid)?;
    let card: TavernCardV2 = serde_json::from_str(&json_str)
        .map_err(|e| AirpError::BadRequest(format!("card.json 解析失败: {}", e)))?;

    // 读世界书（可选）
    let lb_path = data_dir::char_world_lorebook_path(&state.data_root, cid.as_str());
    let lorebook: Option<Lorebook> = if lb_path.exists() {
        std::fs::read_to_string(&lb_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
    } else {
        None
    };

    // C3 修复：读 raw.json 提取 creator/character_version/tags
    let raw_meta = {
        let raw_path = state
            .data_root
            .join("characters")
            .join(cid.as_str())
            .join("card")
            .join("raw.json");
        if raw_path.exists() {
            std::fs::read_to_string(&raw_path)
                .ok()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        } else {
            None
        }
    };

    let analysis_dir = data_dir::ensure_char_analysis_dir(&state.data_root, cid.as_str())?;
    let result = CharacterDecomposer::new()
        .decompose(
            cid.as_str(),
            &card,
            lorebook.as_ref(),
            &analysis_dir,
            raw_meta.as_ref(),
        )
        .await?;

    Ok(Json(DecomposeResponse::from(result)))
}

/// `POST /v1/presets/:preset_id/decompose`
pub(super) async fn decompose_preset(
    State(state): State<Arc<DaemonState>>,
    Path(preset_id): Path<String>,
    Query(query): Query<DecomposeQuery>,
) -> Result<Json<DecomposeResponse>, AirpError> {
    let pid = PresetId::new(preset_id)?;
    // 对齐 get_preset_endpoint：优先 normalized 路径 `presets/{id}/preset.json`，
    // 回退 legacy `presets/{id}.json`。import_preset 写入 normalized 路径，
    // 旧 bug 只查 legacy 会导致新导入的 preset 无法 decompose。
    let normalized_path = data_dir::preset_json_path(&state.data_root, pid.as_str());
    let legacy_path = data_dir::legacy_preset_json_path(&state.data_root, pid.as_str());
    let preset_path = if normalized_path.exists() {
        normalized_path
    } else {
        legacy_path
    };
    if !preset_path.exists() {
        return Err(AirpError::NotFound(format!("preset {} not found", pid)));
    }

    let analysis_dir_path = data_dir::preset_analysis_dir_path(&state.data_root, pid.as_str())?;

    // B4 修复：检测已有非空 analysis 目录
    let has_existing = analysis_dir_path.exists()
        && std::fs::read_dir(&analysis_dir_path)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false);
    if has_existing && !query.force.unwrap_or(false) {
        return Err(AirpError::BadRequest(
            "analysis directory already exists and is non-empty; pass ?force=true to overwrite"
                .into(),
        ));
    }

    let json_str = std::fs::read_to_string(&preset_path)?;
    let preset: TavernPreset = serde_json::from_str(&json_str)
        .map_err(|e| AirpError::BadRequest(format!("preset JSON 解析失败: {}", e)))?;

    let analysis_dir = data_dir::ensure_preset_analysis_dir(&state.data_root, pid.as_str())?;
    let result = PresetDecomposer::new()
        .decompose(pid.as_str(), &preset, &analysis_dir)
        .await?;

    Ok(Json(DecomposeResponse::from(result)))
}

/// `GET /v1/characters/:character_id/analysis` — 列出 analysis 目录的 MD 文件
pub(super) async fn list_character_analysis(
    State(state): State<Arc<DaemonState>>,
    Path(character_id): Path<String>,
) -> Result<Json<AnalysisFileList>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    // E1：读端点用 char_analysis_dir_path，不创建目录
    let dir = data_dir::char_analysis_dir_path(&state.data_root, cid.as_str())?;
    let files = list_md_files_recursive(&dir)?;
    Ok(Json(AnalysisFileList {
        asset_id: cid.as_str().to_string(),
        files,
    }))
}

/// `GET /v1/characters/:character_id/analysis/*filename` — 读单个 analysis MD 文件
///
/// E-P1-3 P0：读盘收口至 `AnalysisService::load_file`（`spawn_blocking` 包装）。
/// - 文件不存在时 Service 返回 `NotFound`（与原 `path.exists()` 行为一致）。
/// - `world_book/` 拒绝 + 路径安全校验已移入 Service。
/// - `spawn_blocking`：Service 内部同步 `std::fs`，原代码直接在 async handler
///   调 `std::fs::read_to_string` 阻塞 tokio worker 线程，本 PR 顺手修复（审计 Point 4）。
pub(super) async fn get_character_analysis_file(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, filename)): Path<(String, String)>,
) -> Result<Json<AnalysisFileContent>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let svc = AnalysisService::new(state.data_root.clone());
    let cid_str = cid.as_str().to_string();
    let filename_str = filename.clone();
    let content = tokio::task::spawn_blocking(move || svc.load_file(&cid_str, &filename_str))
        .await
        .map_err(|e| AirpError::Internal(format!("analysis load task failed: {e}")))??;
    Ok(Json(AnalysisFileContent { filename, content }))
}

/// `POST /v1/characters/:character_id/analysis/*filename`
///
/// 合并 enhance / apply 二合一端点（axum 0.7 通配符冲突修复）。
///
/// - `action: "enhance"` — A1：只读返回 diff 预览，不写盘。ToolSideEffect = Readonly。
/// - `action: "apply"`   — A1：写入用户确认的 enhanced_md 到文件。
///
/// E-P1-3 P0：读/写盘收口至 `AnalysisService`（`spawn_blocking` 包装）。
/// - `world_book/` 拒绝 + 路径安全校验已移入 Service，调用方不再重复实现。
/// - `enhance` 分支：`spawn_blocking` + `load_file`（替换原 `std::fs::read_to_string`，
///   顺手修复原 async handler 直接同步 IO 阻塞 worker 线程的问题）。Service 返回
///   `NotFound` 如文件缺失（与原 `path.exists()` 行为一致）。
/// - `apply` 分支：`spawn_blocking` + `save_file`（替换 `tokio::fs::write`）。
///   原子写 + `character_lock` 串行化。原前置 `path.exists()` 校验删除（与
///   `agent/tools/analysis.rs` slice 3 同决策：apply 正常流程跟在 enhance 之后，
///   race 由 `character_lock` 串行化覆盖；删除后 apply 可创建新文件）。
/// - 错误优先级变化：原代码先做 `world_book/` + 路径校验 + 存在性校验，再 match action；
///   现先 match action 再调 Service。仅影响 "无效 action + 异常 filename" 组合，
///   两种都返回 `BadRequest`，无测试覆盖，此处文档化供审计追溯。
pub(super) async fn enhance_or_apply_character_analysis(
    State(state): State<Arc<DaemonState>>,
    Path((character_id, filename)): Path<(String, String)>,
    Json(body): Json<EnhanceApplyRequest>,
) -> Result<Json<serde_json::Value>, AirpError> {
    let cid = CharacterId::new(character_id)?;
    let svc = AnalysisService::new(state.data_root.clone());
    let cid_str = cid.as_str().to_string();

    match body.action.as_str() {
        "enhance" => {
            // spawn_blocking + load_file：读盘 + 存在性校验合一
            let filename_str = filename.clone();
            let svc_clone = svc.clone();
            let cid_for_load = cid_str.clone();
            let original_md = tokio::task::spawn_blocking(move || {
                svc_clone.load_file(&cid_for_load, &filename_str)
            })
            .await
            .map_err(|e| AirpError::Internal(format!("analysis load task failed: {e}")))??;

            // L3 修复（issue #92）：HTTP 端点也真正调 LLM 增强 MD。
            // 与 EnhanceAnalysisTool 同路径：state.config + http_client + call_streaming_api_auto。
            let enhanced_md = enhance_md_via_llm(&state, &original_md, &filename).await?;
            let has_changes = enhanced_md != original_md.trim();
            // #432: 返回 original_md_hash 供 apply 阶段 CAS 校验
            let original_md_hash = AnalysisService::content_hash(&original_md);
            Ok(Json(serde_json::to_value(EnhancePreview {
                filename,
                original_md,
                original_md_hash,
                enhanced_md,
                has_changes,
            })?))
        }
        "apply" => {
            let enhanced_md = body.enhanced_md.ok_or_else(|| {
                AirpError::BadRequest("action=apply requires enhanced_md field".into())
            })?;
            let filename_str = filename.clone();
            let expected_hash = body.expected_hash;
            tokio::task::spawn_blocking(move || {
                svc.save_file(
                    &cid_str,
                    &filename_str,
                    &enhanced_md,
                    expected_hash.as_deref(),
                )
            })
            .await
            .map_err(|e| AirpError::Internal(format!("analysis save task failed: {e}")))??;
            Ok(Json(serde_json::json!({
                "character_id": cid.as_str(),
                "filename": filename,
                "status": "applied",
            })))
        }
        other => Err(AirpError::BadRequest(format!(
            "invalid action: {} (expected 'enhance' or 'apply')",
            other
        ))),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// L3 修复（issue #92）：调 LLM 增强 analysis MD。
///
/// 薄 wrapper 调共享 helper `crate::agent::tools::enhance_md_via_llm_shared`，
/// 防两路径漂移（审计 CR5）。`has_changes` 比较由调用方处理。
async fn enhance_md_via_llm(
    state: &Arc<DaemonState>,
    original_md: &str,
    filename: &str,
) -> Result<String, AirpError> {
    crate::agent::tools::enhance_md_via_llm_shared(state, original_md, filename).await
}

/// 递归列出目录下所有 .md 文件，返回相对路径列表。
fn list_md_files_recursive(dir: &std::path::Path) -> Result<Vec<AnalysisFileEntry>, AirpError> {
    let mut files = Vec::new();
    if !dir.exists() {
        return Ok(files);
    }
    list_md_recursive_inner(dir, dir, &mut files)?;
    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(files)
}

fn list_md_recursive_inner(
    base: &std::path::Path,
    current: &std::path::Path,
    files: &mut Vec<AnalysisFileEntry>,
) -> Result<(), AirpError> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            list_md_recursive_inner(base, &path, files)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let rel = path
                .strip_prefix(base)
                .map_err(|_| AirpError::BadRequest("path escape".into()))?;
            // 用 / 分隔符（跨平台一致）
            let filename = rel.to_string_lossy().replace('\\', "/");
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files.push(AnalysisFileEntry { filename, size });
        }
    }
    Ok(())
}
